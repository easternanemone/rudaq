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
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use common::error::DaqError;
use db::config_store::{json_to_toml, DbInstrument};
use db::DaqDb;
use hardware::registry::DeviceRegistry;
use tracing::{info, warn};

use crate::db_bridge;

/// Compute a deterministic hash of a JSON config for change detection.
///
/// Uses canonical JSON serialization (sorted keys) to ensure the hash is
/// independent of key insertion order, then hashes with `DefaultHasher`.
/// Note: `DefaultHasher` is not stable across Rust versions, but that's
/// acceptable here — hashes are only compared within a single process run.
fn config_hash(config: &serde_json::Value) -> u64 {
    let s = canonical_json(config);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Produce a canonical JSON string with sorted object keys.
///
/// Ensures deterministic serialization regardless of `serde_json` map backend
/// (BTreeMap vs IndexMap with `preserve_order` feature).
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            pairs.sort_by_key(|(k, _)| *k);
            let inner: String = pairs
                .into_iter()
                .map(|(k, v)| {
                    // Use serde_json for proper key escaping (handles quotes,
                    // backslashes, control chars in keys).
                    let key = serde_json::to_string(k).expect("String serialization is infallible");
                    format!("{}:{}", key, canonical_json(v))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        serde_json::Value::Array(arr) => {
            let inner: String = arr.iter().map(canonical_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        _ => v.to_string(),
    }
}

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
    /// Number of devices unchanged.
    pub unchanged: usize,
}

impl std::fmt::Display for ReconcileReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "reconcile: +{} added, -{} removed, ~{} updated, {} unchanged, {} errors",
            self.added.len(),
            self.removed.len(),
            self.updated.len(),
            self.unchanged,
            self.errors.len()
        )
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
    let current_ids: HashSet<String> = current_devices.iter().map(|d| d.id.clone()).collect();

    // 3a. Remove: in registry but not desired.
    for device in &current_devices {
        if !desired.contains_key(&device.id) {
            match registry.unregister(&device.id).await {
                Ok(true) => {
                    info!(device_id = device.id, "reconciler: removed device");
                    report.removed.push(device.id.clone());
                }
                Ok(false) => {
                    // Already gone (concurrent removal)
                }
                Err(e) => {
                    warn!(device_id = device.id, error = %e, "reconciler: failed to remove");
                    report.errors.push(format!("remove '{}': {e}", device.id));
                }
            }
        }
    }

    // 3b. Add or update: desired but not in registry → add; config changed → update.
    for (id, inst) in &desired {
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
                    .push(format!("update '{}' (unregister): {e}", id));
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
            report
                .errors
                .push(format!("add '{}': conversion failed", id));
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
                report.added.push(id.clone());
            }
            Err(e) => {
                warn!(device_id = id, error = %e, "reconciler: failed to add");
                report.errors.push(format!("add '{}': {e}", id));
            }
        }
    }

    info!(%report, "reconciliation complete");
    Ok(report)
}

/// Start a polling reconciler that runs `reconcile_once` at the given interval.
///
/// Runs until the `shutdown` token is cancelled. Errors are logged but do not
/// stop the loop.
#[allow(dead_code)] // Wired in Phase 3b2 (LIVE SELECT watch)
pub async fn start_polling_reconciler(
    db: DaqDb,
    registry: Arc<DeviceRegistry>,
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
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "db-surreal-mem")]
mod tests {
    use super::*;
    use db::config_store::DbInstrument;
    use db::DbConfig;
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
        assert!(report.errors[0].contains("no factory"));
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
        let ids: HashSet<String> = devices.into_iter().map(|d| d.id).collect();
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
        let s = super::canonical_json(&val);
        // Key with quotes should be properly escaped.
        assert!(
            s.contains(r#""key\"with\\quotes""#),
            "special chars in key should be escaped, got: {s}"
        );
        // Must be valid JSON.
        let reparsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        assert_eq!(reparsed["normal"], 1);
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
            now_ns, run_game_loop, GameLoopConfig, NodeStateUpdate, NodeValue, SystemStateSnapshot,
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use server::grpc::proto::DeleteInstrumentRequest;
        use server::grpc::{
            ConfigService, ConfigServiceImpl, HardwareService, HardwareServiceImpl,
            InstrumentConfig, ListDevicesRequest, UpsertInstrumentRequest,
        };
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
        let config_svc = ConfigServiceImpl::new(db.clone());
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
        use crate::watch_reconciler::{start_watch_reconciler, WatchConfig};
        use tokio_util::sync::CancellationToken;

        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        let registry = Arc::new(test_registry());
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
}
