# SQLite Control Plane and Historical SurrealDB Notes

> **Current status:** SurrealDB is no longer part of the rust-daq codebase. The `db` crate is SQLite-only, implemented with `rusqlite` and `tokio-rusqlite`. The old `db-surreal`, `db-surreal-mem`, `db-surreal-rocksdb`, `kv-mem`, and `kv-rocksdb` feature family has been removed.

This file used to document the embedded SurrealDB backend. It now exists to prevent stale links from sending operators toward removed build flags or removed deployment procedures.

## Current DB Backend

| Layer | Current Implementation |
|-------|------------------------|
| Crate | `crates/db` |
| Engine | SQLite |
| Feature flag | `db` on `bin`, `server`, and `integration-tests` |
| In-memory mode | `DbConfig::in_memory()` |
| File-backed mode | `DbConfig::file(path)` / daemon `--db-path <file>` |
| gRPC service | `ConfigService` when `server/db` is enabled |
| Change propagation | SQLite backend broadcasts `DbChangeEvent`; watch reconciler reacts to the broadcast |

Relevant source files:

- `crates/db/Cargo.toml`
- `crates/db/src/sqlite_backend.rs`
- `crates/server/src/grpc/config_service.rs`
- `crates/bin/src/reconciler.rs`
- `crates/bin/src/watch_reconciler.rs`

## Build And Run

Default daemon builds include the SQLite control plane:

```bash
cargo build -p bin
```

Explicit DB-enabled build:

```bash
cargo build --release -p bin --features db
```

Run with a persistent SQLite file:

```bash
./target/release/rust-daq-daemon daemon \
  --hardware-config config/maitai_universal.toml \
  --db-path data/daq.db
```

Run with a lab deploy script:

```bash
bash scripts/deploy/deploy-maitai.sh --with-db
bash scripts/deploy/deploy-leabs.sh --with-db
```

The deploy scripts may still use `hybrid-db` as a runtime-mode compatibility name. That name no longer implies SurrealDB.

## Verification

```bash
rust-daq-daemon client config-info --addr http://127.0.0.1:50051
rust-daq-daemon client config-list --addr http://127.0.0.1:50051
```

Expected health metadata reports SQLite, not RocksDB or SurrealDB.

## Historical Context

ADR-015 originally described a TOML + SurrealDB + science-writer persistence model. That design was superseded by bd-2a2ne because SurrealDB was too heavy for the control-plane use case. The retained architecture is still three-tiered:

1. TOML for design-time configuration.
2. SQLite for runtime control-plane state.
3. HDF5 / Arrow / Zarr / Parquet / TIFF writers for science data.

If you find a current, non-archive document instructing operators to build with `db-surreal-*`, open a bead and update it to `db`.
