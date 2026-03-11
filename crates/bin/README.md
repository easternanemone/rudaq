# bin

Command-line entrypoint for rust-daq.

## Overview

This crate builds a single executable (`rust-daq-daemon`) that provides
multiple subcommands rather than separate `daq-daemon` / `daq-cli` binaries.
The daemon includes a safety heartbeat task, hardware watchdog, and safety
sentinel for defense-in-depth hardware protection.

## Common Commands

```bash
# Start the daemon with mock/default runtime settings
cargo run -p bin -- daemon --port 50051

# Start the daemon with a hardware config
cargo run -p bin -- daemon --hardware-config config/maitai_universal.toml

# Start the daemon and serve the WASM web UI
cargo run -p bin -- daemon --port 50051 --web-ui-path /path/to/wasm/dist

# Run a Rhai script once
cargo run -p bin -- run examples/demo_scan.rhai

# Query a running daemon
rust-daq-daemon client config-info --addr http://localhost:50051

# Recover a corrupt HDF5 file
rust-daq-daemon recover --input bad.h5 --output recovered.h5
```

## Subcommands

`rust-daq-daemon` currently exposes:

- `daemon` — start the gRPC daemon
- `run` — execute a Rhai script once
- `client` — remote-control commands for a running daemon
- `config` — import/export/list database-backed config state
- `recover` — recover data from a corrupt HDF5 file

## Runtime Modes

The `daemon` subcommand supports explicit runtime selection:

- `mock`
- `native`
- `universal`
- `hybrid-db`

When using built-in Maitai profiles, the runtime currently maps to `config/maitai_universal.toml`.

## Feature Flags

Key `bin` crate features from `Cargo.toml`:

| Feature | Purpose |
|---------|---------|
| `default` | `networking`, `server`, `db-surreal-mem` |
| `storage_hdf5` | Enable HDF5 storage backend passthrough |
| `storage_arrow` | Enable Arrow storage backend passthrough |
| `pvcam_sdk` / `pvcam_hardware` | Real PVCAM support |
| `comedi_hardware` | Real Comedi support |
| `serial` | Serial support via `hardware` and `driver-registry` |
| `modules` | Enable `daq-modules` |
| `all_hardware` | Enable all registry-managed native drivers |
| `full` | `storage_arrow + serial + modules + all_hardware` |
| `db-surreal-mem` | In-memory SurrealDB |
| `db-surreal-rocksdb` | Persistent RocksDB-backed SurrealDB |
| `production` | Production profile with RocksDB + modules + all_hardware |
| `metrics` | Prometheus metrics |
| `maitai` | Maitai hardware profile |
| `leabs` / `leabs_hardware` | LEABS lab profiles |

## Safety

The daemon provides three layers of hardware safety:

- **SafetyHeartbeat** — Toggles a Comedi DIO channel at a configurable interval
  (default 100ms) to drive an external hardware interlock. If the daemon dies,
  the pulse stops and external circuitry cuts laser power. Configured via
  `[safety_heartbeat]` in the hardware TOML config. Only compiled with the
  `comedi_hardware` feature. Source: `src/safety_heartbeat_task.rs`.

- **HardwareWatchdog** — A dedicated OS thread monitors daemon liveness (default
  30s timeout). If the Tokio runtime hangs, it fires a 5-step emergency shutdown:
  close shutters, disable emission, stop motors, zero DAQ outputs. All devices
  are discovered via `DeviceRegistry::devices_with_capability()`. Source:
  `common/src/health/watchdog.rs`.

- **SafetySentinel** — An RAII guard that ensures hardware cleanup runs even if
  the daemon task panics. Source: `src/safety_sentinel.rs`.

See [ADR-004](../../docs/adr/004-panic-safety.md) for the full safety architecture.

## Notes

- The executable name produced by Cargo is `rust-daq-daemon` even though the Clap app name shown in help output is `rust-daq`.
- The `--web-ui-path` flag enables serving the WASM web UI from the daemon's gRPC port.
- Serial/TCP/SCPI devices are typically loaded through `driver-universal` manifests under `config/devices/`.
- For operational procedures, see `docs/how-to/operations.md`.
