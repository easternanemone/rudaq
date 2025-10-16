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

## Recent Architectural Changes

- **bd-25 (Error Handling)**: Enhanced `DaqError` with context-rich variants, improved storage writer error propagation
- **bd-22 (GUI Batching)**: Optimized data dispatch to prevent GUI lag
- **bd-20/21 (Graceful Shutdown)**: Added `InstrumentCommand::Shutdown`, async serial I/O, 5s timeout with fallback abort
- **Measurement Enum**: Introduced `Measurement` enum to replace JSON metadata workarounds for non-scalar data
