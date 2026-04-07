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

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::info;

use crate::error::Result;

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
}

impl std::fmt::Debug for SqliteDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteDb")
            .field("change_subscribers", &self.change_tx.receiver_count())
            .finish_non_exhaustive()
    }
}

/// SQL statements that create the instrument table and its index.
const SCHEMA_SQL: &str = r"
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
";

impl SqliteDb {
    /// Open (or create) a SQLite database at `path` and apply the schema.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Database`] if the file cannot be opened or the
    /// schema migration fails.
    pub async fn init(path: &str) -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open(path).await?;
        let (change_tx, _) = broadcast::channel(64);
        let db = Self { conn, change_tx };
        db.apply_schema().await?;
        info!(path, "SQLite database initialized");
        Ok(db)
    }

    /// Open an in-memory SQLite database (useful for tests).
    pub async fn init_memory() -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open_in_memory().await?;
        let (change_tx, _) = broadcast::channel(64);
        let db = Self { conn, change_tx };
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

    // -------------------------------------------------------------------
    // Instruments
    // -------------------------------------------------------------------

    /// Upsert a batch of instruments.
    ///
    /// Uses `INSERT OR REPLACE` keyed on `device_id`.  Broadcasts
    /// [`DbChangeEvent::InstrumentsUpdated`] once after the batch completes.
    pub async fn upsert_instruments(&self, instruments: &[DbInstrument]) -> Result<()> {
        let instruments = instruments.to_vec();
        self.conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                for inst in &instruments {
                    let config_json = serde_json::to_string(&inst.config)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    tx.execute(
                        "INSERT OR REPLACE INTO instrument \
                         (device_id, name, driver_type, config, enabled, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
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

        // Notify subscribers (best-effort — ignore send errors when no receivers).
        let _ = self.change_tx.send(DbChangeEvent::InstrumentsUpdated);
        Ok(())
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

    // -------------------------------------------------------------------
    // Change subscription
    // -------------------------------------------------------------------

    /// Subscribe to database change notifications.
    ///
    /// Returns a [`broadcast::Receiver`] that yields [`DbChangeEvent`]
    /// values whenever a write operation modifies a table.  This replaces
    /// the SurrealDB `LIVE SELECT` mechanism.
    #[must_use]
    pub fn subscribe_changes(&self) -> broadcast::Receiver<DbChangeEvent> {
        self.change_tx.subscribe()
    }

    // -------------------------------------------------------------------
    // Health check
    // -------------------------------------------------------------------

    /// Lightweight health check — verifies the connection is responsive.
    pub async fn health_check(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute_batch("SELECT 1")?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row mapping helper
// ---------------------------------------------------------------------------

/// Map a `rusqlite::Row` to a `DbInstrument`.
///
/// The `config` column is stored as a JSON text blob; we parse it back into
/// `serde_json::Value`.  If parsing fails we fall back to an empty object.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a sample instrument for testing.
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
            let db = SqliteDb::init(path_str).await.expect("init");
            let inst = sample_instrument("persist_test");
            db.upsert_instruments(&[inst]).await.expect("upsert");
        }

        // Session 2: re-open and verify data persists
        {
            let db = SqliteDb::init(path_str).await.expect("init");
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
}
