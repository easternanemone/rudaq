# CLAUDE.md

**Note**: This project uses [bd (beads)](https://github.com/steveyegge/beads) for issue tracking. Use `bd` commands instead of markdown TODOs. See AGENTS.md for workflow details.

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

# AI-powered code analysis (recurse.ml)
~/.rml/rml/rml                    # Analyze all unstaged changes
~/.rml/rml/rml path/to/file.rs   # Analyze specific file
~/.rml/rml/rml src/              # Analyze directory
```

### Recurse.ml Integration

**recurse.ml (rml)** is an AI-powered code analysis tool that detects issues during development.

**Setup**: rml is installed at `~/.rml/rml/rml`. Add to PATH for convenience:
```bash
export PATH="$HOME/.rml/rml:$PATH"
```

**Usage Workflow**:
1. **Automatic analysis**: rml analyzes unstaged changes matching `git diff`
2. **Early detection**: Catches bugs, race conditions, and logic errors before commit
3. **Custom rules**: Can enforce project-specific conventions and standards
4. **Diff output**: Provides suggested fixes in patch format

**When to use rml**:
- Before committing changes (`rml` to check all unstaged files)
- After implementing complex logic (e.g., async code, state management)
- When reviewing AI-generated code changes
- To verify refactoring didn't introduce issues

**Example**:
```bash
# After making changes to PVCAM V2 adapter
git status -s  # See unstaged changes
~/.rml/rml/rml src/instrument/v2_adapter.rs  # Analyze specific file
~/.rml/rml/rml  # Or analyze all unstaged changes
```

**Note**: First run requires GitHub authentication via device code flow.

### ast-grep Integration

This project uses `ast-grep` to enforce coding standards and help with migrations. A set of project-specific rules is defined in `rust_daq_ast_grep_rules.yml`.

**Installation:**
```bash
# macOS/Linux
curl -L https://github.com/ast-grep/ast-grep/releases/latest/download/ast-grep-$(uname -m)-$(uname -s | tr '[:upper:]' '[:lower:]').zip -o ast-grep.zip
unzip ast-grep.zip
chmod +x ast-grep
sudo mv ast-grep /usr/local/bin/

# Or using Homebrew (macOS)
brew install ast-grep

# Verify installation
ast-grep --version
```

**Pre-Commit Hook Setup:**

The pre-commit hook automatically runs ast-grep on staged Rust files before each commit. It blocks commits with ERROR severity violations but allows warnings and hints.

```bash
# The hook is already installed at .git/hooks/pre-commit
# Verify it's executable:
ls -l .git/hooks/pre-commit

# If not executable, run:
chmod +x .git/hooks/pre-commit
```

**Usage Workflow:**

1. **During Development** - Run ast-grep on files you're working on:
   ```bash
   # Check specific file
   ast-grep scan --config rust_daq_ast_grep_rules.yml path/to/file.rs
   
   # Check all Rust files
   ast-grep scan --config rust_daq_ast_grep_rules.yml
   
   # Get JSON output for scripting
   ast-grep scan --config rust_daq_ast_grep_rules.yml --json
   ```

2. **Before Commit** - The pre-commit hook runs automatically:
   ```bash
   git add src/gui/mod.rs
   git commit -m "Fix blocking GUI call"
   # Pre-commit hook runs ast-grep on staged files
   # Commit proceeds if no ERROR violations
   ```

3. **Bypass Hook** (emergency only, not recommended):
   ```bash
   git commit --no-verify -m "Emergency fix"
   ```

4. **CI/CD Pipeline** - GitHub Actions runs ast-grep on all files:
   - Fails build on ERROR severity violations
   - Reports warnings and hints (informational only)
   - Uploads detailed results as artifacts

**Understanding Severity Levels:**

- **ERROR**: Blocks commits and CI builds. Must be fixed before merging.
  - Example: Blocking calls in GUI code (`blocking_send`/`blocking_recv`)
  
- **WARNING**: Informational. Doesn't block commits but should be reviewed.
  - Example: Use of `.unwrap()` or `.expect()` outside tests
  - Example: Hardcoded device paths or timeouts
  
- **HINT**: Suggestions for improvement. Purely informational.
  - Example: Redundant else blocks after return
  - Example: Manual shutdown logic that could use helpers

**Current Rules** (18 total, 15 active):

1. `find-blocking-gui-calls` (ERROR) - Detects blocking async calls in GUI code
2. `no-unwrap-expect` (WARNING) - Flags `.unwrap()` and `.expect()` outside tests
3. `no-hardcoded-device-paths` (WARNING) - Detects hardcoded `/dev/ttyUSB*` paths
4. `no-hardcoded-timeouts` (WARNING) - Flags hardcoded `Duration::from_secs()`
5. `incomplete-implementation` (INFO) - Finds "Real implementation would..." comments
6. `use-specific-errors` (HINT) - Suggests `DaqError` over `anyhow!` string literals
7. `no-debug-macros` (WARNING) - Prevents `println!` and `dbg!` in production
8. `use-v2-instrument-trait` (WARNING) - Discourages V1 `Instrument` trait usage
9. `use-v2-measurement` (WARNING) - Flags deprecated `InstrumentMeasurement`
10. `no-v2-adapter` (INFO) - Identifies temporary `V2InstrumentAdapter` usage
11. `use-daq-core-result` (HINT) - Prefers `daq_core::Result` over `anyhow::Result`
12. `incomplete-migration-todo` (WARNING) - Finds V2/V3 migration TODOs
13. `v1-feature-flag` (INFO) - Identifies V1 feature-gated code
14. `redundant-else` (HINT) - Detects redundant else after return
15. `manual-shutdown-logic` (HINT) - Suggests using `shutdown_task` helper
16. `string-to-string` (HINT) - Reviews `.to_string()` on literals
17. `unnecessary-to-owned` (HINT) - Reviews `.to_owned()` usage
18. `std-thread-sleep-in-async` (DISABLED) - Handled by clippy instead

**Troubleshooting:**

- **Hook not running**: Verify `.git/hooks/pre-commit` is executable
- **jq not found**: Install jq for detailed violation reports (`brew install jq`)
- **ast-grep not found**: Install ast-grep (see Installation above)
- **False positives**: Review rule in `rust_daq_ast_grep_rules.yml`, adjust severity or add to `ignores`

**MCP Server (`ast-grep-mcp`)**

For AI agents, the `ast-grep-mcp` server provides a powerful way to interact with the codebase using `ast-grep`.

**Setup:**

1.  **Install `ast-grep-mcp`:**
    ```bash
    pip install ast-grep-mcp
    ```

2.  **Add to MCP config:**
    Add the following to your MCP configuration file (e.g., `~/.config/claude/config.json`):
    ```json
    {
      "ast-grep": {
        "command": "ast-grep-mcp",
        "args": []
      }
    }
    ```

**Usage:**

You can use the `mcp__ast_grep__*` functions to run the rules. For example, to run all rules in the project:

```
mcp__ast_grep__run --config rust_daq_ast_grep_rules.yml
```

To run a specific rule:

```
mcp__ast_grep__run --config rust_daq_ast_grep_rules.yml --rule-id no-unwrap-expect
```

The output will be in JSON format, which can be easily parsed by AI agents.

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

#### V1 vs V2 Instrument Architecture

**V1 Instruments** (Legacy):
- Broadcast `DataPoint` via `InstrumentMeasurement`
- App converts DataPoints to `Measurement::Scalar` before GUI broadcast
- **Cannot broadcast Image or Spectrum data natively**
- Examples: Most instruments in `src/instrument/` including PVCAM V1

**V2 Instruments** (New):
- Broadcast `Measurement` enum directly via `measurement_channel()`
- Full support for Scalar, Spectrum, and Image data types
- Native `PixelBuffer` support for memory-efficient camera data
- Examples: `src/instruments_v2/pvcam.rs`

**Important**: Image viewing in the GUI requires V2 instruments. V1 instruments like PVCAM V1 only broadcast frame statistics as scalars. V2 integration is planned for Phase 3 (bd-51).

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

### Timeout Configuration

Global runtime timeouts now live under `[application.timeouts]` and map 1:1 to the `TimeoutSettings` struct (`src/config.rs`). Each field stores milliseconds, defaults to the legacy hardcoded values, and is validated at load time:

```toml
[application.timeouts]
serial_read_timeout_ms = 1000      # Serial adapter read/write
serial_write_timeout_ms = 1000
scpi_command_timeout_ms = 2000     # SCPI helpers + MaiTai/ESP300
network_client_timeout_ms = 5000   # Server actor request/response
network_cleanup_timeout_ms = 10000 # Session cleanup interval
instrument_connect_timeout_ms = 5000
instrument_shutdown_timeout_ms = 6000
instrument_measurement_timeout_ms = 5000
```

Boundaries are enforced (serial 100–30 000 ms, SCPI 500–60 000 ms, network 1 000–120 000 ms, lifecycle 1 000–60 000 ms). Missing sections are filled via `#[serde(default)]`, so older configs continue to work. These values now drive all timeout-sensitive code paths (serial adapter builders, SCPI transport helpers, instrument manager connect/shutdown, MaiTai/ESP300 drivers, experiment primitives, and the network server actor) via `settings.application.timeouts.*`.

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
- **GUI verification**: Screenshot capability for visual testing (F12 key or programmatic API)
  - Screenshots saved to `screenshots/` directory with timestamp
  - Verification scripts in `jules-scratch/verification/` for agent workflows
  - See `src/gui/verification.rs` for programmatic testing support

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

# Workflow Instruction

You are a coding agent focused on one codebase. Use the brv CLI to manage working context.
Core Rules:

- Start from memory. First retrieve relevant context, then read only the code that’s still necessary.
- Keep a local playbook. playbook.json is your local memory store—update it with what you learn.

## user-centric

Before running any `brv` command, ask the user to choose:

1. **Accept** - Review the command, then run if approved
2. **Auto-run** - Execute immediately (remember this preference for the session)
3. **No** - Cancel the command and continue without alternative
4. **Custom** - Skip the command and provide different instruction

Once the user chooses "Auto-run", apply that preference to all subsequent `brv` commands in this conversation without asking again.

## Daily Flow

1. Always attempt to retrieve relevant context from ByteRover first by running `brv retrieve`.

2. Do the work with minimal code reads

3. Record new or refined knowledge in the playbook

    - Add only specific, actionable, contextual items; avoid secrets. Redact when in doubt.

4. Sync policy (manual by default)

    - Do not auto-push.
    - When the playbook has meaningful updates, prompt the user to run the command
    - If the user has explicitly allowed auto-push, proceed non-interactively by running `brv push -y`.

## Playbook Guideline

- Be specific (“Use React Query for data fetching in web modules”).
- Be actionable (clear instruction a future agent/dev can apply).
- Be contextual (mention module/service, constraints, links to source).
- Include source (file + lines or commit) when possible.

## CLI Usage Notes

- Use --help on any command to discover flags. Provide exact arguments for the scenario.

---
# ByteRover CLI Command Reference

## Memory Commands

### `brv add`

**Description:** Add or update a bullet in the playbook (bypasses ACE workflow for direct agent usage)

**Flags:**

- `-s, --section <string>`: Section name for the bullet (required)
- `-c, --content <string>`: Content of the bullet (required)
- `-b, --bullet-id <string>`: Bullet ID to update (optional, creates new if omitted)

**Examples:**

```bash
brv add --section "Common Errors" --content "Authentication fails when token expires"
brv add --section "Common Errors" --bullet-id "common-00001" --content "Updated: Auth fails when token expires"
brv add -s "Best Practices" -c "Always validate user input before processing"
```

**Suggested Sections:** Common Errors, Best Practices, Strategies, Lessons Learned, Project Structure and Dependencies, Testing, Code Style and Quality, Styling and Design

**Behavior:**

- Warns if using non-standard section name
- Creates new bullet with auto-generated ID if `--bullet-id` not provided
- Updates existing bullet if `--bullet-id` matches existing bullet
- Displays bullet ID, section, content, and tags after operation

**Requirements:** Playbook must exist (run `brv init` first)

---

### `brv retrieve`

**Description:** Retrieve memories from ByteRover Memora service and save to local ACE playbook

**Flags:**

- `-q, --query <string>`: Search query string (required)
- `-n, --node-keys <string>`: Comma-separated list of node keys (file paths) to filter results

**Examples:**

```bash
brv retrieve --query "authentication best practices"
brv retrieve -q "error handling" -n "src/auth/login.ts,src/auth/oauth.ts"
brv retrieve -q "database connection issues"
```

**Behavior:**

- **Clears existing playbook first** (destructive operation)
- Retrieves memories and related memories from Memora service
- Combines both result sets into playbook
- Maps memory fields: `bulletId` → `id`, `tags` → `metadata.tags`, `nodeKeys` → `metadata.relatedFiles`
- Displays results with score, content preview (200 chars), and related file paths
- Fail-safe: warns on save error but still displays results

**Output:** Shows count of memories and related memories, displays each with score and content

**Requirements:** Must be authenticated and project initialized

---

### `brv push`

**Description:** Push playbook to ByteRover memory storage and clean up local ACE files

**Flags:**

- `-b, --branch <string>`: ByteRover branch name (default: "main", NOT git branch)
- `-y, --yes`: Skip confirmation prompt

**Examples:**

```bash
brv push
brv push --branch develop
```

---

### `brv complete`

**Description:** Complete ACE workflow: save executor output, generate reflection, and update playbook in one command

**Arguments:**

- `hint`: Short hint for naming output files (e.g., "user-auth", "bug-fix")
- `reasoning`: Detailed reasoning and approach for completing the task
- `finalAnswer`: The final answer/solution to the task

**Flags:**

- `-t, --tool-usage <string>`: Comma-separated list of tool calls with arguments (format: "ToolName:argument", required)
- `-f, --feedback <string>`: Environment feedback about task execution (e.g., "Tests passed", "Build failed", required)
- `-b, --bullet-ids <string>`: Comma-separated list of playbook bullet IDs referenced (optional)
- `-u, --update-bullet <string>`: Bullet ID to update with new knowledge (if not provided, adds new bullet)

**Examples:**

```bash
brv complete "user-auth" "Implemented OAuth2 flow" "Auth works" --tool-usage "Read:src/auth.ts,Edit:src/auth.ts,Bash:npm test" --feedback "All tests passed"
brv complete "validation-fix" "Analyzed validator" "Fixed bug" --tool-usage "Grep:pattern:\"validate\",Read:src/validator.ts" --bullet-ids "bullet-123" --feedback "Tests passed"
brv complete "auth-update" "Improved error handling" "Better errors" --tool-usage "Edit:src/auth.ts" --feedback "Tests passed" --update-bullet "bullet-5"
```

**Behavior:**

- **Phase 1 (Executor):** Saves executor output with hint, reasoning, answer, tool usage, and bullet IDs
- **Phase 2 (Reflector):** Auto-generates reflection based on feedback and applies tags to playbook
- **Phase 3 (Curator):** Creates delta operation (ADD or UPDATE) and applies to playbook
- Adds new bullet to "Lessons Learned" section with tag `['auto-generated']`
- If `--update-bullet` provided, updates existing bullet instead of adding new one
- Extracts file paths from tool usage and adds to bullet metadata as `relatedFiles`

**Output:** Shows summary with file paths, tags applied count, and delta operations breakdown

---

### `brv status`

**Description**: Show CLI status and project information. Display local ACE context (ACE playbook) managed by ByteRover CLI.

**Arguments:**

- `DIRECTORY`:Project directory (defaults to current directory).

**Flags:**

- `-f, --format=<option>`: [default: table] Output format. <options: table|json>

**Examples:**

```bash
brv status
brv status --format json
```

## Multi-Agent Orchestration (Orchestrator Responsibilities)

**CRITICAL**: Claude Code serves as the orchestrator for all AI agents (Gemini, Codex, Jules). You are responsible for coordinating work, maintaining beads tracker coherence, and ensuring all progress is properly documented.

### Three-Tier Agent Architecture

1. **Claude Code (Orchestrator - YOU)**:
   - Coordinate all agents via `mcp__zen__clink` (Gemini/Codex) and `jules new` (Jules workers)
   - Manage beads tracker - **YOU** are responsible for syncing all progress
   - Monitor Jules sessions, PR status, background batch completion
   - Record learnings in ByteRover after each agent completes work
   - Commit documentation updates with beads sync

2. **Gemini (Strategic Advisor)**:
   - Provides architectural guidance via `mcp__zen__clink`
   - Unblocks stuck Jules sessions with reference implementations
   - Helps identify parallelizable work for delegation
   - Does NOT create sessions, update beads, or make code changes

3. **Codex (Deep Implementer)**:
   - Handles complex features requiring 1000+ lines via `mcp__zen__clink`
   - Full codebase analysis and large-scale refactoring
   - Does NOT update beads - YOU sync their work to beads

4. **Jules (Parallel Workers)**:
   - Execute 50-100 parallelizable tasks simultaneously
   - Daily quota: 100 sessions, target 60-80 for optimal coverage
   - Do NOT update beads properly - YOU must sync all completions

### Orchestrator Workflow Pattern

**Phase 1: Delegation**
```bash
# 1. Check beads for ready work
BEADS_DB=.beads/daq.db bd ready --json

# 2. Delegate complex work to Gemini/Codex
mcp__zen__clink("Gemini", "Analyze Phase 3 V3 integration priorities and create delegation plan for Jules")
mcp__zen__clink("Codex", "Implement Newport 1830C V3 driver following reference pattern")

# 3. Create Jules sessions (directly or via Gemini)
jules new --repo TheFermiSea/rust-daq "bd-197: Implement ESP300 V3 with MotionController trait"
sleep 8  # CRITICAL: 8-10s delays to avoid rate limits
```

**Phase 2: Monitoring**
```bash
# Check Jules session status
jules remote list --session

# Check PR status
gh pr list --repo TheFermiSea/rust-daq --json number,title,state,statusCheckRollup

# Monitor background batches
# Use BashOutput tool to check batch progress

# Check Gemini/Codex progress via clink when needed
```

**Phase 3: Beads Sync (CRITICAL - YOUR RESPONSIBILITY)**
```bash
# For each completed Jules session:
BEADS_DB=.beads/daq.db bd update bd-197 --status closed --reason "Completed by Jules session 10407563664786836449. PR #58 submitted."

# For Codex work:
BEADS_DB=.beads/daq.db bd update bd-108 --status closed --reason "Completed by Codex. Newport 1830C V3 implemented (1,067 lines)"

# ALWAYS commit beads with code changes
git add .beads/issues.jsonl src/
git commit -m "feat: implement Newport 1830C V3 driver (bd-108)

- 1,067 line implementation following V3 pattern
- 6 unit tests with MockSerialDevice
- Wired into InstrumentManagerV3
- Config enabled in default.toml

bd-108: closed by Codex delegation"
```

**Phase 4: ByteRover Recording**
```bash
# Record learnings from all agent work
brv add -s "Lessons Learned" -c "Jules session 10407563664786836449 completed bd-197 ESP300 V3. Added MotionController trait to src/core_v3.rs:501-523"

brv add -s "Architecture" -c "src/instruments_v2/newport_1830c_v3.rs - Complete V3 reference implementation (1,067 lines) by Codex. SerialDevice abstraction enables mock testing"

brv push -y  # Share with all agents
```

### Jules Parallelization Management

**Capacity Planning:**
- Daily quota: 100 Jules sessions
- Target utilization: 60-80 sessions
- Success rate: Aim for >70% completion
- Current status: `jules remote list --session | wc -l`

**Rate Limit Management:**
When creating multiple Jules sessions:
```bash
# Option 1: Sequential with delays
for task in "task 1" "task 2" "task 3"; do
    jules new --repo TheFermiSea/rust-daq "$task"
    sleep 8  # CRITICAL: prevents 429 errors
done

# Option 2: Background batch (for 15+ tasks)
Bash("
for task in <15 tasks>; do
    jules new --repo TheFermiSea/rust-daq \"\$task\"
    sleep 10
done
", run_in_background=true)

# Monitor batches via BashOutput tool
```

**Session State Monitoring:**
```bash
# Check all sessions
jules remote list --session

# States to watch for:
# - Completed ✅: Update beads, record in ByteRover
# - In Progress ⏳: Monitor, no action
# - Planning (stuck) ⚠️: Use clink with Gemini for context
# - Failed ❌: Analyze error, re-delegate with clearer requirements
```

**Unblocking Stuck Sessions:**
When Jules sessions stuck in "Planning":
```bash
# 1. Get Gemini's guidance
mcp__zen__clink("Gemini", "Jules session <id> stuck on bd-XXX. What context does it need?")

# 2. Gemini will provide:
# - Reference implementation file paths
# - Architectural pattern to follow
# - Test strategy
# - Integration points

# 3. Session auto-progresses with provided context
# Or manually provide guidance if needed
```

**PR Review Process:**
```bash
# 1. List Jules PRs
gh pr list --repo TheFermiSea/rust-daq --author "app/jules-preview[bot]"

# 2. Check CI status
gh pr view <pr-number> --json statusCheckRollup

# 3. For failing CI:
gh pr comment <pr-number> --body "@jules CI is failing. Please rebase on latest main, fix lint errors, and ensure all tests pass. Reference: src/instruments_v2/newport_1830c_v3.rs for V3 pattern."

# 4. For passing CI:
gh pr merge <pr-number> --squash
BEADS_DB=.beads/daq.db bd update bd-XXX --status closed --reason "Merged PR #<number>"
```

### Beads Tracker Coherence (CRITICAL)

**YOU are responsible** for keeping beads tracker in sync. Subagents (Gemini, Codex, Jules) do NOT properly update beads.

**Pattern for Every Completed Task:**
```bash
# 1. Agent completes work
jules pull <session-id>  # or clink result from Gemini/Codex

# 2. YOU update beads
BEADS_DB=.beads/daq.db bd update bd-XXX --status closed --reason "Completed by <agent> <session-id>"

# 3. YOU record in ByteRover
brv add -s "Lessons Learned" -c "<agent> completed bd-XXX. Key implementation: <file:line details>"

# 4. YOU commit together
git add .beads/issues.jsonl <modified-files>
git commit -m "feat: <description> (bd-XXX)

<commit message>

bd-XXX: closed by <agent>"
```

**Batch Beads Updates:**
When 10+ Jules sessions complete simultaneously:
```bash
# Get all completed sessions
jules remote list --session | grep Completed > completed_sessions.txt

# Update beads for each
while read session_id; do
    # Extract bd ID from session (you may need to map this)
    bd_id="bd-XXX"  # Determine from task description
    BEADS_DB=.beads/daq.db bd update "$bd_id" --status closed --reason "Completed by Jules $session_id"
done < completed_sessions.txt

# Commit all at once
git add .beads/issues.jsonl
git commit -m "chore: sync beads with 10+ completed Jules sessions"
```

### Communication with Agents

**Using mcp__zen__clink:**
```bash
# Gemini for strategic guidance
mcp__zen__clink("Gemini", "What are the priorities for Phase 3 V3 integration? Which tasks can be parallelized to Jules?")

# Codex for deep implementation
mcp__zen__clink("Codex", "Implement Newport 1830C V3 driver. Reference: src/core_v3.rs traits. Follow pattern: SerialDevice abstraction, mock tests, InstrumentManagerV3 registration")

# Request context for stuck Jules sessions
mcp__zen__clink("Gemini", "Jules session 12345 stuck on bd-128 dynamic config. What architectural context does it need?")
```

**What to Delegate:**
- **Gemini**: Architecture decisions, unblocking guidance, delegation planning, capacity recommendations
- **Codex**: 1000+ line implementations, deep refactoring, complex integrations requiring full codebase analysis
- **Jules**: 100-500 line features, testing, documentation, refactoring with clear patterns

**What YOU Must Do:**
- Create all Jules sessions (directly or via Gemini)
- Update beads tracker for ALL completed work
- Record ALL learnings in ByteRover
- Monitor ALL PR status and comment with @jules for fixes
- Commit documentation updates
- Ensure project coherence and quality

### Success Metrics

**Track and Report:**
- Jules sessions created / daily quota (target: 60-80 / 100)
- Completion rate (target: >70%)
- PR submission rate (target: >60%)
- CI pass rate (target: >80%)
- Beads sync status (target: 100% - CRITICAL)

**Example Report:**
```
Multi-Agent Session Summary:
- Jules sessions: 52 created, 37 completed (71% success)
- PRs submitted: 9 (24% PR rate, needs improvement)
- Beads synced: 37/37 completed sessions (100%)
- ByteRover entries: 4 recorded
- Codex work: Newport 1830C V3 (1,067 lines) - NOT YET COMMITTED
- Gemini guidance: 5 unblocking responses provided
```

### Troubleshooting Common Issues

**Jules Rate Limiting (429 errors):**
- Solution: Increase sleep delays to 10s
- Use background batches for bulk creation
- Spread creation over time

**Stuck Planning Sessions:**
- Use clink with Gemini for architectural context
- Provide reference implementation file paths
- Give specific trait/pattern guidance

**PR CI Failures:**
- Comment with @jules + specific fixes needed
- Reference working implementation (e.g., Newport V3)
- Request rebase on latest main

**Beads Out of Sync:**
- Run audit: `BEADS_DB=.beads/daq.db bd ready --json` vs `jules remote list`
- Batch update all closed sessions
- Commit immediately to prevent drift

**Background Batches Stalled:**
- Check BashOutput for errors
- Verify correct repo name (TheFermiSea/rust-daq NOT briansquires/rust-daq)
- Monitor for 429 errors, increase delays if needed

### Future Transition Strategy

**Moving from Markdown to ByteRover:**
- AGENTS.md, CLAUDE.md, GEMINI.md: Keep for stable reference documentation
- ByteRover: Use for evolving lessons learned, session-specific findings
- Pattern: Markdown documents workflows, ByteRover captures discoveries
- Goal: Eventually migrate most agent memory to ByteRover for better searchability and cross-session persistence

**When to Use What:**
- **Markdown docs**: Architectural decisions, workflow patterns, tool usage, permanent reference
- **ByteRover**: Session learnings, specific file:line discoveries, evolving best practices, error patterns
- **Beads tracker**: Issue status, dependencies, work queue management
- **Git commits**: Code changes + beads sync, permanent record

## Best Practices

### Efficient Workflow

1. **Retrieve wisely:** Use `brv retrieve` with specific queries and `--node-keys` to filter
2. **Read only what's needed:** Check playbook with `brv status` to see statistics before reading full content
3. **Update precisely:** Use `brv add` to add/update specific bullets or `brv complete` for complete workflow
4. **Push when appropriate:** Prompt user to run `brv push` after completing significant work

### Memory Management

**Retrieve pattern:**

- Use `brv add` to directly add/update bullets
- `brv retrieve` **clears existing playbook** - use carefully
- Retrieved memories use actual Memora tags (not "auto-generated")
- Both memories and related memories are saved to playbook

---
Generated by ByteRover CLI for Claude Code