# crate: `db`

<!--
last-ingested: 2026-04-19
sources:
  - crates/db/Cargo.toml
  - crates/db/src/
  - docs/adr/015-hybrid-persistence-architecture.md
see-also:
  - ../architecture.md §Persistence
-->

**Role:** Embedded persistence for the control plane (desired device
state, config reconciliation).

**Backend: SQLite only** (bd-2a2ne). Uses `rusqlite = "0.37"` (bundled)
+ `tokio-rusqlite = "0.7"`. There is **no** other backend — the earlier
SurrealDB variant referenced in historical docs has been removed, and
**no `db-surreal` feature exists** in this crate or the workspace.
Source-of-truth files:

- `crates/db/Cargo.toml:20` — "SQLite — primary (and only) backend (bd-2a2ne)."
- `crates/db/src/sqlite_backend.rs:1` — "SQLite-backed persistence layer for rust-daq (bd-2a2ne)."

**Reconciliation:** TOML configs (`config/devices/*.toml`) shadow-write
into the DB at startup; a watcher converges the `DeviceRegistry`
(~300 ms). Stale docs may still refer to SurrealDB's LIVE SELECT — those
are historical.

**Dependents:** `server`, `bin`.
