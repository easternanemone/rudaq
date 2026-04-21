# Feature Matrix

**Status:** Active
**Last Updated:** April 2026
**Source of truth:** crate `Cargo.toml` feature sections and `.github/workflows/feature-matrix.yml`.

This page summarizes the current build features. Removed SurrealDB feature families are mentioned only as historical removals; the control-plane DB is SQLite-only behind the `db` feature.

## Quick Reference

```bash
# Development daemon: networking + gRPC server + SQLite control plane
cargo build -p bin

# Broad native-driver mock build
cargo build -p bin --features full

# GUI development
cargo build -p ui

# Browser GUI compile check
cargo check -p ui --lib --target wasm32-unknown-unknown \
  --no-default-features --features web
```

## High-Level Profiles

The `bin` crate owns the operator-facing profiles and daemon feature aliases.

| Feature | Expands To | Notes |
|---|---|---|
| `all_hardware` | `driver-registry/all_hardware`, `server/comedi` | PVCAM, Comedi, and Andor mock-SDK driver crates. |
| `backend` | `modules`, `all_hardware` | Headless backend profile. |
| `cli` | `all_hardware`, `scripting_python` | CLI automation profile. |
| `comedi_hardware` | `driver-registry/comedi_hardware`, `server/comedi`, `dep:driver-comedi` | Real Comedi hardware and safety heartbeat. |
| `db` | `dep:db`, `server/db` | SQLite control plane and ConfigService support. |
| `full` | `storage_arrow`, `serial`, `modules`, `all_hardware` | Broad daemon build; HDF5 remains explicit. |
| `leabs` | `driver-registry/andor` | LEABS mock-Andor profile. |
| `leabs_hardware` | `driver-registry/andor_hardware` | LEABS real Andor SDK3 profile. |
| `maitai` | `pvcam_hardware`, `comedi_hardware`, `hardware/serial` | Maitai lab profile. |
| `metrics` | `dep:prometheus`, `server/metrics` | Prometheus endpoint. |
| `modules` | `dep:daq-modules` | DAQ module system. |
| `networking` | crate-local | Gates gRPC server startup and daemon networking paths. |
| `production` | `db`, `modules`, `all_hardware` | Production profile with SQLite control plane. |
| `pvcam_hardware` | `pvcam_sdk` | Alias used by lab profiles. |
| `pvcam_sdk` | `driver-registry/pvcam_sdk` | Real PVCAM SDK linkage. |
| `scripting_python` | `scripting/python` | Optional PyO3 bindings. |
| `serial` | `hardware/serial`, `driver-registry/serial` | Serial transport support. |
| `server` | crate-local | Guards server harness compilation. |
| `storage_arrow` | `storage/storage_arrow` | Arrow IPC storage wiring. |
| `storage_hdf5` | `storage/storage_hdf5` | Requires native HDF5 libraries. |

---

## Default Features

`bin` defaults to the development daemon surface:

```text
["networking", "server", "db"]
```

---

## System Features

| Feature | Expands To | Notes |
|---|---|---|
| `db` | `dep:db`, `server/db` | SQLite control-plane database. |
| `metrics` | `dep:prometheus`, `server/metrics` | Prometheus metrics. |
| `modules` | `dep:daq-modules` | DAQ module runtime. |
| `networking` | crate-local | gRPC/networking paths. |
| `scripting_python` | `scripting/python` | Python scripting bindings. |
| `serial` | `hardware/serial`, `driver-registry/serial` | Serial transport support. |
| `server` | crate-local | Server harness support. |
| `storage_arrow` | `storage/storage_arrow` | Arrow storage wiring. |
| `storage_hdf5` | `storage/storage_hdf5` | HDF5 storage wiring. |

---

## Driver Features

### Device Drivers (bin crate)

| Feature | Expands To | Notes |
|---|---|---|
| `all_hardware` | `driver-registry/all_hardware`, `server/comedi` | Enables PVCAM, Comedi, and Andor registry families. |
| `comedi_hardware` | `driver-registry/comedi_hardware`, `server/comedi`, `dep:driver-comedi` | Linux-only Comedi hardware path. |
| `leabs` | `driver-registry/andor` | LEABS mock-Andor registry path. |
| `leabs_hardware` | `driver-registry/andor_hardware` | LEABS real Andor SDK3 path. |
| `maitai` | `pvcam_hardware`, `comedi_hardware`, `hardware/serial` | Maitai hardware profile. |
| `pvcam_hardware` | `pvcam_sdk` | Alias for real PVCAM support. |
| `pvcam_sdk` | `driver-registry/pvcam_sdk` | PVCAM SDK registry path. |
| `serial` | `hardware/serial`, `driver-registry/serial` | Serial drivers and transports. |

#### Driver Registry Features

`driver-registry` always depends on `driver-mock` and `driver-universal`. The registry feature set is:

| Feature | Expands To |
|---|---|
| `all_hardware` | `pvcam`, `comedi`, `andor` |
| `andor` | `dep:driver-andor-sdk3` |
| `andor_hardware` | `andor`, `driver-andor-sdk3/hardware` |
| `comedi` | `dep:driver-comedi` |
| `comedi_hardware` | `comedi`, `driver-comedi/hardware` |
| `full` | `all_hardware` |
| `pvcam` | `dep:driver-pvcam` |
| `pvcam_hardware` | `pvcam_sdk` |
| `pvcam_sdk` | `pvcam`, `driver-pvcam/pvcam_sdk` |
| `runtime_probe` | `dep:libloading` |
| `serial` | `hardware/serial` |
| `test-util` | `hardware/test-util` |

**Always included:** `driver-mock` and `driver-universal` are ordinary dependencies. `driver-dover-motion` is in the workspace but is not registered by `driver-registry`.

Feature dependencies:

```text
bin crate features:
all_hardware -> driver-registry/all_hardware, server/comedi
backend -> modules, all_hardware
cli -> all_hardware, scripting_python
comedi_hardware -> driver-registry/comedi_hardware, server/comedi, dep:driver-comedi
db -> dep:db, server/db
full -> storage_arrow, serial, modules, all_hardware
leabs -> driver-registry/andor
leabs_hardware -> driver-registry/andor_hardware
maitai -> pvcam_hardware, comedi_hardware, hardware/serial
metrics -> dep:prometheus, server/metrics
modules -> dep:daq-modules
networking -> standalone
production -> db, modules, all_hardware
pvcam_hardware -> pvcam_sdk
pvcam_sdk -> driver-registry/pvcam_sdk
scripting_python -> scripting/python
serial -> hardware/serial, driver-registry/serial
server -> standalone
storage_arrow -> storage/storage_arrow
storage_hdf5 -> storage/storage_hdf5

driver-registry features:
all_hardware -> pvcam, comedi, andor
andor -> dep:driver-andor-sdk3
andor_hardware -> andor, driver-andor-sdk3/hardware
comedi -> dep:driver-comedi
comedi_hardware -> comedi, driver-comedi/hardware
full -> all_hardware
pvcam -> dep:driver-pvcam
pvcam_hardware -> pvcam_sdk
pvcam_sdk -> pvcam, driver-pvcam/pvcam_sdk
runtime_probe -> dep:libloading
serial -> hardware/serial
test-util -> hardware/test-util
```

### Camera Hardware

Camera and spectrograph hardware support is split across registry and SDK crates. `driver-andor-sdk3` exposes `camera`, `spectrograph`, `hardware`, and `hardware_tests`; `andor-sdk3-sys` exposes `andor-sdk3`, `camera`, `spectrograph`, and `hardware`. `driver-pvcam` exposes `mock`, `pvcam_hardware`, `pvcam_sdk`, and `hardware_tests`; `pvcam-sys` exposes `pvcam-sdk`.

## Storage Backends

| Feature | Expands To | Notes |
|---|---|---|
| `metrics` | `dep:prometheus`, `pool/metrics` | Storage metrics. |
| `networking` | standalone | Remote-storage plumbing. |
| `storage_arrow` | `dep:arrow`, `common/storage_arrow` | Apache Arrow IPC format. |
| `storage_hdf5` | `dep:hdf5`, `common/storage_hdf5` | HDF5 scientific format. |
| `storage_parquet` | `dep:parquet`, `storage_arrow` | Parquet export. |
| `storage_tiff` | `dep:image` | TIFF image export. |
| `storage_zarr` | `dep:zarrs` | Zarr V3 storage. |

---

## Service And Test Features

`server` defaults to `modules`, `server`, `scripting`, and `alerting`. Optional server features include `comedi`, `comedi_hardware`, `db`, `metrics`, `modules_scripting`, `rerun_sink`, `serial`, `storage_arrow`, and `storage_hdf5`.

`integration-tests` defaults to `server`, `scripting`, `storage_hdf5`, `storage_arrow`, `serial`, `modules`, `pvcam`, `universal`, and `db`. Additional test features include `daq-modules`, `gui_egui`, `hardware_tests`, `libs_drivers`, `libs_spirit_driver`, and `pvcam_sdk`.

## Complete Crate Feature Coverage

This index is intentionally exhaustive so drift checks can verify that every feature declared in `cargo metadata` is documented somewhere on this page.

| Crate | Default Features | Other Features |
|---|---|---|
| `andor-sdk3-sys` | none | `andor-sdk3`, `camera`, `hardware`, `spectrograph` |
| `bin` | `networking`, `server`, `db` | `all_hardware`, `backend`, `cli`, `comedi_hardware`, `full`, `leabs`, `leabs_hardware`, `maitai`, `metrics`, `modules`, `production`, `pvcam_hardware`, `pvcam_sdk`, `scripting_python`, `serial`, `storage_arrow`, `storage_hdf5` |
| `client` | `tls` | none |
| `comedi-sys` | none | `comedi-sdk` |
| `common` | none | `fits`, `schemars`, `serial`, `storage_arrow`, `storage_hdf5` |
| `common-traits` | none | `storage_arrow`, `storage_hdf5` |
| `daq-modules` | none | `scripting`, `scripting_python` |
| `dover-motion-sys` | none | `dover-sdk` |
| `driver-andor-sdk3` | none | `camera`, `hardware`, `hardware_tests`, `spectrograph` |
| `driver-comedi` | none | `hardware` |
| `driver-dover-motion` | none | `dover-hardware`, `hardware` |
| `driver-pvcam` | `mock` | `hardware_tests`, `pvcam_hardware`, `pvcam_sdk` |
| `driver-registry` | `serial`, `runtime_probe` | `all_hardware`, `andor`, `andor_hardware`, `comedi`, `comedi_hardware`, `full`, `pvcam`, `pvcam_hardware`, `pvcam_sdk`, `test-util` |
| `driver-universal` | `serial` | `emulator` |
| `echelle` | none | `fits` |
| `hardware` | `serial` | `binary_protocol`, `plugins_hot_reload`, `simulator`, `test-util` |
| `integration-tests` | `server`, `scripting`, `storage_hdf5`, `storage_arrow`, `serial`, `modules`, `pvcam`, `universal`, `db` | `daq-modules`, `gui_egui`, `hardware_tests`, `libs_drivers`, `libs_spirit_driver`, `pvcam_sdk` |
| `pool` | none | `dlpack`, `metrics` |
| `protocol` | none | `server` |
| `pvcam-sys` | none | `pvcam-sdk` |
| `scripting` | none | `driver-comedi`, `hardware_factories`, `hdf5_scripting`, `libs_scripting`, `polarization`, `python`, `scripting_full`, `scripting_full_libs` |
| `server` | `modules`, `server`, `scripting`, `alerting` | `comedi`, `comedi_hardware`, `db`, `metrics`, `modules_scripting`, `rerun_sink`, `serial`, `storage_arrow`, `storage_hdf5` |
| `storage` | none | `metrics`, `networking`, `storage_arrow`, `storage_hdf5`, `storage_parquet`, `storage_tiff`, `storage_zarr` |
| `ui` | `standalone` | `dark-light`, `pvcam`, `pvcam_hardware`, `pvcam_sdk`, `rerun_viewer`, `storage_hdf5`, `web` |
| `ui-slint` | none | `web` |

## CI Matrix

The current `.github/workflows/feature-matrix.yml` runs these feature jobs on non-PR triggers:

| Job | Package | Features |
|---|---|---|
| storage / hdf5 | `storage` | `storage_hdf5` |
| storage / arrow | `storage` | `storage_arrow` |
| bin / all_hardware mock | `bin` | `all_hardware` |
| server / full stack | `server` | `modules,scripting,storage_hdf5,storage_arrow` |
| runtime / universal-smoke | `integration-tests` | `universal` |
| runtime / universal-db-smoke | `integration-tests` | `universal,db` |
| ui / wasm32 lint + compilation | `ui` | `web` with `--no-default-features` and wasm32 target |

Feature powerset checks cover `common`, `storage`, `driver-registry`, `pool`, and `experiment`; SDK and HDF5 features are skipped where native libraries are not available.

## Removed Features

The `db-surreal`, `db-surreal-mem`, `db-surreal-rocksdb`, `kv-mem`, and `kv-rocksdb` feature family no longer exists in the workspace. Use `db` for the SQLite control plane.
