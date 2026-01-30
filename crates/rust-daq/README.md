# rust-daq

A high-performance, headless-first data acquisition (DAQ) system written in Rust, designed for scientific and industrial applications.

## Features

- **Headless-First Architecture**: Core daemon runs independently of any UI
- **Capability-Based Hardware Abstraction**: Composable traits for motion, sensing, triggering, and camera control
- **YAML Plugin System**: Define new instruments without writing code
- **egui Desktop GUI**: Native Rust/egui control panel (`rust-daq-gui`) that connects to the daemon via gRPC
- **High-Performance Data Pipeline**: Memory-mapped ring buffers with Arrow IPC and HDF5 persistence
- **Script-Driven Automation**: Rhai scripting for experiment control
- **Remote Control**: Full gRPC API for network-transparent operation

## 🚀 Try the Demo (No Hardware Required!)

**Experience rust-daq in 2 minutes** using mock devices:

```bash
# Quick demo (automated launcher)
./scripts/demo.sh

# Or manually:
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml &
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai
```

**Includes:**
- Mock stage, power meter, and camera
- Pre-configured example scripts
- GUI integration support
- Zero hardware dependencies

📖 **[Full Demo Guide](../../DEMO.md)** | 🎯 **Perfect for**: Evaluation, Learning, Development

---

## Crate Layout

- `crates/common/` — Domain types, parameters/observables, error handling, and module domain types (`modules.rs`).
- `crates/protocol/` — Protobuf definitions in `proto/` plus tonic build and domain↔proto converters in `src/convert.rs`.
- `crates/rust-daq/` — Runtime façade that wires hardware, storage, scripting, and gRPC; re-exports hardware from `hardware`.
- `crates/hardware/` — Capability traits and hardware drivers (ell14, esp300, pvcam, maitai, newport_1830c).
- `crates/bin/` — Binaries/CLI entrypoints for the headless daemon and optional GUI frontends.
- `crates/examples/` — Example binaries and integration scenarios.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     GUI (egui / eframe)                         │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐   │
│  │  Move   │ │ Camera  │ │  Scan   │ │ Modules │ │  Data   │   │
│  └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘   │
└───────┼──────────┼─────────┼─────────┼───────────┼──────────────┘
        │          │         │         │           │
        └──────────┴─────────┴─────────┴───────────┘
                             │ gRPC
┌────────────────────────────┴────────────────────────────────────┐
│                         DAQ Daemon                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │   Hardware   │  │    Plugin    │  │   Storage    │           │
│  │   Registry   │  │   Factory    │  │   Service    │           │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘           │
│         │                 │                 │                    │
│  ┌──────┴─────────────────┴─────────────────┴──────┐            │
│  │              Capability Traits                   │            │
│  │  Movable │ Readable │ Triggerable │ FrameProducer │           │
│  └──────────────────────────────────────────────────┘            │
└──────────────────────────────────────────────────────────────────┘
        │
        ▼
┌──────────────────────────────────────────────────────────────────┐
│                           Hardware                               │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐    │
│  │ ELL14   │ │ ESP300  │ │ MaiTai  │ │ PVCAM   │ │ Plugin  │    │
│  │(Thorlab)│ │(Newport)│ │ (Laser) │ │(Camera) │ │ Drivers │    │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘    │
└──────────────────────────────────────────────────────────────────┘
```

Protocol definitions now live in `crates/protocol/` (see `proto/` for `.proto` files and `src/convert.rs` for domain↔proto mappings). Module domain types reside in `crates/common/src/modules.rs` and can be enabled via the `modules` feature without requiring `networking`.

## Quick Start

### Build

```bash
# Build with default features
cargo build

# Build with all features (all storage backends and hardware drivers)
cargo build --all-features

# Build the egui desktop GUI (requires networking feature)
cargo build --features networking --bin rust-daq-gui

# Build with module system (now independent of networking)
cargo build --features modules
# Modules + networking if you need gRPC module control
cargo build --features "modules,networking"
```

**Feature profiles:** see `docs/architecture/FEATURE_MATRIX.md` for recommended feature groups and build recipes.

### Run

```bash
# Start the daemon (with networking)
cargo run --bin rust-daq-daemon --features networking -- daemon --port 50051

# Run a Rhai script
cargo run --bin rust-daq-daemon -- run examples/simple_scan.rhai

# Launch the egui GUI (connects to daemon over gRPC)
cargo run --features networking --bin rust-daq-gui
```

### Test

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --test plugin_system_integration
```

## Capability Traits

Hardware devices implement composable capability traits:

| Trait | Description | Example Devices |
|-------|-------------|-----------------|
| `Movable` | Position control | Stages, rotation mounts |
| `Readable` | Scalar measurements | Power meters, temperature sensors |
| `Settable` | Parameter adjustment | Setpoint controls |
| `Switchable` | On/off control | Shutters, heaters |
| `Triggerable` | External triggering | Cameras, pulse generators |
| `ExposureControl` | Camera exposure | Scientific cameras |
| `FrameProducer` | Image acquisition | PVCAM cameras |
| `Actionable` | One-shot commands | Home, calibrate |
| `Loggable` | Continuous logging | Any readable device |

## Hardware Drivers

### Native Drivers

| Driver | Device | Capabilities |
|--------|--------|--------------|
| `ell14` | Thorlabs Elliptec | Movable |
| `esp300` | Newport ESP300 | Movable |
| `maitai` | Spectra-Physics MaiTai | Readable, Settable |
| `pvcam` | Photometrics Cameras | FrameProducer, ExposureControl, Triggerable |
| `newport_1830c` | Newport 1830-C | Readable |

### Plugin System

Define new instruments with YAML:

```yaml
# plugins/my-device.yaml
metadata:
  id: "my-device"
  name: "My Custom Device"
  version: "1.0.0"
  driver_type: "serial_scpi"

protocol:
  baud_rate: 9600
  termination: "\r\n"

capabilities:
  readable:
    - name: "temperature"
      command: "TEMP?"
      unit: "C"
  movable:
    axes:
      - name: "x"
        unit: "mm"
        min: 0.0
        max: 100.0
    set_cmd: "POS:{axis} {val}"
    get_cmd: "POS:{axis}?"
```

## Data Pipeline ("The Mullet Strategy")

- **Party in front**: Memory-mapped ring buffer for high-throughput Arrow IPC writes (10k+ writes/sec)
- **Business in back**: Background HDF5 writer for Python/MATLAB/Igor compatibility (1 Hz flush)

```rust
// Data flows: Hardware → Ring Buffer → HDF5
let ring = RingBuffer::create("/dev/shm/daq_data", capacity)?;
ring.write(&measurements)?;  // Fast, non-blocking

// Background writer handles persistence
let writer = HDF5Writer::new(ring, output_path)?;
writer.start_background_flush(Duration::from_secs(1));
```

## gRPC Services

| Service | Description |
|---------|-------------|
| `HardwareService` | Device discovery, motion control, value streaming |
| `ScanService` | Coordinated multi-axis scanning |
| `ModuleService` | Module lifecycle and configuration |
| `PresetService` | Hardware state presets |
| `StorageService` | Recording control and data export |
| `PluginService` | Dynamic plugin loading and spawning |
| `RunEngineService` | Plan queue execution (experimental) |

## GUI Options

- **egui desktop GUI (recommended)**
  The `rust-daq-gui` binary is a native Rust/egui application that connects to the headless daemon over gRPC. It runs on macOS, Linux, and Windows without requiring Node.js or the Tauri toolchain.

- **Tauri + React GUI (legacy / optional)**  
  The `gui-tauri/` project provides a richer web‑style UI built with React and Tauri. It is still usable but no longer the primary focus. See `gui-tauri/README.md` if you specifically need a Tauri shell.

## Documentation

### Guides & References

- [Architecture Overview](./docs/architecture/)
- [CLI Guide](./docs/guides/cli_guide.md)
- [PVCAM Operator Guide](./docs/instruments/PVCAM_OPERATOR_GUIDE.md)
- [Morph Integration](./docs/MORPH_INTEGRATION.md)
- [Hybrid Search Setup](./docs/HYBRID_SEARCH_SETUP.md)

### Documentation Search (CocoIndex)

The repository includes **semantic search** over all documentation using CocoIndex. Search 55 indexed markdown files by natural language queries to quickly find architecture decisions, design patterns, and implementation guides.

**Quick Search:**
```bash
# Search documentation (auto-detects best search mode)
python scripts/search_hybrid.py --query "V5 parameter reactive system"

# Direct Python search
python -c "from cocoindex_flows.comprehensive_docs_index import search_docs; \
    print(search_docs('async hardware callbacks'))"
```

**Indexed Content:**
- Architecture decisions (ADRs)
- Implementation guides (driver development, scripting)
- Hardware protocols (MaiTai, PVCAM, Elliptec)
- API references (gRPC, Python client)
- Project management (issue tracking, deployment)

**Categories:** architecture, guides, reference, instruments, getting_started, tools, examples

See [CLAUDE.md](./CLAUDE.md#documentation-search--knowledge-base) for detailed usage.

## Project Status

**Build**: ✅ Passing
**Tests**: ✅ Passing
**Architecture**: V5 Headless-First (stable)

### Known Issues

- Frame streaming in plugin system defaults to mock mode (bd-fsx5)
- Some plugin handles require explicit mock flag (bd-dks4)

## License

Dual-licensed under MIT or Apache 2.0.
