# protocol

Protobuf definitions for the rust-daq API surface.

## Proto Files

The crate compiles these source proto files at build time:

- `daq.proto`
- `experiment.proto`
- `hardware.proto`
- `health.proto`
- `ni_daq.proto`
- `storage.proto`

Generated Rust code is not checked in; it is produced during the build and included with `tonic::include_proto!()`.

## Services Defined Today

The current proto set defines these services:

- `ControlService`
- `HardwareService`
- `PresetService`
- `ScanService`
- `ModuleService`
- `RunEngineService`
- `StorageService`
- `PluginService`
- `ConfigService`
- `HealthService`
- `grpc.health.v1.Health`
- `NiDaqService`

## Workflow

```bash
# Regenerate generated code by rebuilding the crate
cargo build -p protocol
```

When changing the API:

1. edit the relevant `.proto` file under `crates/protocol/proto/`
2. rebuild the crate
3. update downstream server/client code
4. commit the `.proto` source change, not generated artifacts

## Notes

- `ScanService` remains present for backward compatibility even though newer flows prefer `RunEngineService`.
- The `storage.proto` file also carries plugin, config, and custom health services.
- `health.proto` contains the standard gRPC health-check service.
