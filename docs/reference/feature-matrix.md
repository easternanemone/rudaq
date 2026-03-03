# Feature Matrix

**Status:** Active
**Last Updated:** February 2026
**Purpose:** Single source of truth for build profiles, feature groups, and CI matrix.

## Quick Reference

```bash
# Development (fast build, mock hardware)
cargo build

# Full feature build (excludes HDF5)
cargo build --features full

# GUI development
cargo build -p ui --features standalone

# Server with all hardware
cargo build --features "server,all_hardware"
```

---

## Default Features

The default `bin` crate build provides a headless daemon:

```toml
default = ["networking", "server"]
```

- **networking**: gRPC networking layer
- **server**: Full gRPC server

---

## High-Level Profiles

Use these for common build configurations:

| Profile | Features Included | Use Case |
|---------|-------------------|----------|
| `backend` | server, modules, all_hardware, storage_csv | Headless daemon with full hardware |
| `frontend` | gui_egui, networking | Desktop GUI client |
| `cli` | all_hardware, storage_csv, scripting, scripting_python | Command-line automation |
| `full` | storage_arrow, serial, modules, all_hardware | Most features (excludes HDF5) |

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
- `storage_hdf5` propagates to `storage/storage_hdf5`
- `storage_arrow` propagates to `storage/storage_arrow` and `common/storage_arrow`

---

## Hardware Drivers

### Serial Communication

Serial port access uses `serial2_tokio` throughout the codebase.

| Feature | Description |
|---------|-------------|
| `serial` | Enable serial port support (via `hardware/serial`) |

### Device Drivers (bin crate)

Top-level feature flags on the `bin` crate:

| Feature | Description |
|---------|-------------|
| `all_hardware` | All mock-mode drivers |
| `pvcam_hardware` | Real PVCAM SDK (requires PVCAM installed) |
| `comedi_hardware` | Real Comedi DAQ (requires libcomedi) |
| `maitai` | Complete maitai lab profile (all real hardware) |

#### Drivers Metacrate Features (`crates/drivers`)

The `drivers` metacrate provides unified feature flags for all driver crates:

| Feature | Description | Driver Crate |
|---------|-------------|--------------|
| `thorlabs` | Thorlabs ELL14 rotation mounts | `driver-thorlabs` |
| `newport` | Newport ESP300 + 1830-C | `driver-newport` |
| `spectra_physics` | MaiTai laser | `driver-spectra-physics` |
| `pvcam` | PVCAM cameras (mock mode) | `driver-pvcam` |
| `pvcam_sdk` | PVCAM cameras (real SDK) | `driver-pvcam` with `pvcam_sdk` |
| `comedi` | Linux Comedi DAQ (mock mode) | `driver-comedi` |
| `comedi_hardware` | Linux Comedi DAQ (real hardware) | `driver-comedi` with `hardware` |
| `mock` | Mock hardware for testing | `driver-mock` |
| `generic` | Config-driven serial driver (v2) | `driver-generic` |
| `all` | All drivers with mock implementations | All of the above |
| `maitai` | Maitai lab hardware profile | thorlabs + newport + spectra_physics + pvcam_sdk |
| `hardware` | All drivers with real hardware | All + pvcam_sdk + comedi_hardware |

**Note:** `driver-andor-sdk3`, `driver-dover-motion`, and `driver-universal` are not yet integrated into the `drivers` metacrate. They are available as direct workspace dependencies. `driver-universal` (schema v3) is the successor to `driver-generic` (schema v2) and is wired into the hardware registry via `load_all_factories()`.

### Camera Hardware

| Feature | Description | Requirements |
|---------|-------------|--------------|
| `pvcam_sdk` | Real PVCAM hardware support | PVCAM SDK installed, `PVCAM_SDK_DIR` set |
| `hardware_tests` | Enable hardware-in-the-loop tests | Physical devices connected |
| `prime_95b_tests` | Prime 95B camera tests (1200x1200) | Alternative to Prime BSI (2048x2048) |

---

## System Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `networking` | gRPC networking layer | None (base for server) |
| `server` | Full gRPC server | `server`, includes `networking` |
| `scripting` | Rhai scripting engine | `scripting` |
| `scripting_python` | Python bindings for scripting | `scripting/python` (PyO3) |
| `gui_egui` | Desktop GUI application | `egui`, `eframe`, `egui_plot`, `egui_extras` |
| `modules` | Module system with runtime assignment | Requires `scripting` |
| `plugins_hot_reload` | Hot reload plugin configs | `notify` crate |

**Plugin System Notes:**
- `scripting` enables Rhai-based script plugins in `daq-modules/src/plugins/`

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
source scripts/env-check.sh
cargo nextest run --profile hardware --features hardware_tests

# Specific driver tests
cargo nextest run -p driver-thorlabs --features hardware
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
bin crate features:
  maitai → pvcam_hardware + hardware/maitai
  pvcam_hardware → pvcam_sdk → hardware/pvcam_sdk
  comedi_hardware → hardware/comedi_hardware
  all_hardware → hardware/all_hardware
  full → storage_arrow + serial + modules + all_hardware
  backend → modules + all_hardware
  modules → dep:daq-modules

drivers metacrate features:
  maitai → thorlabs + newport + spectra_physics + pvcam_sdk + serial + comedi
  all → mock + thorlabs + newport + spectra_physics + pvcam + comedi + generic
  thorlabs → dep:driver-thorlabs
  newport → dep:driver-newport
  spectra_physics → dep:driver-spectra-physics
  pvcam → dep:driver-pvcam
  pvcam_sdk → pvcam + driver-pvcam/pvcam_sdk
  comedi → dep:driver-comedi
  comedi_hardware → comedi + driver-comedi/hardware
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
Ensure windowing dependencies are installed. See [Platform Notes](../how-to/platform-notes.md).
