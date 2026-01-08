# Rust DAQ System Architecture

## Overview

`rust-daq` is a modular, high-performance data acquisition system built in Rust. It is designed for scientific experiments requiring low-latency hardware control, high-throughput data streaming, and crash-resilient operation.

The architecture follows a **Headless-First** design: the core daemon runs as a robust, autonomous process that owns the hardware, while the user interface runs as a separate, lightweight client. This ensures that a GUI crash never interrupts a running experiment.

## Core Design Principles

1.  **Crash Resilience:** Strict separation between the Daemon (Rust) and the Client (`egui`).
2.  **Capability-Based Hardware:** Drivers are composed of atomic traits (`Movable`, `Triggerable`) rather than monolithic inheritance.
3.  **Hot-Swappable Logic:** Experiments are defined in **Rhai** scripts, allowing logic changes without recompiling the daemon.
4.  **Zero-Copy Data Path:** High-speed data flows through a memory-mapped ring buffer (Arrow IPC) for visualization and storage.

---

## System Components

The project is structured as a Cargo workspace with distinct responsibilities:

### 1. Application Layer
*   **`daq-bin`**: The entry point for the daemon (`rust-daq-daemon`). Wires together the system based on compile-time features.
*   **`daq-egui`**: The desktop client application. Built with `egui` and `egui_dock` for a flexible, pane-based layout. Connects to the daemon via gRPC. Features auto-reconnect with exponential backoff, health monitoring, and real-time logging panel.
*   **`rust-daq`**: A facade crate providing a clean `prelude` for external consumers and integration tests. Feature-gates optional dependencies (`server`, `scripting`).

### 2. Domain Logic
*   **`daq-experiment`**: The orchestration engine ("RunEngine"). Executes declarative plans and manages the experiment state machine.
*   **`daq-scripting`**: Embeds the **Rhai** scripting engine. Provides a safe sandbox for user scripts to control hardware (10k operation limit, timeout protection). Optional Python bindings via PyO3.
*   **`daq-server`**: The network interface. Implements a gRPC server (`tonic`) exposing hardware control, script execution, and data streaming. Includes token-based authentication and CORS configuration.

### 3. Infrastructure
*   **`daq-hardware`**: The Hardware Abstraction Layer (HAL). Defines capability traits and contains drivers for serial devices (Thorlabs, Newport, MaiTai, etc.).
*   **`daq-driver-pvcam`**: Dedicated driver crate for Photometrics PVCAM cameras (Prime 95B, Prime BSI). Requires PVCAM SDK.
*   **`daq-driver-comedi`**: Driver for Linux Comedi DAQ boards. Provides analog/digital I/O capabilities.
*   **`daq-storage`**: Handles data persistence. Implements the "Mullet Strategy": fast **Arrow** ring buffer in the front, reliable **HDF5** writer in the back. Also supports CSV, MATLAB (.mat), and NetCDF formats.
*   **`daq-proto`**: Defines the wire protocol (Protobuf) for all network communication. Includes domain↔proto conversion utilities.

### 4. Core
*   **`daq-core`**: The foundation. Defines shared types (`Parameter<T>`, `Observable<T>`), error handling, size limits (`limits.rs`), and module domain types.

### 5. FFI Bindings
*   **`pvcam-sys`**: Raw FFI bindings to the PVCAM C library (nested under `daq-driver-pvcam`).
*   **`comedi-sys`**: Raw FFI bindings to the Linux Comedi library.

---

## Architectural Diagrams

### High-Level Topology

```mermaid
graph TD
    subgraph "Host Machine"
        subgraph "Daemon Process (rust-daq-daemon)"
            Server[gRPC Server]
            Script[Rhai Engine]
            HW[Hardware Manager]
            Ring[Ring Buffer / Arrow]
            Writer[HDF5 Writer]
        end

        subgraph "Client Process (rust-daq-gui)"
            GUI[egui Interface]
            Dock[Docking System]
            Plot[Real-time Plots]
        end
    end

    GUI <-->|gRPC / HTTP2| Server
    
    Server --> Script
    Script --> HW
    HW -->|Frame Data| Ring
    Ring -->|Zero-Copy| Writer
    Ring -.->|Stream| Server
```

### Data Pipeline (The "Mullet Strategy")

To resolve the conflict between high-throughput reliable storage and low-latency live visualization, the system implements a **Tee-based Pipeline**:

1.  **Source:** Hardware drivers produce data (e.g., `Arc<Frame>`).
2.  **Ring Buffer:** Data is written to a lock-free, memory-mapped Ring Buffer using Apache Arrow IPC format.
3.  **Storage Path:** A dedicated background thread reads from the Ring Buffer and writes to HDF5 files.
4.  **Live Stream:** The `DaqServer` subscribes to the stream and broadcasts it via gRPC to the GUI.

---

## Key Features

### Hardware Abstraction
Hardware is modeled by **Capabilities**, not identities. A device is defined by what it can *do*:
*   `Movable`: Can move to a position (e.g., Motors, Piezo stages).
*   `Triggerable`: Can accept a start signal (e.g., Cameras).
*   `Readable`: Can return a scalar value (e.g., Sensors).
*   `FrameProducer`: Can stream 2D image data (e.g., Detectors).
*   `ExposureControl`: Can set integration time.

This allows generic experiment scripts to work with any compatible hardware (e.g., `scan(movable, triggerable)`).

### Reactive Parameters
All hardware state is managed via `Parameter<T>`. This provides:
*   **Observability:** Changes are broadcast to all subscribers (GUI, Scripts).
*   **Validation:** Setters can reject invalid values.
*   **Persistence:** Parameter values are snapshotted to HDF5.

### Scripting (Rhai)
Experiments are written in [Rhai](https://rhai.rs), a scripting language designed for Rust.
*   **Safety:** Scripts run in a sandbox with operation limits to prevent infinite loops.
*   **Integration:** Rust async functions are exposed as synchronous Rhai functions (e.g., `stage.move_abs(10.0)`).
*   **Hot-Swap:** Scripts are uploaded via gRPC and executed immediately.

---

## Directory Structure

```
.
├── crates/
│   ├── daq-bin/            # Application entry points (daemon, CLI)
│   ├── daq-core/           # Foundation types, errors, parameters, limits
│   ├── daq-driver-comedi/  # Comedi DAQ driver for Linux boards
│   ├── daq-driver-pvcam/   # PVCAM camera driver
│   │   └── pvcam-sys/      # Raw FFI bindings to PVCAM
│   ├── daq-egui/           # Desktop GUI (egui + egui_dock)
│   ├── daq-examples/       # Example code and usage patterns
│   ├── daq-experiment/     # RunEngine and Plan definitions
│   ├── daq-hardware/       # HAL with capability traits and drivers
│   ├── daq-proto/          # Protobuf definitions and conversions
│   ├── daq-scripting/      # Rhai scripting engine integration
│   ├── daq-server/         # gRPC server implementation
│   ├── daq-storage/        # Ring buffers, CSV, HDF5, Arrow storage
│   ├── comedi-sys/         # Raw FFI bindings to Comedi
│   └── rust-daq/           # Integration layer with prelude module
├── config/                 # Runtime configuration (TOML)
├── docs/                   # Documentation
│   ├── architecture/       # ADRs and design decisions
│   ├── benchmarks/         # Performance documentation
│   ├── project_management/ # Roadmaps and planning
│   └── troubleshooting/    # Platform notes and setup guides
├── examples/               # Rhai script examples
└── proto/                  # Protobuf source files
```
