# Feature Matrix

**Status:** Active
**Last Updated:** March 2026
**Purpose:** Single source of truth for build profiles, feature groups, and CI matrix.

## Quick Reference

```bash
# Development (fast build, mock hardware, daemon)
cargo build -p bin

# Full feature build (excludes HDF5)
cargo build -p bin --features full

# GUI development (native standalone app)
cargo build -p ui --features standalone

# Daemon with all hardware
cargo build -p bin --features all_hardware

# WASM GUI for browser
cargo build -p ui --target wasm32-unknown-unknown --no-default-features --features web
```

---

## Default Features

The default `bin` crate build provides a headless daemon:

```toml
default = ["networking", "server", "db-surreal-mem"]
```

- **networking**: gRPC networking layer
- **server**: Full gRPC server
- **db-surreal-mem**: In-memory SurrealDB for device/experiment metadata

---

## High-Level Profiles

Use these for common build configurations:

| Profile | Features Included | Use Case |
|---------|-------------------|----------|
| `backend` | modules, all_hardware | Headless daemon with full hardware |
| `cli` | all_hardware, scripting_python | Command-line automation |
| `full` | storage_arrow, serial, modules, all_hardware | Most features (excludes HDF5) |
| `production` | db-surreal-rocksdb, modules, all_hardware | Production with persistent DB |
| `leabs` | andor (mock) | Leabs lab profile (mock mode) |
| `leabs_hardware` | andor_hardware | Leabs lab profile (real Andor SDK3) |

**Note:** `storage_hdf5` is intentionally excluded from `full` because it requires native HDF5 libraries. Enable explicitly when available.

---

## Storage Backends

These storage features are owned by the `storage` crate. Some are passed through by `bin`, `server`, or `integration-tests`, but not every storage feature is exposed at every layer.

| Feature | Crate Owner | Description | Dependencies |
|---------|-------------|-------------|--------------|
| `storage_hdf5` | `storage` | HDF5 scientific format | `hdf5-metno`, requires `libhdf5-dev` |
| `storage_arrow` | `storage` | Apache Arrow IPC format | `arrow` crate |
| `storage_parquet`| `storage` | Apache Parquet format | `parquet` (requires `storage_arrow`) |
| `storage_tiff` | `storage` | TIFF image export | `image` crate |
| `storage_zarr` | `storage` | Zarr V3 array storage | `zarrs` crate |

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

| Feature | Crate Owner | Description |
|---------|-------------|-------------|
| `all_hardware` | `bin` | Enable all native driver crates in default mode |
| `pvcam_hardware` | `bin` | Real PVCAM SDK (via `driver-pvcam/hardware`) |
| `comedi_hardware` | `bin` | Real Comedi DAQ (via `driver-registry/comedi_hardware`) |
| `maitai` | `bin` | Complete maitai lab profile |

#### Driver Registry Features (`crates/driver-registry`)

The `driver-registry` crate provides unified feature flags for all hardware drivers:

| Feature | Description | Driver Crate |
|---------|-------------|--------------|
| `serial` | Serial port support (default) | `hardware/serial` |
| `pvcam` | PVCAM cameras (mock mode) | `driver-pvcam` |
| `pvcam_sdk` | PVCAM cameras (real SDK) | `driver-pvcam` with `pvcam_sdk` |
| `pvcam_hardware` | PVCAM cameras (real hardware) | Alias for `pvcam_sdk` |
| `comedi` | Linux Comedi DAQ (mock mode) | `driver-comedi` |
| `comedi_hardware` | Linux Comedi DAQ (real hardware) | `driver-comedi` with `hardware` |
| `andor` | Andor SDK3 cameras (mock mode) | `driver-andor-sdk3` |
| `andor_hardware` | Andor SDK3 cameras (real hardware) | `driver-andor-sdk3` with `hardware` |
| `all_hardware` | All drivers with mock implementations | `pvcam` + `comedi` + `andor` |
| `full` | Full feature set | Alias for `all_hardware` |

**Always included (non-optional):**
- `driver-mock` — Mock hardware for testing
- `driver-universal` — TOML-manifest driven driver (schema v3)

**Manifest-based devices** (via `driver-universal`, config files in `config/devices/`):
- Thorlabs ELL14 rotation mounts (`ell14.toml`)
- Newport ESP300/ESP301 motion controllers (`esp300.toml`, `esp301_example.toml`)
- Newport 1830-C power meter (`newport_1830c.toml`)
- Spectra-Physics MaiTai laser (`maitai.toml`)
- Thorlabs PM400 power meter (`thorlabs_pm400.toml`)
- IPG laser controllers (`ipg_laser.toml`)
- Red Pitaya PID controllers (`red_pitaya_pid.toml`)
- Generic Modbus devices (`modbus_example.toml`)

These devices require **no feature flags** — they load at runtime via TOML config.

### Camera Hardware

| Feature | Description | Requirements |
|---------|-------------|--------------|
| `pvcam_sdk` | Real PVCAM hardware support | PVCAM SDK installed, `PVCAM_SDK_DIR` set |
| `hardware_tests` | Enable hardware-in-the-loop tests | Physical devices connected |


---

## System Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `networking` | gRPC networking layer | None (base for server) |
| `server` | Full gRPC server | `server`, includes `networking` |
| `modules` | Module system with runtime assignment | `daq-modules` |
| `scripting_python` | Python bindings for scripting | `scripting/python` (PyO3) |
| `db-surreal-mem` | In-memory SurrealDB (default) | `db` with kv-mem |
| `db-surreal-rocksdb` | Persistent SurrealDB with RocksDB | `db` with kv-rocksdb |
| `production` | Production profile | `db-surreal-rocksdb` + `modules` + `all_hardware` |
| `metrics` | Prometheus metrics for observability | `prometheus` |

**GUI Applications** (separate `ui` crate):
- `standalone` — Native desktop GUI (default, uses `eframe` + `egui`)
- `web` — WASM browser GUI (same panels as standalone)
- `rerun_viewer` — Embedded Rerun viewer with camera streaming

**Scripting:**
- Base Rhai scripting is always available via `scripting` crate
- `scripting_python` enables PyO3 Python bindings (optional)

---

## Recommended Build Profiles

### For Development

```bash
# Fast iteration (daemon, defaults only)
cargo build -p bin

# Native GUI for testing
cargo build -p ui

# Full feature testing (daemon)
cargo build -p bin --features full
```

### For Deployment

```bash
# Headless daemon (backend profile)
cargo build --release -p bin --features backend

# GUI operator workstation
cargo build --release -p ui

# Production daemon (with RocksDB persistence)
cargo build --release -p bin --features production

# Full lab system (with HDF5)
cargo build --release -p bin --features "full,storage_hdf5"
```

### For Hardware Testing

```bash
# Mock hardware (no physical devices)
cargo test

# Real hardware on maitai
source scripts/env-check.sh
cargo nextest run --profile hardware --features hardware_tests

# Specific driver tests
cargo nextest run -p driver-pvcam --features pvcam_sdk
```

---

## CI Build Matrix

The CI system tests these combinations:

**CI workflow (`ci.yml`):**

| Step | Purpose |
|------|---------|
| **Format check** | `cargo fmt --all -- --check` |
| **Clippy** | Workspace-wide lint |
| **Unit & integration tests** | `cargo nextest run` (default features) |
| **Dependency hygiene + SBOM** | `cargo-audit`, `cargo-deny`, `cargo-machete`, plus CycloneDX SBOM on push; `cargo-deny` excludes the evaluation-only `ui-slint` crate pending upstream license review |
| **Performance regression gate** | Checked ring buffer + Hg2 extraction benchmarks vs committed baseline |
| **Ring buffer benchmark** | Performance regression check |
| **SBOM generation** | CycloneDX bill of materials |

**Feature Matrix workflow (`feature-matrix.yml`):**

| Job | Features | Purpose |
|-----|----------|---------|
| **storage / hdf5** | storage_hdf5 | HDF5 storage backend |
| **storage / arrow** | storage_arrow | Arrow IPC storage backend |
| **db / rocksdb** | kv-rocksdb | RocksDB persistence |
| **bin / all_hardware mock** | all_hardware | All mock drivers compile |
| **server / full stack** | modules, scripting, storage_hdf5, storage_arrow | Full server features |
| **runtime / universal-smoke** | universal | driver-universal smoke test |
| **runtime / hybrid-db-mem-smoke** | universal, db-surreal-mem | In-memory DB runtime |
| **runtime / hybrid-db-rocksdb-smoke** | (many) | RocksDB runtime integration |
| **ui / wasm32 lint + compilation** | web | Browser UI target compiles and passes clippy |

**Current platform coverage:**

| Platform | Automation | Notes |
|----------|------------|-------|
| **Linux (self-hosted `leabs`)** | `ci.yml`, `feature-matrix.yml`, `docs.yml`, scheduled/manual `ops.yml` | Primary gate for format, workspace clippy, nextest, storage/db/runtime feature matrix, and the WASM browser target |
| **Windows (GitHub-hosted)** | `libs-windows.yml`, manual `ops.yml` windows driver check | Covers LIBS mock smoke tests plus targeted Windows driver cross-checks |
| **macOS** | None in routine CI | Manual verification only today |

**Not covered in routine PR CI:**
- Real hardware feature paths such as `pvcam_hardware`, `comedi_hardware`, `andor_hardware`, and `hardware_tests`
- Manual Tailscale-driven lab orchestration in `hardware-tailscale.yml`

**Benchmark artifacts:** `ci.yml` uploads the structured regression-gate results plus the Criterion HTML report for ring-buffer write throughput.

---

## Feature Dependencies

```
bin crate features:
  maitai → pvcam_hardware + comedi_hardware + hardware/serial
  pvcam_hardware → pvcam_sdk → driver-registry/pvcam_hardware
  comedi_hardware → driver-registry/comedi_hardware
  all_hardware → driver-registry/all_hardware
  full → storage_arrow + serial + modules + all_hardware
  backend → modules + all_hardware
  production → db-surreal-rocksdb + modules + all_hardware
  leabs → driver-registry/andor
  leabs_hardware → driver-registry/andor_hardware
  modules → dep:daq-modules
  serial → hardware/serial + driver-registry/serial

driver-registry features:
  full → all_hardware
  all_hardware → pvcam + comedi + andor
  pvcam_hardware → pvcam_sdk
  pvcam_sdk → pvcam + driver-pvcam/pvcam_sdk
  pvcam → dep:driver-pvcam
  comedi_hardware → comedi + driver-comedi/hardware
  comedi → dep:driver-comedi
  andor_hardware → andor + driver-andor-sdk3/hardware
  andor → dep:driver-andor-sdk3
  serial → hardware/serial
  default → serial
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
