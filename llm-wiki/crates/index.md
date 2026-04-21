# Crate Index

<!--
last-ingested: 2026-04-19
sources:
  - Cargo.toml (workspace members — authoritative)
  - crates/*/Cargo.toml
  - docs/reference/inventory.md
  - docs/explanation/architecture.md
see-also:
  - ../architecture.md
-->

**30 workspace members.** Authoritative list: root `Cargo.toml`.

## By layer

### Foundation

| Crate | Purpose |
|-------|---------|
| [`common`](./common.md) | Shared types (`Parameter<T>`, `Observable<T>`), errors, size limits. |
| [`common-traits`](./common-traits.md) | Capability traits, `DriverFactory`, extracted from `common`. |
| [`pool`](./pool.md) | Lock-free object pool; `Pool<T>`, `BufferPool`, `ForeignView`, DLPack. |
| [`protocol`](./protocol.md) | Protobuf defs, wire types, domain↔proto conversions, `compress_frame_into` / `decompress_frame_into`. |

### Hardware abstraction

| Crate | Purpose |
|-------|---------|
| [`hardware`](./hardware.md) | HAL: `DeviceRegistry`, config/schema loading (driver selection lives in `driver-registry`). |
| [`driver-registry`](./driver-registry.md) | Factory registration + hardware feature gating. Always registers mock + universal. |
| [`driver-mock`](./driver-mock.md) | Always-compiled mock drivers. Fidelity levels + scenario seeds. |
| [`driver-universal`](./driver-universal.md) | TOML-manifest-driven serial/TCP/SCPI driver (schema v3). **Forward path.** |

### SDK drivers (feature-gated)

| Crate | Feature | SDK |
|-------|---------|-----|
| [`driver-pvcam`](./driver-pvcam.md) | `pvcam` / `pvcam_sdk` / `pvcam_hardware` | Photometrics PVCAM |
| [`driver-pvcam/pvcam-sys`](./pvcam-sys.md) | (via parent) | Raw FFI to PVCAM |
| [`driver-andor-sdk3`](./driver-andor-sdk3.md) | `andor` / `andor_hardware` | Andor SDK3 |
| [`andor-sdk3-sys`](./andor-sdk3-sys.md) | (paired) | Raw FFI to Andor SDK3 |
| [`driver-comedi`](./driver-comedi.md) | `comedi` / `comedi_hardware` | Linux Comedi |
| [`comedi-sys`](./comedi-sys.md) | (paired) | Raw FFI to Comedi |
| [`driver-dover-motion`](./driver-dover-motion.md) | *(not wired)* | Dover Motion API |
| [`dover-motion-sys`](./dover-motion-sys.md) | (paired) | Raw FFI to Dover Motion |

### Engine

| Crate | Purpose |
|-------|---------|
| [`experiment`](./experiment.md) | `RunEngine`, `Plan`s, `TaskQueue`, `WatchdogManager`, `AcquisitionCoordinator`. |
| [`scripting`](./scripting.md) | Rhai engine integration (10 k op limit, timeout). Optional PyO3. |
| [`daq-modules`](./daq-modules.md) | Experiment modules + plugin system, runtime module assignment. |

### Services

| Crate | Purpose |
|-------|---------|
| [`server`](./server.md) | gRPC server (`tonic`), token auth, CORS, alerting, heartbeat log. |
| [`client`](./client.md) | gRPC client library for daemon. |
| [`db`](./db.md) | Embedded SQLite persistence (control plane). No alternate DB backend. |
| [`storage`](./storage.md) | HDF5 / Arrow IPC / Parquet / TIFF / Zarr. `DocumentSink` impls. Ring buffer. |

### Apps

| Crate | Produces | Notes |
|-------|----------|-------|
| [`bin`](./bin.md) | `rust-daq-daemon` | Daemon entry point. |
| [`ui`](./ui.md) | `rust-daq-gui` (native) + `rust-daq-web` (WASM) | Primary operator UI. |
| [`ui-graph`](./ui-graph.md) | — | Node-graph editor for experiment design. |
| [`ui-slint`](./ui-slint.md) | — | **Experimental.** Slint evaluation; not production parity. |

### Domain

| Crate | Purpose |
|-------|---------|
| [`atomic-reference`](./atomic-reference.md) | NIST ASD atomic-line reference data for spectroscopy calibration and LIBS species identification. |
| [`echelle`](./echelle.md) | Echelle spectroscopy: calibration, order extraction, simulation. |

### Testing

| Crate | Purpose |
|-------|---------|
| [`integration-tests`](./integration-tests.md) | Workspace-level multi-crate tests. |

## Support levels

| Crate | Support |
|-------|---------|
| `ui-slint` | **Experimental** — excluded from CI test matrix and clippy gates. |
| `driver-dover-motion` + `dover-motion-sys` | **Experimental** — not wired into `driver-registry` yet. |
| `comedi-sys` + `driver-comedi` | **Platform-restricted** — Linux-only; excluded from default workspace / CI clippy via `--exclude`. |
| all others | **Stable** — in CI. |

## Feature-flag summary

See [`../invariants.md`](../invariants.md) §Drivers and
[`../workflows/build-test-lint.md`](../workflows/build-test-lint.md) for
the CI-parity clippy exclusions. Detailed per-driver flags in each
`drivers/<name>.md`.
