# crate: `bin`

<!--
last-ingested: 2026-04-19
sources:
  - crates/bin/Cargo.toml
  - crates/bin/src/
  - docs/reference/inventory.md
see-also:
  - ./server.md
  - ./driver-registry.md
-->

**Role:** Daemon entry point. Produces the `rust-daq-daemon` binary.

**Wires together:**

- `driver-registry::register_all_factories(...)` — factories behind feature flags.
- `DeviceRegistry` construction from TOML config.
- gRPC server (`server` crate).
- Safety stack (safety-heartbeat task, HardwareWatchdog, panic hook).
- Storage sinks.

**Run (mock):**

```
cargo run -p bin -- daemon --hardware-config config/demo.toml
```

**Run (maitai production):**

```
bash scripts/ops/build-maitai.sh      # always — don't bypass
./target/release/rust-daq-daemon daemon --hardware-config config/maitai_universal.toml
```

**Feature flags** (passed through to `driver-registry`):
`pvcam_sdk` / `pvcam_hardware`, `comedi_hardware`, `all_hardware`,
`maitai`, `leabs`, `leabs_hardware`, `full`, `production`,
`storage_arrow`, `storage_hdf5`, `metrics`, `db`. The `db` feature
enables the SQLite control plane. There is **no** `db-surreal` feature —
the `db` crate is SQLite-only per bd-2a2ne; historical docs that reference
SurrealDB are stale.
