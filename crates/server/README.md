# server

gRPC server implementation for rust-daq.

## Overview

The `server` crate exposes the runtime over gRPC and gRPC-web. It sits on top of the `DeviceRegistry`, `RunEngine`, storage pipeline, and optional SurrealDB config layer.

## Registered Services

The current server wiring in `src/grpc/server.rs` registers these services (subject to feature gating):

- `ControlService`
- `HardwareService`
- `PresetService`
- `ScanService` (deprecated but still registered for compatibility)
- `RunEngineService`
- `ModuleService`
- `StorageService`
- `PluginService`
- `ConfigService` (when DB is available)
- `HealthService` (custom health/status API)
- `grpc.health.v1.Health` (standard gRPC health)
- `NiDaqService`

## Configuration

Primary daemon settings live in `config/config.v4.toml`.

Relevant gRPC settings include:

```toml
[grpc]
bind_address = "0.0.0.0"
auth_enabled=***
# auth_token = "replace-me"
allowed_origins = ["*"]
```

`allowed_origins` controls browser/gRPC-web access. Tighten this for production deployments.

## Feature Flags

Key crate-local features from `Cargo.toml`:

| Feature | Purpose |
|---------|---------|
| `scripting` | Enable script execution endpoints |
| `modules` | Enable module lifecycle APIs |
| `modules_scripting` | Enable modules + scripting together |
| `storage_hdf5` | HDF5 persistence support |
| `storage_arrow` | Arrow storage support |
| `serial` | Serial-aware service paths |
| `comedi` | NI/Comedi service support |
| `comedi_hardware` | Real Comedi driver support |
| `networking` | gRPC networking support |
| `rerun_sink` | Rerun visualization integration |
| `db-surreal` | Database-backed config service |
| `db-surreal-mem` | In-memory SurrealDB |
| `db-surreal-rocksdb` | RocksDB-backed SurrealDB |
| `metrics` | Prometheus metrics |

## Transport Notes

- Native clients use gRPC over HTTP/2.
- Browser clients use gRPC-web through `tonic_web`.
- Hardware frame streaming enables gzip compression for large responses.

## Proto Sources

The server uses proto definitions from `crates/protocol/proto/`:

- `daq.proto`
- `experiment.proto`
- `hardware.proto`
- `health.proto`
- `ni_daq.proto`
- `storage.proto`

## Related Crates

- `protocol` — protobuf definitions
- `hardware` — registry and capabilities
- `experiment` — RunEngine and plan orchestration
- `storage` — ring buffer and persistence backends
- `db` — SurrealDB control plane
