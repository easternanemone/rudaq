# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This repository contains documentation and architectural guidance for building a scientific data acquisition (DAQ) application in Rust. It is designed as a high-performance, modular alternative to Python-based solutions like PyMoDAQ, ScopeFoundry, or Qudi.

**Current State**: The repository now contains live Rust source under `src/` and `rust_daq/`. Architectural docs remain authoritative, but agents must treat the workspace as production code—changes compile under `cargo check`.

## Core Architecture Principles

1. **Modular Plugin System**: Instruments, GUIs, and data processors are designed as separate, dynamically loadable modules using trait-based interfaces
2. **Async-First Design**: Built on Tokio runtime with channel-based communication for non-blocking operations
3. **Type Safety**: Leverages Rust's type system and Result-based error handling for reliability

## Technology Stack

- **Runtime**: Tokio (async)
- **GUI Framework**: egui/eframe
- **Data Handling**: ndarray, polars, serde, HDF5, Apache Arrow
- **Instrument Control**: SCPI protocol, serialport
- **Configuration**: config crate with TOML files
- **Logging**: tracing/tracing-subscriber
- **Error Handling**: thiserror, anyhow

## Common Commands

### Development Workflow
```bash
# Run in development with hot-reload
cargo watch -x run

# Run in release mode
cargo run --release

# Run with specific features
cargo run --features hdf5-support
```

### Testing
```bash
# Run all tests
cargo test

# Run with output visible
cargo test -- --nocapture

# Run specific test
cargo test test_instrument_initialization

# Run integration tests
cargo test --test integration
```

### Code Quality
```bash
# Format code
cargo fmt

# Lint and check for issues
cargo clippy

# Security audit
cargo audit

# Generate documentation
cargo doc --open
```

### Multi-Agent Coordination
- Obtain a unique `git worktree` before editing. The main checkout may be in use by automation; overlapping directories risk undoing another agent’s work.
- When using `bd`, set `BEADS_DB=.beads/daq.db` to reuse the project-local tracker. The sandbox blocks writing to `$HOME/.beads`.
- Finish sessions with `cargo check` and `git status -sb`; confirm recent error UX improvements in `src/error.rs` and configuration messaging in `src/app.rs` still stand.

### Cross-Platform Builds
```bash
# Add targets
rustup target add x86_64-pc-windows-gnu
rustup target add aarch64-apple-darwin

# Build for specific target
cargo build --release --target x86_64-pc-windows-gnu
```

## Key Design Patterns

### Instrument Trait Pattern
All instruments implement the async `Instrument` trait with methods:
- `initialize()` - Setup with configuration
- `start_acquisition()` / `stop_acquisition()` - Control data collection
- `read_data()` - Retrieve measurements
- `send_command()` - Execute instrument commands
- `shutdown()` - Clean disconnect

### Message-Based Communication
System components communicate via typed messages through channels:
```rust
enum SystemMessage {
    InstrumentData { source: String, data: Vec<u8>, timestamp: Instant },
    ConfigUpdate { target: String, config: Value },
    Command { target: String, command: String, params: Vec<String> },
    Error { source: String, error: String },
}
```

### Real-Time Buffering
Ring buffers (ringbuf crate) with overflow handling for continuous data streams. Lock-free implementations where possible for minimal latency.

### Error Handling
Custom error types using thiserror for domain-specific errors:
- `InstrumentError` - Connection, timeout, invalid response
- `DataProcessingError` - Buffer overflow, invalid data
- `ConfigError` - Missing/invalid configuration

## Project Structure (When Implemented)

```
src/
├── main.rs                  # Entry point, GUI launch
├── lib.rs                   # Public API exports
├── core/
│   ├── instrument.rs        # Instrument trait definitions
│   ├── data_processor.rs    # Data processing pipeline
│   └── plugin_manager.rs    # Dynamic plugin loading
├── gui/
│   ├── main_window.rs       # Primary GUI window
│   └── components/          # Reusable UI components
├── instruments/
│   ├── mock.rs              # Mock instrument for testing
│   └── scpi/                # SCPI protocol support
├── data/
│   ├── buffer.rs            # Real-time ring buffers
│   └── storage.rs           # HDF5/CSV persistence
└── utils/
    ├── config.rs            # Configuration loading
    └── logging.rs           # Tracing setup
```

## Configuration System

Hierarchical configuration with precedence:
1. `config/default.toml` - Base configuration
2. `config/local.toml` - Local overrides (gitignored)
3. Environment variables with `RUSTDAQ_` prefix

Key config sections:
- `[application]` - Name, version, log level
- `[instruments.*]` - Per-instrument settings with plugin name, connection params
- `[data_acquisition]` - Buffer sizes, sample rates, auto-save
- `[gui]` - Theme, update rate, plot buffer size

## Performance Considerations

1. **Zero-Copy**: Use `Arc<[T]>` for shared data, memory-mapped files for large datasets
2. **Lock-Free**: Prefer lock-free data structures (DashMap, ring buffers) for hot paths
3. **SIMD**: Use explicit SIMD for bulk data processing when needed
4. **Async Batching**: Batch small operations to reduce context switching overhead
5. **Backpressure**: Implement channel-based backpressure to prevent memory exhaustion

## Testing Strategy

- **Unit Tests**: Mock instruments with tokio-test for async testing
- **Integration Tests**: Full data pipeline with mock instruments
- **Performance Tests**: Benchmarks with criterion for critical paths
- **Property Tests**: Use proptest for data processing correctness

## Documentation Files

- `rust-daq-getting-started.md` - Setup, dependencies, initial implementation
- `rust-daq-app-architecture.md` - System design, core traits, plugin system
- `rust-daq-instrument-guide.md` - SCPI, serial, USB/Ethernet instrument control
- `rust-daq-data-guide.md` - Buffering, persistence, storage backends
- `rust-daq-gui-guide.md` - egui implementation, real-time plotting
- `rust-daq-deployment.md` - Release builds, packaging, cross-platform
- `rust-daq-performance-test.md` - Optimization, profiling, benchmarking
- `GEMINI.md` - High-level project overview

## Implementation Notes

When implementing features from this documentation:

1. Start with core traits (Instrument, DataProcessor) before concrete implementations
2. Build mock instruments first for testing infrastructure
3. Implement basic GUI with instrument controls before advanced features
4. Add real hardware support incrementally after mock testing works
5. Use feature flags for optional dependencies (HDF5, Arrow) to keep builds lean
6. Profile early and often - real-time performance is critical
7. Document all public APIs with doc comments including examples
8. Thread safety is essential - all instrument operations must be Send + Sync
