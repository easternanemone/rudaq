//! Reconciler: diffs DB desired state against live DeviceRegistry and applies changes.
//!
//! Follows the Kubernetes reconciler pattern:
//! 1. Read desired state from SurrealDB
//! 2. Read observed state from DeviceRegistry (DashMap)
//! 3. Compute diff (add/remove)
//! 4. Apply changes
//!
//! This module provides `reconcile_once()` for startup and periodic resync.
//! A polling loop (`start_polling_reconciler`) can run in the background.

use std::collections::{HashMap, HashSet};

use common::error::DaqError;
use db::DaqDb;
use db::config_store::{DbDeviceFeature, DbDriver, DbInstrument, config_hash, json_to_toml};
use hardware::registry::DeviceRegistry;
use tracing::{info, warn};

use crate::db_bridge;

/// Summary of changes applied during a single reconciliation pass.
#[derive(Debug, Default)]
pub struct ReconcileReport {
    /// Device IDs added to the registry.
    pub added: Vec<String>,
    /// Device IDs removed from the registry.
    pub removed: Vec<String>,
    /// Device IDs reconfigured in place (hot-swap).
    pub updated: Vec<String>,
    /// Errors encountered during reconciliation.
    pub errors: Vec<String>,
    /// Driver types where DB metadata diverged from factory metadata.
    pub metadata_drifts: Vec<String>,
    /// Driver types repaired by writing canonical metadata back to DB.
    pub metadata_repairs: Vec<String>,
    /// Number of devices unchanged.
    pub unchanged: usize,
}

impl std::fmt::Display for ReconcileReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "reconcile: +{} added, -{} removed, ~{} updated, {} unchanged, {} errors, ^{} metadata drift, *{} metadata repairs",
            self.added.len(),
            self.removed.len(),
            self.updated.len(),
            self.unchanged,
            self.errors.len(),
            self.metadata_drifts.len(),
            self.metadata_repairs.len()
        )
    }
}

fn normalized(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn driver_from_factory(
    driver_type: String,
    factory_name: String,
    capabilities: Vec<String>,
    available_commands: Vec<String>,
) -> DbDriver {
    DbDriver {
        driver_type,
        name: factory_name,
        capabilities: normalized(capabilities),
        commands: normalized(available_commands),
    }
}

fn metadata_needs_repair(existing: &DbDriver, canonical: &DbDriver) -> bool {
    existing.name != canonical.name
        || normalized(existing.capabilities.clone()) != canonical.capabilities
        || normalized(existing.commands.clone()) != canonical.commands
}

/// Ensure DB driver metadata is present and complete for desired instruments.
///
/// Returns driver types that are blocked because metadata was incomplete and
/// could not be repaired.
async fn repair_driver_metadata(
    db: &DaqDb,
    registry: &DeviceRegistry,
    desired: &HashMap<String, &DbInstrument>,
    report: &mut ReconcileReport,
) -> Result<HashSet<String>, DaqError> {
    let mut blocked = HashSet::new();
    let mut to_upsert: Vec<DbDriver> = Vec::new();

    let existing_drivers: HashMap<String, DbDriver> = db
        .get_all_drivers()
        .await
        .map_err(|e| DaqError::Configuration(format!("DB driver read failed: {e}")))?
        .into_iter()
        .map(|driver| (driver.driver_type.clone(), driver))
        .collect();

    let mut desired_driver_types: Vec<String> = desired
        .values()
        .map(|instrument| instrument.driver_type.clone())
        .collect();
    desired_driver_types.sort();
    desired_driver_types.dedup();

    for driver_type in desired_driver_types {
        let factory_info = registry.factory_info(&driver_type);
        let existing = existing_drivers.get(&driver_type);

        match (existing, factory_info) {
            (Some(existing_driver), Some(factory)) => {
                let canonical = driver_from_factory(
                    driver_type.clone(),
                    factory.name,
                    factory.capabilities,
                    factory.available_commands,
                );
                if metadata_needs_repair(existing_driver, &canonical) {
                    warn!(
                        driver_type = %driver_type,
                        existing_capabilities = ?existing_driver.capabilities,
                        canonical_capabilities = ?canonical.capabilities,
                        existing_commands = ?existing_driver.commands,
                        canonical_commands = ?canonical.commands,
                        "reconciler: driver metadata drift detected"
                    );
                    report.metadata_drifts.push(driver_type.clone());
                    to_upsert.push(canonical);
                }
            }
            (None, Some(factory)) => {
                warn!(
                    driver_type = %driver_type,
                    "reconciler: missing driver metadata record, recovering from factory"
                );
                report.metadata_drifts.push(driver_type.clone());
                to_upsert.push(driver_from_factory(
                    driver_type.clone(),
                    factory.name,
                    factory.capabilities,
                    factory.available_commands,
                ));
            }
            (Some(existing_driver), None) => {
                if existing_driver.capabilities.is_empty() && existing_driver.commands.is_empty() {
                    let msg = format!("metadata '{driver_type}': no factory and metadata is empty");
                    warn!(driver_type = %driver_type, "reconciler: {}", msg);
                    report.errors.push(msg);
                    blocked.insert(driver_type);
                }
            }
            (None, None) => {
                let msg = format!("metadata '{driver_type}': missing driver record and factory");
                warn!(driver_type = %driver_type, "reconciler: {}", msg);
                report.errors.push(msg);
                blocked.insert(driver_type);
            }
        }
    }

    for driver in to_upsert {
        let driver_type = driver.driver_type.clone();
        if let Err(err) = db.upsert_drivers(&[driver]).await {
            let msg = format!("metadata '{driver_type}': repair failed: {err}");
            warn!(driver_type = %driver_type, error = %err, "reconciler: metadata repair failed");
            report.errors.push(msg);
            blocked.insert(driver_type);
            continue;
        }
        info!(driver_type = %driver_type, "reconciler: metadata repaired");
        report.metadata_repairs.push(driver_type);
    }

    Ok(blocked)
}

/// Convert a device's parameter metadata into `DbDeviceFeature` rows and
/// persist them to SurrealDB.
///
/// Called after successful device registration or reconfiguration.
///
/// **Primary path** (native drivers): Uses `DeviceRegistry::get_parameterized()`
/// to access the device's `ParameterSet`, then maps each parameter's
/// `ObservableMetadata` to a `DbDeviceFeature`.
///
/// **Fallback path** (universal/manifest-driven drivers): When `Parameterized`
/// is not available, uses `manifest_features` from the device's metadata.
/// These are static feature descriptors extracted from the TOML manifest.
async fn persist_device_features(db: &DaqDb, registry: &DeviceRegistry, device_id: &str) {
    // Primary path: native drivers with live Parameterized trait
    if let Some(parameterized) = registry.get_parameterized(device_id) {
        let params = parameterized.parameters();
        let features: Vec<DbDeviceFeature> = params
            .iter()
            .map(|(name, param)| {
                let meta = param.metadata();
                DbDeviceFeature {
                    device_id: device_id.to_owned(),
                    feature_name: name.to_owned(),
                    feature_type: meta.dtype.clone(),
                    readable: true,
                    writable: !meta.read_only,
                    min_value: meta.min_value,
                    max_value: meta.max_value,
                    step: meta.step,
                    enum_values: meta.enum_values.clone(),
                    unit: meta.units.clone(),
                    description: meta.description.clone(),
                    group_name: None, // Populated by C.2 when group metadata is added
                }
            })
            .collect();

        upsert_features(db, device_id, &features, "parameterized").await;
        return;
    }

    // Fallback path: manifest-driven devices (universal driver)
    let Some(device_info) = registry.get_device_info(device_id) else {
        return;
    };

    if device_info.metadata.manifest_features.is_empty() {
        return;
    }

    let features: Vec<DbDeviceFeature> = device_info
        .metadata
        .manifest_features
        .iter()
        .map(|mf| DbDeviceFeature {
            device_id: device_id.to_owned(),
            feature_name: mf.name.clone(),
            feature_type: mf.feature_type.clone(),
            readable: mf.readable,
            writable: mf.writable,
            min_value: mf.min_value,
            max_value: mf.max_value,
            step: None,
            enum_values: Vec::new(),
            unit: mf.unit.clone(),
            description: mf.description.clone(),
            group_name: None,
        })
        .collect();

    upsert_features(db, device_id, &features, "manifest").await;
}

/// Shared helper to upsert device features and log the result.
async fn upsert_features(db: &DaqDb, device_id: &str, features: &[DbDeviceFeature], source: &str) {
    if features.is_empty() {
        return;
    }

    match db.upsert_device_features(features).await {
        Ok(count) => {
            info!(
                device_id,
                count, source, "reconciler: persisted device feature metadata"
            );
        }
        Err(e) => {
            warn!(
                device_id,
                error = %e,
                source,
                "reconciler: failed to persist device feature metadata"
            );
        }
    }
}

/// Restore persisted parameter values from SurrealDB after device registration (bd-oqo7.8).
///
/// Reads the `device_runtime_state` table for the device and applies each
/// persisted value to the device's Parameters via the `Parameterized` trait.
/// Skips read-only parameters and logs warnings for values that fail to apply
/// (e.g., hardware-rejected ranges after a firmware update).
async fn restore_device_parameters(db: &DaqDb, registry: &DeviceRegistry, device_id: &str) {
    let saved_state = match db.get_device_state(device_id).await {
        Ok(state) if !state.is_empty() => state,
        Ok(_) => return, // No persisted state — first boot
        Err(e) => {
            warn!(
                device_id,
                error = %e,
                "reconciler: failed to read persisted parameter state"
            );
            return;
        }
    };

    let Some(parameterized) = registry.get_parameterized(device_id) else {
        return;
    };

    let params = parameterized.parameters();
    let mut restored = 0u32;
    let mut skipped = 0u32;

    for saved in &saved_state {
        // Skip internal pseudo-parameters (e.g., _lifecycle_state)
        if saved.param_name.starts_with('_') {
            continue;
        }

        let Some(param) = params.get(&saved.param_name) else {
            tracing::debug!(
                device_id,
                param = saved.param_name,
                "reconciler: persisted param not found on device, skipping"
            );
            skipped += 1;
            continue;
        };

        if param.metadata().read_only {
            continue;
        }

        match param.set_json(saved.param_value.clone()) {
            Ok(()) => {
                restored += 1;
            }
            Err(e) => {
                warn!(
                    device_id,
                    param = saved.param_name,
                    value = %saved.param_value,
                    error = %e,
                    "reconciler: failed to restore parameter"
                );
                skipped += 1;
            }
        }
    }

    if restored > 0 {
        info!(
            device_id,
            restored, skipped, "reconciler: restored persisted parameter state from DB"
        );
    }
}

/// Remove cached feature metadata for a device from SurrealDB.
///
/// Called after successful device removal.
async fn cleanup_device_features(db: &DaqDb, device_id: &str) {
    match db.delete_device_features(device_id).await {
        Ok(count) if count > 0 => {
            info!(
                device_id,
                count, "reconciler: cleaned up device feature metadata"
            );
        }
        Ok(_) => {} // Nothing to clean up
        Err(e) => {
            warn!(
                device_id,
                error = %e,
                "reconciler: failed to clean up device feature metadata"
            );
        }
    }
}

/// Perform a single reconciliation pass.
///
/// Compares the desired state in the database against the current state
/// in the device registry, then adds, removes, or updates devices to converge.
///
/// **Add**: instrument is enabled in DB but not present in registry.
/// **Remove**: device is in registry but not present (or disabled) in DB.
/// **Update**: config hash changed — try hot-reload via `Reconfigurable`,
/// fall back to unregister + register if the driver doesn't support it.
#[tracing::instrument(skip(db, registry), name = "reconcile_once")]
pub async fn reconcile_once(
    db: &DaqDb,
    registry: &DeviceRegistry,
) -> Result<ReconcileReport, DaqError> {
    let mut report = ReconcileReport::default();

    // 1. Read desired state from DB (only enabled instruments).
    let db_instruments = db
        .get_all_instruments()
        .await
        .map_err(|e| DaqError::Configuration(format!("DB read failed: {e}")))?;

    let desired: HashMap<String, &DbInstrument> = db_instruments
        .iter()
        .filter(|i| i.enabled)
        .map(|i| (i.device_id.clone(), i))
        .collect();

    // 2. Read current state from registry.
    let current_devices = registry.list_devices();
    let current_ids: HashSet<String> = current_devices.iter().map(|d| d.id.to_string()).collect();

    // 3a. Remove: in registry but not desired.
    for device in &current_devices {
        let id_str = device.id.to_string();
        if !desired.contains_key(&id_str) {
            match registry.unregister(&device.id).await {
                Ok(true) => {
                    info!(device_id = %device.id, "reconciler: removed device");
                    cleanup_device_features(db, &device.id).await;
                    report.removed.push(id_str);
                }
                Ok(false) => {
                    // Already gone (concurrent removal)
                }
                Err(e) => {
                    warn!(device_id = %device.id, error = %e, "reconciler: failed to remove");
                    report.errors.push(format!("remove '{}': {e}", device.id));
                }
            }
        }
    }

    let blocked_driver_types = repair_driver_metadata(db, registry, &desired, &mut report).await?;

    // 3b. Add or update: desired but not in registry → add; config changed → update.
    for (id, inst) in &desired {
        if blocked_driver_types.contains(&inst.driver_type) {
            continue;
        }

        if current_ids.contains(id) {
            // Device exists — check for config changes via hash.
            let new_hash = config_hash(&inst.config);
            let old_hash = registry.config_hash(id).unwrap_or(0);
            if new_hash == old_hash {
                report.unchanged += 1;
                continue;
            }

            // Config changed — try hot-reload via Reconfigurable trait.
            // Safety: check MeasurementLock before reconfiguring (laser safety).
            if !registry.is_device_idle(id) {
                info!(
                    device_id = id,
                    "reconciler: device is measuring, deferring reconfiguration"
                );
                report.unchanged += 1;
                continue;
            }
            let config_toml = json_to_toml(&inst.config);
            if let Some(reconfigurable) = registry.get_reconfigurable(id) {
                match reconfigurable.reconfigure(config_toml).await {
                    Ok(()) => {
                        registry.set_config_hash(id, new_hash);
                        registry.set_config_source(id, "db");
                        info!(
                            device_id = id,
                            config_source = "db",
                            "reconciler: reconfigured in place"
                        );
                        persist_device_features(db, registry, id).await;
                        report.updated.push(id.clone());
                        continue;
                    }
                    Err(e) => {
                        info!(
                            device_id = id,
                            error = %e,
                            "reconciler: hot-reload failed, falling back to restart"
                        );
                    }
                }
            }

            // Fall back: unregister + register (full restart).
            if let Err(e) = registry.unregister(id).await {
                report
                    .errors
                    .push(format!("update '{id}' (unregister): {e}"));
                continue;
            }
            // Re-register with new config (falls through to the add logic below).
        }

        // Check factory availability before attempting registration.
        if !registry.has_factory(&inst.driver_type) {
            let msg = format!(
                "add '{}': no factory for driver_type '{}'",
                id, inst.driver_type
            );
            warn!(
                device_id = id,
                driver_type = inst.driver_type,
                "reconciler: {}",
                msg
            );
            report.errors.push(msg);
            continue;
        }

        // Convert DB instrument to DeviceConfig via bridge.
        let hw_config = db_bridge::db_to_hardware_config(&[(*inst).clone()]);
        let Some(device_config) = hw_config.devices.into_iter().next() else {
            report.errors.push(format!("add '{id}': conversion failed"));
            continue;
        };

        match registry.register(device_config).await {
            Ok(()) => {
                registry.set_config_hash(id, config_hash(&inst.config));
                registry.set_config_source(id, "db");
                info!(
                    device_id = id,
                    config_source = "db",
                    "reconciler: added device"
                );
                persist_device_features(db, registry, id).await;
                // bd-oqo7.8: Restore persisted parameter values from SurrealDB.
                // This applies the last-known parameter state (exposure, temperature,
                // speed table, etc.) so the device resumes where it left off.
                restore_device_parameters(db, registry, id).await;
                report.added.push(id.clone());
            }
            Err(e) => {
                warn!(device_id = id, error = %e, "reconciler: failed to add");
                report.errors.push(format!("add '{id}': {e}"));
            }
        }
    }

    info!(%report, "reconciliation complete");
    Ok(report)
}

/// Detect and clean up runs with stale heartbeats (daemon crash recovery).
///
/// Finds runs where `status = 'running'` but `heartbeat_at` is older than
/// `stale_threshold` (or never set), and marks them as `"failed"` with an
/// explanatory exit reason.
pub async fn cleanup_stale_runs(db: &DaqDb, stale_threshold: std::time::Duration) {
    match db.find_stale_runs(stale_threshold).await {
        Ok(stale_runs) => {
            for stale in stale_runs {
                warn!(
                    run_uid = %stale.run_uid,
                    "reconciler: detected stale run (heartbeat timeout) — marking as failed"
                );
                if let Err(e) = db
                    .finish_run(
                        &stale.run_uid,
                        "failed",
                        0,
                        Some("stale heartbeat — daemon likely crashed"),
                    )
                    .await
                {
                    warn!(
                        run_uid = %stale.run_uid,
                        error = %e,
                        "reconciler: failed to mark stale run as failed"
                    );
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "reconciler: failed to query stale runs");
        }
    }
}

/// Start a polling reconciler that runs `reconcile_once` at the given interval.
///
/// Also runs stale-run detection on each tick (heartbeat crash recovery).
/// Runs until the `shutdown` token is cancelled. Errors are logged but do not
/// stop the loop.
#[allow(dead_code)] // Wired in Phase 3b2 (LIVE SELECT watch)
pub async fn start_polling_reconciler(
    db: DaqDb,
    registry: DeviceRegistry,
    interval: std::time::Duration,
    shutdown: tokio_util::sync::CancellationToken,
) {
    info!(
        interval_ms = interval.as_millis(),
        "starting polling reconciler"
    );
    let mut tick = tokio::time::interval(interval);

    loop {
        tokio::select! {
            () = shutdown.cancelled() => {
                info!("polling reconciler shutting down");
                break;
            }
            _ = tick.tick() => {
                match reconcile_once(&db, &registry).await {
                    Ok(report) => {
                        if !report.added.is_empty() || !report.removed.is_empty() || !report.updated.is_empty() || !report.errors.is_empty() {
                            info!(%report, "reconciler tick");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "reconciler tick failed");
                    }
                }

                // Check for stale runs (daemon crash recovery).
                cleanup_stale_runs(&db, std::time::Duration::from_secs(60)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "db")]
mod tests {
    use super::*;
    use db::DbConfig;
    use db::config_store::DbInstrument;
    use driver_mock::MockPowerMeterFactory;

    /// Create a registry with mock factories pre-registered.
    fn test_registry() -> DeviceRegistry {
        let registry = DeviceRegistry::new();
        // Register mock_power_meter factory so reconciler can add devices.
        registry.register_factory(Box::new(MockPowerMeterFactory));
        registry
    }

    fn sample_instrument(id: &str) -> DbInstrument {
        DbInstrument {
            device_id: id.into(),
            name: format!("Test {id}"),
            driver_type: "mock_power_meter".into(),
            config: serde_json::json!({}),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_reconcile_adds_missing_devices() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Add instrument to DB
        db.upsert_instruments(&[sample_instrument("pm_1")])
            .await
            .unwrap();

        // Reconcile should add it to registry
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.added, vec!["pm_1"]);
        assert!(report.removed.is_empty());
        assert!(report.errors.is_empty());

        // Verify it's in the registry
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "pm_1");
    }

    #[tokio::test]
    async fn test_reconcile_removes_extra_devices() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Manually register a device in the registry
        registry
            .register_from_toml(
                "orphan_1",
                "Orphan Device",
                "mock_power_meter",
                toml::Value::Table(Default::default()),
            )
            .await
            .unwrap();
        assert_eq!(registry.list_devices().len(), 1);

        // DB is empty — reconcile should remove the orphan
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.removed, vec!["orphan_1"]);
        assert!(report.added.is_empty());

        assert!(registry.list_devices().is_empty());
    }

    #[tokio::test]
    async fn test_reconcile_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        db.upsert_instruments(&[sample_instrument("pm_1")])
            .await
            .unwrap();

        // First reconcile adds the device
        let r1 = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(r1.added.len(), 1);

        // Second reconcile should be a no-op
        let r2 = reconcile_once(&db, &registry).await.unwrap();
        assert!(r2.added.is_empty());
        assert!(r2.removed.is_empty());
        assert_eq!(r2.unchanged, 1);
    }

    #[tokio::test]
    async fn test_reconcile_skips_disabled_instruments() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        let mut inst = sample_instrument("pm_disabled");
        inst.enabled = false;
        db.upsert_instruments(&[inst]).await.unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(report.added.is_empty());
        assert!(registry.list_devices().is_empty());
    }

    #[tokio::test]
    async fn test_reconcile_reports_missing_factory() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = DeviceRegistry::new(); // No factories registered

        db.upsert_instruments(&[sample_instrument("pm_1")])
            .await
            .unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(report.added.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("missing driver record and factory"));
    }

    #[tokio::test]
    async fn test_reconcile_repairs_incomplete_driver_metadata() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        db.upsert_drivers(&[DbDriver {
            driver_type: "mock_power_meter".to_string(),
            name: "mock_power_meter".to_string(),
            capabilities: vec![],
            commands: vec![],
        }])
        .await
        .unwrap();
        db.upsert_instruments(&[sample_instrument("pm_1")])
            .await
            .unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(report.errors.is_empty());
        assert_eq!(report.metadata_drifts, vec!["mock_power_meter"]);
        assert_eq!(report.metadata_repairs, vec!["mock_power_meter"]);

        let repaired = db
            .get_all_drivers()
            .await
            .unwrap()
            .into_iter()
            .find(|driver| driver.driver_type == "mock_power_meter")
            .expect("mock_power_meter driver metadata should exist");
        assert!(
            !repaired.capabilities.is_empty(),
            "metadata repair should restore capabilities from factory"
        );
    }

    #[tokio::test]
    async fn test_reconcile_blocks_unrepairable_metadata() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = DeviceRegistry::new(); // no factory + no driver metadata

        let unknown = DbInstrument {
            driver_type: "unknown_driver".to_string(),
            ..sample_instrument("unknown_1")
        };
        db.upsert_instruments(&[unknown]).await.unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(report.added.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("missing driver record and factory"));
    }

    #[tokio::test]
    async fn test_reconcile_add_and_remove_mix() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Pre-register a device that should be removed
        registry
            .register_from_toml(
                "old_device",
                "Old",
                "mock_power_meter",
                toml::Value::Table(Default::default()),
            )
            .await
            .unwrap();

        // Add a new device to DB that should be added
        db.upsert_instruments(&[sample_instrument("new_device")])
            .await
            .unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.added, vec!["new_device"]);
        assert_eq!(report.removed, vec!["old_device"]);

        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "new_device");
    }

    #[tokio::test]
    async fn test_end_to_end_db_to_registry() {
        // Contract test: add instrument to DB → reconcile → appears in list_devices
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // 1. Add via DB CRUD
        let instruments = vec![sample_instrument("pm_1"), sample_instrument("pm_2")];
        db.upsert_instruments(&instruments).await.unwrap();

        // 2. Reconcile
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.added.len(), 2);

        // 3. Verify in registry
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 2);
        let ids: HashSet<String> = devices.into_iter().map(|d| d.id.to_string()).collect();
        assert!(ids.contains("pm_1"));
        assert!(ids.contains("pm_2"));

        // 4. Remove one from DB
        db.delete_instrument("pm_1").await.unwrap();

        // 5. Re-reconcile
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.removed, vec!["pm_1"]);
        assert_eq!(report.unchanged, 1);

        // 6. Verify final state
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "pm_2");
    }

    /// Config change should trigger the update (hot-reload) path, falling back
    /// to unregister+register when the device doesn't support Reconfigurable.
    #[tokio::test]
    async fn test_reconcile_updates_changed_config() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Initial reconcile — add the device.
        db.upsert_instruments(&[sample_instrument("pm_update")])
            .await
            .unwrap();
        let r1 = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(r1.added, vec!["pm_update"]);

        // Change config — reconciler should detect hash change.
        let mut updated = sample_instrument("pm_update");
        updated.config = serde_json::json!({"new_key": "new_value"});
        db.upsert_instruments(&[updated]).await.unwrap();

        // Second reconcile should detect the change and re-register
        // (mock_power_meter doesn't implement Reconfigurable, so it falls
        // back to unregister+register which shows up as "added").
        let r2 = reconcile_once(&db, &registry).await.unwrap();
        assert!(
            r2.added.contains(&"pm_update".to_string()),
            "changed config should trigger re-registration, got: {r2}"
        );
        assert_eq!(r2.unchanged, 0);

        // Device should still be in the registry.
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "pm_update");
    }

    /// Canonical JSON hash should be independent of key insertion order.
    #[test]
    fn test_canonical_json_key_order_independent() {
        let a = serde_json::json!({"z": 1, "a": 2, "m": 3});
        let b = serde_json::json!({"a": 2, "m": 3, "z": 1});
        assert_eq!(super::config_hash(&a), super::config_hash(&b));
    }

    /// Canonical JSON hash should distinguish different values.
    #[test]
    fn test_canonical_json_different_values() {
        let a = serde_json::json!({"port": "/dev/ttyUSB0", "baud": 9600});
        let b = serde_json::json!({"port": "/dev/ttyUSB1", "baud": 9600});
        assert_ne!(super::config_hash(&a), super::config_hash(&b));
    }

    /// Canonical JSON should properly escape special characters in keys.
    #[test]
    fn test_canonical_json_escapes_special_keys() {
        let val = serde_json::json!({"key\"with\\quotes": 42, "normal": 1});
        // Hashing should be stable and not panic for escaped/special key names.
        let h1 = super::config_hash(&val);
        let h2 = super::config_hash(&val);
        assert_eq!(h1, h2);
    }

    /// MeasurementLock should defer reconfiguration when device is measuring.
    #[tokio::test]
    async fn test_reconcile_defers_when_measuring() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Add device and reconcile.
        db.upsert_instruments(&[sample_instrument("pm_locked")])
            .await
            .unwrap();
        reconcile_once(&db, &registry).await.unwrap();

        // Mark device as measuring.
        registry.set_measurement_lock(
            "pm_locked",
            common::capabilities::MeasurementLock::Measuring,
        );

        // Change config.
        let mut updated = sample_instrument("pm_locked");
        updated.config = serde_json::json!({"changed": true});
        db.upsert_instruments(&[updated]).await.unwrap();

        // Reconcile should defer — device stays, counted as unchanged.
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(report.added.is_empty(), "should not re-add while measuring");
        assert!(
            report.updated.is_empty(),
            "should not update while measuring"
        );
        assert_eq!(report.unchanged, 1, "should count as unchanged (deferred)");

        // Release the lock.
        registry.set_measurement_lock("pm_locked", common::capabilities::MeasurementLock::Idle);

        // Now reconcile should detect the change.
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert!(
            report.added.contains(&"pm_locked".to_string()),
            "should re-register after lock released, got: {report}"
        );
    }

    /// Full E2E: DB → reconciler → registry → state poller → game loop → broadcast.
    ///
    /// Proves the complete data path works end-to-end: inserting an instrument
    /// into the DB, reconciling it into the registry, polling its Readable value,
    /// feeding the game loop, and receiving a SystemStateSnapshot via broadcast.
    #[tokio::test]
    async fn test_e2e_db_to_game_loop_broadcast() {
        use common::state_cache::{
            GameLoopConfig, NodeStateUpdate, NodeValue, SystemStateSnapshot, now_ns, run_game_loop,
        };
        use tokio::sync::{broadcast, mpsc};
        use tokio_util::sync::CancellationToken;

        // 1. Init DB and registry.
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // 2. Add mock_power_meter to DB and reconcile into registry.
        db.upsert_instruments(&[sample_instrument("pm_e2e")])
            .await
            .unwrap();
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.added, vec!["pm_e2e"]);

        // 3. Verify the device is Readable.
        let readable = registry.get_readable("pm_e2e");
        assert!(readable.is_some(), "mock_power_meter should be Readable");

        // 4. Start the game loop.
        let (update_tx, update_rx) = mpsc::channel::<NodeStateUpdate>(64);
        let (broadcast_tx, mut broadcast_rx) = broadcast::channel::<SystemStateSnapshot>(16);
        let shutdown = CancellationToken::new();

        let shutdown_gl = shutdown.clone();
        tokio::spawn(run_game_loop(
            update_rx,
            broadcast_tx,
            GameLoopConfig {
                tick_rate_hz: 100,
                broadcast_capacity: 16,
            },
            shutdown_gl,
        ));

        // 5. Simulate the state poller: read the device and send an update.
        let readable = readable.unwrap();
        let value = readable.read().await.unwrap();
        update_tx
            .send(NodeStateUpdate {
                device_id: "pm_e2e".into(),
                timestamp_ns: now_ns(),
                value: NodeValue::Analog(value),
                metadata: std::collections::HashMap::new(),
            })
            .await
            .unwrap();

        // 6. Receive the broadcast snapshot.
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(2), broadcast_rx.recv())
            .await
            .expect("timeout waiting for broadcast")
            .expect("broadcast should not be closed");

        // 7. Verify the snapshot contains our device.
        assert!(!snapshot.nodes.is_empty(), "snapshot should have nodes");
        let node = snapshot
            .nodes
            .iter()
            .find(|n| n.device_id == "pm_e2e")
            .expect("snapshot should contain pm_e2e");
        assert!(
            matches!(node.value, NodeValue::Analog(_)),
            "value should be Analog"
        );

        // 8. Clean up.
        shutdown.cancel();
    }

    /// E2E: watch reconciler → DB change → device appears in registry → readable.
    ///
    /// Exercises the reactive path: LIVE SELECT notification triggers
    /// reconcile_once, which adds the device to the registry. Then verifies
    /// the device is Readable (proving the full factory → capabilities path).
    #[tokio::test]
    async fn test_e2e_watch_to_readable() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        // Start watch reconciler.
        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(50),
                    max_debounce_wait: std::time::Duration::from_secs(1),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    resync_interval: std::time::Duration::ZERO,
                },
                shutdown2,
            )
            .await;
        });

        // Let the watch reconciler establish the LIVE SELECT stream.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Insert an instrument — LIVE SELECT should trigger reconciliation.
        db.upsert_instruments(&[sample_instrument("pm_reactive")])
            .await
            .unwrap();

        // Wait for debounce + reconcile.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Verify the device is in the registry AND is Readable.
        let devices = registry.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "pm_reactive"),
            "pm_reactive should be in registry"
        );
        let readable = registry.get_readable("pm_reactive");
        assert!(readable.is_some(), "pm_reactive should be Readable");

        // Read a value to prove the driver is fully operational.
        let value = readable.unwrap().read().await.unwrap();
        assert!(value.is_finite(), "read value should be finite");

        shutdown.cancel();
    }

    /// E2E: watch reconciler detects DB delete → device removed from registry.
    ///
    /// Exercises the reactive delete path: insert a device, let the watch
    /// reconciler add it, then delete from DB and verify the watch reconciler
    /// removes it from the registry via LIVE SELECT → reconcile_once.
    #[tokio::test]
    async fn test_e2e_watch_detects_delete() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        // Start watch reconciler.
        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(50),
                    max_debounce_wait: std::time::Duration::from_secs(1),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    resync_interval: std::time::Duration::ZERO,
                },
                shutdown2,
            )
            .await;
        });

        // Let the watch reconciler establish the LIVE SELECT stream.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Insert an instrument — watch reconciler should add it.
        db.upsert_instruments(&[sample_instrument("pm_del")])
            .await
            .unwrap();

        // Wait for debounce + reconcile.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            registry.list_devices().iter().any(|d| d.id == "pm_del"),
            "pm_del should be in registry after insert"
        );

        // Delete from DB — LIVE SELECT should trigger reconcile → removal.
        db.delete_instrument("pm_del").await.unwrap();

        // Wait for debounce + reconcile.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !registry.list_devices().iter().any(|d| d.id == "pm_del"),
            "pm_del should be removed from registry after delete"
        );

        shutdown.cancel();
    }

    /// E2E: full lifecycle through watch reconciler — insert → update → delete.
    ///
    /// Proves the watch reconciler handles all three LIVE SELECT action types
    /// in sequence: Create → Update → Delete, each triggering reconcile_once.
    #[tokio::test]
    async fn test_e2e_watch_full_lifecycle() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(50),
                    max_debounce_wait: std::time::Duration::from_secs(1),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    resync_interval: std::time::Duration::ZERO,
                },
                shutdown2,
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Phase 1: Insert — device should appear.
        db.upsert_instruments(&[sample_instrument("pm_lifecycle")])
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let devices = registry.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "pm_lifecycle"),
            "phase 1 (insert): pm_lifecycle should be in registry"
        );

        // Phase 2: Update config — device should be re-registered with new config.
        let mut updated = sample_instrument("pm_lifecycle");
        updated.config = serde_json::json!({"wavelength_nm": 1064});
        db.upsert_instruments(&[updated]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Device should still be present (re-registered, not removed).
        let devices = registry.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "pm_lifecycle"),
            "phase 2 (update): pm_lifecycle should still be in registry"
        );

        // Phase 3: Delete — device should be removed.
        db.delete_instrument("pm_lifecycle").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let devices = registry.list_devices();
        assert!(
            !devices.iter().any(|d| d.id == "pm_lifecycle"),
            "phase 3 (delete): pm_lifecycle should be removed from registry"
        );

        shutdown.cancel();
    }

    /// E2E hot-swap test: gRPC → DB → LIVE SELECT → reconciler → registry → gRPC.
    ///
    /// Exercises the full ConfigService + watch reconciler pipeline:
    /// 1. Start watch reconciler + gRPC service impls sharing the same DB + registry
    /// 2. UpsertInstrument via ConfigService → device appears in HardwareService.ListDevices
    /// 3. DeleteInstrument via ConfigService → device disappears from ListDevices
    ///
    /// This proves the complete hot-swap flow without network transport overhead.
    #[tokio::test]
    #[cfg(feature = "networking")]
    async fn test_e2e_grpc_config_hot_swap() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use server::grpc::proto::DeleteInstrumentRequest;
        use server::grpc::{
            ConfigService, ConfigServiceImpl, HardwareService, HardwareServiceImpl,
            InstrumentConfig, ListDevicesRequest, UpsertInstrumentRequest,
        };
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        // Start watch reconciler.
        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(50),
                    max_debounce_wait: std::time::Duration::from_secs(1),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    resync_interval: std::time::Duration::ZERO,
                },
                shutdown2,
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Create gRPC service impls sharing the same DB + registry.
        let config_svc = ConfigServiceImpl::new(db.clone(), Some(registry.clone()));
        let hw_svc = HardwareServiceImpl::new(registry.clone());

        // Verify initially empty.
        let resp = hw_svc
            .list_devices(tonic::Request::new(ListDevicesRequest {
                capability_filter: None,
            }))
            .await
            .unwrap();
        assert!(
            resp.into_inner().devices.is_empty(),
            "registry should be empty initially"
        );

        // Step 1: UpsertInstrument via gRPC ConfigService.
        let upsert_resp = config_svc
            .upsert_instrument(tonic::Request::new(UpsertInstrumentRequest {
                instrument: Some(InstrumentConfig {
                    device_id: "pm_hotswap".into(),
                    name: "Hot-Swap Test".into(),
                    driver_type: "mock_power_meter".into(),
                    config_json: "{}".into(),
                    enabled: true,
                }),
            }))
            .await
            .unwrap();
        assert!(upsert_resp.into_inner().success);

        // Wait for LIVE SELECT → debounce → reconcile.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Step 2: Verify device appears in HardwareService.ListDevices.
        let resp = hw_svc
            .list_devices(tonic::Request::new(ListDevicesRequest {
                capability_filter: None,
            }))
            .await
            .unwrap();
        let devices = resp.into_inner().devices;
        assert!(
            devices.iter().any(|d| d.id == "pm_hotswap"),
            "pm_hotswap should appear in ListDevices after upsert, got: {:?}",
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        );

        // Step 3: DeleteInstrument via gRPC ConfigService.
        let del_resp = config_svc
            .delete_instrument(tonic::Request::new(DeleteInstrumentRequest {
                device_id: "pm_hotswap".into(),
            }))
            .await
            .unwrap();
        assert!(del_resp.into_inner().success);

        // Wait for LIVE SELECT → debounce → reconcile.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Step 4: Verify device is gone from HardwareService.ListDevices.
        let resp = hw_svc
            .list_devices(tonic::Request::new(ListDevicesRequest {
                capability_filter: None,
            }))
            .await
            .unwrap();
        assert!(
            resp.into_inner().devices.is_empty(),
            "pm_hotswap should be gone after delete"
        );

        shutdown.cancel();
    }

    /// MeasurementLock safety: watch reconciler defers reconfiguration
    /// while a device is actively measuring, then applies it after release.
    ///
    /// Exercises the safety interlock through the full watch reconciler path:
    /// LIVE SELECT detects the config change, but reconcile_once skips the
    /// device because MeasurementLock::Measuring is set. After releasing the
    /// lock, the periodic resync (or next notification) picks it up.
    #[tokio::test]
    async fn test_e2e_measurement_lock_defers_reconfig() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        // Start watch reconciler with short resync for the "after release" phase.
        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(50),
                    max_debounce_wait: std::time::Duration::from_secs(1),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    // Short resync so the lock-release is picked up quickly.
                    resync_interval: std::time::Duration::from_millis(500),
                },
                shutdown2,
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Insert device and wait for reconcile.
        db.upsert_instruments(&[sample_instrument("pm_lock_test")])
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            registry
                .list_devices()
                .iter()
                .any(|d| d.id == "pm_lock_test"),
            "pm_lock_test should be in registry"
        );

        // Lock the device (simulates active measurement).
        registry.set_measurement_lock(
            "pm_lock_test",
            common::capabilities::MeasurementLock::Measuring,
        );

        // Change config — LIVE SELECT fires, but reconcile should defer.
        let mut updated = sample_instrument("pm_lock_test");
        updated.config = serde_json::json!({"wavelength_nm": 1064});
        db.upsert_instruments(&[updated]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Device should still be present (not removed, not updated).
        assert!(
            registry
                .list_devices()
                .iter()
                .any(|d| d.id == "pm_lock_test"),
            "device should remain during measurement"
        );

        // Release the lock.
        registry.set_measurement_lock("pm_lock_test", common::capabilities::MeasurementLock::Idle);

        // Wait for periodic resync to pick up the deferred change.
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        // Device should still be present (re-registered with new config).
        assert!(
            registry
                .list_devices()
                .iter()
                .any(|d| d.id == "pm_lock_test"),
            "device should be re-registered after lock release"
        );

        shutdown.cancel();
    }

    /// Concurrent modification stress test: N simultaneous upserts.
    ///
    /// Spawns N tasks that each call `db.upsert_instruments` with different
    /// devices. Verifies: no panics, no data loss, final state consistent,
    /// reconciler converges.
    #[tokio::test]
    async fn test_e2e_concurrent_upserts_converge() {
        use crate::watch_reconciler::{WatchConfig, start_watch_reconciler};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();
        let shutdown = CancellationToken::new();

        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            start_watch_reconciler(
                db2,
                reg2,
                WatchConfig {
                    debounce: std::time::Duration::from_millis(100),
                    max_debounce_wait: std::time::Duration::from_secs(2),
                    fallback_poll_interval: std::time::Duration::from_secs(60),
                    resync_interval: std::time::Duration::ZERO,
                },
                shutdown2,
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Spawn N concurrent upserts.
        let n = 10;
        let mut handles = Vec::new();
        for i in 0..n {
            let db_clone = db.clone();
            handles.push(tokio::spawn(async move {
                db_clone
                    .upsert_instruments(&[DbInstrument {
                        device_id: format!("concurrent_{i}"),
                        name: format!("Concurrent Device {i}"),
                        driver_type: "mock_power_meter".into(),
                        config: serde_json::json!({"index": i}),
                        enabled: true,
                    }])
                    .await
                    .unwrap();
            }));
        }

        // Wait for all upserts to complete.
        for h in handles {
            h.await.unwrap();
        }

        // Wait for debounce + reconcile to converge.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Verify: all N devices in DB.
        let instruments = db.get_all_instruments().await.unwrap();
        assert_eq!(instruments.len(), n, "all {n} instruments should be in DB");

        // Verify: all N devices in registry.
        let devices = registry.list_devices();
        assert_eq!(
            devices.len(),
            n,
            "all {n} devices should be in registry, found: {:?}",
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        );

        // Verify each device is present.
        for i in 0..n {
            let id = format!("concurrent_{i}");
            assert!(
                devices.iter().any(|d| d.id == id),
                "device {id} should be in registry"
            );
        }

        shutdown.cancel();
    }

    /// Feature persistence: adding a device should cache its parameter metadata
    /// in the device_feature table.
    #[tokio::test]
    async fn test_reconcile_persists_device_features() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Add device to DB and reconcile — should trigger persist_device_features.
        db.upsert_instruments(&[sample_instrument("pm_feat")])
            .await
            .unwrap();
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.added, vec!["pm_feat"]);

        // Verify features were persisted in device_feature table.
        let features = db.get_device_features("pm_feat").await.unwrap();
        assert!(
            !features.is_empty(),
            "device_feature table should have entries after registration"
        );

        // MockPowerMeter has at least 2 parameters (power, reading).
        assert!(
            features.len() >= 2,
            "expected at least 2 features, got {}",
            features.len()
        );

        // Verify fields are populated.
        for feat in &features {
            assert_eq!(feat.device_id, "pm_feat");
            assert!(!feat.feature_name.is_empty());
            assert!(!feat.feature_type.is_empty());
        }
    }

    /// Feature cleanup: removing a device should delete its cached features.
    #[tokio::test]
    async fn test_reconcile_cleans_up_features_on_removal() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Add and reconcile.
        db.upsert_instruments(&[sample_instrument("pm_cleanup")])
            .await
            .unwrap();
        reconcile_once(&db, &registry).await.unwrap();

        // Verify features exist.
        let features = db.get_device_features("pm_cleanup").await.unwrap();
        assert!(!features.is_empty(), "features should be persisted");

        // Remove from DB — reconcile should clean up features.
        db.delete_instrument("pm_cleanup").await.unwrap();
        let report = reconcile_once(&db, &registry).await.unwrap();
        assert_eq!(report.removed, vec!["pm_cleanup"]);

        // Verify features were cleaned up.
        let features = db.get_device_features("pm_cleanup").await.unwrap();
        assert!(
            features.is_empty(),
            "device_feature table should be empty after removal"
        );
    }

    /// Feature re-persistence: reconfiguring a device should refresh features.
    #[tokio::test]
    async fn test_reconcile_repersists_features_on_config_change() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        // Add and reconcile.
        db.upsert_instruments(&[sample_instrument("pm_reconfig")])
            .await
            .unwrap();
        reconcile_once(&db, &registry).await.unwrap();

        let features_before = db.get_device_features("pm_reconfig").await.unwrap();
        assert!(!features_before.is_empty());

        // Change config — should trigger re-registration and re-persist features.
        let mut updated = sample_instrument("pm_reconfig");
        updated.config = serde_json::json!({"wavelength_nm": 1064});
        db.upsert_instruments(&[updated]).await.unwrap();

        let report = reconcile_once(&db, &registry).await.unwrap();
        // MockPowerMeter doesn't support Reconfigurable, so it falls back to
        // unregister+register — features get re-persisted on the new registration.
        assert!(
            report.added.contains(&"pm_reconfig".to_string()),
            "should re-register, got: {report}"
        );

        let features_after = db.get_device_features("pm_reconfig").await.unwrap();
        assert!(
            !features_after.is_empty(),
            "features should be re-persisted after reconfig"
        );
        assert_eq!(features_before.len(), features_after.len());
    }

    /// Feature persistence is idempotent — running reconcile twice doesn't
    /// duplicate features.
    #[tokio::test]
    async fn test_reconcile_feature_persistence_idempotent() {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = test_registry();

        db.upsert_instruments(&[sample_instrument("pm_idem")])
            .await
            .unwrap();
        reconcile_once(&db, &registry).await.unwrap();

        let features_first = db.get_device_features("pm_idem").await.unwrap();

        // Second reconcile — no-op, but should not duplicate features.
        reconcile_once(&db, &registry).await.unwrap();

        let features_second = db.get_device_features("pm_idem").await.unwrap();
        assert_eq!(
            features_first.len(),
            features_second.len(),
            "idempotent reconcile should not duplicate features"
        );
    }
}
