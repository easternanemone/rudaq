# rust-daq Documentation Hub

Welcome to the comprehensive documentation for the rust-daq data acquisition system. This page serves as your entry point to all available documentation, guides, and reference materials.

## Quick Links

**Just getting started?** Begin here:
- [Demo Mode Guide](../DEMO.md) - Try rust-daq without hardware (2 minutes)
- [Quick Start Setup](../README.md) - Installation and basic usage
- [Hardware Setup (Maitai)](MAITAI_SETUP.md) - Real hardware configuration

---

## Documentation Categories

### Getting Started

Learn the basics of rust-daq and get your first experiment running.

| Resource | Purpose |
|----------|---------|
| [Demo Mode Guide](../DEMO.md) | Run rust-daq with mock hardware in 30 seconds |
| [Quick Start Setup](../README.md) | Install Rust, build the daemon, and explore features |
| [Build Verification](BUILD_VERIFICATION.md) | Verify your build is correct and complete |

### Architecture & Design

Understand the system design, decision-making, and architectural patterns.

| Resource | Focus | Status |
|----------|-------|--------|
| [System Architecture](architecture/ARCHITECTURE.md) | Complete system overview, component diagrams, data flow | Current |
| [Feature Matrix](architecture/FEATURE_MATRIX.md) | Implementation status for all major features | Current |
| ADRs (Architecture Decision Records) | Specific design decisions and rationale | |
| [ADR: PVCAM Continuous Acquisition](architecture/adr-pvcam-continuous-acquisition.md) | Camera frame buffering strategies and performance tradeoffs | Active |
| [ADR: PVCAM Driver Architecture](architecture/adr-pvcam-driver-architecture.md) | Multi-layer PVCAM driver design patterns | Active |
| [ADR: Pool Migration Results](architecture/adr-pvcam-pool-migration-results.md) | Object pool performance improvements | Complete |
| [ADR: Connection Reliability](architecture/adr-connection-reliability.md) | Serial device connection handling and robustness | Current |
| [ADR: gRPC Validation Layer](architecture/adr-grpc-validation-layer.md) | Network protocol validation strategy | Current |
| [ADR: Pool Error Handling](architecture/adr-pool-error-handling.md) | Error management in high-performance buffers | Active |
| [Performance Analysis: PVCAM Gap](architecture/analysis-pvcam-performance-gap.md) | Benchmarking results and optimization opportunities | Reference |
| [Integration Map](architecture/pvcam-integration-map.md) | Visual mapping of PVCAM driver integration points | Reference |

### User Guides

Step-by-step guides for common tasks and workflows.

| Guide | Purpose | Audience |
|-------|---------|----------|
| [Scripting Guide](guides/scripting.md) | Write Rhai scripts to control hardware and automate experiments | Experiment Users |
| [Storage Formats Guide](guides/storage-formats.md) | Choose and configure data storage (HDF5, Arrow, CSV, NetCDF) | Data Scientists |
| [Hardware Drivers Guide](guides/hardware-drivers.md) | Understand and configure hardware drivers for motion, lasers, and sensors | Hardware Integration |
| [Testing Guide](guides/testing.md) | Run tests, verify features, debug issues | Developers |

### Developer Guides

Technical deep-dives for extending and maintaining rust-daq.

| Guide | Coverage | Level |
|-------|----------|-------|
| [Plugin System](plugins/README.md) | Build custom hardware drivers and instrument plugins | Intermediate |
| [Plugin Quick Start](plugins/QUICK_START.md) | Minimal example for creating your first plugin | Beginner |
| [Hardware Drivers Guide](guides/hardware-drivers.md) | Implement new serial drivers, handle device communication | Advanced |

### API Reference

Interface documentation for using rust-daq.

| Resource | Type | Details |
|----------|------|---------|
| [gRPC API](../docs/) | Network Protocol | Remote control, streaming, device management |
| Rust Crates | Embedded | See per-crate documentation below |
| [Python Client](../python/) | Python Bindings | PyO3-based Python integration |

### Hardware Reference

Device-specific setup, configuration, and troubleshooting.

| Device | Setup Guide | Status | Notes |
|--------|------------|--------|-------|
| **Photometrics Cameras** | [PVCAM Setup](troubleshooting/PVCAM_SETUP.md) | Active | Prime 95B, Prime BSI |
| **Maitai Hardware Stack** | [Maitai Setup](MAITAI_SETUP.md) | Current | Multi-device system at maitai machine |
| **Linux Comedi DAQ** | [Build Verification](BUILD_VERIFICATION.md) | Current | NI PCI-MIO-16XE-10 integration |
| **Serial Devices** | [Hardware Drivers Guide](guides/hardware-drivers.md) | Current | Thorlabs, Newport, Spectra Physics |
| **Platform Notes** | [Platform Guide](troubleshooting/PLATFORM_NOTES.md) | Reference | OS-specific considerations |

### Troubleshooting & Reference

Solutions for common issues and detailed reference material.

| Resource | Purpose |
|----------|---------|
| [PVCAM SDK Reference](reference/PVCAM_SDK_REFERENCE.md) | Complete PVCAM API reference and error codes |
| [Rerun Refactoring Guide](architecture/rerun-refactoring-guide.md) | Visualization debugging with Rerun.io |

### Phase & Release Documentation

Historical and operational documentation for releases and phases.

| Document | Purpose | Status |
|----------|---------|--------|
| [Phase 1-2 Test Results](PHASE1_PHASE2_TEST_RESULTS.md) | Baseline functionality testing | Complete |
| [Phase 3 Test Results](PHASE3_TEST_RESULTS.md) | Advanced features validation | Complete |
| [Phase 4 Test Results](PHASE4_TEST_RESULTS.md) | Final integration testing | Complete |
| [v0.6.0 Completion Summary](project_management/v0.6.0_completion_summary.md) | Release notes and features | Archive |
| [v0.6.0 Validation](project_management/v0.6.0_final_validation.md) | Pre-release verification | Archive |

---

## Per-Crate Documentation

The workspace is organized into specialized crates. Each has its own detailed README:

### Domain Logic & Automation

| Crate | Purpose | README |
|-------|---------|--------|
| **daq-scripting** | Rhai scripting engine for automation | [daq-scripting README](../crates/daq-scripting/README.md) |
| **daq-experiment** | RunEngine and experiment orchestration | Embedded in core |

### Hardware & Drivers

| Crate | Purpose | README |
|-------|---------|--------|
| **common** | Foundation types, parameters, error handling | [common README](../crates/common/README.md) |
| **daq-hardware** | Hardware abstraction layer and device registry | [daq-hardware README](../crates/daq-hardware/README.md) |
| **daq-driver-pvcam** | Photometrics PVCAM camera driver | [daq-driver-pvcam README](../crates/daq-driver-pvcam/README.md) |
| **daq-driver-comedi** | Linux Comedi DAQ board driver | [daq-driver-comedi README](../crates/daq-driver-comedi/README.md) |
| **daq-driver-thorlabs** | Thorlabs ELL14 rotator driver | [daq-driver-thorlabs README](../crates/daq-driver-thorlabs/README.md) |
| **daq-driver-newport** | Newport ESP300 motion and 1830-C power meter | [daq-driver-newport README](../crates/daq-driver-newport/README.md) |
| **daq-driver-spectra-physics** | Spectra Physics MaiTai laser driver | [daq-driver-spectra-physics README](../crates/daq-driver-spectra-physics/README.md) |
| **daq-driver-mock** | Mock drivers for testing | [daq-driver-mock README](../crates/daq-driver-mock/README.md) |
| **daq-driver-red-pitaya** | Red Pitaya FPGA board support | [daq-driver-red-pitaya README](../crates/daq-driver-red-pitaya/README.md) |

### Infrastructure & Storage

| Crate | Purpose | README |
|-------|---------|--------|
| **daq-storage** | Data persistence (HDF5, Arrow, CSV, NetCDF) | Embedded in core |
| **daq-pool** | High-performance object pool for frame handling | [daq-pool README](../crates/daq-pool/README.md) |
| **daq-proto** | Protobuf definitions and gRPC interfaces | Embedded in core |
| **daq-plugin-api** | Native plugin FFI system (abi_stable) | Embedded in core |

### User Interfaces & Servers

| Crate | Purpose | README |
|-------|---------|--------|
| **daq-server** | gRPC server with auth and streaming | Embedded in core |
| **daq-egui** | Desktop GUI (egui + egui_dock) | [daq-egui README](../crates/daq-egui/README.md) |
| **daq-bin** | CLI and daemon entry points | Part of main README |

### Integration

| Crate | Purpose | README |
|-------|---------|--------|
| **rust-daq** | Workspace facade and prelude | [rust-daq README](../crates/rust-daq/README.md) |

---

## Navigation by Task

### I want to...

**Run an experiment:**
1. [Try the demo](../DEMO.md) to understand the workflow
2. [Write a Rhai script](guides/scripting.md) to control hardware
3. [Configure your hardware](MAITAI_SETUP.md) or use mock devices

**Understand the system:**
1. Start with [Quick Start](../README.md) for a high-level overview
2. Read [System Architecture](architecture/ARCHITECTURE.md) for detailed design
3. Explore ADRs for specific design decisions

**Extend rust-daq:**
1. [Build a plugin](plugins/QUICK_START.md) using the plugin system
2. [Implement a driver](guides/hardware-drivers.md) for a new instrument
3. Follow [Hardware Drivers Guide](guides/hardware-drivers.md) for serial device patterns

**Troubleshoot problems:**
1. [Check platform notes](troubleshooting/PLATFORM_NOTES.md) for OS-specific issues
2. [Verify your build](BUILD_VERIFICATION.md) is correct
3. Review [PVCAM setup](troubleshooting/PVCAM_SETUP.md) if using cameras
4. Check [Hardware setup guide](MAITAI_SETUP.md) for device configuration

**Contribute to development:**
1. Read [Testing Guide](guides/testing.md) to run tests
2. Review relevant ADR for the component you're working on
3. Follow Rust code style and feature flags

---

## Documentation Status Legend

- **Current** - Actively maintained, up-to-date with latest code
- **Active** - In active use, may receive updates for new features
- **Complete** - Feature-complete, receives only bug fixes
- **Archive** - Historical reference, may be outdated
- **Reference** - Optional reference material

---

## Getting Help

**Can't find what you're looking for?**

1. Check the [Feature Matrix](architecture/FEATURE_MATRIX.md) for implementation status
2. Review the [Quick Links](#quick-links) at the top of this page
3. Explore the relevant crate README from [Per-Crate Documentation](#per-crate-documentation)
4. Check [Troubleshooting](#troubleshooting--reference) for common issues

**For architecture questions:**
- Review the relevant ADR in the [Architecture & Design](#architecture--design) section
- Check the [integration maps](architecture/pvcam-integration-map.md) for complex subsystems

**For hardware setup:**
- [Maitai Hardware Setup](MAITAI_SETUP.md) covers multi-device integration
- Device-specific guides in [Hardware Reference](#hardware-reference)

---

## Project Structure

```
docs/
├── README.md                          (You are here - Navigation hub)
├── MAITAI_SETUP.md                    (Real hardware configuration)
├── BUILD_VERIFICATION.md              (Build validation guide)
├── architecture/                      (Design decisions & system design)
│   ├── ARCHITECTURE.md                (System overview)
│   ├── FEATURE_MATRIX.md              (Implementation status)
│   ├── adr-*.md                       (Architecture Decision Records)
│   ├── analysis-*.md                  (Performance & analysis)
│   └── *-integration-map.md           (Component integration diagrams)
├── guides/                            (Step-by-step tutorials)
│   ├── scripting.md                   (Rhai experiment scripts)
│   ├── storage-formats.md             (Data storage options)
│   ├── hardware-drivers.md            (Driver implementation)
│   └── testing.md                     (Testing guide)
├── plugins/                           (Plugin system documentation)
│   ├── README.md                      (Plugin overview)
│   └── QUICK_START.md                 (Minimal plugin example)
├── troubleshooting/                   (Issue resolution)
│   ├── PVCAM_SETUP.md                 (Camera configuration)
│   └── PLATFORM_NOTES.md              (OS-specific notes)
├── reference/                         (Technical reference)
│   └── PVCAM_SDK_REFERENCE.md         (PVCAM API documentation)
└── project_management/                (Release & planning docs)
    └── *.md                           (v0.6.0 releases, roadmap)

crates/
├── common/README.md                 (Foundation types)
├── daq-hardware/README.md             (HAL and device registry)
├── daq-scripting/README.md            (Rhai scripting)
├── daq-pool/README.md                 (Object pool)
├── daq-egui/README.md                 (Desktop GUI)
├── daq-driver-*/README.md             (Device drivers)
└── rust-daq/README.md                 (Integration facade)
```

---

## Contributing to Documentation

Documentation is a critical part of rust-daq. When contributing:

1. **Keep docs with code** - Update documentation when you modify features
2. **Use clear language** - Assume readers are unfamiliar with your code
3. **Include examples** - Show how to use new features
4. **Update the hub** - Link new docs from this README
5. **Mark status** - Indicate whether docs are current, archive, or WIP

See individual guide READMEs for contribution guidelines.

---

## Quick Reference Commands

```bash
# Run the demo (no hardware needed)
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Build for real hardware (Maitai)
bash scripts/build-maitai.sh

# Run tests
cargo nextest run

# Format and lint
cargo fmt --all && cargo clippy --all-targets
```

For more commands, see [README.md](../README.md).

---

**Last Updated:** 2026-01-25
**Coverage:** All major documentation categories
**Status:** Current
