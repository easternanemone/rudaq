//! Core database wrapper and configuration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use tracing::info;

use crate::error::Result;
use crate::schema;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Which storage engine to use.
#[derive(Debug, Clone)]
pub enum DbEngine {
    /// In-memory (no persistence, fast startup). Requires `kv-mem` feature.
    Memory,
    /// RocksDB on-disk. Requires `kv-rocksdb` feature.
    RocksDb(PathBuf),
}

/// Configuration for [`DaqDb`].
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// Storage engine.
    pub engine: DbEngine,
    /// SurrealDB namespace (default: `"daq"`).
    pub namespace: String,
    /// SurrealDB database name (default: `"live"`).
    pub database: String,
}

impl DbConfig {
    /// In-memory database (useful for tests and development).
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            engine: DbEngine::Memory,
            namespace: "daq".into(),
            database: "live".into(),
        }
    }

    /// RocksDB-backed database at the given path.
    #[must_use]
    pub fn rocksdb(path: impl Into<PathBuf>) -> Self {
        Self {
            engine: DbEngine::RocksDb(path.into()),
            namespace: "daq".into(),
            database: "live".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// DaqDb
// ---------------------------------------------------------------------------

/// Thread-safe handle to the embedded SurrealDB instance.
///
/// This is the main entry point for all database operations.  It is cheap to
/// clone (wraps an `Arc`).
#[derive(Clone)]
pub struct DaqDb {
    client: Arc<Surreal<Any>>,
    config: DbConfig,
    /// Wall-clock time of `init()`. Used for uptime reporting.
    started_at: Instant,
}

impl DaqDb {
    /// Initialize the database, apply schema migrations, and return a handle.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Database`] if the engine fails to start or the
    /// schema migration fails.
    pub async fn init(config: DbConfig) -> Result<Self> {
        let t0 = Instant::now();

        // Use the `any::connect()` API so the engine is selected at runtime.
        // Endpoint strings: "mem://" for in-memory, "rocksdb://path" for disk.
        let endpoint = match &config.engine {
            DbEngine::Memory => {
                info!("initializing in-memory SurrealDB");
                "mem://".to_string()
            }
            DbEngine::RocksDb(path) => {
                info!(path = %path.display(), "initializing RocksDB SurrealDB");
                format!("rocksdb://{}", path.display())
            }
        };

        let client = surrealdb::engine::any::connect(&endpoint).await?;

        // Select namespace + database
        client
            .use_ns(&config.namespace)
            .use_db(&config.database)
            .await?;

        let db = Self {
            client: Arc::new(client),
            config,
            started_at: t0,
        };

        // Apply schema migrations
        schema::apply_schema(&db.client).await?;

        let elapsed = t0.elapsed();
        info!(elapsed_ms = elapsed.as_millis(), "database initialized");

        Ok(db)
    }

    /// Returns `true` if the database is responsive.
    pub async fn health_check(&self) -> bool {
        self.client
            .query("RETURN true")
            .await
            .map(|_| true)
            .unwrap_or(false)
    }

    /// Returns diagnostic information about the database.
    pub async fn info(&self) -> DbInfo {
        let uptime = self.started_at.elapsed();
        let engine = match &self.config.engine {
            DbEngine::Memory => "mem".to_string(),
            DbEngine::RocksDb(p) => format!("rocksdb:{}", p.display()),
        };
        let healthy = self.health_check().await;

        // Count records in each table
        let driver_count = self.count_table("driver").await.unwrap_or(0);
        let instrument_count = self.count_table("instrument").await.unwrap_or(0);

        DbInfo {
            engine,
            namespace: self.config.namespace.clone(),
            database: self.config.database.clone(),
            schema_version: schema::SCHEMA_VERSION,
            uptime_secs: uptime.as_secs(),
            healthy,
            driver_count,
            instrument_count,
        }
    }

    /// Low-level access to the SurrealDB client.
    ///
    /// Prefer the typed methods on `DaqDb` when possible.  This escape hatch
    /// is available for raw SurrealQL queries during prototyping.
    pub fn client(&self) -> &Surreal<Any> {
        &self.client
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn count_table(&self, table: &str) -> Result<u64> {
        // Validate table name against allow-list to prevent SurrealQL injection.
        // SurrealDB does not support parameterized table names.
        const ALLOWED_TABLES: &[&str] = &[
            "driver",
            "instrument",
            "experiment",
            "experiment_plan",
            "run_record",
        ];
        if !ALLOWED_TABLES.contains(&table) {
            return Err(crate::error::DbError::Database(format!(
                "count_table: unknown table '{table}'"
            )));
        }
        let query = format!("SELECT count() AS total FROM {table} GROUP ALL");
        let mut response = self.client.query(&query).await?;
        let rows: Vec<CountResult> = response.take(0)?;
        Ok(rows.first().map(|r| r.total).unwrap_or(0))
    }
}

impl std::fmt::Debug for DaqDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaqDb")
            .field("engine", &self.config.engine)
            .field("namespace", &self.config.namespace)
            .field("database", &self.config.database)
            .field("uptime_ms", &self.started_at.elapsed().as_millis())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Info / diagnostics
// ---------------------------------------------------------------------------

/// Diagnostic snapshot of the database state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbInfo {
    pub engine: String,
    pub namespace: String,
    pub database: String,
    pub schema_version: u32,
    pub uptime_secs: u64,
    pub healthy: bool,
    pub driver_count: u64,
    pub instrument_count: u64,
}

#[derive(Debug, Deserialize)]
struct CountResult {
    total: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "kv-mem")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_in_memory() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("init should succeed");
        assert!(db.health_check().await, "health check should pass");
    }

    #[tokio::test]
    async fn test_schema_applied() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("init should succeed");

        // Verify schema version was recorded
        let mut response = db
            .client()
            .query("SELECT version FROM schema_version")
            .await
            .expect("query should succeed");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take should succeed");
        assert_eq!(rows.len(), 1, "should have one schema_version record");
        assert_eq!(
            rows[0]["version"],
            serde_json::json!(schema::SCHEMA_VERSION)
        );
    }

    #[tokio::test]
    async fn test_schema_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("init should succeed");

        // Apply schema again — should not error or duplicate
        schema::apply_schema(db.client())
            .await
            .expect("re-apply should succeed");

        let mut response = db
            .client()
            .query("SELECT version FROM schema_version")
            .await
            .expect("query should succeed");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take should succeed");
        // Should still have at least one record (idempotent DDL + version check)
        assert!(!rows.is_empty(), "should have schema_version records");
    }

    #[tokio::test]
    async fn test_info() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("init should succeed");
        let info = db.info().await;
        assert_eq!(info.engine, "mem");
        assert_eq!(info.namespace, "daq");
        assert_eq!(info.database, "live");
        assert_eq!(info.schema_version, schema::SCHEMA_VERSION);
        assert!(info.healthy);
        assert_eq!(info.driver_count, 0);
        assert_eq!(info.instrument_count, 0);
    }

    #[tokio::test]
    async fn test_crud_round_trip() {
        let db = DaqDb::init(DbConfig::in_memory())
            .await
            .expect("init should succeed");

        // Insert a driver record
        db.client()
            .query(
                r"CREATE driver SET
                    driver_type = 'mock',
                    name = 'Mock Driver',
                    capabilities = ['readable', 'movable'],
                    commands = [],
                    created_at = time::now()",
            )
            .await
            .expect("create driver should succeed");

        // Insert an instrument record
        db.client()
            .query(
                r"CREATE instrument SET
                    device_id = 'test_device_1',
                    name = 'Test Device',
                    driver_type = 'mock',
                    config = { port: '/dev/null' },
                    enabled = true,
                    status = 'offline',
                    created_at = time::now(),
                    updated_at = time::now()",
            )
            .await
            .expect("create instrument should succeed");

        // Verify counts
        let info = db.info().await;
        assert_eq!(info.driver_count, 1);
        assert_eq!(info.instrument_count, 1);

        // Read back the instrument (exclude `id` which is a RecordId, not JSON)
        let mut response = db
            .client()
            .query("SELECT device_id, name, driver_type, enabled, status FROM instrument WHERE device_id = 'test_device_1'")
            .await
            .expect("query should succeed");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "Test Device");
        assert_eq!(rows[0]["driver_type"], "mock");
    }
}

// ---------------------------------------------------------------------------
// RocksDB persistence tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "kv-rocksdb")]
mod rocksdb_tests {
    use super::*;
    use crate::config_store::{DbDriver, DbInstrument};

    /// Helper test that writes seed data to a RocksDB database.
    ///
    /// This runs in a **separate process** (invoked by the main test via
    /// `Command`) so that the RocksDB file lock is released when the
    /// process exits.  SurrealDB provides no `disconnect()` API, and the
    /// underlying RocksDB C++ library tracks open databases in a
    /// process-global static set, so reopening the same path within the
    /// same process is impossible.
    #[tokio::test]
    #[ignore = "only invoked as a subprocess"]
    async fn rocksdb_writer_helper() {
        let db_path = std::env::var("ROCKSDB_TEST_PATH").expect("ROCKSDB_TEST_PATH must be set");
        let db = DaqDb::init(DbConfig::rocksdb(&db_path)).await.unwrap();
        assert!(db.health_check().await);

        let instruments = vec![DbInstrument {
            device_id: "persist_test".into(),
            name: "Persistence Test Device".into(),
            driver_type: "mock".into(),
            config: serde_json::json!({"port": "/dev/null"}),
            enabled: true,
        }];
        db.upsert_instruments(&instruments).await.unwrap();

        let drivers = vec![DbDriver {
            driver_type: "mock".into(),
            name: "Mock".into(),
            capabilities: vec!["readable".into()],
            commands: vec![],
        }];
        db.upsert_drivers(&drivers).await.unwrap();

        let info = db.info().await;
        assert_eq!(info.instrument_count, 1);
        assert_eq!(info.driver_count, 1);
    }

    #[tokio::test]
    async fn test_rocksdb_persistence_across_restart() {
        let tmpdir = tempfile::tempdir().unwrap();
        let db_path = tmpdir.path().join("test.db");

        // First session: spawn the writer helper as a separate OS process.
        // RocksDB's C++ layer uses a process-global lock set, so the only
        // way to release the lock is to exit the process.
        let test_bin = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&test_bin)
            .arg("core::rocksdb_tests::rocksdb_writer_helper")
            .arg("--exact")
            .arg("--ignored")
            .arg("--nocapture")
            .env("ROCKSDB_TEST_PATH", db_path.to_str().unwrap())
            .output()
            .expect("failed to spawn writer subprocess");
        assert!(
            output.status.success(),
            "writer subprocess failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        // Second session: re-open and verify data persists
        let db = DaqDb::init(DbConfig::rocksdb(&db_path)).await.unwrap();
        assert!(db.health_check().await);

        // Data should persist
        let instruments = db.get_all_instruments().await.unwrap();
        assert_eq!(
            instruments.len(),
            1,
            "instrument should persist across restart"
        );
        assert_eq!(instruments[0].device_id, "persist_test");

        let drivers = db.get_all_drivers().await.unwrap();
        assert_eq!(drivers.len(), 1, "driver should persist across restart");

        // Schema version should not accumulate
        let mut response = db
            .client()
            .query("SELECT version FROM schema_version")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = response.take(0).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "schema_version should not accumulate on restart"
        );
        assert_eq!(
            rows[0]["version"],
            serde_json::json!(crate::schema::SCHEMA_VERSION)
        );

        let info = db.info().await;
        assert_eq!(info.schema_version, crate::schema::SCHEMA_VERSION);
    }
}
