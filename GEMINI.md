# GEMINI.md

## Project Overview

This project contains both documentation and the live Rust implementation for a scientific data acquisition (DAQ) application. Treat `src/` and `rust_daq/` as active crates that must compile under `cargo check`.

The core architectural principles are:
*   **Modular Plugin System:** Instruments, GUIs, and data processors are designed as separate, dynamically loadable modules using a trait-based interface.
*   **Async-First Design:** The application is built on the Tokio runtime, using async-first principles and channel-based communication for non-blocking operations.
*   **Type Safety and Reliability:** Leverages Rust's strong type system and `Result`-based error handling to ensure safety and reliability.

The technology stack includes:
*   **Core:** Rust
*   **Asynchronous Runtime:** Tokio
*   **GUI:** egui
*   **Data Handling:** ndarray, polars, serde, HDF5
*   **Instrument Control:** scpi, serialport

## Building and Running

The following commands are based on the provided documentation for building, running, and testing the application.

### Running the Application

```bash
# Run in development mode with hot-reloading
cargo watch -x run

# Run in release mode
cargo run --release

# Run with specific features (e.g., HDF5 support)
cargo run --features hdf5-support
```

### Testing the Application

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_instrument_initialization

# Run integration tests
cargo test --test integration
```

## Development Conventions

*   **Code Formatting:** Use `cargo fmt` to format the code.
*   **Linting:** Use `cargo clippy` to check for common issues.
*   **Dependency Auditing:** Use `cargo audit` to check for security vulnerabilities in dependencies.
*   **Error Handling:** The project uses the `thiserror` crate for custom error types; user-facing messages live in `src/error.rs` and `src/app.rs`.
*   **Testing:** The project follows a comprehensive testing strategy, including unit tests with mock instruments, integration tests for data flow, and performance tests.
*   **Multi-Agent Coordination:**
    - Request a dedicated `git worktree` before modifying files so parallel agents do not collide.
    - Prefix beads commands with `BEADS_DB=.beads/daq.db` to reuse the repo-local tracker (creating `$HOME/.beads` is blocked).
    - End runs with `cargo check` + `git status -sb` to confirm a clean tree.

## Directory Overview

This directory serves as the central documentation hub for the Rust-based scientific data acquisition application. It contains detailed guides on the application's architecture, data management, deployment, GUI development, and instrument control.

## Key Files

*   `rust-daq-app-architecture.md`: Provides a detailed overview of the application's architecture, including the modular plugin system, async-first design, and core components.
*   `rust-daq-data-guide.md`: Covers data management strategies, including real-time buffering, data persistence, and storage backends like HDF5 and CSV.
*   `rust-daq-deployment.md`: Describes deployment strategies, including optimized release builds, cross-platform packaging, and containerization with Docker.
*   `rust-daq-getting-started.md`: A guide for setting up the development environment, project structure, and initial implementation.
*   `rust-daq-gui-guide.md`: Explains the GUI development process using the `egui` framework, including real-time data visualization, and instrument control panels.
*   `rust-daq-instrument-guide.md`: Details the implementation of instrument control, including support for SCPI, serial communication, and a plugin architecture for different instrument types.
*   `rust-daq-performance-test.md`: Outlines performance optimization strategies, benchmarking, profiling, and testing to ensure real-time performance and reliability.
*   `logs/`: Contains log files from the application.

## Usage

This directory is intended to be used as a comprehensive reference for understanding, developing, and deploying the Rust-based scientific data acquisition application. The guides provide a solid foundation for developers working on the project.
