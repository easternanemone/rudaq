# rust-daq

**A modular, high-performance, headless-first Data Acquisition (DAQ) system written in Rust.**

`rust-daq` is designed for scientific experiments requiring precise hardware control, high-throughput data streaming, and robust automation. It decouples experiment logic from hardware implementation, enabling reproducible, scriptable, and scalable data acquisition.

![Architecture Status](https://img.shields.io/badge/Architecture-V5_Complete-green)
![Build Status](https://img.shields.io/badge/build-passing-brightgreen)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)

## ⚡ Try It Now (No Hardware Required)

Get a complete DAQ system running in under 30 seconds using mock devices:

```bash
# Terminal 1: Start daemon with demo hardware
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Terminal 2: Run a demo scan
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

**That's it!** You just ran an automated scan with mock stage, power meter, and camera.

Want the GUI? See [**Demo Mode Guide**](DEMO.md) for interactive control, custom scripts, and transitioning to real hardware.

---

## 🚀 Key Features

*   **Headless-First Design**: The core system runs as a lightweight daemon, controllable via gRPC or local scripts. Perfect for long-running experiments or embedded controllers.
*   **Capability-Based HAL**: Hardware is abstracted by *what it does* (e.g., `Movable`, `Readable`, `Triggerable`, `FrameProducer`), not just what it is. This allows flexible composition and easy mocking.
*   **Bluesky-Inspired Orchestration**: Separates **Plans** (declarative experiment logic) from the **RunEngine** (execution). Supports pause/resume, adaptive scanning, and structured data documents.
*   **High-Performance Data Pipeline**: Uses **Apache Arrow** for zero-copy in-memory data handling and **HDF5** for efficient, standard storage.
*   **Scripting & Automation**: First-class support for **Rhai** scripting to define experiments dynamically without recompilation. Python client bindings available.
*   **Modular Workspace**: Organized as a cargo workspace for clean separation of concerns.

## 🏗️ Architecture

The system is built as a collection of crates:

| Crate | Description |
|-------|-------------|
| **`daq-core`** | Foundation types, error handling, parameters, observables, and size limits. |
| **`daq-hardware`** | Hardware Abstraction Layer (HAL) with capability traits (`Movable`, `Readable`, `FrameProducer`, etc.) and drivers. |
| **`daq-driver-pvcam`** | PVCAM camera driver for Photometrics cameras (requires PVCAM SDK). |
| **`daq-driver-comedi`** | Comedi DAQ driver for Linux data acquisition boards. |
| **`daq-experiment`** | RunEngine and Plan definitions for experiment orchestration. |
| **`daq-server`** | gRPC server implementation exposing control and data streams. |
| **`daq-storage`** | Data persistence with ring buffers, CSV, HDF5, and Arrow formats. |
| **`daq-scripting`** | Rhai scripting engine integration with Python bindings. |
| **`daq-proto`** | Protocol Buffer definitions and domain↔proto conversions. |
| **`daq-egui`** | Desktop GUI application with docking panels, auto-reconnect, and real-time logging. |
| **`daq-bin`** | CLI binaries and daemon entry points. |
| **`rust-daq`** | Integration layer providing `prelude` module for convenient imports. |

For a deep dive, see [Architecture Documentation](docs/architecture/ARCHITECTURE.md).

## 🛠️ Getting Started

### Prerequisites

-   **Rust**: Stable toolchain (1.75+).
-   **System Libraries** (Optional, depending on features):
    -   `libhdf5-dev` (if using HDF5 storage)
    -   PVCAM SDK (if using Photometrics cameras)

### Building

Build the daemon for your environment:

```bash
# Basic build (Mock hardware, CSV storage)
cargo build -p daq-bin

# With HDF5 support
cargo build -p daq-bin --features storage_hdf5

# With all hardware drivers and server features
cargo build -p daq-bin --features "server,all_hardware,storage_hdf5"
```

**For Maitai Lab Machine (Real Hardware):**

The maitai machine requires ALL real hardware drivers enabled:

```bash
# CORRECT way - uses build script with full clean
bash scripts/build-maitai.sh

# This enables ALL hardware:
#   - PVCAM (real SDK, not mock camera)
#   - Thorlabs ELL14 rotators (3x devices)
#   - Newport ESP300 motion controller
#   - Newport 1830-C power meter
#   - Spectra-Physics MaiTai laser
#   - Serial port communication

# Verify build shows: "Registered 7 device(s)" in daemon log
```

**WARNING:** Building without the `maitai` feature produces mock hardware only!

### Running

Start the DAQ daemon:

```bash
# Run with default settings (starts gRPC server on 0.0.0.0:50051)
cargo run -p daq-bin --features server

# Run a specific script
cargo run -p daq-bin --features scripting_rhai -- run my_experiment.rhai
```

## 🔌 Hardware Support

Drivers are included for:
-   **Simulation**: Mock stage, Mock camera, Mock power meter.
-   **Motion Control**: Newport ESP300, Thorlabs Elliptec (ELL14).
-   **Cameras**: Photometrics PVCAM (Prime 95B, Prime BSI).
-   **Lasers**: Spectra-Physics MaiTai.
-   **Sensors**: Newport 1830-C Power Meter.

## 🧪 Testing

We use [cargo-nextest](https://nexte.st/) as the primary test runner:

```bash
# Install nextest
cargo install cargo-nextest --locked

# Run all tests
cargo nextest run

# Run with CI profile (includes retries)
cargo nextest run --profile ci

# Run documentation tests (not supported by nextest)
cargo test --doc
```

See [**Testing Guide**](docs/guides/testing.md) for comprehensive testing documentation including timing tests, hardware tests, and CI integration.

## 📚 Documentation

**[📖 Documentation Hub](docs/README.md)** - Complete navigation guide to all documentation.

### Quick Links

| Category | Resource |
|----------|----------|
| **Getting Started** | [Demo Mode Guide](DEMO.md) - Try without hardware |
| **User Guides** | [Scripting Guide](docs/guides/scripting.md) · [Storage Formats](docs/guides/storage-formats.md) |
| **Developer Guides** | [Hardware Drivers](docs/guides/hardware-drivers.md) · [Testing Guide](docs/guides/testing.md) |
| **Architecture** | [System Architecture](docs/architecture/ARCHITECTURE.md) · [Feature Matrix](docs/architecture/FEATURE_MATRIX.md) |

### Crate Documentation

| Crate | Purpose |
|-------|---------|
| [daq-core](crates/daq-core/README.md) | Foundation types, parameters, error handling |
| [daq-hardware](crates/daq-hardware/README.md) | Hardware abstraction, capability traits |
| [daq-scripting](crates/daq-scripting/README.md) | Rhai scripting engine bindings |
| [daq-server](crates/daq-server/README.md) | gRPC services and client examples |

See the [Documentation Hub](docs/README.md) for the complete list of all crate READMEs and guides.

## 📄 License

Dual-licensed under MIT or Apache 2.0.
