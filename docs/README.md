# rust-daq Documentation Hub

Welcome to the complete documentation for **rust-daq**, a modular, high-performance Data Acquisition system written in Rust. This page serves as your entry point to all available guides, architecture docs, and reference materials.

---

## Getting Started (New Users)

If you're new to rust-daq, start here:

- **[Main README](../README.md)** - Project overview, features, and quick build instructions
- **[DEMO.md](../DEMO.md)** - Try rust-daq without hardware in 2 minutes using mock devices
- **[Scripting Guide](guides/scripting.md)** - Learn Rhai scripting to define experiments

### For Maitai Lab Users

- **[MAITAI_SETUP.md](MAITAI_SETUP.md)** - Hardware-specific build and setup
- **[CLAUDE.md](../CLAUDE.md)** - Developer workflow and hardware inventory

---

## Architecture & Design Decisions

Understanding system design through decision documentation:

- **[System Architecture](architecture/ARCHITECTURE.md)** - Overall system design and data flow
- **[Feature Matrix](architecture/FEATURE_MATRIX.md)** - Cargo features and build profiles

### Architecture Decision Records (ADRs)

Key design decisions and rationales:

- **[PVCAM Continuous Acquisition](architecture/adr-pvcam-continuous-acquisition.md)** - Camera buffer modes and streaming strategy
- **[PVCAM Driver Architecture](architecture/adr-pvcam-driver-architecture.md)** - Multi-layer driver design
- **[Connection Reliability](architecture/adr-connection-reliability.md)** - Network and reconnection strategies
- **[gRPC Validation Layer](architecture/adr-grpc-validation-layer.md)** - API request validation
- **[Pool Error Handling](architecture/adr-pool-error-handling.md)** - Buffer pool robustness
- **[Pool Migration Results](architecture/adr-pvcam-pool-migration-results.md)** - Performance optimization outcomes

---

## User Guides

Practical guides for common tasks:

- **[Testing Guide](guides/testing.md)** - Test runner setup, timing tests, hardware validation
- **[Scripting Guide](guides/scripting.md)** - Rhai DSL for experiment definition
- **[Hardware Drivers](guides/hardware-drivers.md)** - Supported hardware and driver development
- **[Storage Formats](guides/storage-formats.md)** - Data persistence options (CSV, HDF5, Arrow)

---

## Crate Documentation

Per-crate reference for understanding individual components:

### Core System
- **[daq-core](../crates/daq-core/README.md)** - Foundation types, errors, and size limits
- **[daq-hardware](../crates/daq-hardware/README.md)** - Hardware abstraction layer with capability traits

### Experiment Engine
- **[daq-experiment](../crates/daq-experiment/README.md)** - RunEngine and Plan orchestration
- **[daq-scripting](../crates/daq-scripting/README.md)** - Rhai scripting integration

### Data & Storage
- **[daq-storage](../crates/daq-storage/README.md)** - Ring buffers and data persistence
- **[daq-proto](../crates/daq-proto/README.md)** - Protocol Buffer definitions

### Server & API
- **[daq-server](../crates/daq-server/README.md)** - gRPC server implementation
- **[daq-pool](../crates/daq-pool/README.md)** - Thread pool for async operations

### User Interface
- **[daq-egui](../crates/daq-egui/README.md)** - Desktop GUI application
- **[daq-bin](../crates/daq-bin/README.md)** - CLI binaries and daemon entry points

### Hardware Drivers
- **[daq-driver-pvcam](../crates/daq-driver-pvcam/README.md)** - Photometrics camera support
- **[daq-driver-comedi](../crates/daq-driver-comedi/README.md)** - Linux DAQ boards
- **[daq-driver-thorlabs](../crates/daq-driver-thorlabs/README.md)** - ELL14 rotators
- **[daq-driver-newport](../crates/daq-driver-newport/README.md)** - ESP300 motion controller
- **[daq-driver-spectra-physics](../crates/daq-driver-spectra-physics/README.md)** - MaiTai laser
- **[daq-driver-mock](../crates/daq-driver-mock/README.md)** - Mock devices for simulation

### Integration
- **[rust-daq](../crates/rust-daq/README.md)** - Prelude and convenience layer

---

## Hardware Reference

### Supported Hardware

Comprehensive hardware support table and setup instructions:

- **[Platform Notes](troubleshooting/PLATFORM_NOTES.md)** - Linux, macOS, and Windows specifics
- **[PVCAM Setup](troubleshooting/PVCAM_SETUP.md)** - Camera configuration and SDK setup
- **[MAITAI_SETUP.md](MAITAI_SETUP.md)** - Maitai lab machine hardware inventory
- **[CLAUDE.md Hardware Section](../CLAUDE.md#hardware-inventory-maitai)** - Stable device paths and verification

### Inventory (maitai machine)

Located in [CLAUDE.md](../CLAUDE.md):
- Photometrics Prime BSI camera (PVCAM)
- Spectra-Physics MaiTai Ti:Sapphire laser
- Newport 1830-C power meter
- Newport ESP300 motion controller
- Thorlabs ELL14 rotators (3x)
- NI PCI-MIO-16XE-10 DAQ card (Comedi)
- Photodiode signal input

---

## API Reference

### Client Libraries

- **[Python Client](../python/README.md)** - Python bindings and example usage

### Protocol Buffers

Protocol definitions for gRPC API:
- Located in `proto/` directory
- Covers device control, data streaming, and experiment lifecycle

---

## Building & Deployment

### Build Instructions

- **[Main README Build Section](../README.md#-getting-started)** - Standard builds
- **[Feature Matrix](architecture/FEATURE_MATRIX.md)** - Feature flags and profiles
- **[MAITAI_SETUP.md](MAITAI_SETUP.md)** - Production hardware build

### Environment Setup

Required for PVCAM and hardware builds:
- See [CLAUDE.md Environment Setup](../CLAUDE.md#environment-setup)
- Use `source scripts/env-check.sh` for automatic configuration

---

## Performance & Benchmarks

- **[Throughput & Latency](benchmarks/tee.md)** - Data pipeline performance metrics

---

## Troubleshooting

- **[Platform Notes](troubleshooting/PLATFORM_NOTES.md)** - OS-specific setup issues
- **[Build Verification](BUILD_VERIFICATION.md)** - Verifying correct hardware initialization
- **[Common Pitfalls](../CLAUDE.md#common-pitfalls)** - Debugging tips in CLAUDE.md

---

## Project Status

**Last Updated:** January 2026

Documentation covers:
- Architecture Version 5 (complete)
- Core system stable
- All documented hardware drivers supported
- Ongoing performance optimizations

For development workflow, issue tracking, and multi-Claude coordination, see [CLAUDE.md](../CLAUDE.md).

---

## Quick Navigation

| I want to... | Start here |
|-------------|-----------|
| Try rust-daq without hardware | [DEMO.md](../DEMO.md) |
| Understand system design | [ARCHITECTURE.md](architecture/ARCHITECTURE.md) |
| Write experiment scripts | [scripting.md](guides/scripting.md) |
| Add new hardware | [hardware-drivers.md](guides/hardware-drivers.md) |
| Run tests | [testing.md](guides/testing.md) |
| Deploy to maitai | [MAITAI_SETUP.md](MAITAI_SETUP.md) |
| Debug issues | [CLAUDE.md troubleshooting](../CLAUDE.md#when-stuck) |
