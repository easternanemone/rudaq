# rust-daq

**A modular, high-performance Data Acquisition system for scientific research.**

[![Architecture Status](https://img.shields.io/badge/Architecture-V6_In_Progress-blue)](docs/explanation/architecture.md)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#building)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#license)
[![Rust 1.75+](https://img.shields.io/badge/Rust-1.75%2B-orange)](#prerequisites)

Acquire high-throughput data from scientific instruments. Execute reproducible experiments with automated workflows. Stream live data to analysis pipelines. All in Rust—fast, safe, and production-ready.

> **Built for the lab.** Whether you're controlling a microscope, laser system, or multi-instrument experimental setup, rust-daq handles hardware abstraction, timing synchronization, and data persistence so you can focus on science.

---

## Quick Demo (30 seconds, no hardware needed)

Get a complete DAQ system running with mock devices:

```bash
# Terminal 1: Start daemon
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Terminal 2: Run automated scan
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

**That's it!** You just executed an automated scan with mock motion stage, power meter, and camera. Ready for real hardware? See the [Demo Mode Guide](docs/tutorials/demo-mode.md).

---

## Why rust-daq?

| Feature | Benefit |
|---------|---------|
| **Headless-First Architecture** | Run on servers, embedded systems, or lab machines without GUI dependencies. Control via gRPC, CLI, or scripts. |
| **Capability-Based Abstraction** | Define devices by what they do (move, measure, image) not what they are. Swap hardware without changing code. |
| **Unified Hardware Layer** | One API for Photometrics cameras, Newport motion controllers, Thorlabs rotators, lasers, sensors, and custom serial devices. |
| **High-Speed Data Streaming** | Apache Arrow zero-copy frames, HDF5 storage, and gRPC streaming for real-time analysis. |
| **Automation & Scripting** | Rhai scripts for complex experiments. Python bindings for custom analysis. Pause/resume and adaptive scanning. |
| **Production-Ready** | Robust error handling, connection recovery, health monitoring, and comprehensive testing. |

---

## Architecture at a Glance

```
┌─────────────────────────────────────────────────────────┐
│                 User Interfaces                         │
│  Desktop GUI (egui)  │  CLI Tools  │  gRPC Clients      │
└───────────────┬───────────────────┬─────────────────────┘
                │                   │
┌───────────────▼───────────────────▼─────────────────────┐
│          gRPC Server & Scripting Engine (Rhai)          │
└───────────────┬─────────────────────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────┐
│                  Core Experiment Engine                 │
│   RunEngine  │  Plans  │  Observable State Management   │
└───────────────┬─────────────────────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────┐
│          Hardware Abstraction Layer (HAL)               │
│  Capability Traits: Movable, Readable, FrameProducer    │
│            Device Registry & Plugin System              │
└───────────────┬─────────────────────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────┐
│                 Hardware Drivers                        │
│ PVCAM │ Comedi │ Thorlabs │ Newport │ Spectra Physics   │
│         Serial Port Abstraction (RS-485, USB)           │
└─────────────────────────────────────────────────────────┘
                │
        ┌───────┴──────────┬─────────┬──────────┐
        │                  │         │          │
    ┌───▼───┐      ┌──────▼──┐ ┌───▼──┐   ┌──▼───┐
    │Camera │      │ Motion  │ │Laser │   │Sensor│
    └───────┘      └─────────┘ └──────┘   └──────┘
```

**Crate Organization:**

| Tier | Crates | Purpose |
|------|--------|---------|
| **Core** | `common`, `hardware` | Foundations: error handling, device traits, registry |
| **Drivers** | `driver-pvcam`, `driver-*` | Hardware integrations |
| **Engine** | `experiment`, `scripting` | Orchestration and automation |
| **Interfaces** | `server`, `ui`, `protocol` | gRPC, GUI, and network protocol |
| **Data** | `storage`, `pool` | Persistence and high-performance buffers |

Full architecture docs: [System Architecture](docs/explanation/architecture.md)

---

## Hardware Support Matrix

| Device Type | Models | Capabilities | Status | Feature Flag |
|-------------|--------|--------------|--------|--------------|
| **Cameras** | Photometrics Prime 95B, Prime BSI | FrameProducer, Triggerable, ExposureControl | Production | `pvcam_hardware` |
| **Motion** | Newport ESP300 | Movable, Parameterized | Production | `newport` |
| **Rotators** | Thorlabs ELL14 (RS-485) | Movable, Parameterized | Production | `thorlabs` |
| **Lasers** | Spectra-Physics MaiTai | Readable, ShutterControl, WavelengthTunable | Production | `spectra_physics` |
| **Sensors** | Newport 1830-C Power Meter | Readable, WavelengthTunable, Parameterized | Production | `newport_power_meter` |
| **DAQ** | NI PCI-MIO-16XE-10 | Readable, Settable (Comedi) | Production | `comedi_hardware` |
| **Simulation** | Mock Stage, Mock Camera, Mock Sensors | All traits | Production | Built-in |

**Maitai Lab Configuration:** All 9+ devices integrated and tested. See [Maitai Setup Guide](docs/how-to/hardware-setup.md).

---

## Features by Category

### Core Capabilities
- **Headless Daemon**: Run on any Linux machine, controlled via gRPC or local scripts
- **Capability-Based Abstraction**: Hardware defined by what it does, not what it is
- **Device Registry**: Dynamic device discovery and composition
- **Bluesky-Inspired Orchestration**: Plans + RunEngine for structured experiments
- **Observable State**: Reactive parameters with validation and notifications

### Data Handling
- **Apache Arrow**: Zero-copy frame encoding for efficient streaming
- **HDF5 Storage**: Industry-standard scientific data format with metadata
- **Ring Buffers**: High-performance circular buffers for continuous acquisition
- **CSV & NetCDF**: Additional format support
- **Data Persistence**: Automatic frame buffering and disk writing

### Automation
- **Rhai Scripting**: Dynamic experiment scripts without recompilation
- **gRPC Clients**: Control from Python, Go, or any gRPC-capable language
- **Pause/Resume**: Control experiment flow and state
- **Adaptive Scanning**: Respond to live data during acquisition
- **Batch Operations**: Queue and execute multiple scans

### User Interfaces
- **Desktop GUI**: egui-based docking interface with real-time updates
- **CLI Tools**: Command-line control and scripting
- **gRPC API**: Remote control and streaming
- **Web-Compatible**: Standard protobuf and REST support

### Production & Reliability
- **Robust Error Handling**: Categorized errors with recovery strategies
- **Connection Recovery**: Automatic reconnection with exponential backoff
- **Health Monitoring**: System health tracking and diagnostics
- **Comprehensive Testing**: Unit, integration, and hardware tests
- **Logging & Diagnostics**: Structured logging for debugging

---

## Getting Started

### Prerequisites

- **Rust**: 1.75 or later ([Install](https://rustup.rs/))
- **System Libraries** (optional, depends on features):
  - `libhdf5-dev` - For HDF5 storage support
  - `libudev-dev` - For USB serial device detection (Linux)
  - PVCAM SDK - For real Photometrics cameras (not needed for mock mode)

### Building

#### Quick Build (Mock Hardware)
```bash
# Build daemon with mock devices (no external dependencies)
cargo build -p bin

# Or with HDF5 support
cargo build -p bin --features storage_hdf5

# Or build GUI separately
cargo build -p ui --release
```

#### Full Build (All Features)
```bash
# Everything: all drivers, HDF5, server, scripting
cargo build -p bin --features "server,all_hardware,storage_hdf5,scripting_rhai"
```

#### Maitai Hardware Build
```bash
# Use build script for real hardware (CRITICAL: full clean + all drivers)
bash scripts/build-maitai.sh

# Verify: daemon log should show "Registered 9 device(s)"
# with camera, laser, power meter, rotators, motion, and DAQ
```

**Important:** The `maitai` feature flag enables all real hardware drivers and prevents mock mode. Always use the build script on the maitai machine.

### Running

Start the daemon:

```bash
# With mock devices (no hardware needed)
cargo run -p bin -- daemon --hardware-config config/demo.toml

# With real hardware (Maitai)
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --hardware-config config/maitai_hardware.toml

# Run a script (while daemon is running in another terminal)
cargo run -p bin -- run examples/demo_scan.rhai

# Start GUI (connects to daemon)
cargo run -p ui --release -- --daemon-url http://localhost:50051
```

---

## Quick Examples

### 1. Run a Demo Scan (Command Line)

```bash
# Terminal 1
cargo run -p bin -- daemon --hardware-config config/demo.toml

# Terminal 2
cargo run -p bin -- run examples/demo_scan.rhai
```

Output shows mock stage moving, power meter readings, and camera frames acquired.

### 2. Write a Rhai Script

Create `my_experiment.rhai`:

```rhai
// Initialize hardware
let stage = create_elliptec("/dev/serial/by-id/...", "2");
let pm = create_newport_1830c("/dev/ttyS0");

// Move stage and measure
for angle in [0.0, 45.0, 90.0, 135.0, 180.0] {
    stage.move_abs(angle);
    stage.wait_settled();

    let power = pm.read();
    print(`Angle: ${angle} deg, Power: ${power} W`);
}
```

Run it:
```bash
./target/release/rhai-runner my_experiment.rhai
```

### 3. Use the gRPC API (Python)

```python
# Python client example (requires grpcio + generated stubs)
import grpc
from daq_proto import hardware_service_pb2, hardware_service_pb2_grpc

channel = grpc.insecure_channel('localhost:50051')
hw = hardware_service_pb2_grpc.HardwareServiceStub(channel)

# List devices
devices = hw.ListDevices(hardware_service_pb2.ListDevicesRequest())
for device in devices.devices:
    print(f"{device.id}: {device.name}")

# Move stage
hw.MoveAbsolute(hardware_service_pb2.MoveRequest(
    device_id="mock_stage",
    value=5.0,
    wait_for_completion=True
))
```

### 4. Connect GUI to Daemon

```bash
# Terminal 1: Start daemon
cargo run -p bin -- daemon --hardware-config config/demo.toml

# Terminal 2: Start GUI
cargo run -p ui --release
```

In the GUI, click "Connect" and enter `http://localhost:50051`. You'll see:
- Instrument control panels for each device
- Real-time frame viewer for cameras
- Live plots of sensor data
- Script execution panel

---

## Testing

We use [cargo-nextest](https://nexte.st/) for fast, parallel testing:

```bash
# Run all tests
cargo nextest run

# Run specific crate tests
cargo nextest run -p common
cargo nextest run -p hardware

# Run with CI profile (includes retries for flaky tests)
cargo nextest run --profile ci

# Run documentation tests (not supported by nextest)
cargo test --doc

# Run hardware tests (requires real hardware + maitai environment)
source scripts/env-check.sh && cargo nextest run --features hardware_tests
```

See [Testing Guide](docs/how-to/testing.md) for comprehensive testing documentation.

---

## Documentation

### Quick Navigation

| Document | Purpose |
|----------|---------|
| **[Demo Mode Guide](docs/tutorials/demo-mode.md)** | Try rust-daq without hardware in 2 minutes |
| **[System Architecture](docs/explanation/architecture.md)** | Deep dive into design and component interaction |
| **[Scripting Guide](docs/how-to/scripting.md)** | Write Rhai scripts to control hardware |
| **[Hardware Drivers Guide](docs/how-to/hardware-drivers.md)** | Implement drivers for new instruments |
| **[Storage Formats Guide](docs/how-to/storage-formats.md)** | Choose data format (HDF5, Arrow, CSV) |
| **[Testing Guide](docs/how-to/testing.md)** | Run and write tests |
| **[Maitai Hardware Setup](docs/how-to/hardware-setup.md)** | Configure real hardware on maitai machine |

### Complete Documentation Hub

**[📖 Documentation Hub](docs/README.md)** - Comprehensive navigation for all guides, tutorials, and reference material.

### Crate Documentation

Each crate has detailed README with API examples:

- [**common**](crates/common/README.md) - Foundation types, error handling, observable parameters
- [**hardware**](crates/hardware/README.md) - HAL, device registry, driver factory
- [**scripting**](crates/scripting/README.md) - Rhai engine integration
- [**ui**](crates/ui/README.md) - Desktop GUI components
- [**server**](crates/server/README.md) - gRPC server and client examples
- [**experiment**](crates/experiment/README.md) - RunEngine and experiment orchestration

---

## Architecture Decision Records (ADRs)

Major design decisions are documented in [docs/adr/](docs/adr/):

- **[ADR: PVCAM Continuous Acquisition](docs/adr/008-pvcam-continuous-acquisition.md)** - Camera buffering strategies
- **[ADR: PVCAM Driver Architecture](docs/adr/009-pvcam-driver-architecture.md)** - Multi-layer driver patterns
- **[ADR: Connection Reliability](docs/adr/002-connection-reliability.md)** - Serial device robustness
- **[ADR: gRPC Validation Layer](docs/adr/003-grpc-validation-layer.md)** - Protocol validation strategy

See [Feature Matrix](docs/reference/feature-matrix.md) for implementation status of all major features.

---

## Performance & Optimization

- **Zero-Copy Streaming**: Apache Arrow frames avoid memory copies
- **High-Performance Buffers**: Object pooling and ring buffers for continuous acquisition
- **Async I/O**: Tokio-based non-blocking hardware communication
- **Parallel Testing**: Nextest runs tests in parallel with optimized scheduling
- **Adaptive Quality Modes**: Stream quality selection (Full/Preview/Fast) for bandwidth control

See [Performance Analysis](docs/adr/analysis-pvcam-performance-gap.md) for benchmarking results.

---

## Extending rust-daq

### Create a Custom Driver

Implement the `DriverFactory` trait:

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};
use futures::future::BoxFuture;

pub struct MyDriverFactory;

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_device" }
    fn name(&self) -> &'static str { "My Custom Device" }
    fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new(&config).await?);
            Ok(DeviceComponents::new().with_movable(driver))
        })
    }
}

// Register in daemon startup
registry.register_factory(Box::new(MyDriverFactory));
```

See [Hardware Drivers Guide](docs/how-to/hardware-drivers.md) for patterns and examples.

### Use the Plugin System

Build native plugins with the FFI layer:

```rust
use plugin_api::prelude::*;

#[plugin_entry]
pub fn create_plugin() -> Box<dyn Plugin> {
    Box::new(MyPlugin)
}
```

See [Plugin Quick Start](docs/how-to/plugins.md).

---

## Troubleshooting

### Build Issues

**Problem**: Build fails with "feature not found"
**Solution**: Check your feature flags. Common issue: `pvcam_hardware` requires PVCAM SDK.

```bash
# Check available features
cargo build -p bin --features ?

# Use env-check.sh on maitai
source scripts/env-check.sh
bash scripts/build-maitai.sh
```

### Local Target Cache Maintenance

If local `target/` growth gets out of hand after many builds, tests, and feature
switches, use the maintenance scripts:

```bash
# Run immediately (full cleanup when target >= 30 GiB by default)
bash scripts/target-maintenance.sh

# Force cleanup now
bash scripts/target-maintenance.sh --force --mode full

# Install periodic cleanup (weekly, local machine only)
bash scripts/install-target-maintenance.sh

# Optional: lighter partial mode
bash scripts/install-target-maintenance.sh --mode partial --threshold-gb 20

# Uninstall scheduled cleanup
bash scripts/install-target-maintenance.sh --uninstall
```

### Hardware Not Detected

**Problem**: Daemon starts but shows no devices
**Solution**: Verify hardware configuration:

```bash
# Check hardware config
cat config/maitai_hardware.toml

# Verify build includes real drivers
cargo build -p bin --features pvcam_hardware,thorlabs,newport

# Check daemon log for device registration
cargo run -p bin -- daemon --hardware-config config/demo.toml 2>&1 | grep "Registered"
```

### Connection Issues

**Problem**: Serial device connection fails
**Solution**: Use stable `/dev/serial/by-id/` paths, not `/dev/ttyUSB0`:

```bash
# List stable device paths
ls /dev/serial/by-id/

# Update config with correct path
# config/maitai_hardware.toml
```

See [Troubleshooting Guide](docs/README.md#troubleshooting--reference) for more help.

---

## Contributing

We welcome contributions! Start here:

1. **Report Issues**: Use GitHub issues with detailed reproduction steps
2. **Write Tests**: All new features require tests. See [Testing Guide](docs/how-to/testing.md)
3. **Follow Style**: Run `cargo fmt --all` and `cargo clippy --all-targets`
4. **Document Changes**: Update relevant README and ADR docs
5. **Read CLAUDE.md**: Project-specific development guidelines in [CLAUDE.md](CLAUDE.md)

For larger features, consider opening a discussion before starting work.

---

## License

Dual-licensed under **MIT** or **Apache 2.0** at your option.

Choose whichever license works best for your use:

- **MIT**: Permissive, short license text, minimal restrictions
- **Apache 2.0**: Includes explicit patent grants, more detailed terms

Both are compatible with most commercial and open-source projects.

---

## Getting Help

- **Quick Question?** Check [Documentation Hub](docs/README.md) or [Demo Guide](docs/tutorials/demo-mode.md)
- **Build Problem?** See [Build Verification](docs/how-to/build-and-run.md)
- **Hardware Issue?** Check [Maitai Setup](docs/how-to/hardware-setup.md) or [Platform Notes](docs/how-to/platform-notes.md)
- **Want to Extend?** Read [Hardware Drivers Guide](docs/how-to/hardware-drivers.md) or [Plugin Quick Start](docs/how-to/plugins.md)
- **Found a Bug?** Open an issue with reproduction steps

---

## Project Status

- **V6 Architecture**: In progress — V5 stable, V6 usability improvements underway (See [ARCHITECTURE.md](docs/explanation/architecture.md))
- **Core Features**: Production-ready (scripting, drivers, storage, gRPC)
- **Hardware Support**: 9+ devices tested and verified on maitai machine (camera, laser, power meter, 3 rotators, motion controller, 2 Comedi channels)
- **Documentation**: Comprehensive with ADRs for all major design decisions
- **Testing**: Full test coverage with CI/CD pipeline

---

**Built with ❤️ for scientific research.**

For the latest updates, see the [Architecture Status](docs/reference/feature-matrix.md).
