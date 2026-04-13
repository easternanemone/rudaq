#![forbid(unsafe_code)]
//! Embedded SQLite persistence layer for rust-daq.
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
pub mod sqlite_backend;

// Re-export all types at crate root so downstream code uses `db::DaqDb`, etc.
pub use sqlite_backend::{
    DbChangeEvent, DbConfig, DbDeviceFeature, DbDriver, DbExperimentPlan, DbInstrument,
    DbRunRecord, DeviceLifecycleEvent, DeviceParamState, ImportReport, PlanSummary, SqliteDb,
    SqliteDbInfo, StaleRun, config_hash, json_to_toml, toml_to_json,
};

/// Backward-compatible type alias: `DaqDb` → `SqliteDb`.
pub type DaqDb = SqliteDb;

/// Backward-compatible type alias: `DbInfo` → `SqliteDbInfo`.
pub type DbInfo = SqliteDbInfo;

// Backward-compatible module re-exports so `db::config_store::DbInstrument`
// and `db::experiment_store::DbExperimentPlan` paths continue to resolve.
pub mod config_store {
    //! Compatibility shim — re-exports SQLite types under the old module path.
    pub use crate::sqlite_backend::{
        DbDeviceFeature, DbDriver, DbInstrument, DeviceLifecycleEvent, DeviceParamState,
        ImportReport, config_hash, json_to_toml, toml_to_json,
    };
}

pub mod experiment_store {
    //! Compatibility shim — re-exports SQLite types under the old module path.
    pub use crate::sqlite_backend::{DbExperimentPlan, DbRunRecord, PlanSummary, StaleRun};
}
