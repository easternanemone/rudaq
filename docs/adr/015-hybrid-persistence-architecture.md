# ADR: Hybrid Persistence Architecture

**Status:** Accepted
**Date:** 2026-03-17
**Author:** Architecture Review
**Related Issues:** bd-4wf7, bd-kctc, bd-7xqd

---

## Context

The rust-daq system manages three fundamentally different categories of data:

1. **Design-time configuration** — Hardware manifests, calibration profiles, device TOML configs. These are authored by humans, reviewed in PRs, and must be version-controlled.
2. **Runtime control-plane state** — Parameter values, run records, device health, config reconciliation. This is machine-generated, changes frequently, and must survive daemon restarts.
3. **Science data** — Camera frames, scan datasets, spectral profiles. These are high-throughput (100+ MB/s), append-only, and must be stored in domain-specific formats that downstream analysis tools expect.

Early in the project, we considered unifying all persistence under SurrealDB. This was rejected because:

- **Version control**: TOML configs must live in git. A database cannot replace `git diff` and `git blame` for understanding config changes across deployments.
- **Format requirements**: Science data consumers (Python/NumPy, MATLAB, ImageJ) expect HDF5, TIFF, Zarr, or Arrow — not database exports. Storing frames as BLOBs would add an unnecessary extraction step.
- **Performance**: The mmap-backed ring buffer delivers zero-copy frame access at camera line rate. No general-purpose database can match this for the write-heavy, append-only science data path.
- **Operational simplicity**: Lab machines are often managed by scientists, not DBAs. TOML files are human-readable and can be edited with any text editor. If the database corrupts, the daemon falls back to TOML and keeps running.

---

## Decision

**Use a three-tier persistence model, each tier optimized for its data category.**

### Tier 1 — TOML (Design-Time, Version-Controlled)

| What | Where | Why |
|------|-------|-----|
| Hardware configs | `config/*.toml` | Git-tracked, human-editable, PR-reviewable |
| Device manifests | `config/devices/*.toml` | Schema v3 universal driver definitions |
| Calibration profiles | `config/calibration/*.toml` | Versioned with `profile_version` field |
| Server config | `config/config.v4.toml` | gRPC, storage, alerting settings |
| Feature flags | `config/feature_flags.toml` | Runtime toggles |

TOML is **always** the authoritative source of truth for device configuration. If TOML and SurrealDB disagree, TOML wins (via the shadow-write + reconciliation loop).

### Tier 2 — SurrealDB (Runtime Control Plane)

| What | Table | Why |
|------|-------|-----|
| Device desired state | `instrument` | Reconciler convergence target |
| Driver catalog | `driver` | Queryable capability introspection |
| Run history | `run_record` | Execution audit trail, crash detection via `heartbeat_at` |
| Experiment plans | `experiment_plan` | Persistent plan definitions with graph data |
| Device feature cache | `device_feature` | Offline UI parameter rendering |
| Parameter state | `device_runtime_state` | Last-known values for restart recovery, favorites |
| Schema version | `schema_version` | Forward-only migration tracking |

SurrealDB is **optional** — the daemon degrades gracefully without it (no ConfigService, no parameter persistence, no run history, but hardware still works from TOML).

### Tier 3 — Specialized Writers (Science Data)

| Format | Use Case | Module |
|--------|----------|--------|
| Arrow IPC | Ring buffer (mmap, zero-copy live visualization) | `storage::ring_buffer` |
| HDF5 | Primary scan data persistence | `storage::hdf5_writer` |
| Parquet | Columnar analytics export | `storage::parquet_writer` |
| TIFF | Single-frame image export | `storage::tiff_writer` |
| Zarr V3 | Chunked multi-dimensional scan data | `storage::zarr_sink` |

Science data writers implement the `DocumentSink` trait, decoupling the RunEngine's document stream from storage format.

### Reconciliation Loop

The bridge between Tier 1 (TOML) and Tier 2 (SurrealDB) is the reconciler:

```
Daemon startup
    │
    ▼
Parse TOML hardware config
    │
    ▼
shadow_write() ──► SurrealDB (non-fatal mirror)
    │
    ▼
reconcile_once() ──► Diff DB vs DeviceRegistry ──► Add/Remove/Update devices
    │
    ▼
spawn watch_reconciler ──► LIVE SELECT on instrument table
    │                          │
    │                    Debounce (200ms, 2s max)
    │                          │
    │                          ▼
    │                    reconcile_once()
    │                          │
    │                    ┌─────┴─────┐
    │                    ▼           ▼
    │              DeviceRegistry  gRPC clients
    │              (data plane)   see changes
    │
    ▼
restore_parameter_state() ──► Read device_runtime_state
                                  │
                                  ▼
                           Parameter::set_json() on each device
```

### Feature Gating and Graceful Degradation

| Feature | Missing? | Consequence |
|---------|----------|-------------|
| `db-surreal` | No DB | TOML-only operation, no ConfigService, no param persistence |
| `kv-rocksdb` | In-memory only | State lost on restart (acceptable for dev/test) |
| `storage_hdf5` | No HDF5 | Other writers still active |
| `storage_zarr` | No Zarr | Falls back to HDF5 or Arrow |

---

## Consequences

### Positive

- **Resilience**: Database corruption never prevents the daemon from starting. TOML is always recoverable from git.
- **Right tool for the job**: Science data stays in formats that analysis pipelines already understand.
- **Zero-ops for small labs**: A single binary with TOML files works without any database setup.
- **Incremental adoption**: Labs can start TOML-only and add SurrealDB persistence later without changing workflows.

### Negative

- **Three places to look**: Debugging a "where is this data?" question requires understanding which tier owns it.
- **Shadow-write complexity**: The TOML → SurrealDB mirror adds a startup step and potential for transient inconsistency (resolved within one reconcile cycle).
- **Schema migration burden**: SurrealDB schema changes require forward-only migrations (currently at v8).

### Risks

- **Tier drift**: If someone edits the database directly (via CLI) without updating TOML, the next daemon restart from TOML will overwrite their changes. Mitigated by: the shadow-write pattern always re-mirrors TOML on startup.
- **Feature flag sprawl**: The number of `#[cfg(feature = "...")]` gates grows with each optional tier. Mitigated by: the `db-surreal` meta-feature gates all DB code in one place.

---

## References

- [SurrealDB Integration Guide](../how-to/surrealdb-integration.md)
- [Reconciler](../../crates/bin/src/reconciler.rs)
- [Watch Reconciler](../../crates/bin/src/watch_reconciler.rs)
- [Config Bridge](../../crates/bin/src/db_bridge.rs)
- [Schema Migrations](../../crates/db/src/schema.rs)
- [DocumentSink trait](../../crates/storage/src/document_sink.rs)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2026-03-17 | Architecture Review | Initial hybrid persistence architecture documentation |
