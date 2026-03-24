//! Schema definitions and migrations for the DAQ database.
//!
//! The database models these record types:
//!
//! - **`driver`** — Static driver definitions (imported from TOML plugin files).
//! - **`instrument`** — Physical device instances, each linked to a driver.
//! - **`experiment`** — Named containers for experiment runs (Phase 4).
//!
//! Relationships (graph edges):
//!
//! - `instance_of` — instrument → driver
//! - `connects_to` — instrument → instrument (physical cabling)

/// Current schema version. Bump this when adding migrations.
pub const SCHEMA_VERSION: u32 = 9;

/// A single schema migration step.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
struct Migration {
    /// Version this migration upgrades TO.
    version: u32,
    /// SurrealQL to execute for this migration.
    sql: &'static str,
}

/// All migrations in order. Each is only applied if current_version < migration.version.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: SCHEMA_V1,
    },
    Migration {
        version: 2,
        sql: SCHEMA_V2,
    },
    Migration {
        version: 3,
        sql: SCHEMA_V3,
    },
    Migration {
        version: 4,
        sql: SCHEMA_V4,
    },
    Migration {
        version: 5,
        sql: SCHEMA_V5,
    },
    Migration {
        version: 6,
        sql: SCHEMA_V6,
    },
    Migration {
        version: 7,
        sql: SCHEMA_V7,
    },
    Migration {
        version: 8,
        sql: SCHEMA_V8,
    },
    Migration {
        version: 9,
        sql: SCHEMA_V9,
    },
];

/// SurrealQL statements that define the v1 schema.
///
/// These are idempotent — safe to re-run on an existing database.
pub const SCHEMA_V1: &str = r"
-- Schema version tracking
DEFINE TABLE IF NOT EXISTS schema_version SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON schema_version TYPE int;
DEFINE FIELD IF NOT EXISTS applied_at ON schema_version TYPE datetime;

-- Driver definitions (imported from TOML device configs)
DEFINE TABLE IF NOT EXISTS driver SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS driver_type ON driver TYPE string;
DEFINE FIELD IF NOT EXISTS name ON driver TYPE string;
DEFINE FIELD IF NOT EXISTS capabilities ON driver FLEXIBLE TYPE array;
DEFINE FIELD IF NOT EXISTS config_schema ON driver FLEXIBLE TYPE option<object>;
DEFINE FIELD IF NOT EXISTS created_at ON driver TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_driver_type ON driver FIELDS driver_type UNIQUE;

-- Instrument instances (physical devices)
DEFINE TABLE IF NOT EXISTS instrument SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id ON instrument TYPE string;
DEFINE FIELD IF NOT EXISTS name ON instrument TYPE string;
DEFINE FIELD IF NOT EXISTS driver_type ON instrument TYPE string;
DEFINE FIELD IF NOT EXISTS config ON instrument FLEXIBLE TYPE object;
DEFINE FIELD IF NOT EXISTS enabled ON instrument TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS status ON instrument TYPE string DEFAULT 'offline';
DEFINE FIELD IF NOT EXISTS created_at ON instrument TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON instrument TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_device_id ON instrument FIELDS device_id UNIQUE;

-- Experiment containers (Phase 4 — placeholder)
DEFINE TABLE IF NOT EXISTS experiment SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON experiment TYPE string;
DEFINE FIELD IF NOT EXISTS description ON experiment TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON experiment TYPE datetime DEFAULT time::now();

-- Graph edges: instrument -> driver
DEFINE TABLE IF NOT EXISTS instance_of TYPE RELATION
    FROM instrument TO driver;

-- Graph edges: instrument -> instrument (physical cabling)
DEFINE TABLE IF NOT EXISTS connects_to TYPE RELATION
    FROM instrument TO instrument;
";

/// v1→v2 migration: no-op structural migration to establish the chain pattern.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V2: &str = "";

/// v2→v3 migration: extend experiment table. Topology tables (process, stream)
/// were removed as dead code — the version number is preserved for RocksDB
/// database compatibility (existing databases already record version 3).
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V3: &str = r"
-- Extend experiment table with experiment_id key
DEFINE FIELD IF NOT EXISTS experiment_id ON experiment TYPE string;
DEFINE INDEX IF NOT EXISTS idx_experiment_id ON experiment FIELDS experiment_id UNIQUE;
";

/// v3→v4 migration: add driver command catalog field for DB-backed command
/// introspection in universal/TOML drivers.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V4: &str = r"
DEFINE FIELD IF NOT EXISTS commands ON driver FLEXIBLE TYPE array;
";

/// v4→v5 migration: experiment plan storage and run history.
///
/// Adds `experiment_plan` (persistent plan definitions with pre-translated commands
/// and graph data) and `run_record` (execution history linked to plans).
/// Graph edges: `executed_from` (run→plan) and `uses_instrument` (plan→instrument).
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V5: &str = r"
-- Experiment plan definitions
DEFINE TABLE IF NOT EXISTS experiment_plan SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS plan_id      ON experiment_plan TYPE string;
DEFINE FIELD IF NOT EXISTS name         ON experiment_plan TYPE string;
DEFINE FIELD IF NOT EXISTS description  ON experiment_plan TYPE option<string>;
DEFINE FIELD IF NOT EXISTS plan_type    ON experiment_plan TYPE string;
DEFINE FIELD IF NOT EXISTS commands     ON experiment_plan FLEXIBLE TYPE option<array>;
DEFINE FIELD IF NOT EXISTS movers       ON experiment_plan FLEXIBLE TYPE option<array>;
DEFINE FIELD IF NOT EXISTS detectors    ON experiment_plan FLEXIBLE TYPE option<array>;
DEFINE FIELD IF NOT EXISTS num_points   ON experiment_plan TYPE option<int>;
DEFINE FIELD IF NOT EXISTS graph_data   ON experiment_plan FLEXIBLE TYPE option<object>;
DEFINE FIELD IF NOT EXISTS parameters   ON experiment_plan FLEXIBLE TYPE option<object>;
DEFINE FIELD IF NOT EXISTS device_mapping ON experiment_plan FLEXIBLE TYPE option<object>;
DEFINE FIELD IF NOT EXISTS created_at   ON experiment_plan TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at   ON experiment_plan TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_plan_id  ON experiment_plan FIELDS plan_id UNIQUE;

-- Run execution history
DEFINE TABLE IF NOT EXISTS run_record SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_uid      ON run_record TYPE string;
DEFINE FIELD IF NOT EXISTS plan_id      ON run_record TYPE option<string>;
DEFINE FIELD IF NOT EXISTS plan_type    ON run_record TYPE string;
DEFINE FIELD IF NOT EXISTS plan_name    ON run_record TYPE string;
DEFINE FIELD IF NOT EXISTS status       ON run_record TYPE string DEFAULT 'queued';
DEFINE FIELD IF NOT EXISTS num_events   ON run_record TYPE option<int>;
DEFINE FIELD IF NOT EXISTS metadata     ON run_record FLEXIBLE TYPE option<object>;
DEFINE FIELD IF NOT EXISTS started_at   ON run_record TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS finished_at  ON run_record TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS exit_reason  ON run_record TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_run_uid  ON run_record FIELDS run_uid UNIQUE;

-- Graph edges: run_record -> experiment_plan (which plan spawned this run)
DEFINE TABLE IF NOT EXISTS executed_from TYPE RELATION
    FROM run_record TO experiment_plan;

-- Graph edges: experiment_plan -> instrument (which instruments a plan uses)
DEFINE TABLE IF NOT EXISTS uses_instrument TYPE RELATION
    FROM experiment_plan TO instrument;
";

/// v5→v6 migration: device feature metadata cache.
///
/// Stores discovered parameter metadata (types, ranges, enum values) for each
/// device. This enables UI pre-rendering and offline feature queries without
/// requiring the device to be online.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V6: &str = r"
-- Device feature metadata cache (parameter names, types, ranges — not live values)
DEFINE TABLE IF NOT EXISTS device_feature SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id ON device_feature TYPE string;
DEFINE FIELD IF NOT EXISTS feature_name ON device_feature TYPE string;
DEFINE FIELD IF NOT EXISTS feature_type ON device_feature TYPE string;
DEFINE FIELD IF NOT EXISTS readable ON device_feature TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS writable ON device_feature TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS min_value ON device_feature TYPE option<float>;
DEFINE FIELD IF NOT EXISTS max_value ON device_feature TYPE option<float>;
DEFINE FIELD IF NOT EXISTS step ON device_feature TYPE option<float>;
DEFINE FIELD IF NOT EXISTS enum_values ON device_feature FLEXIBLE TYPE option<array>;
DEFINE FIELD IF NOT EXISTS unit ON device_feature TYPE option<string>;
DEFINE FIELD IF NOT EXISTS description ON device_feature TYPE option<string>;
DEFINE FIELD IF NOT EXISTS group_name ON device_feature TYPE option<string>;
DEFINE FIELD IF NOT EXISTS discovered_at ON device_feature TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_device_feature ON device_feature FIELDS device_id, feature_name UNIQUE;
";

/// v6→v7 migration: heartbeat monitoring and device runtime state persistence.
///
/// Adds `heartbeat_at` to `run_record` for crash detection (stale heartbeat
/// indicates a daemon crash mid-experiment). Adds `device_runtime_state` table
/// for persisting last-known device parameter values across restarts.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
const SCHEMA_V7: &str = r"
-- Fix: add DEFAULT [] to commands field on driver (v4 omitted this, causing
-- silent CREATE failures on SCHEMAFULL tables in SurrealDB 2.x)
DEFINE FIELD commands ON driver FLEXIBLE TYPE array DEFAULT [];

-- Heartbeat timestamp for crash detection (updated every ~10s during plan execution)
DEFINE FIELD IF NOT EXISTS heartbeat_at ON run_record TYPE option<datetime>;

-- Device runtime state: persists last-known parameter values for restart recovery
DEFINE TABLE IF NOT EXISTS device_runtime_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id   ON device_runtime_state TYPE string;
DEFINE FIELD IF NOT EXISTS param_name  ON device_runtime_state TYPE string;
DEFINE FIELD IF NOT EXISTS param_value ON device_runtime_state FLEXIBLE TYPE any;
DEFINE FIELD IF NOT EXISTS updated_at  ON device_runtime_state TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_device_param ON device_runtime_state FIELDS device_id, param_name UNIQUE;
";

/// V8: Add is_favorite flag to device_runtime_state for UI quick-access pinning (bd-4wf7).
const SCHEMA_V8: &str = r"
DEFINE FIELD IF NOT EXISTS is_favorite ON device_runtime_state TYPE bool DEFAULT false;
";

/// V9: Device lifecycle event log for PVCAM state machine tracking (bd-oqo7.9).
///
/// Records state transitions with timestamps, enabling post-mortem analysis
/// of camera health over long experiments. Indexed by device_id for efficient
/// per-device queries.
const SCHEMA_V9: &str = r"
DEFINE TABLE IF NOT EXISTS device_lifecycle_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id   ON device_lifecycle_event TYPE string;
DEFINE FIELD IF NOT EXISTS from_state  ON device_lifecycle_event TYPE string;
DEFINE FIELD IF NOT EXISTS to_state    ON device_lifecycle_event TYPE string;
DEFINE FIELD IF NOT EXISTS reason      ON device_lifecycle_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS timestamp   ON device_lifecycle_event TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_lifecycle_device ON device_lifecycle_event FIELDS device_id;
";

/// Apply the schema to the database.
///
/// This is called once during [`DaqDb::init`] and is idempotent.
/// Iterates through [`MIGRATIONS`] and applies only those with
/// `version > current_version`.
#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
pub(crate) async fn apply_schema(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
) -> crate::error::Result<()> {
    use tracing::info;

    // Check current version using a fixed record ID to avoid accumulation.
    let mut response = db
        .query("SELECT version FROM schema_version:current")
        .await?;
    let existing: Vec<serde_json::Value> = response.take(0)?;
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    let current_version = existing
        .first()
        .and_then(|v| v.get("version"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;

    if current_version >= SCHEMA_VERSION {
        info!(current_version, "schema is up to date");
        return Ok(());
    }

    info!(
        from = current_version,
        to = SCHEMA_VERSION,
        "applying schema migrations"
    );

    // Apply each pending migration in order.
    for migration in MIGRATIONS {
        if migration.version <= current_version {
            continue;
        }
        if !migration.sql.is_empty() {
            db.query(migration.sql).await?;
        }
        info!(version = migration.version, "applied migration");
    }

    // Record the final version using UPSERT with a fixed record ID.
    // This ensures only one schema_version record ever exists.
    db.query("UPSERT schema_version:current SET version = $version, applied_at = time::now()")
        .bind(("version", SCHEMA_VERSION))
        .await?;

    info!(version = SCHEMA_VERSION, "schema migrations complete");
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "kv-mem")]
mod tests {
    use super::*;

    /// Create a fresh in-memory SurrealDB client for testing.
    async fn test_db() -> surrealdb::Surreal<surrealdb::engine::any::Any> {
        let db = surrealdb::engine::any::connect("mem://")
            .await
            .expect("connect should succeed");
        db.use_ns("test").use_db("test").await.expect("use_ns/db");
        db
    }

    #[tokio::test]
    async fn test_migration_chain_fresh() {
        let db = test_db().await;
        apply_schema(&db)
            .await
            .expect("apply_schema should succeed");

        // Verify version matches current SCHEMA_VERSION.
        let mut response = db
            .query("SELECT version FROM schema_version:current")
            .await
            .expect("query should succeed");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["version"], serde_json::json!(SCHEMA_VERSION));

        // Verify DDL tables exist by querying INFO FOR DB.
        let mut response = db.query("INFO FOR DB").await.expect("info query");
        let info: Vec<serde_json::Value> = response.take(0).expect("take");
        let info_str = serde_json::to_string(&info).expect("serialize");
        for table in &[
            "driver",
            "instrument",
            "experiment",
            "instance_of",
            "connects_to",
            "experiment_plan",
            "run_record",
            "executed_from",
            "uses_instrument",
            "device_feature",
            "device_runtime_state",
            "device_lifecycle_event",
        ] {
            assert!(
                info_str.contains(table),
                "table {table} should exist in DB info"
            );
        }
    }

    #[tokio::test]
    async fn test_v7_heartbeat_and_device_state() {
        let db = test_db().await;
        apply_schema(&db).await.expect("apply_schema");

        // Verify heartbeat_at field accepts inserts on run_record
        db.query(
            "CREATE run_record SET run_uid = 'test-hb', plan_type = 'count', \
             plan_name = 'test', heartbeat_at = time::now()",
        )
        .await
        .expect("insert run_record with heartbeat_at");

        let mut resp = db
            .query("SELECT heartbeat_at FROM run_record WHERE run_uid = 'test-hb'")
            .await
            .expect("query");
        let rows: Vec<serde_json::Value> = resp.take(0).expect("take");
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0]["heartbeat_at"] != serde_json::Value::Null,
            "heartbeat_at should be set"
        );

        // Verify device_runtime_state accepts inserts and enforces unique index
        db.query(
            "CREATE device_runtime_state SET device_id = 'stage_1', \
             param_name = 'position', param_value = 42.5",
        )
        .await
        .expect("insert device_runtime_state");

        let mut resp = db
            .query(
                "SELECT device_id, param_name, param_value FROM device_runtime_state \
                 WHERE device_id = 'stage_1' AND param_name = 'position'",
            )
            .await
            .expect("query");
        let rows: Vec<serde_json::Value> = resp.take(0).expect("take");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["param_value"], serde_json::json!(42.5));
    }

    #[tokio::test]
    async fn test_migration_chain_idempotent() {
        let db = test_db().await;
        apply_schema(&db).await.expect("first apply");
        apply_schema(&db).await.expect("second apply");

        // Only one schema_version record should exist.
        let mut response = db
            .query("SELECT version FROM schema_version")
            .await
            .expect("query");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take");
        assert_eq!(
            rows.len(),
            1,
            "should have exactly one schema_version record"
        );
        assert_eq!(rows[0]["version"], serde_json::json!(SCHEMA_VERSION));
    }

    #[tokio::test]
    async fn test_migration_no_version_accumulation() {
        let db = test_db().await;

        // Simulate multiple restarts.
        for _ in 0..5 {
            apply_schema(&db).await.expect("apply_schema");
        }

        let mut response = db
            .query("SELECT version FROM schema_version")
            .await
            .expect("query");
        let rows: Vec<serde_json::Value> = response.take(0).expect("take");
        assert_eq!(
            rows.len(),
            1,
            "should have exactly ONE schema_version record after multiple inits, got {}",
            rows.len()
        );
    }
}
