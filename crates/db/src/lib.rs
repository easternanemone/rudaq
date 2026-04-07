#![forbid(unsafe_code)]
//! Embedded SurrealDB persistence layer for rust-daq.
//!
//! This crate provides the **control plane** database — configuration, topology,
//! and desired state.  The **data plane** (100 Hz hardware loop, ring buffers,
//! Arrow IPC, HDF5) is intentionally excluded.
//!
//! # Architecture: Two-Plane Model
//!
//! The system separates **desired state** (Plane A — what the user configured)
//! from **observed state** (Plane B — what the hardware is actually doing).
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │  Plane A: SurrealDB (Control Plane / Desired State)      │
//! │  ─ driver definitions         (config_store)             │
//! │  ─ instrument configs         (config_store)             │
//! │  ─ experiment presets          (future)                   │
//! │  ─ experiment presets          (Phase 4)                  │
//! │  ─ audit log                  (Phase 4)                  │
//! └────────────────────┬─────────────────────────────────────┘
//!                      │  Reconciler (Phase 3)
//!                      │  Diff desired vs. observed → apply
//!                      ▼
//! ┌──────────────────────────────────────────────────────────┐
//! │  Plane B: DashMap DeviceRegistry (Observed State)        │
//! │  ─ live device trait objects, 100 Hz acquisition loop    │
//! │  ─ health/status (runtime only, not persisted)           │
//! │  ─ measurements & telemetry (ring buffers, Arrow IPC)    │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Source of Truth per Field
//!
//! | Field                | Source   | Rationale                              |
//! |----------------------|----------|----------------------------------------|
//! | `instrument.config`  | DB       | User-authored, persisted across runs   |
//! | `instrument.enabled` | DB       | Desired state — survives restart       |
//! | `instrument.status`  | Memory   | Observed from hardware, volatile       |
//! | `driver_type`        | DB       | Part of configuration, immutable-ish   |
//! | Measurements         | Memory   | High-frequency, broadcast via channels |
//! | Experiment presets    | DB       | Structural, changes infrequently       |
//!
//! # Feature Flags
//!
//! - **`kv-mem`** — In-memory SurrealDB engine (fast compile, no persistence). Use for tests.
//! - **`kv-rocksdb`** — RocksDB SurrealDB engine (persistent, production). Requires librocksdb.
//! - **`sqlite`** — Lightweight SQLite backend via `rusqlite`/`tokio-rusqlite` (~15 deps
//!   vs SurrealDB's 118).  Provides [`SqliteDb`] instead of [`DaqDb`].
//!
//! The `sqlite` feature is independent of the `kv-*` features and can be enabled
//! alongside them (e.g., for migration testing).  For production use, pick
//! **one** backend: either `sqlite` or one of the `kv-*` features.
//! Without any engine feature, the crate compiles but no database type is
//! available (useful as a documentation-only dependency).
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use db::{DaqDb, DbConfig};
//!
//! let config = DbConfig::in_memory();
//! let db = DaqDb::init(config).await?;
//! println!("DB healthy: {}", db.health_check().await);
//! ```

pub mod error;
pub mod schema;

#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
mod core;
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub use self::core::{DaqDb, DbConfig, DbEngine, DbInfo};

#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub(crate) mod bench;
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub mod config_store;
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub mod experiment_store;

// Re-export surrealdb for downstream crates that need raw access.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub use surrealdb;

#[cfg(feature = "sqlite")]
pub mod sqlite_backend;

// When sqlite is the active backend, re-export SqliteDb and all types
// so downstream code uses `db::SqliteDb`, `db::DbInstrument`, etc.
#[cfg(feature = "sqlite")]
pub use sqlite_backend::{
    DbChangeEvent, DbConfig as SqliteDbConfig, DbDeviceFeature, DbDriver, DbExperimentPlan,
    DbInstrument, DbRunRecord, DeviceLifecycleEvent, DeviceParamState, InstrumentSummary, SqliteDb,
    SqliteDbInfo, config_hash, json_to_toml, toml_to_json,
};
