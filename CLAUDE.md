# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

High-performance scientific data acquisition (DAQ) system in Rust, designed as a modular alternative to Python frameworks like PyMoDAQ. Built on async-first architecture with Tokio runtime, egui GUI, and trait-based plugin system for instruments and processors.

## Common Commands

### Development
```bash
# Build and run with hot-reload
cargo watch -x run

# Run with release optimization
cargo run --release

# Run with all features enabled (HDF5, Arrow, VISA)
cargo run --features full
```

### Testing
```bash
# Run all tests
cargo test

# Run specific test with output
cargo test test_name -- --nocapture

# Run tests for specific module
cargo test instrument::

# Run integration tests only
cargo test --test integration_test
```

### Code Quality
```bash
# Format code
cargo fmt

# Check for issues (stricter than build)
cargo clippy

# Build without running
cargo check
```

## Architecture Overview

### Core Traits System (src/core.rs)

The system is built around three primary traits that define plugin interfaces:

- **`Instrument`**: Async trait for hardware communication. All instruments implement `connect()`, `disconnect()`, `data_stream()`, and `handle_command()`. Each instrument runs in its own Tokio task with broadcast channels for data distribution.

- **`DataProcessor`**: Stateful, synchronous trait for real-time signal processing. Processors operate on batches of `DataPoint` slices and return transformed data. Can be chained into pipelines.

- **`StorageWriter`**: Async trait for data persistence. Supports CSV, HDF5, and Arrow formats via feature flags. Implements batched writes with graceful shutdown.

### Application State (src/app.rs)

`DaqApp` is the central orchestrator wrapping `DaqAppInner` in `Arc<Mutex<>>`:

- **Threading Model**: Main thread runs egui GUI, Tokio runtime owns all async tasks
- **Data Flow**: Instrument tasks → broadcast channel (capacity: 1024) → GUI + Storage + Processors
- **Lifecycle**: `new()` spawns all configured instruments → Running → `shutdown()` with 5s timeout per instrument

Key implementation detail: `_data_receiver_keeper` holds broadcast channel open until GUI subscribes, preventing data loss during startup.

### Instrument Registry Pattern (src/instrument/mod.rs)

Factory-based registration system:
```rust
instrument_registry.register("mock", |id| Box::new(MockInstrument::new()));
```

Instruments configured in TOML are spawned automatically in `DaqApp::new()`. Each gets:
- Dedicated Tokio task with `tokio::select!` event loop
- Command channel (mpsc, capacity 32) for parameter updates
- Broadcast sender (shared) for data streaming

### Data Processing Pipeline

When processors are configured for an instrument in TOML:
```toml
[[processors.instrument_id]]
type = "iir_filter"
[processors.instrument_id.config]
cutoff_hz = 10.0
```

Data flows: Instrument → Processor Chain → Broadcast. Processors are created via `ProcessorRegistry` during instrument spawn (src/app.rs:704-718).

### Measurement Enum Architecture

The `Measurement` enum (src/core.rs:229-276) supports multiple data types:
- `Scalar(DataPoint)` - Traditional scalar measurements
- `Spectrum(SpectrumData)` - FFT/frequency analysis output
- `Image(ImageData)` - 2D camera/sensor data

Migration from scalar-only `DataPoint` to strongly-typed `Measurement` variants is in progress. See docs/adr/001-measurement-enum-architecture.md for design rationale.

## Key Files and Responsibilities

- **src/core.rs**: Trait definitions, `DataPoint`/`Measurement` types, `InstrumentCommand` enum
- **src/app.rs**: `DaqApp`/`DaqAppInner`, instrument lifecycle, storage control
- **src/error.rs**: `DaqError` enum with `thiserror` variants
- **src/config.rs**: TOML configuration loading and validation
- **src/instrument/**: Concrete instrument implementations (mock, ESP300, Newport 1830C, MaiTai, SCPI, VISA)
- **src/data/**: Storage writers, processors (FFT, IIR, trigger), processor registry
- **src/gui/**: egui implementation with docking layout

## Configuration System

Hierarchical TOML configuration (config/default.toml):

```toml
[application]
name = "Rust DAQ"

[[instruments.my_instrument]]
type = "mock"  # Must match registry key
[instruments.my_instrument.params]
channel_count = 4

[[processors.my_instrument]]
type = "iir_filter"
[processors.my_instrument.config]
cutoff_hz = 10.0

[storage]
default_format = "csv"  # or "hdf5", "arrow"
default_path = "./data"
```

Processors are optional per-instrument. Missing processor config means raw data flows directly to broadcast channel.

## Feature Flags

```toml
default = ["storage_csv", "instrument_serial"]
full = ["storage_csv", "storage_hdf5", "storage_arrow", "instrument_serial", "instrument_visa"]
```

Use `cargo build --features full` to enable all backends. HDF5 requires system library (macOS: `brew install hdf5`).

## Error Handling Patterns

1. **Instrument failures are isolated**: One instrument crash doesn't terminate the app
2. **Graceful shutdown with timeout**: 5-second timeout per instrument before force abort
3. **Storage errors abort recording**: But don't stop data acquisition
4. **Command send failures**: Indicate terminated instrument task (logged, task aborted)

## Async Patterns

Instruments use `tokio::select!` for concurrent operations:
```rust
loop {
    tokio::select! {
        data = stream.recv() => { /* process and broadcast */ }
        cmd = command_rx.recv() => { if Shutdown => break; }
        _ = sleep(1s) => { /* idle timeout */ }
    }
}
disconnect() // Called after loop breaks for cleanup
```

Shutdown command breaks the loop, then `disconnect()` is called outside for guaranteed cleanup (bd-20).

## Testing Infrastructure

- **Unit tests**: In-module `#[cfg(test)]` blocks
- **Integration tests**: tests/*.rs (integration_test, storage_shutdown_test, measurement_enum_test)
- **Mock instruments**: src/instrument/mock.rs for testing without hardware
- **Test helpers**: Use `tempfile` crate for temporary storage, `serial_test` for shared resource tests

## Multi-Agent Coordination

This workspace supports concurrent agent work:
- Obtain unique `git worktree` before editing to avoid overlapping changes
- Set `BEADS_DB=.beads/daq.db` to use project-local issue tracker
- Finish with `cargo check && git status -sb` to verify state
- See AGENTS.md and BD_JULES_INTEGRATION.md for detailed workflow

## Beads Issue Tracker Integration

This project uses **[beads](https://github.com/steveyegge/beads)** as an AI-friendly issue tracker and agentic memory system. Beads provides dependency tracking, ready work detection, and git-based distribution that's purpose-built for AI coding agents.

### Why Beads?

- ✨ **Zero setup** - `bd init` creates project-local database in `.beads/`
- 🔗 **Dependency tracking** - Four dependency types (blocks, related, parent-child, discovered-from)
- 📋 **Ready work detection** - `bd ready` finds issues with no open blockers
- 🤖 **Agent-friendly** - All commands support `--json` for programmatic use
- 📦 **Git-versioned** - JSONL records in `.beads/issues.jsonl` sync across machines
- 🌍 **Distributed by design** - Multiple agents share one logical database via git
- 💾 **Full audit trail** - Every change is logged with actor tracking

### Installation

Beads requires both the `bd` CLI tool and the `beads-mcp` MCP server for Claude Code integration.

**📋 For detailed installation instructions, troubleshooting, and environment-specific guidance, see [BEADS_INSTALLATION.md](BEADS_INSTALLATION.md)**

#### Option 1: Quick Install (Recommended)

```bash
# Install bd CLI
curl -fsSL https://raw.githubusercontent.com/steveyegge/beads/main/scripts/install.sh | bash

# Verify installation
bd version

# Initialize beads in this project
cd /path/to/rust-daq
bd init --prefix daq
```

#### Option 2: Using Go

```bash
# Install bd CLI using Go (requires Go 1.24+)
go install github.com/steveyegge/beads/cmd/bd@latest

# Add to PATH if needed
export PATH="$PATH:$(go env GOPATH)/bin"

# Initialize in project
cd /path/to/rust-daq
bd init --prefix daq
```

#### Option 3: Homebrew (macOS/Linux)

```bash
brew tap steveyegge/beads
brew install bd
bd init --prefix daq
```

### MCP Server Installation (For Claude Code)

The MCP server enables Claude Code to use beads directly via MCP tools:

```bash
# Install beads-mcp using pip
pip install beads-mcp

# Or using uv (recommended)
uv tool install beads-mcp
```

**Note:** The MCP server requires the `bd` CLI to be installed first (see above).

The MCP server will be automatically discovered by Claude Code. No additional configuration is needed.

### Environment Variables

Set these in your shell or Claude Code configuration:

```bash
# Point to project-local database
export BEADS_DB=.beads/daq.db

# Set actor name for audit trail (defaults to $USER)
export BD_ACTOR="claude-agent"

# Enable debug logging (optional)
export BD_DEBUG=1
```

### Basic Workflow

```bash
# Find ready work (no blockers)
bd ready --json

# Create issues during work
bd create "Add spectrum visualization module" -t feature -p 1

# Link discovered work back to parent
bd dep add <new-id> <parent-id> --type discovered-from

# Update status
bd update <issue-id> --status in_progress

# Complete work
bd close <issue-id> --reason "Implemented with tests"
```

### Dependency Types

- **blocks**: Hard blocker (affects ready work calculation)
- **related**: Soft relationship (context only)
- **parent-child**: Epic/subtask hierarchy
- **discovered-from**: Issues discovered while working on another issue

Only `blocks` dependencies affect the ready work queue.

### Git Integration

Beads automatically syncs between SQLite (`.beads/daq.db`) and JSONL (`.beads/issues.jsonl`):

```bash
# After creating/updating issues
bd create "Fix instrument timeout" -p 0
# Changes auto-export to .beads/issues.jsonl after 5 seconds

# Commit and push
git add .beads/issues.jsonl
git commit -m "Add timeout fix issue"
git push

# On another machine after git pull
git pull
bd ready  # Automatically imports from JSONL if newer
```

### Daemon Mode (Optional)

For continuous syncing across multiple terminals/agents:

```bash
# Start global daemon (serves all beads projects)
bd daemon --global --auto-commit --auto-push

# Check daemon status
bd daemon --status

# Stop daemon
bd daemon --stop
```

The daemon:
- Auto-exports changes to JSONL
- Auto-commits and pushes (with flags)
- Auto-imports after git pull
- Serves all beads projects system-wide

### Multi-Repository Setup

When working on multiple rust-daq components or related projects:

```bash
# Each project gets its own database
cd ~/rust-daq && bd init --prefix daq
cd ~/rust-daq-gui && bd init --prefix gui
cd ~/rust-daq-python && bd init --prefix py

# Use global daemon to serve all projects
bd daemon --global

# View work across all repos
bd repos ready --group
bd repos stats
```

### Troubleshooting

**`bd: command not found`**
```bash
# Check installation
which bd

# Add Go bin to PATH
export PATH="$PATH:$(go env GOPATH)/bin"
```

**`database is locked`**
```bash
# Find and kill hanging processes
ps aux | grep bd
kill <pid>

# Remove lock files if no bd processes running
rm .beads/*.db-journal .beads/*.db-wal .beads/*.db-shm
```

**Git merge conflict in `issues.jsonl`**
```bash
# Usually safe to keep both sides
# Each line is independent unless IDs conflict
# Resolve manually, then:
git add .beads/issues.jsonl
git commit
bd import -i .beads/issues.jsonl
```

### Advanced Features

- **Compaction**: `bd compact --all` - AI-powered semantic compression of old closed issues
- **Labels**: `bd label add <id> performance,backend` - Flexible metadata for filtering
- **Bulk operations**: `bd delete --from-file deletions.txt --force` - Batch deletions
- **Cross-references**: Issues automatically link to each other in descriptions/notes
- **Prefix rename**: `bd rename-prefix kw-` - Change issue prefix for all issues

### Resources

- **Main docs**: https://github.com/steveyegge/beads
- **Quick start**: `bd quickstart` (interactive guide)
- **Agent integration**: See beads AGENTS.md for AI workflow patterns
- **MCP server docs**: https://github.com/steveyegge/beads/tree/main/integrations/beads-mcp

## Recent Architectural Changes

- **bd-25 (Error Handling)**: Enhanced `DaqError` with context-rich variants, improved storage writer error propagation
- **bd-22 (GUI Batching)**: Optimized data dispatch to prevent GUI lag
- **bd-20/21 (Graceful Shutdown)**: Added `InstrumentCommand::Shutdown`, async serial I/O, 5s timeout with fallback abort
- **Measurement Enum**: Introduced `Measurement` enum to replace JSON metadata workarounds for non-scalar data
