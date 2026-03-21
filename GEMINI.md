# GEMINI.md - Project Context & Instructions

## Project Overview
**rust-daq** is a modular, high-performance Data Acquisition (DAQ) system designed for scientific research. It enables high-throughput data acquisition from diverse instruments (cameras, lasers, motion controllers, etc.) with automated workflows and live data streaming.

### Key Technologies
- **Language**: Rust (edition 2021, MSRV 1.75+)
- **Async Runtime**: `tokio` (1.36+)
- **Communication**: gRPC (`tonic`) with Protobuf (`prost`)
- **Data Serialization**: `serde`, Apache Arrow (zero-copy streaming)
- **Storage**: HDF5 (`hdf5-metno`), Apache Arrow IPC, Parquet, Tiff, Zarr
- **Scripting**: Rhai (`rhai`) for experiment automation
- **GUI**: `egui` with `egui_dock` (native via `eframe`, WASM via `trunk`, Rerun viewer integration)
- **Tracing**: `tracing` for structured logging

### Architecture
The project is a Rust workspace with 26 crates organized into layers:
1.  **Core (`common`)**: Shared types, error handling, observable parameters.
2.  **HAL (`hardware`)**: Capability-based Hardware Abstraction Layer (Movable, Readable, FrameProducer traits).
3.  **Drivers (`driver-*`)**: Native SDK drivers (PVCAM, Andor SDK3, Comedi, Dover Motion) for FFI-bound hardware; `driver-universal` TOML manifests for serial/TCP/SCPI devices; `driver-mock` always compiled.
4.  **Driver Registry (`driver-registry`)**: Hardware feature gating and factory orchestration.
5.  **Engine (`experiment`, `scripting`)**: Orchestration (RunEngine) and automation.
6.  **Interfaces (`server`, `ui`, `protocol`)**: gRPC server, desktop GUI, and wire protocol.
7.  **Data (`storage`, `pool`)**: High-performance buffering and persistence.

---

## Development Workflows

### Task & Issue Tracking
- **System**: `bd` (beads) issue tracker.
- **Workflow**: Always check `bd ready` for active tasks. Update issue notes with `COMPLETED`, `IN PROGRESS`, and `NEXT` markers.
- **Note**: Prefer JSON output (`--json`) for automated tools.

### Building
- **Mock Mode (Development)**: `cargo build -p bin`
- **GUI**: `cargo build -p ui --release`
- **Maitai Hardware (Production)**: `bash scripts/ops/build-maitai.sh` (Required for real hardware to ensure feature flags and environment are correct).

### Running
See [Project Inventory](docs/reference/inventory.md) for canonical binaries and config paths.

- **Daemon (Mock)**: `cargo run -p bin -- daemon --hardware-config config/demo.toml`
- **Daemon (Maitai)**: `./target/release/rust-daq-daemon daemon --hardware-config config/maitai_universal.toml`
- **Run Script**: `cargo run -p bin -- run examples/demo_scan.rhai`
- **Launch GUI**: `cargo run -p ui --release`

### Testing
- **Test Runner**: `cargo nextest run` (Parallel execution)
- **Doc Tests**: `cargo test --doc`
- **Hardware Tests**: Requires `maitai` features and environment: `source scripts/ops/env-check.sh && cargo nextest run --features hardware_tests`

---

## Coding Conventions

### Style & Lints
- **Linting**: Workspace-level Clippy lints are defined in `Cargo.toml`. Many pedantic lints are allowed to reduce noise in hardware-specific code (e.g., bit manipulation, float comparisons).
- **Safety**: `unsafe_code = "warn"` is enforced. Unsafe blocks are primarily in FFI `*-sys` crates.
- **Formatting**: `cargo fmt --all` before committing.

### Error Handling
- Use `anyhow` for top-level application code.
- Use `thiserror` for library-level error definitions.
- Errors are categorized for recovery strategies (e.g., `HardwareError`, `ProtocolError`).

### Hardware Patterns
- **Capability Traits**: Hardware is defined by what it *does* (e.g., `Movable`), not what it *is*.
- **Reactive Parameters**: Use `Parameter<T>` for observable state that syncs with the GUI and scripts.
- **Mullet Strategy**: Front-end Ring Buffer (Arrow) for fast streaming, Back-end HDF5 for reliable storage.

---

## Key Files & Directories
- `CLAUDE.md`: Concise operational guide for AI agents.
- `AGENTS.md`: Canonical agent policy.
- `docs/explanation/architecture.md`: Detailed system design.
- `docs/adr/`: Architecture Decision Records for major design choices.
- `config/feature_flags.toml`: Centralized feature flag management.
- `scripts/`: Critical build, maintenance, and environment scripts.

---

## Interaction Rules for Gemini
1.  **Always use `bd` (beads)** to track progress and issues.
2.  **Consult `CLAUDE.md`** for project-specific fast-start commands.
3.  **Respect Feature Flags**: Do not assume all hardware drivers are available; check `Cargo.toml` or `config/feature_flags.toml`.
4.  **Verification**: After significant changes, run `cargo nextest run` and `cargo clippy`.
5.  **Maitai Builds**: Always remind the user to use `scripts/ops/build-maitai.sh` when working with real hardware.
