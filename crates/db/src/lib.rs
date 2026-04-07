#![forbid(unsafe_code)]
//! Embedded persistence layer for rust-daq.
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
//! │  Plane A: SQLite (Control Plane / Desired State)         │
//! │  ─ driver definitions                                    │
//! │  ─ instrument configs                                    │
//! │  ─ experiment plans                                      │
//! │  ─ run records                                           │
//! │  ─ device runtime state & lifecycle events               │
//! └────────────────────┬─────────────────────────────────────┘
//!                      │  Reconciler
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
//! # Feature Flags
//!
//! - **`sqlite`** (default) — SQLite backend via rusqlite + tokio-rusqlite.
//!   ~15 deps, ~20s compile.  Uses bundled SQLite (no system dep).
//!   In-memory mode for tests, file-backed for production.
//! - **`kv-mem`** — Legacy SurrealDB in-memory engine (deprecated).
//! - **`kv-rocksdb`** — Legacy SurrealDB RocksDB engine (deprecated).
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use db::{DaqDb, DbConfig};
//!
//! let config = DbConfig::in_memory();
//! let db = DaqDb::init(config).await?;
//! let info = db.info().await?;
//! println!("DB healthy: {}", info.healthy);
//! ```

pub mod error;
pub mod schema;

// Compile-time guard: sqlite and kv-mem/kv-rocksdb are mutually exclusive.
#[cfg(all(feature = "sqlite", any(feature = "kv-mem", feature = "kv-rocksdb")))]
compile_error!(
    "Features `sqlite` and `kv-mem`/`kv-rocksdb` are mutually exclusive. \
     Use `sqlite` (default) or one of the legacy SurrealDB backends, not both."
);

// =========================================================================
// SQLite backend (default, forward path)
// =========================================================================

#[cfg(feature = "sqlite")]
pub mod sqlite_backend;

// Re-export all types at crate root so downstream code uses `db::DaqDb`, etc.
#[cfg(feature = "sqlite")]
pub use sqlite_backend::{
    DbChangeEvent, DbConfig, DbDeviceFeature, DbDriver, DbExperimentPlan, DbInstrument,
    DbRunRecord, DeviceLifecycleEvent, DeviceParamState, ImportReport, PlanSummary, SqliteDb,
    SqliteDbInfo, StaleRun, config_hash, json_to_toml, toml_to_json,
};

/// Backward-compatible type alias: `DaqDb` → `SqliteDb`.
#[cfg(feature = "sqlite")]
pub type DaqDb = SqliteDb;

/// Backward-compatible type alias: `DbInfo` → `SqliteDbInfo`.
#[cfg(feature = "sqlite")]
pub type DbInfo = SqliteDbInfo;

// Backward-compatible module re-exports so `db::config_store::DbInstrument`
// and `db::experiment_store::DbExperimentPlan` paths continue to resolve.
#[cfg(feature = "sqlite")]
pub mod config_store {
    //! Compatibility shim — re-exports sqlite types under the old module path.
    pub use crate::sqlite_backend::{
        DbDeviceFeature, DbDriver, DbInstrument, DeviceLifecycleEvent, DeviceParamState,
        ImportReport, config_hash, json_to_toml, toml_to_json,
    };
}

#[cfg(feature = "sqlite")]
pub mod experiment_store {
    //! Compatibility shim — re-exports sqlite types under the old module path.
    pub use crate::sqlite_backend::{DbExperimentPlan, DbRunRecord, PlanSummary, StaleRun};
}

// =========================================================================
// Legacy SurrealDB backend (only when sqlite is NOT active)
// =========================================================================

#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
mod core;
#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
pub use self::core::{DaqDb, DbConfig, DbEngine, DbInfo};

#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
pub(crate) mod bench;
#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
pub mod config_store;
#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
pub mod experiment_store;

#[cfg(all(
    any(feature = "kv-mem", feature = "kv-rocksdb"),
    not(feature = "sqlite")
))]
pub use surrealdb;
