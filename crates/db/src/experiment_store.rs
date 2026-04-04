//! CRUD operations for experiment plan and run history records.
//!
//! Stores plan definitions (with pre-translated commands and optional graph data)
//! and tracks run execution history in SurrealDB. Types here are DB-native
//! (no dependency on `experiment` or `ui` crates) — conversion to/from domain
//! types happens at the gRPC boundary.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::DaqDb;
use crate::error::Result;

// ---------------------------------------------------------------------------
// DB-native types
// ---------------------------------------------------------------------------

/// An experiment plan stored in SurrealDB.
///
/// Plans contain pre-translated commands (from graph or parametric plans),
/// optional graph data for visual editing, and device metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    pub plan_id: String,
    pub name: String,
    pub plan_type: String,
    pub num_points: Option<i64>,
}

/// A run execution record stored in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRunRecord {
    /// Unique run identifier (from RunEngine).
    pub run_uid: String,
    /// Plan that spawned this run (if from a stored plan).
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

/// A run record that has a stale heartbeat, indicating a likely daemon crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleRun {
    /// Unique run identifier.
    pub run_uid: String,
    /// When the run started.
    pub started_at: Option<String>,
}

impl DaqDb {
    // -------------------------------------------------------------------
    // Experiment Plans
    // -------------------------------------------------------------------

    /// Upsert an experiment plan (insert or update by plan_id).
    pub async fn upsert_experiment_plan(&self, plan: &DbExperimentPlan) -> Result<()> {
        self.client()
            .query(
                "UPSERT experiment_plan SET \
                 plan_id = $plan_id, \
                 name = $name, \
                 description = $description, \
                 plan_type = $plan_type, \
                 commands = $commands, \
                 movers = $movers, \
                 detectors = $detectors, \
                 num_points = $num_points, \
                 graph_data = $graph_data, \
                 parameters = $parameters, \
                 device_mapping = $device_mapping, \
                 updated_at = time::now() \
                 WHERE plan_id = $plan_id",
            )
            .bind(("plan_id", plan.plan_id.clone()))
            .bind(("name", plan.name.clone()))
            .bind(("description", plan.description.clone()))
            .bind(("plan_type", plan.plan_type.clone()))
            .bind(("commands", plan.commands.clone()))
            .bind(("movers", plan.movers.clone()))
            .bind(("detectors", plan.detectors.clone()))
            .bind(("num_points", plan.num_points))
            .bind(("graph_data", plan.graph_data.clone()))
            .bind(("parameters", plan.parameters.clone()))
            .bind(("device_mapping", plan.device_mapping.clone()))
            .await?;

        info!(plan_id = %plan.plan_id, name = %plan.name, "experiment plan upserted");
        Ok(())
    }

    /// Retrieve an experiment plan by plan_id.
    pub async fn get_experiment_plan(&self, plan_id: &str) -> Result<Option<DbExperimentPlan>> {
        let mut response = self
            .client()
            .query(
                "SELECT plan_id, name, description, plan_type, commands, movers, \
                 detectors, num_points, graph_data, parameters, device_mapping \
                 FROM experiment_plan WHERE plan_id = $plan_id",
            )
            .bind(("plan_id", plan_id.to_owned()))
            .await?;
        let rows: Vec<DbExperimentPlan> = response.take(0)?;
        Ok(rows.into_iter().next())
    }

    /// List all experiment plans (lightweight summary).
    pub async fn list_experiment_plans(&self) -> Result<Vec<PlanSummary>> {
        let mut response = self
            .client()
            .query(
                "SELECT plan_id, name, plan_type, num_points, updated_at \
                 FROM experiment_plan ORDER BY updated_at DESC",
            )
            .await?;
        let rows: Vec<PlanSummary> = response.take(0)?;
        Ok(rows)
    }

    /// Delete an experiment plan by plan_id. Returns true if it existed.
    pub async fn delete_experiment_plan(&self, plan_id: &str) -> Result<bool> {
        // Check existence first, then delete — avoids RETURN BEFORE datetime
        // serialization issues with SurrealDB native datetime types.
        let exists = self.get_experiment_plan(plan_id).await?.is_some();
        if exists {
            self.client()
                .query("DELETE FROM experiment_plan WHERE plan_id = $plan_id")
                .bind(("plan_id", plan_id.to_owned()))
                .await?;
            info!(plan_id, "experiment plan deleted");
        }
        Ok(exists)
    }

    // -------------------------------------------------------------------
    // Run Records
    // -------------------------------------------------------------------

    /// Create a new run record.
    pub async fn create_run_record(&self, record: &DbRunRecord) -> Result<()> {
        self.client()
            .query(
                "UPSERT run_record SET \
                 run_uid = $run_uid, \
                 plan_id = $plan_id, \
                 plan_type = $plan_type, \
                 plan_name = $plan_name, \
                 status = $status, \
                 num_events = $num_events, \
                 metadata = $metadata, \
                 started_at = time::now() \
                 WHERE run_uid = $run_uid",
            )
            .bind(("run_uid", record.run_uid.clone()))
            .bind(("plan_id", record.plan_id.clone()))
            .bind(("plan_type", record.plan_type.clone()))
            .bind(("plan_name", record.plan_name.clone()))
            .bind(("status", record.status.clone()))
            .bind(("num_events", record.num_events))
            .bind(("metadata", record.metadata.clone()))
            .await?;

        info!(run_uid = %record.run_uid, plan_type = %record.plan_type, "run record created");
        Ok(())
    }

    /// Update a run record's status.
    pub async fn update_run_status(&self, run_uid: &str, status: &str) -> Result<()> {
        self.client()
            .query(
                "UPDATE run_record SET status = $status \
                 WHERE run_uid = $run_uid",
            )
            .bind(("run_uid", run_uid.to_owned()))
            .bind(("status", status.to_owned()))
            .await?;
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
        self.client()
            .query(
                "UPDATE run_record SET \
                 status = $status, \
                 num_events = $num_events, \
                 exit_reason = $exit_reason, \
                 finished_at = time::now() \
                 WHERE run_uid = $run_uid",
            )
            .bind(("run_uid", run_uid.to_owned()))
            .bind(("status", status.to_owned()))
            .bind(("num_events", num_events))
            .bind(("exit_reason", reason.map(str::to_owned)))
            .await?;

        info!(run_uid, status, num_events, "run record finished");
        Ok(())
    }

    /// List recent run records.
    pub async fn list_runs(&self, limit: Option<u32>) -> Result<Vec<DbRunRecord>> {
        let limit = limit.unwrap_or(50);
        let mut response = self
            .client()
            .query(
                "SELECT run_uid, plan_id, plan_type, plan_name, status, \
                 num_events, metadata, exit_reason, started_at \
                 FROM run_record ORDER BY started_at DESC LIMIT $limit",
            )
            .bind(("limit", limit))
            .await?;
        let rows: Vec<DbRunRecord> = response.take(0)?;
        Ok(rows)
    }

    // -------------------------------------------------------------------
    // Heartbeat Monitoring
    // -------------------------------------------------------------------

    /// Update the heartbeat timestamp for an active run.
    ///
    /// Called periodically (~every 10s) by the `RunEngine` during plan
    /// execution via the `RunLifecycleHook` trait. The reconciler uses
    /// stale heartbeats to detect daemon crashes.
    pub async fn update_run_heartbeat(&self, run_uid: &str) -> Result<()> {
        self.client()
            .query(
                "UPDATE run_record SET heartbeat_at = time::now() \
                 WHERE run_uid = $run_uid",
            )
            .bind(("run_uid", run_uid.to_owned()))
            .await?;
        Ok(())
    }

    /// Find runs with stale heartbeats (daemon likely crashed).
    ///
    /// Returns runs where `status = 'running'` and either:
    /// - `heartbeat_at` is older than `stale_threshold`, or
    /// - `heartbeat_at` is `NONE` (run never sent a heartbeat).
    pub async fn find_stale_runs(&self, stale_threshold: Duration) -> Result<Vec<StaleRun>> {
        let threshold_secs = stale_threshold.as_secs();
        let mut response = self
            .client()
            .query(
                "SELECT run_uid, started_at FROM run_record \
                 WHERE status = 'running' AND (\
                     heartbeat_at IS NONE OR \
                     heartbeat_at < time::now() - type::duration($threshold)\
                 )",
            )
            .bind(("threshold", format!("{threshold_secs}s")))
            .await?;
        let rows: Vec<StaleRun> = response.take(0)?;
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "kv-mem")]
mod tests {
    use super::*;
    use crate::DbConfig;

    fn sample_plan() -> DbExperimentPlan {
        DbExperimentPlan {
            plan_id: "plan_001".into(),
            name: "Line Scan Test".into(),
            description: Some("Scan stage_x from 0 to 10".into()),
            plan_type: "graph_plan".into(),
            commands: Some(vec![
                serde_json::json!({"MoveTo": {"device_id": "stage_x", "position": 0.0}}),
                serde_json::json!({"EmitEvent": {"stream": "primary", "data": {}, "positions": {"stage_x": 0.0}}}),
                serde_json::json!({"MoveTo": {"device_id": "stage_x", "position": 10.0}}),
                serde_json::json!({"EmitEvent": {"stream": "primary", "data": {}, "positions": {"stage_x": 10.0}}}),
            ]),
            movers: Some(vec!["stage_x".into()]),
            detectors: Some(vec!["power_meter".into()]),
            num_points: Some(2),
            graph_data: Some(serde_json::json!({"version": 1, "metadata": {}, "graph": {}})),
            parameters: None,
            device_mapping: None,
        }
    }

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
    async fn test_upsert_and_get_plan() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let plan = sample_plan();

        db.upsert_experiment_plan(&plan).await.unwrap();

        let loaded = db.get_experiment_plan("plan_001").await.unwrap().unwrap();
        assert_eq!(loaded.plan_id, "plan_001");
        assert_eq!(loaded.name, "Line Scan Test");
        assert_eq!(loaded.plan_type, "graph_plan");
        assert_eq!(loaded.commands.as_ref().unwrap().len(), 4);
        assert_eq!(loaded.movers.as_ref().unwrap(), &["stage_x"]);
        assert_eq!(loaded.num_points, Some(2));
    }

    #[tokio::test]
    async fn test_upsert_plan_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let plan = sample_plan();

        db.upsert_experiment_plan(&plan).await.unwrap();
        db.upsert_experiment_plan(&plan).await.unwrap();

        let list = db.list_experiment_plans().await.unwrap();
        assert_eq!(list.len(), 1, "upsert should not duplicate plans");
    }

    #[tokio::test]
    async fn test_upsert_plan_update() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let mut plan = sample_plan();
        db.upsert_experiment_plan(&plan).await.unwrap();

        plan.name = "Updated Name".into();
        plan.num_points = Some(42);
        db.upsert_experiment_plan(&plan).await.unwrap();

        let loaded = db.get_experiment_plan("plan_001").await.unwrap().unwrap();
        assert_eq!(loaded.name, "Updated Name");
        assert_eq!(loaded.num_points, Some(42));
    }

    #[tokio::test]
    async fn test_list_plans() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let mut plan1 = sample_plan();
        plan1.plan_id = "plan_a".into();
        plan1.name = "Plan A".into();

        let mut plan2 = sample_plan();
        plan2.plan_id = "plan_b".into();
        plan2.name = "Plan B".into();

        db.upsert_experiment_plan(&plan1).await.unwrap();
        db.upsert_experiment_plan(&plan2).await.unwrap();

        let list = db.list_experiment_plans().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_plan() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.upsert_experiment_plan(&sample_plan()).await.unwrap();

        let deleted = db.delete_experiment_plan("plan_001").await.unwrap();
        assert!(deleted);

        let deleted_again = db.delete_experiment_plan("plan_001").await.unwrap();
        assert!(!deleted_again);

        let loaded = db.get_experiment_plan("plan_001").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_plan() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let loaded = db.get_experiment_plan("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_create_and_list_runs() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let run = sample_run();

        db.create_run_record(&run).await.unwrap();

        let runs = db.list_runs(None).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_uid, "run_abc123");
        assert_eq!(runs[0].status, "queued");
        assert_eq!(runs[0].plan_id, Some("plan_001".into()));
    }

    #[tokio::test]
    async fn test_update_run_status() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.create_run_record(&sample_run()).await.unwrap();

        db.update_run_status("run_abc123", "running").await.unwrap();

        let runs = db.list_runs(None).await.unwrap();
        assert_eq!(runs[0].status, "running");
    }

    #[tokio::test]
    async fn test_finish_run() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.create_run_record(&sample_run()).await.unwrap();

        db.finish_run("run_abc123", "completed", 42, None)
            .await
            .unwrap();

        let runs = db.list_runs(None).await.unwrap();
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].num_events, Some(42));
        assert!(runs[0].exit_reason.is_none());
    }

    #[tokio::test]
    async fn test_finish_run_with_reason() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.create_run_record(&sample_run()).await.unwrap();

        db.finish_run("run_abc123", "aborted", 5, Some("user requested abort"))
            .await
            .unwrap();

        let runs = db.list_runs(None).await.unwrap();
        assert_eq!(runs[0].status, "aborted");
        assert_eq!(runs[0].num_events, Some(5));
        assert_eq!(runs[0].exit_reason.as_deref(), Some("user requested abort"));
    }

    // ---------------------------------------------------------------
    // Heartbeat monitoring tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_update_run_heartbeat() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        db.create_run_record(&sample_run()).await.unwrap();
        db.update_run_status("run_abc123", "running").await.unwrap();

        // Update heartbeat — should succeed without error.
        db.update_run_heartbeat("run_abc123").await.unwrap();

        // Verify heartbeat_at is set (query raw field).
        let mut resp = db
            .client()
            .query("SELECT heartbeat_at FROM run_record WHERE run_uid = 'run_abc123'")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = resp.take(0).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0]["heartbeat_at"] != serde_json::Value::Null,
            "heartbeat_at should be set after update"
        );
    }

    #[tokio::test]
    async fn test_find_stale_runs_returns_never_heartbeated() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        // Create a running record but never send a heartbeat.
        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.unwrap();
        db.update_run_status("run_abc123", "running").await.unwrap();

        // A run with no heartbeat_at should be stale.
        let stale = db.find_stale_runs(Duration::from_secs(60)).await.unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].run_uid, "run_abc123");
    }

    #[tokio::test]
    async fn test_find_stale_runs_excludes_completed() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        // Create a completed run (no heartbeat).
        db.create_run_record(&sample_run()).await.unwrap();
        db.finish_run("run_abc123", "completed", 10, None)
            .await
            .unwrap();

        // Completed runs should NOT appear as stale.
        let stale = db.find_stale_runs(Duration::from_secs(60)).await.unwrap();
        assert!(stale.is_empty(), "completed runs should not be stale");
    }

    #[tokio::test]
    async fn test_find_stale_runs_excludes_fresh_heartbeat() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.unwrap();
        db.update_run_status("run_abc123", "running").await.unwrap();

        // Send a heartbeat — should NOT appear stale with a 60s threshold.
        db.update_run_heartbeat("run_abc123").await.unwrap();

        let stale = db.find_stale_runs(Duration::from_secs(60)).await.unwrap();
        assert!(
            stale.is_empty(),
            "freshly heartbeated run should not be stale"
        );
    }

    #[tokio::test]
    async fn test_find_stale_runs_time_based_heartbeat_threshold() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

        let mut run = sample_run();
        run.status = "running".into();
        db.create_run_record(&run).await.unwrap();
        db.update_run_status("run_abc123", "running").await.unwrap();

        // Send an initial heartbeat for the running run.
        db.update_run_heartbeat("run_abc123").await.unwrap();

        // Use a 2s threshold — large enough to avoid flakiness from query overhead.
        let threshold = Duration::from_secs(2);

        // Immediately after heartbeat, the run should NOT be stale.
        let stale = db.find_stale_runs(threshold).await.unwrap();
        assert!(
            stale.is_empty(),
            "run with heartbeat younger than threshold should not be stale"
        );

        // After crossing the threshold, the same run should now be detected as stale.
        tokio::time::sleep(Duration::from_millis(2100)).await;
        let stale = db.find_stale_runs(threshold).await.unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].run_uid, "run_abc123");
    }
}
