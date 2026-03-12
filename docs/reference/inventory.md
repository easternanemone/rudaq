# Project Inventory & Canonical Paths

> **Source of Truth**
> This document is the canonical reference for binaries, runtime config paths, crate structure, and feature flags. If a document needs to refer to these, link here rather than duplicating the information.

## Binaries (Executables)

| Name | Cargo Command | Location | Description |
|---|---|---|---|
| **Daemon** | `cargo run -p bin -- daemon` | `crates/bin` | The headless gRPC server and experiment orchestrator. |
| **GUI** | `cargo run -p ui --release` | `crates/ui` | The egui/Rerun desktop client. |
| **CLI Client**| `cargo run -p client --` | `crates/client` | Headless CLI for interacting with the daemon. |

## Runtime Config Paths

| Config Path | Environment / Mode | Description |
|---|---|---|
| `config/demo.toml` | `mock` / CI | Basic mock hardware config for tests. |
| `config/demo_mock_all.toml` | `mock` (extended) | Full simulated lab environment (cameras, stages, lasers). |
| `config/maitai_universal.toml`| `native`, `universal`, `hybrid-db` | Production hardware layout for the 'Maitai' optical table. |
| `config/feature_flags.toml` | All | Runtime feature toggles (streaming perf, debug panels, etc.). |

*Note: The DAQ_RUNTIME_MODE environment variable maps directly to these configurations unless overridden by DAQ_CONFIG_PATH.*

## Crate Layout

| Layer | Crates | Purpose |
|---|---|---|
| **Core** | `common` | Shared types, observables (`Parameter<T>`), errors. |
| **HAL** | `hardware` | Trait definitions (`Movable`, `Readable`, `FrameProducer`). |
| **Registry** | `driver-registry` | Feature gating, driver instantiations, mapping strings to drivers. |
| **Drivers** | `driver-*`, `*-sys` | FFI bindings (PVCAM, Comedi, Andor) and driver implementations. |
| **Protocol** | `protocol` | Protobuf definitions and gRPC service traits. |
| **Server** | `server` | The gRPC implementation, orchestrating calls to hardware and storage. |
| **Engine** | `experiment`, `scripting`| Rhai engine integration, task sequences, coordinated multi-axis scans. |
| **Storage** | `storage`, `pool` | Fast streaming (Ring Buffer, Arrow) and persistence (HDF5). |
| **UI** | `ui`, `ui-slint` | Client-facing applications (egui, WASM, Slint evaluation). |

## Feature Flags (Compile Time)

| Flag | Crate Owner | Description |
|---|---|---|
| `comedi_hardware` | `bin`, `driver-registry` | Enables real Comedi DAQ driver compilation (Linux/Windows only). |
| `comedi-sdk` | `comedi-sys` | Links the Linux Comedi C SDK. |
| `db-surreal` | `bin`, `server`, `db` | Compiles SurrealDB for configuration persistence. |
| `storage_arrow` | `storage` | Enables Apache Arrow IPC and Tensor formatting backends. |
| `mock_only` | `driver-registry` | Forces the registry to exclude FFI drivers even if `comedi_hardware` is set. |

*See `config/feature_flags.toml` for runtime toggles rather than compile-time features.*
