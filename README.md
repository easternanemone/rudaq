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
| **`daq-core`** | Common types, traits, and data models used across the system. |
| **`daq-hardware`** | Hardware Abstraction Layer (HAL). Contains drivers (Thorlabs, Newport, etc.) and capability traits. |
| **`daq-experiment`** | The RunEngine and Plan definitions. Orchestrates experiment execution. |
| **`daq-server`** | gRPC server exposing control and data streams to the network. |
| **`daq-storage`** | Data persistence handling CSV, HDF5, and Arrow formats. |
| **`daq-scripting`** | Scripting engine integrations (Rhai, Python). |
| **`daq-proto`** | Protocol Buffer definitions and generated code for the gRPC API. |
| **`daq-bin`** | Application entry points (CLI, Daemon). |

For a deep dive, see [Architecture Documentation](docs/architecture/ARCHITECTURE.md).

## 🛠️ Getting Started

### Prerequisites

-   **Rust**: Stable toolchain (1.75+).
-   **System Libraries** (Optional, depending on features):
    -   `libhdf5-dev` (if using HDF5 storage)
    -   PVCAM SDK (if using Photometrics cameras)

### Building

Build the main daemon:

```bash
# Basic build (Mock hardware, CSV storage)
cargo build -p daq-bin

# With HDF5 support
cargo build -p daq-bin --features storage_hdf5

# With all hardware drivers and server features
cargo build -p daq-bin --features "server,all_hardware,storage_hdf5"
```

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

## 📚 Documentation

-   [**Demo Mode Guide**](DEMO.md): Try rust-daq without hardware in 30 seconds.
-   [**System Architecture**](docs/architecture/ARCHITECTURE.md): Detailed breakdown of system design.
-   [**Feature Matrix**](docs/architecture/FEATURE_MATRIX.md): Guide to cargo features and build profiles.
-   [**Benchmarks**](docs/benchmarks/tee.md): Tee pipeline throughput/latency numbers and how to run `tee_bench` (see CI artifact `tee-bench-*` for latest sample outputs).
-   [**Platform Notes**](docs/troubleshooting/PLATFORM_NOTES.md): Linux (Wayland/X11), macOS, and Windows-specific setup.

## 📄 License

Dual-licensed under MIT or Apache 2.0.
