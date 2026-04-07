//! SQLite-backed persistence layer for rust-daq (bd-ba6cd).
//!
//! Drop-in replacement for the SurrealDB control plane with ~15 deps instead
//! of 118.  Uses `rusqlite` (bundled SQLite) + `tokio-rusqlite` for async.
//!
//! # Feature gate
//!
//! This module is only compiled when the `sqlite` feature is enabled:
//!
//! ```bash
//! cargo check -p db --features sqlite
//! ```

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::info;

use crate::error::Result;
use crate::schema::SCHEMA_VERSION;

/// Summary of a config import operation.
///
/// Compatible with the SurrealDB `ImportReport` so downstream callers
/// (e.g., `db_bridge::shadow_write`) work unchanged.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub drivers_upserted: usize,
    pub instruments_upserted: usize,
    pub errors: Vec<String>,
}

// Re-use the existing DB-native types from config_store so we don't diverge.
// These are gated on surreal features in config_store.rs, so we re-declare
// a compatible subset here for the sqlite backend.  When migration is
// complete the canonical types will move to a shared location.

/// An instrument (device instance) stored in SQLite.
///
/// Field-compatible with [`crate::config_store::DbInstrument`] so callers
/// can convert freely between the two backends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbInstrument {
    /// Unique device ID (e.g., "rotator_2").
    pub device_id: String,
    /// Human-readable name (e.g., "ELL14 Rotator (Address 2)").
    pub name: String,
    /// Driver type that created this device.
    pub driver_type: String,
    /// Driver-specific configuration as JSON.
    pub config: serde_json::Value,
    /// Whether this device is active.
    pub enabled: bool,
}

/// A driver definition stored in SQLite.
///
/// Field-compatible with [`crate::config_store::DbDriver`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbDriver {
    /// Driver type identifier (e.g., "ell14", "pvcam", "mock").
    pub driver_type: String,
    /// Human-readable name.
    pub name: String,
    /// Capability strings (e.g., `["movable", "readable"]`).
    pub capabilities: Vec<String>,
    /// Available command names (primarily for universal TOML drivers).
    pub commands: Vec<String>,
}

/// A device feature metadata record stored in SQLite.
///
/// Field-compatible with [`crate::config_store::DbDeviceFeature`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbDeviceFeature {
    /// Device ID this feature belongs to.
    pub device_id: String,
    /// Feature/parameter name.
    pub feature_name: String,
    /// Data type: "float", "int", "bool", "enum", "string", "command".
    pub feature_type: String,
    /// Whether this feature can be read.
    pub readable: bool,
    /// Whether this feature can be written.
    pub writable: bool,
    /// Minimum value for numeric features.
    pub min_value: Option<f64>,
    /// Maximum value for numeric features.
    pub max_value: Option<f64>,
    /// Step size for numeric features.
    pub step: Option<f64>,
    /// Allowed values for enum features.
    #[serde(default)]
    pub enum_values: Vec<String>,
    /// Physical unit (e.g., "s", "C", "ps").
    pub unit: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// UI grouping category.
    pub group_name: Option<String>,
}

/// An experiment plan stored in SQLite.
///
/// Field-compatible with [`crate::experiment_store::DbExperimentPlan`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbExperimentPlan {
    /// Unique plan identifier (UUID or user-defined slug).
    pub plan_id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Plan origin type (e.g., "graph_plan", "line_scan", "grid_scan").
    pub plan_type: String,
    /// Pre-translated PlanCommands as JSON values.
    pub commands: Option<Vec<serde_json::Value>>,
    /// Devices that will be moved.
    pub movers: Option<Vec<String>>,
    /// Devices that will be read.
    pub detectors: Option<Vec<String>>,
    /// Total number of data points.
    pub num_points: Option<i64>,
    /// Visual graph data (GraphFile JSON) for graph-based plans.
    pub graph_data: Option<serde_json::Value>,
    /// Plan parameters (for parametric plans like LineScan).
    pub parameters: Option<serde_json::Value>,
    /// Device role mappings.
    pub device_mapping: Option<serde_json::Value>,
}

/// Lightweight summary of a plan for list operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanSummary {
    pub plan_id: String,
    pub name: String,
    pub plan_type: String,
    pub num_points: Option<i64>,
}

/// A run execution record stored in SQLite.
///
/// Field-compatible with [`crate::experiment_store::DbRunRecord`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbRunRecord {
    /// Unique run identifier (from RunEngine).
    pub run_uid: String,
    /// Plan that spawned this run.
    pub plan_id: Option<String>,
    /// Plan type (e.g., "graph_plan", "line_scan").
    pub plan_type: String,
    /// Human-readable plan name.
    pub plan_name: String,
    /// Current status: "queued", "running", "completed", "failed", "aborted".
    pub status: String,
    /// Number of events emitted.
    pub num_events: Option<i64>,
    /// User-provided metadata.
    pub metadata: Option<serde_json::Value>,
    /// Exit reason (for non-success statuses).
    pub exit_reason: Option<String>,
}

/// A run record that has a stale heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaleRun {
    /// Unique run identifier.
    pub run_uid: String,
    /// When the run started (ISO-8601 text).
    pub started_at: Option<String>,
}

/// A device parameter's persisted runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceParamState {
    /// Device ID this state belongs to.
    pub device_id: String,
    /// Parameter name.
    pub param_name: String,
    /// Last-known parameter value as JSON text.
    pub param_value: serde_json::Value,
    /// Whether this parameter is pinned to the UI quick-access section.
    #[serde(default)]
    pub is_favorite: bool,
}

/// A device lifecycle state transition event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceLifecycleEvent {
    /// Device ID that transitioned.
    pub device_id: String,
    /// State before the transition.
    pub from_state: String,
    /// State after the transition.
    pub to_state: String,
    /// Human-readable reason for the transition.
    pub reason: Option<String>,
    /// ISO-8601 timestamp (populated on read).
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// Diagnostic snapshot of the SQLite database state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteDbInfo {
    pub engine: String,
    pub schema_version: u32,
    pub uptime_secs: u64,
    pub healthy: bool,
    pub driver_count: u64,
    pub instrument_count: u64,
    pub plan_count: u64,
    pub run_count: u64,
    pub device_feature_count: u64,
    pub device_state_count: u64,
    pub lifecycle_event_count: u64,
}

// ---------------------------------------------------------------------------
// Change notifications (replaces LIVE SELECT)
// ---------------------------------------------------------------------------

/// Lightweight change event broadcast from the SQLite backend.
///
/// Subscribers receive these via [`SqliteDb::subscribe_changes`] to replace
/// the SurrealDB `LIVE SELECT` mechanism.
#[derive(Clone, Debug)]
pub enum DbChangeEvent {
    /// The `instrument` table was modified (insert, update, or delete).
    InstrumentsUpdated,
    /// The `driver` table was modified.
    DriversUpdated,
    /// Device features were modified for a specific device.
    DeviceFeaturesUpdated { device_id: String },
    /// The `experiment_plan` table was modified.
    ExperimentPlansUpdated,
    /// A run record was created or updated.
    RunRecordUpdated { run_uid: String },
    /// Device runtime state was modified.
    DeviceStateUpdated { device_id: String },
    /// A lifecycle event was recorded.
    LifecycleEventRecorded { device_id: String },
}

// ---------------------------------------------------------------------------
// SqliteDb
// ---------------------------------------------------------------------------

/// Async SQLite database handle for the control plane.
///
/// Wraps [`tokio_rusqlite::Connection`] which spawns a dedicated background
/// thread and proxies calls via an async channel.  All public methods are
/// `async` and safe to call from Tokio tasks.
#[derive(Clone)]
pub struct SqliteDb {
    conn: tokio_rusqlite::Connection,
    change_tx: broadcast::Sender<DbChangeEvent>,
    started_at: Instant,
}

impl std::fmt::Debug for SqliteDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteDb")
            .field("change_subscribers", &self.change_tx.receiver_count())
            .finish_non_exhaustive()
    }
}

/// SQL statements that create all tables and indexes.
const SCHEMA_SQL: &str = r"
-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    key TEXT PRIMARY KEY DEFAULT 'current',
    version INTEGER NOT NULL DEFAULT 9
);
INSERT OR IGNORE INTO schema_version (key, version) VALUES ('current', 9);

-- Instrument instances (physical devices)
CREATE TABLE IF NOT EXISTS instrument (
    device_id  TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    driver_type TEXT NOT NULL,
    config     TEXT NOT NULL DEFAULT '{}',
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_instrument_device_id ON instrument(device_id);

-- Driver definitions
CREATE TABLE IF NOT EXISTS driver (
    driver_type TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    capabilities TEXT NOT NULL DEFAULT '[]',
    commands TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Device feature metadata cache
CREATE TABLE IF NOT EXISTS device_feature (
    device_id TEXT NOT NULL,
    feature_name TEXT NOT NULL,
    feature_type TEXT NOT NULL DEFAULT '',
    readable INTEGER NOT NULL DEFAULT 1,
    writable INTEGER NOT NULL DEFAULT 1,
    min_value REAL,
    max_value REAL,
    step REAL,
    enum_values TEXT NOT NULL DEFAULT '[]',
    unit TEXT,
    description TEXT,
    group_name TEXT,
    discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (device_id, feature_name)
);

-- Experiment plan definitions
CREATE TABLE IF NOT EXISTS experiment_plan (
    plan_id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    description TEXT,
    plan_type TEXT NOT NULL DEFAULT '',
    commands TEXT NOT NULL DEFAULT '[]',
    movers TEXT NOT NULL DEFAULT '[]',
    detectors TEXT NOT NULL DEFAULT '[]',
    num_points INTEGER,
    graph_data TEXT,
    parameters TEXT,
    device_mapping TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Run execution history
CREATE TABLE IF NOT EXISTS run_record (
    run_uid TEXT PRIMARY KEY,
    plan_id TEXT,
    plan_type TEXT NOT NULL DEFAULT '',
    plan_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'queued',
    num_events INTEGER,
    metadata TEXT,
    exit_reason TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT,
    heartbeat_at TEXT
);

-- Device runtime state: persists last-known parameter values
CREATE TABLE IF NOT EXISTS device_runtime_state (
    device_id TEXT NOT NULL,
    param_name TEXT NOT NULL,
    param_value TEXT NOT NULL DEFAULT '',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (device_id, param_name)
);

-- Device lifecycle event log
CREATE TABLE IF NOT EXISTS device_lifecycle_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL,
    from_state TEXT NOT NULL DEFAULT '',
    to_state TEXT NOT NULL,
    reason TEXT,
    timestamp TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_lifecycle_device ON device_lifecycle_event(device_id, timestamp);
";

// ---------------------------------------------------------------------------
// DbConfig
// ---------------------------------------------------------------------------

/// Configuration for [`SqliteDb`].
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// File path for persistent storage, or `None` for in-memory.
    pub path: Option<String>,
}

impl DbConfig {
    /// In-memory database (useful for tests and development).
    #[must_use]
    pub fn in_memory() -> Self {
        Self { path: None }
    }

    /// File-backed SQLite database at the given path.
    #[must_use]
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// Alias for [`Self::file`] — accepts a `PathBuf` for compatibility with
    /// callers that previously used `DbConfig::rocksdb(path)`.
    pub fn rocksdb(path: impl AsRef<std::path::Path>) -> Self {
        Self {
            path: Some(path.as_ref().display().to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Convert a TOML value to a JSON value (recursive).
pub fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::json!(*i),
        toml::Value::Float(f) => serde_json::json!(*f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(a) => serde_json::Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            let map = t
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

/// Convert a JSON value to a TOML value (recursive).
pub fn json_to_toml(v: &serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else {
                toml::Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Array(a) => toml::Value::Array(a.iter().map(json_to_toml).collect()),
        serde_json::Value::Object(m) => {
            let map = m
                .iter()
                .map(|(k, v)| (k.clone(), json_to_toml(v)))
                .collect();
            toml::Value::Table(map)
        }
    }
}

/// Compute a hash of a JSON config value for change detection.
pub fn config_hash(config: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let s = serde_json::to_string(config).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

impl SqliteDb {
    /// Initialize from a [`DbConfig`].
    ///
    /// This is the primary constructor — matches `DaqDb::init(config)` from the
    /// SurrealDB backend so callers don't need to change.
    pub async fn init(config: DbConfig) -> Result<Self> {
        match config.path {
            Some(path) => Self::open(&path).await,
            None => Self::init_memory().await,
        }
    }

    /// Open (or create) a SQLite database at `path` and apply the schema.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Database`] if the file cannot be opened or the
    /// schema migration fails.
    pub async fn open(path: &str) -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open(path).await?;
        let (change_tx, _) = broadcast::channel(64);
        let db = Self {
            conn,
            change_tx,
            started_at: Instant::now(),
        };
        db.apply_schema().await?;
        info!(path, "SQLite database initialized");
        Ok(db)
    }

    /// Open an in-memory SQLite database (useful for tests).
    pub async fn init_memory() -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open_in_memory().await?;
        let (change_tx, _) = broadcast::channel(64);
        let db = Self {
            conn,
            change_tx,
            started_at: Instant::now(),
        };
        db.apply_schema().await?;
        info!("SQLite in-memory database initialized");
        Ok(db)
    }

    /// Apply the schema DDL.  Idempotent (uses `IF NOT EXISTS`).
    async fn apply_schema(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute_batch(SCHEMA_SQL)?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // ===================================================================
    // Instruments
    // ===================================================================

    /// Upsert a batch of instruments.
    ///
    /// Uses `INSERT ... ON CONFLICT` keyed on `device_id`.  Broadcasts
    /// [`DbChangeEvent::InstrumentsUpdated`] once after the batch completes.
    /// Returns an [`ImportReport`] for compatibility with downstream callers.
    pub async fn upsert_instruments(&self, instruments: &[DbInstrument]) -> Result<ImportReport> {
        let count = instruments.len();
        let instruments = instruments.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for inst in &instruments {
                    let config_json = serde_json::to_string(&inst.config)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    tx.execute(
                        "INSERT INTO instrument \
                         (device_id, name, driver_type, config, enabled, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
                         ON CONFLICT(device_id) DO UPDATE SET \
                         name = excluded.name, \
                         driver_type = excluded.driver_type, \
                         config = excluded.config, \
                         enabled = excluded.enabled, \
                         updated_at = datetime('now')",
                        rusqlite::params![
                            inst.device_id,
                            inst.name,
                            inst.driver_type,
                            config_json,
                            inst.enabled,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;

        // Notify subscribers (best-effort -- ignore send errors when no receivers).
        let _ = self.change_tx.send(DbChangeEvent::InstrumentsUpdated);
        Ok(ImportReport {
            instruments_upserted: count,
            ..ImportReport::default()
        })
    }

    /// Retrieve a single instrument by device ID.
    pub async fn get_instrument(&self, device_id: &str) -> Result<Option<DbInstrument>> {
        let device_id = device_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, name, driver_type, config, enabled \
                     FROM instrument WHERE device_id = ?1",
                )?;
                let mut rows = stmt.query(rusqlite::params![device_id])?;
                match rows.next()? {
                    Some(row) => Ok(Some(row_to_instrument(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(Into::into)
    }

    /// Retrieve all instruments, ordered by `device_id`.
    pub async fn get_all_instruments(&self) -> Result<Vec<DbInstrument>> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, name, driver_type, config, enabled \
                     FROM instrument ORDER BY device_id",
                )?;
                let rows = stmt.query_map([], row_to_instrument)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// List instruments ordered by `name`.
    pub async fn list_instruments(&self) -> Result<Vec<DbInstrument>> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, name, driver_type, config, enabled \
                     FROM instrument ORDER BY name",
                )?;
                let rows = stmt.query_map([], row_to_instrument)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Delete an instrument by device ID.  Returns `true` if a row was removed.
    pub async fn delete_instrument(&self, device_id: &str) -> Result<bool> {
        let device_id = device_id.to_owned();
        let deleted = self
            .conn
            .call(move |conn| {
                let count =
                    conn.execute("DELETE FROM instrument WHERE device_id = ?1", [&device_id])?;
                Ok(count > 0)
            })
            .await?;

        if deleted {
            let _ = self.change_tx.send(DbChangeEvent::InstrumentsUpdated);
        }
        Ok(deleted)
    }

    // ===================================================================
    // Drivers
    // ===================================================================

    /// Upsert a batch of drivers.
    ///
    /// Uses `INSERT ... ON CONFLICT` keyed on `driver_type`.
    /// Returns the number of drivers upserted.
    pub async fn upsert_drivers(&self, drivers: &[DbDriver]) -> Result<usize> {
        let count = drivers.len();
        let drivers = drivers.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for drv in &drivers {
                    let capabilities_json = serde_json::to_string(&drv.capabilities)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    let commands_json = serde_json::to_string(&drv.commands)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    tx.execute(
                        "INSERT INTO driver \
                         (driver_type, name, capabilities, commands, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
                         ON CONFLICT(driver_type) DO UPDATE SET \
                         name = excluded.name, \
                         capabilities = excluded.capabilities, \
                         commands = excluded.commands, \
                         updated_at = datetime('now')",
                        rusqlite::params![
                            drv.driver_type,
                            drv.name,
                            capabilities_json,
                            commands_json,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::DriversUpdated);
        Ok(count)
    }

    /// Retrieve all drivers, ordered by `driver_type`.
    pub async fn get_all_drivers(&self) -> Result<Vec<DbDriver>> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT driver_type, name, capabilities, commands \
                     FROM driver ORDER BY driver_type",
                )?;
                let rows = stmt.query_map([], row_to_driver)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    // ===================================================================
    // Device Features
    // ===================================================================

    /// Upsert device feature metadata in a single transaction.
    ///
    /// Uses `(device_id, feature_name)` as the unique key.
    pub async fn upsert_device_features(&self, features: &[DbDeviceFeature]) -> Result<usize> {
        if features.is_empty() {
            return Ok(0);
        }
        let count = features.len();
        // Clone for notifications after the closure consumes the data.
        let device_ids_for_notify: Vec<String> =
            features.iter().map(|f| f.device_id.clone()).collect();
        let features_owned = features.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for feat in &features_owned {
                    let enum_values_json = serde_json::to_string(&feat.enum_values)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    tx.execute(
                        "INSERT OR REPLACE INTO device_feature \
                         (device_id, feature_name, feature_type, readable, writable, \
                          min_value, max_value, step, enum_values, unit, description, \
                          group_name, discovered_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))",
                        rusqlite::params![
                            feat.device_id,
                            feat.feature_name,
                            feat.feature_type,
                            feat.readable,
                            feat.writable,
                            feat.min_value,
                            feat.max_value,
                            feat.step,
                            enum_values_json,
                            feat.unit,
                            feat.description,
                            feat.group_name,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;

        // Emit one notification per unique device_id in the batch.
        let mut seen = std::collections::HashSet::new();
        for device_id in device_ids_for_notify {
            if seen.insert(device_id.clone()) {
                let _ = self
                    .change_tx
                    .send(DbChangeEvent::DeviceFeaturesUpdated { device_id });
            }
        }
        Ok(count)
    }

    /// Retrieve all feature metadata for a specific device.
    pub async fn get_device_features(&self, device_id: &str) -> Result<Vec<DbDeviceFeature>> {
        let device_id = device_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, feature_name, feature_type, readable, writable, \
                     min_value, max_value, step, enum_values, unit, description, group_name \
                     FROM device_feature WHERE device_id = ?1 \
                     ORDER BY feature_name",
                )?;
                let rows = stmt.query_map(rusqlite::params![device_id], row_to_device_feature)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Delete all feature metadata for a specific device.
    pub async fn delete_device_features(&self, device_id: &str) -> Result<usize> {
        let device_id_owned = device_id.to_owned();
        let count = self
            .conn
            .call(move |conn| {
                let c = conn.execute(
                    "DELETE FROM device_feature WHERE device_id = ?1",
                    [&device_id_owned],
                )?;
                Ok(c)
            })
            .await?;

        if count > 0 {
            let _ = self.change_tx.send(DbChangeEvent::DeviceFeaturesUpdated {
                device_id: device_id.to_owned(),
            });
        }
        Ok(count)
    }

    // ===================================================================
    // Experiment Plans
    // ===================================================================

    /// Upsert an experiment plan (insert or update by `plan_id`).
    pub async fn upsert_experiment_plan(&self, plan: &DbExperimentPlan) -> Result<()> {
        let plan = plan.clone();
        self.conn
            .call(move |conn| {
                let commands_json = opt_vec_json(plan.commands.as_ref())?;
                let movers_json = opt_string_vec_json(plan.movers.as_ref())?;
                let detectors_json = opt_string_vec_json(plan.detectors.as_ref())?;
                let graph_data_json = opt_value_json(plan.graph_data.as_ref())?;
                let parameters_json = opt_value_json(plan.parameters.as_ref())?;
                let device_mapping_json = opt_value_json(plan.device_mapping.as_ref())?;

                conn.execute(
                    "INSERT OR REPLACE INTO experiment_plan \
                     (plan_id, name, description, plan_type, commands, movers, \
                      detectors, num_points, graph_data, parameters, device_mapping, \
                      updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'))",
                    rusqlite::params![
                        plan.plan_id,
                        plan.name,
                        plan.description,
                        plan.plan_type,
                        commands_json,
                        movers_json,
                        detectors_json,
                        plan.num_points,
                        graph_data_json,
                        parameters_json,
                        device_mapping_json,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::ExperimentPlansUpdated);
        Ok(())
    }

    /// Retrieve an experiment plan by `plan_id`.
    pub async fn get_experiment_plan(&self, plan_id: &str) -> Result<Option<DbExperimentPlan>> {
        let plan_id = plan_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT plan_id, name, description, plan_type, commands, movers, \
                     detectors, num_points, graph_data, parameters, device_mapping \
                     FROM experiment_plan WHERE plan_id = ?1",
                )?;
                let mut rows = stmt.query(rusqlite::params![plan_id])?;
                match rows.next()? {
                    Some(row) => Ok(Some(row_to_experiment_plan(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(Into::into)
    }

    /// List all experiment plans (lightweight summary), newest first.
    pub async fn list_experiment_plans(&self) -> Result<Vec<PlanSummary>> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT plan_id, name, plan_type, num_points \
                     FROM experiment_plan ORDER BY updated_at DESC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(PlanSummary {
                        plan_id: row.get(0)?,
                        name: row.get(1)?,
                        plan_type: row.get(2)?,
                        num_points: row.get(3)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Delete an experiment plan by `plan_id`. Returns `true` if it existed.
    pub async fn delete_experiment_plan(&self, plan_id: &str) -> Result<bool> {
        let plan_id = plan_id.to_owned();
        let deleted = self
            .conn
            .call(move |conn| {
                let count =
                    conn.execute("DELETE FROM experiment_plan WHERE plan_id = ?1", [&plan_id])?;
                Ok(count > 0)
            })
            .await?;

        if deleted {
            let _ = self.change_tx.send(DbChangeEvent::ExperimentPlansUpdated);
        }
        Ok(deleted)
    }

    // ===================================================================
    // Run Records
    // ===================================================================

    /// Create a new run record.
    pub async fn create_run_record(&self, record: &DbRunRecord) -> Result<()> {
        let run_uid_notify = record.run_uid.clone();
        let record = record.clone();
        self.conn
            .call(move |conn| {
                let metadata_json = opt_value_json(record.metadata.as_ref())?;
                conn.execute(
                    "INSERT OR REPLACE INTO run_record \
                     (run_uid, plan_id, plan_type, plan_name, status, \
                      num_events, metadata, started_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                    rusqlite::params![
                        record.run_uid,
                        record.plan_id,
                        record.plan_type,
                        record.plan_name,
                        record.status,
                        record.num_events,
                        metadata_json,
                    ],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::RunRecordUpdated {
            run_uid: run_uid_notify,
        });
        Ok(())
    }

    /// Update a run record's status.
    pub async fn update_run_status(&self, run_uid: &str, status: &str) -> Result<()> {
        let run_uid_notify = run_uid.to_owned();
        let run_uid = run_uid.to_owned();
        let status = status.to_owned();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE run_record SET status = ?1 WHERE run_uid = ?2",
                    rusqlite::params![status, run_uid],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::RunRecordUpdated {
            run_uid: run_uid_notify,
        });
        Ok(())
    }

    /// Finish a run record (set final status, event count, and timestamp).
    pub async fn finish_run(
        &self,
        run_uid: &str,
        status: &str,
        num_events: i64,
        reason: Option<&str>,
    ) -> Result<()> {
        let run_uid_notify = run_uid.to_owned();
        let run_uid = run_uid.to_owned();
        let status = status.to_owned();
        let reason = reason.map(str::to_owned);
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE run_record SET \
                     status = ?1, num_events = ?2, exit_reason = ?3, \
                     finished_at = datetime('now') \
                     WHERE run_uid = ?4",
                    rusqlite::params![status, num_events, reason, run_uid],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::RunRecordUpdated {
            run_uid: run_uid_notify,
        });
        Ok(())
    }

    /// List recent run records.
    pub async fn list_runs(&self, limit: Option<u32>) -> Result<Vec<DbRunRecord>> {
        let limit = limit.unwrap_or(50);
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT run_uid, plan_id, plan_type, plan_name, status, \
                     num_events, metadata, exit_reason \
                     FROM run_record ORDER BY started_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![limit], row_to_run_record)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    // ===================================================================
    // Heartbeat Monitoring
    // ===================================================================

    /// Update the heartbeat timestamp for an active run.
    pub async fn update_run_heartbeat(&self, run_uid: &str) -> Result<()> {
        let run_uid = run_uid.to_owned();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE run_record SET heartbeat_at = datetime('now') \
                     WHERE run_uid = ?1",
                    [&run_uid],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Find runs with stale heartbeats (daemon likely crashed).
    ///
    /// Returns runs where `status = 'running'` and either:
    /// - `heartbeat_at` is NULL (run never sent a heartbeat), or
    /// - `heartbeat_at` is older than `stale_threshold`.
    pub async fn find_stale_runs(&self, stale_threshold: Duration) -> Result<Vec<StaleRun>> {
        let threshold_secs = stale_threshold.as_secs();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT run_uid, started_at FROM run_record \
                     WHERE status = 'running' AND (\
                         heartbeat_at IS NULL OR \
                         heartbeat_at < datetime('now', '-' || ?1 || ' seconds')\
                     )",
                )?;
                let rows = stmt.query_map(rusqlite::params![threshold_secs], |row| {
                    Ok(StaleRun {
                        run_uid: row.get(0)?,
                        started_at: row.get(1)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    // ===================================================================
    // Device Runtime State
    // ===================================================================

    /// Upsert a single device parameter's runtime state.
    pub async fn upsert_device_state(
        &self,
        device_id: &str,
        param_name: &str,
        param_value: &serde_json::Value,
    ) -> Result<()> {
        let device_id_notify = device_id.to_owned();
        let device_id = device_id.to_owned();
        let param_name = param_name.to_owned();
        let value_json =
            serde_json::to_string(param_value).map_err(crate::error::DbError::Serde)?;
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO device_runtime_state \
                     (device_id, param_name, param_value, \
                      is_favorite, updated_at) \
                     VALUES (?1, ?2, ?3, \
                      COALESCE((SELECT is_favorite FROM device_runtime_state \
                                WHERE device_id = ?1 AND param_name = ?2), 0), \
                      datetime('now'))",
                    rusqlite::params![device_id, param_name, value_json],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::DeviceStateUpdated {
            device_id: device_id_notify,
        });
        Ok(())
    }

    /// Batch upsert multiple parameter states in a single transaction.
    pub async fn batch_upsert_device_state(
        &self,
        states: &[(String, String, serde_json::Value)],
    ) -> Result<()> {
        if states.is_empty() {
            return Ok(());
        }

        // Pre-serialize JSON values before moving into closure.
        let serialized: Vec<(String, String, String)> = states
            .iter()
            .map(|(device_id, param_name, param_value)| {
                let json =
                    serde_json::to_string(param_value).map_err(crate::error::DbError::Serde)?;
                Ok((device_id.clone(), param_name.clone(), json))
            })
            .collect::<Result<Vec<_>>>()?;

        let device_ids: Vec<String> = states.iter().map(|(d, _, _)| d.clone()).collect();

        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for (device_id, param_name, value_json) in &serialized {
                    tx.execute(
                        "INSERT OR REPLACE INTO device_runtime_state \
                         (device_id, param_name, param_value, \
                          is_favorite, updated_at) \
                         VALUES (?1, ?2, ?3, \
                          COALESCE((SELECT is_favorite FROM device_runtime_state \
                                    WHERE device_id = ?1 AND param_name = ?2), 0), \
                          datetime('now'))",
                        rusqlite::params![device_id, param_name, value_json],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;

        // Notify per unique device_id.
        let mut seen = std::collections::HashSet::new();
        for device_id in &device_ids {
            if seen.insert(device_id.clone()) {
                let _ = self.change_tx.send(DbChangeEvent::DeviceStateUpdated {
                    device_id: device_id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Retrieve all persisted runtime state for a device.
    pub async fn get_device_state(&self, device_id: &str) -> Result<Vec<DeviceParamState>> {
        let device_id = device_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, param_name, param_value, is_favorite \
                     FROM device_runtime_state WHERE device_id = ?1 \
                     ORDER BY param_name",
                )?;
                let rows =
                    stmt.query_map(rusqlite::params![device_id], row_to_device_param_state)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Set the `is_favorite` flag for a specific parameter.
    ///
    /// Creates the record if it doesn't exist (with empty param_value).
    pub async fn set_parameter_favorite(
        &self,
        device_id: &str,
        param_name: &str,
        is_favorite: bool,
    ) -> Result<()> {
        let device_id_notify = device_id.to_owned();
        let device_id = device_id.to_owned();
        let param_name = param_name.to_owned();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO device_runtime_state \
                     (device_id, param_name, param_value, is_favorite, updated_at) \
                     VALUES (?1, ?2, \
                      COALESCE((SELECT param_value FROM device_runtime_state \
                                WHERE device_id = ?1 AND param_name = ?2), ''), \
                      ?3, datetime('now')) \
                     ON CONFLICT(device_id, param_name) \
                     DO UPDATE SET is_favorite = ?3, updated_at = datetime('now')",
                    rusqlite::params![device_id, param_name, is_favorite],
                )?;
                Ok(())
            })
            .await?;

        let _ = self.change_tx.send(DbChangeEvent::DeviceStateUpdated {
            device_id: device_id_notify,
        });
        Ok(())
    }

    /// Get all favorite parameter names for a device.
    pub async fn get_favorites(&self, device_id: &str) -> Result<Vec<String>> {
        let device_id = device_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT param_name FROM device_runtime_state \
                     WHERE device_id = ?1 AND is_favorite = 1 \
                     ORDER BY param_name",
                )?;
                let rows =
                    stmt.query_map(rusqlite::params![device_id], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Delete all persisted runtime state for a device.
    pub async fn delete_device_state(&self, device_id: &str) -> Result<usize> {
        let device_id_owned = device_id.to_owned();
        let count = self
            .conn
            .call(move |conn| {
                let c = conn.execute(
                    "DELETE FROM device_runtime_state WHERE device_id = ?1",
                    [&device_id_owned],
                )?;
                Ok(c)
            })
            .await?;

        if count > 0 {
            let _ = self.change_tx.send(DbChangeEvent::DeviceStateUpdated {
                device_id: device_id.to_owned(),
            });
        }
        Ok(count)
    }

    // ===================================================================
    // Device Lifecycle Events
    // ===================================================================

    /// Record a device lifecycle state transition.
    ///
    /// Also updates the `_lifecycle_state` pseudo-parameter in
    /// `device_runtime_state` so the current state is queryable.
    pub async fn record_lifecycle_event(&self, event: &DeviceLifecycleEvent) -> Result<()> {
        let event = event.clone();
        let device_id_for_state = event.device_id.clone();
        let to_state_for_state = event.to_state.clone();

        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO device_lifecycle_event \
                     (device_id, from_state, to_state, reason, timestamp) \
                     VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                    rusqlite::params![
                        event.device_id,
                        event.from_state,
                        event.to_state,
                        event.reason,
                    ],
                )?;
                Ok(())
            })
            .await?;

        // Also persist the current state for restart recovery.
        self.upsert_device_state(
            &device_id_for_state,
            "_lifecycle_state",
            &serde_json::json!(to_state_for_state),
        )
        .await?;

        let _ = self.change_tx.send(DbChangeEvent::LifecycleEventRecorded {
            device_id: device_id_for_state,
        });
        Ok(())
    }

    /// Get the most recent lifecycle events for a device, newest first.
    pub async fn get_lifecycle_events(
        &self,
        device_id: &str,
        limit: u32,
    ) -> Result<Vec<DeviceLifecycleEvent>> {
        let device_id = device_id.to_owned();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, from_state, to_state, reason, timestamp \
                     FROM device_lifecycle_event \
                     WHERE device_id = ?1 \
                     ORDER BY timestamp DESC, id DESC \
                     LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(rusqlite::params![device_id, limit], row_to_lifecycle_event)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(Into::into)
    }

    /// Delete all lifecycle events for a device.
    pub async fn delete_lifecycle_events(&self, device_id: &str) -> Result<usize> {
        let device_id_owned = device_id.to_owned();
        let count = self
            .conn
            .call(move |conn| {
                let c = conn.execute(
                    "DELETE FROM device_lifecycle_event WHERE device_id = ?1",
                    [&device_id_owned],
                )?;
                Ok(c)
            })
            .await?;

        if count > 0 {
            let _ = self.change_tx.send(DbChangeEvent::LifecycleEventRecorded {
                device_id: device_id.to_owned(),
            });
        }
        Ok(count)
    }

    // ===================================================================
    // Change subscription
    // ===================================================================

    /// Subscribe to database change notifications.
    ///
    /// Returns a [`broadcast::Receiver`] that yields [`DbChangeEvent`]
    /// values whenever a write operation modifies a table.
    #[must_use]
    pub fn subscribe_changes(&self) -> broadcast::Receiver<DbChangeEvent> {
        self.change_tx.subscribe()
    }

    // ===================================================================
    // Health check
    // ===================================================================

    /// Lightweight health check -- verifies the connection is responsive.
    pub async fn health_check(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute_batch("SELECT 1")?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // ===================================================================
    // Info / diagnostics
    // ===================================================================

    /// Returns diagnostic information about the database.
    pub async fn info(&self) -> Result<SqliteDbInfo> {
        let uptime = self.started_at.elapsed();
        let healthy = self.health_check().await.is_ok();

        let counts = self
            .conn
            .call(|conn| {
                let count = |table: &str| -> rusqlite::Result<u64> {
                    let sql = format!("SELECT COUNT(*) FROM {table}");
                    conn.query_row(&sql, [], |row| row.get(0))
                };
                Ok((
                    count("driver")?,
                    count("instrument")?,
                    count("experiment_plan")?,
                    count("run_record")?,
                    count("device_feature")?,
                    count("device_runtime_state")?,
                    count("device_lifecycle_event")?,
                ))
            })
            .await?;

        Ok(SqliteDbInfo {
            engine: "sqlite".to_string(),
            schema_version: SCHEMA_VERSION,
            uptime_secs: uptime.as_secs(),
            healthy,
            driver_count: counts.0,
            instrument_count: counts.1,
            plan_count: counts.2,
            run_count: counts.3,
            device_feature_count: counts.4,
            device_state_count: counts.5,
            lifecycle_event_count: counts.6,
        })
    }

    /// Returns the schema version stored in the database.
    pub async fn schema_version(&self) -> Result<u32> {
        self.conn
            .call(|conn| {
                let version: u32 = conn.query_row(
                    "SELECT version FROM schema_version WHERE key = 'current'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(version)
            })
            .await
            .map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

/// Map a `rusqlite::Row` to a `DbInstrument`.
fn row_to_instrument(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbInstrument> {
    let config_str: String = row.get(3)?;
    let config: serde_json::Value =
        serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(DbInstrument {
        device_id: row.get(0)?,
        name: row.get(1)?,
        driver_type: row.get(2)?,
        config,
        enabled: row.get(4)?,
    })
}

/// Map a `rusqlite::Row` to a `DbDriver`.
fn row_to_driver(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbDriver> {
    let capabilities_str: String = row.get(2)?;
    let commands_str: String = row.get(3)?;
    let capabilities: Vec<String> = serde_json::from_str(&capabilities_str).unwrap_or_default();
    let commands: Vec<String> = serde_json::from_str(&commands_str).unwrap_or_default();
    Ok(DbDriver {
        driver_type: row.get(0)?,
        name: row.get(1)?,
        capabilities,
        commands,
    })
}

/// Map a `rusqlite::Row` to a `DbDeviceFeature`.
fn row_to_device_feature(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbDeviceFeature> {
    let enum_values_str: String = row.get(8)?;
    let enum_values: Vec<String> = serde_json::from_str(&enum_values_str).unwrap_or_default();
    Ok(DbDeviceFeature {
        device_id: row.get(0)?,
        feature_name: row.get(1)?,
        feature_type: row.get(2)?,
        readable: row.get(3)?,
        writable: row.get(4)?,
        min_value: row.get(5)?,
        max_value: row.get(6)?,
        step: row.get(7)?,
        enum_values,
        unit: row.get(9)?,
        description: row.get(10)?,
        group_name: row.get(11)?,
    })
}

/// Map a `rusqlite::Row` to a `DbExperimentPlan`.
fn row_to_experiment_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbExperimentPlan> {
    let commands_str: Option<String> = row.get(4)?;
    let movers_str: Option<String> = row.get(5)?;
    let detectors_str: Option<String> = row.get(6)?;
    let graph_data_str: Option<String> = row.get(8)?;
    let parameters_str: Option<String> = row.get(9)?;
    let device_mapping_str: Option<String> = row.get(10)?;

    let commands: Option<Vec<serde_json::Value>> = commands_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let movers: Option<Vec<String>> = movers_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let detectors: Option<Vec<String>> = detectors_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let graph_data: Option<serde_json::Value> = graph_data_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let parameters: Option<serde_json::Value> = parameters_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let device_mapping: Option<serde_json::Value> = device_mapping_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(DbExperimentPlan {
        plan_id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        plan_type: row.get(3)?,
        commands,
        movers,
        detectors,
        num_points: row.get(7)?,
        graph_data,
        parameters,
        device_mapping,
    })
}

/// Map a `rusqlite::Row` to a `DbRunRecord`.
fn row_to_run_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbRunRecord> {
    let metadata_str: Option<String> = row.get(6)?;
    let metadata: Option<serde_json::Value> = metadata_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    Ok(DbRunRecord {
        run_uid: row.get(0)?,
        plan_id: row.get(1)?,
        plan_type: row.get(2)?,
        plan_name: row.get(3)?,
        status: row.get(4)?,
        num_events: row.get(5)?,
        metadata,
        exit_reason: row.get(7)?,
    })
}

/// Map a `rusqlite::Row` to a `DeviceParamState`.
fn row_to_device_param_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceParamState> {
    let value_str: String = row.get(2)?;
    let param_value: serde_json::Value =
        serde_json::from_str(&value_str).unwrap_or(serde_json::Value::String(value_str));
    Ok(DeviceParamState {
        device_id: row.get(0)?,
        param_name: row.get(1)?,
        param_value,
        is_favorite: row.get(3)?,
    })
}

/// Map a `rusqlite::Row` to a `DeviceLifecycleEvent`.
fn row_to_lifecycle_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceLifecycleEvent> {
    Ok(DeviceLifecycleEvent {
        device_id: row.get(0)?,
        from_state: row.get(1)?,
        to_state: row.get(2)?,
        reason: row.get(3)?,
        timestamp: row.get(4)?,
    })
}

// ---------------------------------------------------------------------------
// JSON serialization helpers
// ---------------------------------------------------------------------------

/// Serialize an `Option<Vec<serde_json::Value>>` to a JSON string.
fn opt_vec_json(
    v: Option<&Vec<serde_json::Value>>,
) -> std::result::Result<Option<String>, rusqlite::Error> {
    match v {
        Some(vec) => serde_json::to_string(vec)
            .map(Some)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        None => Ok(None),
    }
}

/// Serialize an `Option<Vec<String>>` to a JSON string.
fn opt_string_vec_json(
    v: Option<&Vec<String>>,
) -> std::result::Result<Option<String>, rusqlite::Error> {
    match v {
        Some(vec) => serde_json::to_string(vec)
            .map(Some)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        None => Ok(None),
    }
}

/// Serialize an `Option<serde_json::Value>` to a JSON string.
fn opt_value_json(
    v: Option<&serde_json::Value>,
) -> std::result::Result<Option<String>, rusqlite::Error> {
    match v {
        Some(val) => serde_json::to_string(val)
            .map(Some)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // =======================================================================
    // Instrument tests
    // =======================================================================

    fn sample_instrument(id: &str) -> DbInstrument {
        DbInstrument {
            device_id: id.to_owned(),
            name: format!("Test Device {id}"),
            driver_type: "mock".into(),
            config: serde_json::json!({"port": "/dev/null", "baud": 9600}),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_init_memory() {
        let db = SqliteDb::init_memory()
            .await
            .expect("init_memory should succeed");
        db.health_check().await.expect("health check should pass");
    }

    #[tokio::test]
    async fn test_crud_cycle() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Create
        let inst = sample_instrument("stage_1");
        db.upsert_instruments(std::slice::from_ref(&inst))
            .await
            .expect("upsert");

        // Read
        let fetched = db
            .get_instrument("stage_1")
            .await
            .expect("get")
            .expect("should exist");
        assert_eq!(fetched.device_id, "stage_1");
        assert_eq!(fetched.name, "Test Device stage_1");
        assert_eq!(fetched.driver_type, "mock");
        assert_eq!(
            fetched.config,
            serde_json::json!({"port": "/dev/null", "baud": 9600})
        );
        assert!(fetched.enabled);

        // Update
        let mut updated = inst;
        updated.name = "Renamed Stage".into();
        updated.enabled = false;
        db.upsert_instruments(&[updated]).await.expect("upsert");
        let fetched = db
            .get_instrument("stage_1")
            .await
            .expect("get")
            .expect("should exist");
        assert_eq!(fetched.name, "Renamed Stage");
        assert!(!fetched.enabled);

        // Delete
        let deleted = db.delete_instrument("stage_1").await.expect("delete");
        assert!(deleted);
        let gone = db.get_instrument("stage_1").await.expect("get");
        assert!(gone.is_none());

        // Delete nonexistent returns false
        let deleted_again = db.delete_instrument("stage_1").await.expect("delete");
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_upsert_idempotency() {
        let db = SqliteDb::init_memory().await.expect("init");

        let inst = sample_instrument("cam_0");

        // Upsert the same instrument three times.
        for _ in 0..3 {
            db.upsert_instruments(std::slice::from_ref(&inst))
                .await
                .expect("upsert");
        }

        // Should still be exactly one record.
        let all = db.get_all_instruments().await.expect("get_all");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].device_id, "cam_0");
    }

    #[tokio::test]
    async fn test_get_all_and_list_ordering() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Insert instruments with names that sort differently than device_ids.
        let instruments = vec![
            DbInstrument {
                device_id: "z_stage".into(),
                name: "Alpha Stage".into(),
                driver_type: "mock".into(),
                config: serde_json::json!({}),
                enabled: true,
            },
            DbInstrument {
                device_id: "a_cam".into(),
                name: "Zeta Camera".into(),
                driver_type: "mock".into(),
                config: serde_json::json!({}),
                enabled: true,
            },
        ];
        db.upsert_instruments(&instruments).await.expect("upsert");

        // get_all_instruments: ordered by device_id
        let all = db.get_all_instruments().await.expect("get_all");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].device_id, "a_cam");
        assert_eq!(all[1].device_id, "z_stage");

        // list_instruments: ordered by name
        let listed = db.list_instruments().await.expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "Alpha Stage");
        assert_eq!(listed[1].name, "Zeta Camera");
    }

    #[tokio::test]
    async fn test_broadcast_notification() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut rx = db.subscribe_changes();

        let inst = sample_instrument("notifier_test");
        db.upsert_instruments(&[inst]).await.expect("upsert");

        // Should receive a notification.
        let event = rx.try_recv().expect("should have received a change event");
        assert!(matches!(event, DbChangeEvent::InstrumentsUpdated));
    }

    #[tokio::test]
    async fn test_broadcast_on_delete() {
        let db = SqliteDb::init_memory().await.expect("init");

        let inst = sample_instrument("del_notify");
        db.upsert_instruments(&[inst]).await.expect("upsert");

        // Drain the upsert notification.
        let mut rx = db.subscribe_changes();

        let deleted = db.delete_instrument("del_notify").await.expect("delete");
        assert!(deleted);

        let event = rx
            .try_recv()
            .expect("should have received a delete notification");
        assert!(matches!(event, DbChangeEvent::InstrumentsUpdated));
    }

    #[tokio::test]
    async fn test_concurrent_readers() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Seed data
        let instruments: Vec<DbInstrument> = (0..10)
            .map(|i| sample_instrument(&format!("dev_{i}")))
            .collect();
        db.upsert_instruments(&instruments).await.expect("upsert");

        // Spawn several concurrent reads.
        let mut handles = Vec::new();
        for _ in 0..5 {
            let db_clone = db.clone();
            handles.push(tokio::spawn(async move {
                db_clone
                    .get_all_instruments()
                    .await
                    .expect("concurrent get_all")
            }));
        }

        for handle in handles {
            let result = handle.await.expect("task should not panic");
            assert_eq!(result.len(), 10);
        }
    }

    #[tokio::test]
    async fn test_json_config_roundtrip() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Complex nested config
        let inst = DbInstrument {
            device_id: "complex_cfg".into(),
            name: "Complex Config Device".into(),
            driver_type: "universal".into(),
            config: serde_json::json!({
                "serial": {
                    "port": "/dev/ttyUSB0",
                    "baud": 115200,
                    "parity": "none"
                },
                "commands": ["*IDN?", "MEAS:VOLT?"],
                "timeout_ms": 5000,
                "calibration": {
                    "offsets": [0.1, -0.2, 0.05],
                    "enabled": true
                }
            }),
            enabled: true,
        };

        db.upsert_instruments(std::slice::from_ref(&inst))
            .await
            .expect("upsert");
        let fetched = db
            .get_instrument("complex_cfg")
            .await
            .expect("get")
            .expect("should exist");

        assert_eq!(fetched.config, inst.config);
    }

    #[tokio::test]
    async fn test_file_backed_persistence() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let db_path = tmpdir.path().join("test.db");
        let path_str = db_path.to_str().expect("path to str");

        // Session 1: write data
        {
            let db = SqliteDb::open(path_str).await.expect("init");
            let inst = sample_instrument("persist_test");
            db.upsert_instruments(&[inst]).await.expect("upsert");
        }

        // Session 2: re-open and verify data persists
        {
            let db = SqliteDb::open(path_str).await.expect("init");
            let fetched = db
                .get_instrument("persist_test")
                .await
                .expect("get")
                .expect("should persist across sessions");
            assert_eq!(fetched.device_id, "persist_test");
            assert_eq!(fetched.name, "Test Device persist_test");
        }
    }

    #[tokio::test]
    async fn test_empty_database_queries() {
        let db = SqliteDb::init_memory().await.expect("init");

        // All queries on empty DB should return empty/None, not error.
        let all = db.get_all_instruments().await.expect("get_all");
        assert!(all.is_empty());

        let listed = db.list_instruments().await.expect("list");
        assert!(listed.is_empty());

        let single = db.get_instrument("nonexistent").await.expect("get");
        assert!(single.is_none());

        let deleted = db.delete_instrument("nonexistent").await.expect("delete");
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_batch_upsert() {
        let db = SqliteDb::init_memory().await.expect("init");

        let instruments: Vec<DbInstrument> = (0..50)
            .map(|i| sample_instrument(&format!("batch_{i:03}")))
            .collect();

        db.upsert_instruments(&instruments)
            .await
            .expect("batch upsert");

        let all = db.get_all_instruments().await.expect("get_all");
        assert_eq!(all.len(), 50);

        // Verify ordering (device_id is zero-padded so lexicographic = numeric).
        assert_eq!(all[0].device_id, "batch_000");
        assert_eq!(all[49].device_id, "batch_049");
    }

    // =======================================================================
    // Driver tests
    // =======================================================================

    fn sample_drivers() -> Vec<DbDriver> {
        vec![
            DbDriver {
                driver_type: "ell14".into(),
                name: "Thorlabs ELL14".into(),
                capabilities: vec!["movable".into()],
                commands: vec![],
            },
            DbDriver {
                driver_type: "newport1830_c".into(),
                name: "Newport 1830-C".into(),
                capabilities: vec!["readable".into()],
                commands: vec!["*IDN?".into()],
            },
        ]
    }

    #[tokio::test]
    async fn test_driver_upsert_and_get_all() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_drivers(&sample_drivers()).await.expect("upsert");

        let all = db.get_all_drivers().await.expect("get_all");
        assert_eq!(all.len(), 2);
        // Ordered by driver_type
        assert_eq!(all[0].driver_type, "ell14");
        assert_eq!(all[0].name, "Thorlabs ELL14");
        assert_eq!(all[0].capabilities, vec!["movable"]);
        assert!(all[0].commands.is_empty());

        assert_eq!(all[1].driver_type, "newport1830_c");
        assert_eq!(all[1].commands, vec!["*IDN?"]);
    }

    #[tokio::test]
    async fn test_driver_upsert_idempotent() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_drivers(&sample_drivers()).await.expect("upsert");
        db.upsert_drivers(&sample_drivers()).await.expect("upsert");

        let all = db.get_all_drivers().await.expect("get_all");
        assert_eq!(all.len(), 2, "upsert should not duplicate drivers");
    }

    #[tokio::test]
    async fn test_driver_update() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut drivers = sample_drivers();
        db.upsert_drivers(&drivers).await.expect("upsert");

        // Update name and add a capability.
        drivers[0].name = "ELL14 v2".into();
        drivers[0].capabilities.push("settable".into());
        db.upsert_drivers(&drivers).await.expect("upsert");

        let all = db.get_all_drivers().await.expect("get_all");
        assert_eq!(all[0].name, "ELL14 v2");
        assert_eq!(all[0].capabilities, vec!["movable", "settable"]);
    }

    #[tokio::test]
    async fn test_driver_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.upsert_drivers(&sample_drivers()).await.expect("upsert");

        let event = rx.try_recv().expect("should receive notification");
        assert!(matches!(event, DbChangeEvent::DriversUpdated));
    }

    // =======================================================================
    // Device Feature tests
    // =======================================================================

    fn sample_features() -> Vec<DbDeviceFeature> {
        vec![
            DbDeviceFeature {
                device_id: "andor_0".into(),
                feature_name: "ExposureTime".into(),
                feature_type: "float".into(),
                readable: true,
                writable: true,
                min_value: Some(0.001),
                max_value: Some(30.0),
                step: Some(0.001),
                enum_values: vec![],
                unit: Some("s".into()),
                description: Some("Camera exposure time".into()),
                group_name: Some("Acquisition".into()),
            },
            DbDeviceFeature {
                device_id: "andor_0".into(),
                feature_name: "ReadoutRate".into(),
                feature_type: "enum".into(),
                readable: true,
                writable: true,
                min_value: None,
                max_value: None,
                step: None,
                enum_values: vec!["100 MHz".into(), "200 MHz".into(), "280 MHz".into()],
                unit: None,
                description: Some("Sensor readout rate".into()),
                group_name: Some("Acquisition".into()),
            },
        ]
    }

    #[tokio::test]
    async fn test_device_feature_upsert_and_get() {
        let db = SqliteDb::init_memory().await.expect("init");

        let count = db
            .upsert_device_features(&sample_features())
            .await
            .expect("upsert");
        assert_eq!(count, 2);

        let features = db.get_device_features("andor_0").await.expect("get");
        assert_eq!(features.len(), 2);
        // Ordered by feature_name
        assert_eq!(features[0].feature_name, "ExposureTime");
        assert_eq!(features[0].feature_type, "float");
        assert_eq!(features[0].min_value, Some(0.001));
        assert_eq!(features[0].unit.as_deref(), Some("s"));

        assert_eq!(features[1].feature_name, "ReadoutRate");
        assert_eq!(
            features[1].enum_values,
            vec!["100 MHz", "200 MHz", "280 MHz"]
        );
    }

    #[tokio::test]
    async fn test_device_feature_upsert_idempotent() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_features(&sample_features())
            .await
            .expect("upsert");
        db.upsert_device_features(&sample_features())
            .await
            .expect("upsert");

        let features = db.get_device_features("andor_0").await.expect("get");
        assert_eq!(features.len(), 2, "upsert should not duplicate features");
    }

    #[tokio::test]
    async fn test_device_feature_delete() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_features(&sample_features())
            .await
            .expect("upsert");

        let count = db.delete_device_features("andor_0").await.expect("delete");
        assert_eq!(count, 2);

        let features = db.get_device_features("andor_0").await.expect("get");
        assert!(features.is_empty());

        // Delete nonexistent returns 0
        let count = db
            .delete_device_features("nonexistent")
            .await
            .expect("delete");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_device_feature_empty_get() {
        let db = SqliteDb::init_memory().await.expect("init");
        let features = db.get_device_features("nonexistent").await.expect("get");
        assert!(features.is_empty());
    }

    #[tokio::test]
    async fn test_device_feature_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.upsert_device_features(&sample_features())
            .await
            .expect("upsert");

        let event = rx.try_recv().expect("should receive notification");
        match event {
            DbChangeEvent::DeviceFeaturesUpdated { device_id } => {
                assert_eq!(device_id, "andor_0");
            }
            _ => panic!("expected DeviceFeaturesUpdated"),
        }
    }

    // =======================================================================
    // Experiment Plan tests
    // =======================================================================

    fn sample_plan() -> DbExperimentPlan {
        DbExperimentPlan {
            plan_id: "plan_001".into(),
            name: "Line Scan Test".into(),
            description: Some("Scan stage_x from 0 to 10".into()),
            plan_type: "graph_plan".into(),
            commands: Some(vec![
                serde_json::json!({"MoveTo": {"device_id": "stage_x", "position": 0.0}}),
                serde_json::json!({"EmitEvent": {"stream": "primary"}}),
            ]),
            movers: Some(vec!["stage_x".into()]),
            detectors: Some(vec!["power_meter".into()]),
            num_points: Some(2),
            graph_data: Some(serde_json::json!({"version": 1, "graph": {}})),
            parameters: None,
            device_mapping: None,
        }
    }

    #[tokio::test]
    async fn test_plan_upsert_and_get() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_experiment_plan(&sample_plan())
            .await
            .expect("upsert");

        let loaded = db
            .get_experiment_plan("plan_001")
            .await
            .expect("get")
            .expect("should exist");
        assert_eq!(loaded.plan_id, "plan_001");
        assert_eq!(loaded.name, "Line Scan Test");
        assert_eq!(loaded.plan_type, "graph_plan");
        assert_eq!(loaded.commands.as_ref().expect("commands").len(), 2);
        assert_eq!(loaded.movers.as_ref().expect("movers"), &["stage_x"]);
        assert_eq!(loaded.num_points, Some(2));
        assert_eq!(
            loaded.description.as_deref(),
            Some("Scan stage_x from 0 to 10")
        );
    }

    #[tokio::test]
    async fn test_plan_upsert_idempotent() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_experiment_plan(&sample_plan())
            .await
            .expect("upsert");
        db.upsert_experiment_plan(&sample_plan())
            .await
            .expect("upsert");

        let list = db.list_experiment_plans().await.expect("list");
        assert_eq!(list.len(), 1, "upsert should not duplicate plans");
    }

    #[tokio::test]
    async fn test_plan_update() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut plan = sample_plan();
        db.upsert_experiment_plan(&plan).await.expect("upsert");

        plan.name = "Updated Plan".into();
        plan.num_points = Some(42);
        db.upsert_experiment_plan(&plan).await.expect("upsert");

        let loaded = db
            .get_experiment_plan("plan_001")
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(loaded.name, "Updated Plan");
        assert_eq!(loaded.num_points, Some(42));
    }

    #[tokio::test]
    async fn test_plan_list_ordering() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut plan1 = sample_plan();
        plan1.plan_id = "plan_a".into();
        plan1.name = "Plan A".into();

        let mut plan2 = sample_plan();
        plan2.plan_id = "plan_b".into();
        plan2.name = "Plan B".into();

        db.upsert_experiment_plan(&plan1).await.expect("upsert");
        // Sleep to ensure different updated_at timestamps (SQLite second resolution).
        tokio::time::sleep(Duration::from_millis(1100)).await;
        db.upsert_experiment_plan(&plan2).await.expect("upsert");

        let list = db.list_experiment_plans().await.expect("list");
        assert_eq!(list.len(), 2);
        // Ordered by updated_at DESC, so plan_b should be first
        assert_eq!(list[0].plan_id, "plan_b");
        assert_eq!(list[1].plan_id, "plan_a");
    }

    #[tokio::test]
    async fn test_plan_delete() {
        let db = SqliteDb::init_memory().await.expect("init");
        db.upsert_experiment_plan(&sample_plan())
            .await
            .expect("upsert");

        let deleted = db.delete_experiment_plan("plan_001").await.expect("delete");
        assert!(deleted);

        let deleted_again = db.delete_experiment_plan("plan_001").await.expect("delete");
        assert!(!deleted_again);

        let loaded = db.get_experiment_plan("plan_001").await.expect("get");
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_plan_get_nonexistent() {
        let db = SqliteDb::init_memory().await.expect("init");
        let loaded = db.get_experiment_plan("nonexistent").await.expect("get");
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_plan_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.upsert_experiment_plan(&sample_plan())
            .await
            .expect("upsert");

        let event = rx.try_recv().expect("should receive notification");
        assert!(matches!(event, DbChangeEvent::ExperimentPlansUpdated));
    }

    // =======================================================================
    // Run Record tests
    // =======================================================================

    fn sample_run() -> DbRunRecord {
        DbRunRecord {
            run_uid: "run_abc123".into(),
            plan_id: Some("plan_001".into()),
            plan_type: "graph_plan".into(),
            plan_name: "Line Scan Test".into(),
            status: "queued".into(),
            num_events: None,
            metadata: Some(serde_json::json!({"operator": "alice"})),
            exit_reason: None,
        }
    }

    #[tokio::test]
    async fn test_run_create_and_list() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.create_run_record(&sample_run()).await.expect("create");

        let runs = db.list_runs(None).await.expect("list");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_uid, "run_abc123");
        assert_eq!(runs[0].status, "queued");
        assert_eq!(runs[0].plan_id, Some("plan_001".into()));
    }

    #[tokio::test]
    async fn test_run_status_update() {
        let db = SqliteDb::init_memory().await.expect("init");
        db.create_run_record(&sample_run()).await.expect("create");

        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");

        let runs = db.list_runs(None).await.expect("list");
        assert_eq!(runs[0].status, "running");
    }

    #[tokio::test]
    async fn test_run_finish() {
        let db = SqliteDb::init_memory().await.expect("init");
        db.create_run_record(&sample_run()).await.expect("create");

        db.finish_run("run_abc123", "completed", 42, None)
            .await
            .expect("finish");

        let runs = db.list_runs(None).await.expect("list");
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].num_events, Some(42));
        assert!(runs[0].exit_reason.is_none());
    }

    #[tokio::test]
    async fn test_run_finish_with_reason() {
        let db = SqliteDb::init_memory().await.expect("init");
        db.create_run_record(&sample_run()).await.expect("create");

        db.finish_run("run_abc123", "aborted", 5, Some("user abort"))
            .await
            .expect("finish");

        let runs = db.list_runs(None).await.expect("list");
        assert_eq!(runs[0].status, "aborted");
        assert_eq!(runs[0].num_events, Some(5));
        assert_eq!(runs[0].exit_reason.as_deref(), Some("user abort"));
    }

    #[tokio::test]
    async fn test_run_lifecycle_complete() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Create -> running -> completed
        db.create_run_record(&sample_run()).await.expect("create");
        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");
        db.finish_run("run_abc123", "completed", 100, None)
            .await
            .expect("finish");

        let runs = db.list_runs(None).await.expect("list");
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].num_events, Some(100));
    }

    #[tokio::test]
    async fn test_run_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.create_run_record(&sample_run()).await.expect("create");

        let event = rx.try_recv().expect("should receive notification");
        match event {
            DbChangeEvent::RunRecordUpdated { run_uid } => {
                assert_eq!(run_uid, "run_abc123");
            }
            _ => panic!("expected RunRecordUpdated"),
        }
    }

    // =======================================================================
    // Heartbeat tests
    // =======================================================================

    #[tokio::test]
    async fn test_heartbeat_update() {
        let db = SqliteDb::init_memory().await.expect("init");
        db.create_run_record(&sample_run()).await.expect("create");
        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");

        db.update_run_heartbeat("run_abc123")
            .await
            .expect("heartbeat");
        // No error means success.
    }

    #[tokio::test]
    async fn test_stale_runs_no_heartbeat() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.expect("create");
        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");

        // A run with no heartbeat_at should be stale.
        let stale = db
            .find_stale_runs(Duration::from_secs(60))
            .await
            .expect("find_stale");
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].run_uid, "run_abc123");
    }

    #[tokio::test]
    async fn test_stale_runs_excludes_completed() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.create_run_record(&sample_run()).await.expect("create");
        db.finish_run("run_abc123", "completed", 10, None)
            .await
            .expect("finish");

        let stale = db
            .find_stale_runs(Duration::from_secs(60))
            .await
            .expect("find_stale");
        assert!(stale.is_empty(), "completed runs should not be stale");
    }

    #[tokio::test]
    async fn test_stale_runs_excludes_fresh_heartbeat() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.expect("create");
        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");

        db.update_run_heartbeat("run_abc123")
            .await
            .expect("heartbeat");

        let stale = db
            .find_stale_runs(Duration::from_secs(60))
            .await
            .expect("find_stale");
        assert!(
            stale.is_empty(),
            "freshly heartbeated run should not be stale"
        );
    }

    #[tokio::test]
    async fn test_stale_runs_time_based() {
        let db = SqliteDb::init_memory().await.expect("init");

        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.expect("create");
        db.update_run_status("run_abc123", "running")
            .await
            .expect("update");

        db.update_run_heartbeat("run_abc123")
            .await
            .expect("heartbeat");

        // With a 3s threshold, immediately after heartbeat should not be stale.
        let stale = db
            .find_stale_runs(Duration::from_secs(3))
            .await
            .expect("find_stale");
        assert!(stale.is_empty());

        // After crossing the threshold (SQLite datetime has second resolution,
        // so we need to sleep longer than the threshold + 1s margin).
        tokio::time::sleep(Duration::from_millis(4100)).await;
        let stale = db
            .find_stale_runs(Duration::from_secs(3))
            .await
            .expect("find_stale");
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].run_uid, "run_abc123");
    }

    // =======================================================================
    // Device Runtime State tests
    // =======================================================================

    #[tokio::test]
    async fn test_device_state_upsert_and_get() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_state("stage_1", "position", &serde_json::json!(42.5))
            .await
            .expect("upsert");

        let states = db.get_device_state("stage_1").await.expect("get");
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].device_id, "stage_1");
        assert_eq!(states[0].param_name, "position");
        assert_eq!(states[0].param_value, serde_json::json!(42.5));
        assert!(!states[0].is_favorite);
    }

    #[tokio::test]
    async fn test_device_state_update() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_state("stage_1", "position", &serde_json::json!(0.0))
            .await
            .expect("upsert");
        db.upsert_device_state("stage_1", "position", &serde_json::json!(99.9))
            .await
            .expect("upsert");

        let states = db.get_device_state("stage_1").await.expect("get");
        assert_eq!(states.len(), 1, "upsert should not duplicate state");
        assert_eq!(states[0].param_value, serde_json::json!(99.9));
    }

    #[tokio::test]
    async fn test_device_state_batch_upsert() {
        let db = SqliteDb::init_memory().await.expect("init");

        let states = vec![
            ("stage_1".into(), "position".into(), serde_json::json!(10.0)),
            ("stage_1".into(), "velocity".into(), serde_json::json!(5.0)),
            ("cam_0".into(), "exposure".into(), serde_json::json!(0.1)),
        ];

        db.batch_upsert_device_state(&states)
            .await
            .expect("batch_upsert");

        let stage_states = db.get_device_state("stage_1").await.expect("get");
        assert_eq!(stage_states.len(), 2);

        let cam_states = db.get_device_state("cam_0").await.expect("get");
        assert_eq!(cam_states.len(), 1);
        assert_eq!(cam_states[0].param_value, serde_json::json!(0.1));
    }

    #[tokio::test]
    async fn test_device_state_favorites() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Create state first.
        db.upsert_device_state("stage_1", "position", &serde_json::json!(0.0))
            .await
            .expect("upsert");
        db.upsert_device_state("stage_1", "velocity", &serde_json::json!(5.0))
            .await
            .expect("upsert");

        // Set one as favorite.
        db.set_parameter_favorite("stage_1", "position", true)
            .await
            .expect("set_favorite");

        let favorites = db.get_favorites("stage_1").await.expect("get_favorites");
        assert_eq!(favorites, vec!["position"]);

        // Un-favorite it.
        db.set_parameter_favorite("stage_1", "position", false)
            .await
            .expect("set_favorite");

        let favorites = db.get_favorites("stage_1").await.expect("get_favorites");
        assert!(favorites.is_empty());
    }

    #[tokio::test]
    async fn test_device_state_favorite_preserves_value() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_state("stage_1", "position", &serde_json::json!(42.5))
            .await
            .expect("upsert");

        // Setting favorite should NOT change the param_value.
        db.set_parameter_favorite("stage_1", "position", true)
            .await
            .expect("set_favorite");

        let states = db.get_device_state("stage_1").await.expect("get");
        assert_eq!(states[0].param_value, serde_json::json!(42.5));
        assert!(states[0].is_favorite);
    }

    #[tokio::test]
    async fn test_device_state_upsert_preserves_favorite() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_state("stage_1", "position", &serde_json::json!(10.0))
            .await
            .expect("upsert");
        db.set_parameter_favorite("stage_1", "position", true)
            .await
            .expect("set_favorite");

        // Upsert new value should preserve is_favorite.
        db.upsert_device_state("stage_1", "position", &serde_json::json!(20.0))
            .await
            .expect("upsert");

        let states = db.get_device_state("stage_1").await.expect("get");
        assert_eq!(states[0].param_value, serde_json::json!(20.0));
        assert!(
            states[0].is_favorite,
            "is_favorite should be preserved across upsert"
        );
    }

    #[tokio::test]
    async fn test_device_state_delete() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.upsert_device_state("stage_1", "position", &serde_json::json!(0.0))
            .await
            .expect("upsert");
        db.upsert_device_state("stage_1", "velocity", &serde_json::json!(5.0))
            .await
            .expect("upsert");

        let count = db.delete_device_state("stage_1").await.expect("delete");
        assert_eq!(count, 2);

        let states = db.get_device_state("stage_1").await.expect("get");
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn test_device_state_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.upsert_device_state("stage_1", "position", &serde_json::json!(0.0))
            .await
            .expect("upsert");

        let event = rx.try_recv().expect("should receive notification");
        match event {
            DbChangeEvent::DeviceStateUpdated { device_id } => {
                assert_eq!(device_id, "stage_1");
            }
            _ => panic!("expected DeviceStateUpdated"),
        }
    }

    // =======================================================================
    // Lifecycle Event tests
    // =======================================================================

    fn sample_lifecycle_event() -> DeviceLifecycleEvent {
        DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "idle".into(),
            to_state: "streaming".into(),
            reason: Some("user started acquisition".into()),
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn test_lifecycle_record_and_get() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.record_lifecycle_event(&sample_lifecycle_event())
            .await
            .expect("record");

        let events = db.get_lifecycle_events("pvcam_0", 10).await.expect("get");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].device_id, "pvcam_0");
        assert_eq!(events[0].from_state, "idle");
        assert_eq!(events[0].to_state, "streaming");
        assert_eq!(
            events[0].reason.as_deref(),
            Some("user started acquisition")
        );
        assert!(events[0].timestamp.is_some());
    }

    #[tokio::test]
    async fn test_lifecycle_ordering() {
        let db = SqliteDb::init_memory().await.expect("init");

        let event1 = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "idle".into(),
            to_state: "streaming".into(),
            reason: None,
            timestamp: None,
        };
        let event2 = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "streaming".into(),
            to_state: "error".into(),
            reason: Some("frame timeout".into()),
            timestamp: None,
        };
        let event3 = DeviceLifecycleEvent {
            device_id: "pvcam_0".into(),
            from_state: "error".into(),
            to_state: "idle".into(),
            reason: Some("recovery".into()),
            timestamp: None,
        };

        db.record_lifecycle_event(&event1).await.expect("record");
        db.record_lifecycle_event(&event2).await.expect("record");
        db.record_lifecycle_event(&event3).await.expect("record");

        let events = db.get_lifecycle_events("pvcam_0", 10).await.expect("get");
        assert_eq!(events.len(), 3);
        // Newest first
        assert_eq!(events[0].to_state, "idle");
        assert_eq!(events[1].to_state, "error");
        assert_eq!(events[2].to_state, "streaming");
    }

    #[tokio::test]
    async fn test_lifecycle_limit() {
        let db = SqliteDb::init_memory().await.expect("init");

        for i in 0..5 {
            let event = DeviceLifecycleEvent {
                device_id: "cam_0".into(),
                from_state: format!("state_{i}"),
                to_state: format!("state_{}", i + 1),
                reason: None,
                timestamp: None,
            };
            db.record_lifecycle_event(&event).await.expect("record");
        }

        let events = db.get_lifecycle_events("cam_0", 3).await.expect("get");
        assert_eq!(events.len(), 3, "should respect limit");
    }

    #[tokio::test]
    async fn test_lifecycle_delete() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.record_lifecycle_event(&sample_lifecycle_event())
            .await
            .expect("record");

        let count = db.delete_lifecycle_events("pvcam_0").await.expect("delete");
        assert_eq!(count, 1);

        let events = db.get_lifecycle_events("pvcam_0", 10).await.expect("get");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_lifecycle_updates_state() {
        let db = SqliteDb::init_memory().await.expect("init");

        db.record_lifecycle_event(&sample_lifecycle_event())
            .await
            .expect("record");

        // Should have created a _lifecycle_state entry.
        let states = db.get_device_state("pvcam_0").await.expect("get");
        let lifecycle_state = states
            .iter()
            .find(|s| s.param_name == "_lifecycle_state")
            .expect("should have _lifecycle_state");
        assert_eq!(lifecycle_state.param_value, serde_json::json!("streaming"));
    }

    #[tokio::test]
    async fn test_lifecycle_broadcast() {
        let db = SqliteDb::init_memory().await.expect("init");
        let mut rx = db.subscribe_changes();

        db.record_lifecycle_event(&sample_lifecycle_event())
            .await
            .expect("record");

        // There will be a DeviceStateUpdated (from upsert_device_state) and a
        // LifecycleEventRecorded. Drain to find the lifecycle one.
        let mut found_lifecycle = false;
        for _ in 0..5 {
            match rx.try_recv() {
                Ok(DbChangeEvent::LifecycleEventRecorded { device_id }) => {
                    assert_eq!(device_id, "pvcam_0");
                    found_lifecycle = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            found_lifecycle,
            "should have received LifecycleEventRecorded"
        );
    }

    // =======================================================================
    // Info / schema version tests
    // =======================================================================

    #[tokio::test]
    async fn test_schema_version() {
        let db = SqliteDb::init_memory().await.expect("init");
        let version = db.schema_version().await.expect("version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn test_info() {
        let db = SqliteDb::init_memory().await.expect("init");

        // Seed some data.
        db.upsert_instruments(&[sample_instrument("dev_0")])
            .await
            .expect("upsert");
        db.upsert_drivers(&[DbDriver {
            driver_type: "mock".into(),
            name: "Mock".into(),
            capabilities: vec![],
            commands: vec![],
        }])
        .await
        .expect("upsert");

        let info = db.info().await.expect("info");
        assert_eq!(info.engine, "sqlite");
        assert_eq!(info.schema_version, SCHEMA_VERSION);
        assert!(info.healthy);
        assert_eq!(info.instrument_count, 1);
        assert_eq!(info.driver_count, 1);
        assert_eq!(info.plan_count, 0);
        assert_eq!(info.run_count, 0);
    }

    #[tokio::test]
    async fn test_info_empty_db() {
        let db = SqliteDb::init_memory().await.expect("init");

        let info = db.info().await.expect("info");
        assert_eq!(info.engine, "sqlite");
        assert!(info.healthy);
        assert_eq!(info.driver_count, 0);
        assert_eq!(info.instrument_count, 0);
        assert_eq!(info.plan_count, 0);
        assert_eq!(info.run_count, 0);
        assert_eq!(info.device_feature_count, 0);
        assert_eq!(info.device_state_count, 0);
        assert_eq!(info.lifecycle_event_count, 0);
    }
}
