# Feature Matrix

**Status:** Active
**Last Updated:** January 2026
**Purpose:** Single source of truth for build profiles, feature groups, and CI matrix.

## Quick Reference

```bash
# Development (fast build, mock hardware)
cargo build

# Full feature build (excludes HDF5)
cargo build --features full

# GUI development
cargo build -p daq-egui --features standalone

# Server with all hardware
cargo build --features "server,all_hardware"
```

---

## Default Features

The default build provides a minimal headless setup:

```toml
default = ["storage_csv", "instrument_serial"]
```

- **storage_csv**: CSV data export
- **instrument_serial**: Basic serial port support

---

## High-Level Profiles

Use these for common build configurations:

| Profile | Features Included | Use Case |
|---------|-------------------|----------|
| `backend` | server, modules, all_hardware, storage_csv | Headless daemon with full hardware |
| `frontend` | gui_egui, networking | Desktop GUI client |
| `cli` | all_hardware, storage_csv, scripting, scripting_python | Command-line automation |
| `full` | storage_csv, storage_arrow, storage_matlab, instrument_serial, modules, server, all_hardware | Most features (excludes HDF5) |

**Note:** `storage_hdf5` is intentionally excluded from `full` because it requires native HDF5 libraries. Enable explicitly when available.

---

## Storage Backends

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `storage_csv` | CSV file export (default) | `csv` crate |
| `storage_hdf5` | HDF5 scientific format | `hdf5-metno`, requires `libhdf5-dev` |
| `storage_arrow` | Apache Arrow IPC format | `arrow` crate |
| `storage_matlab` | MATLAB .mat files | `matrw` crate |

**Storage Feature Propagation:**
- `storage_hdf5` propagates to `daq-storage/storage_hdf5`
- `storage_arrow` propagates to `daq-storage/storage_arrow` and `daq-core/storage_arrow`

---

## Hardware Drivers

### Serial Communication

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `instrument_serial` | Synchronous serial port (default) | `serialport` |
| `tokio_serial` | Async serial port (recommended) | `tokio-serial`, includes `instrument_serial` |
| `instrument_visa` | VISA instrument control | `visa-rs` |

### Device Drivers

| Feature | Description | Propagates To |
|---------|-------------|---------------|
| `instrument_thorlabs` | Thorlabs ELL14 rotators | `daq-hardware/driver-thorlabs` |
| `instrument_newport` | Newport ESP300 controller | `daq-hardware/driver-newport` |
| `instrument_photometrics` | PVCAM camera support | `daq-hardware/instrument_photometrics` |
| `instrument_spectra_physics` | MaiTai laser | `daq-hardware/driver-spectra-physics` |
| `instrument_newport_power_meter` | Newport 1830-C | tokio_serial only |
| `all_hardware` | All above drivers | All driver features |

### Camera Hardware

| Feature | Description | Requirements |
|---------|-------------|--------------|
| `pvcam_hardware` | Real PVCAM hardware support | PVCAM SDK installed, `PVCAM_SDK_DIR` set |
| `hardware_tests` | Enable hardware-in-the-loop tests | Physical devices connected |
| `prime_95b_tests` | Prime 95B camera tests (1200x1200) | Alternative to Prime BSI (2048x2048) |

---

## System Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `networking` | gRPC networking layer | None (base for server) |
| `server` | Full gRPC server | `daq-server`, includes `networking` |
| `scripting` | Rhai scripting engine | `daq-scripting` |
| `scripting_python` | Python bindings for scripting | `daq-scripting/python` (PyO3) |
| `gui_egui` | Desktop GUI application | `egui`, `eframe`, `egui_plot`, `egui_extras` |
| `modules` | Module system with runtime assignment | Requires `scripting` |
| `plugins_hot_reload` | Hot reload plugin configs | `notify` crate |

---

## Recommended Build Profiles

### For Development

```bash
# Fast iteration (defaults only)
cargo build

# With GUI for testing
cargo build --features gui_egui

# Full feature testing
cargo build --features full
```

### For Deployment

```bash
# Headless server
cargo build --release --features backend

# GUI operator workstation
cargo build --release --features "frontend,storage_csv"

# Full lab system (with HDF5)
cargo build --release --features "full,storage_hdf5"
```

### For Hardware Testing

```bash
# Mock hardware (no physical devices)
cargo test

# Real hardware on maitai
cargo test --features "hardware_tests,pvcam_hardware" -- --nocapture

# Specific driver tests
cargo test --features "instrument_thorlabs,hardware_tests"
```

---

## CI Build Matrix

The CI system tests these combinations:

| Job | Features | Purpose |
|-----|----------|---------|
| **check-fast** | defaults | Quick compilation check |
| **test-core** | defaults | Unit tests without hardware |
| **test-storage** | storage_csv, storage_arrow | Storage backend tests |
| **test-server** | server, scripting | gRPC + scripting tests |
| **lint-all** | full | Clippy with most features |
| **format** | - | cargo fmt check |

**Note:** HDF5 and PVCAM tests run only on dedicated hardware runners.

---

## Feature Dependencies

```
server
  └── networking
  └── daq-server (optional dep)
  └── tokio/full

modules
  └── scripting

scripting
  └── daq-scripting (optional dep)

scripting_python
  └── daq-scripting/python

pvcam_hardware
  └── daq-hardware/pvcam_hardware
  └── instrument_photometrics

instrument_thorlabs
  └── tokio_serial
  └── daq-hardware/driver-thorlabs

tokio_serial
  └── instrument_serial
  └── daq-hardware/tokio_serial
```

---

## Platform-Specific Notes

### Linux
- All features supported
- GUI requires: `libxkbcommon-dev`, `libwayland-dev`, `libxcb-shape0-dev`
- HDF5 requires: `libhdf5-dev`
- PVCAM requires: PVCAM SDK from Photometrics

### macOS
- Most features supported
- No PVCAM support (Linux-only SDK)
- No Comedi support (Linux-only)
- HDF5 via Homebrew: `brew install hdf5`

### Windows
- Core features supported
- GUI supported via Win32
- Serial ports work with appropriate drivers
- No PVCAM support
- No Comedi support

---

## Troubleshooting

### "Feature X not found"
Ensure you're in the correct crate directory. Many features are defined on `rust-daq`, not on individual crates.

### HDF5 build fails
Install system HDF5 libraries:
```bash
# Debian/Ubuntu
sudo apt install libhdf5-dev

# Fedora
sudo dnf install hdf5-devel

# macOS
brew install hdf5
```

### PVCAM build fails
Set environment variables:
```bash
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export LD_LIBRARY_PATH=$PVCAM_LIB_DIR:$LD_LIBRARY_PATH
```

### GUI doesn't compile
Ensure windowing dependencies are installed. See [Platform Notes](../troubleshooting/PLATFORM_NOTES.md).
