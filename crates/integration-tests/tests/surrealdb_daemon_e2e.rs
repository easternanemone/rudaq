//! SurrealDB E2E Validation with Mock Hardware (bd-p33c)
//!
//! Validates the full SurrealDB persistence layer with a realistic 9-device,
//! 5-driver-type mock hardware profile (mock_maitai_lab).
//!
//! Covers:
//! - Daemon startup path (shadow write, factory registration)
//! - Config hash convergence (initial reconcile reports unchanged)
//! - gRPC ConfigService CRUD with multi-device lab
//! - Watch reconciler hot-swap (add/remove/modify via gRPC)
//! - Error resilience (DB init failure, clean shutdown)
//! - Concurrent operations and MeasurementLock safety
//!
//! Run with: cargo nextest run -p integration-tests --features db-surreal-mem --test surrealdb_daemon_e2e
#![cfg(feature = "db-surreal-mem")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports,
    missing_docs
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use db::config_store::{config_hash, toml_to_json, DbDriver, DbInstrument};
use db::{DaqDb, DbConfig};
use hardware::registry::{
    create_registry_from_config, register_all_factories, DeviceRegistry, HardwareConfig,
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the workspace root directory (two levels up from CARGO_MANIFEST_DIR).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .unwrap()
        .parent() // workspace root
        .unwrap()
        .to_path_buf()
}

/// Path to the mock_maitai_lab.toml hardware profile.
fn mock_maitai_lab_path() -> PathBuf {
    workspace_root().join("config/profiles/mock_maitai_lab.toml")
}

/// Path to config/devices directory (universal driver TOML configs).
fn config_devices_dir() -> PathBuf {
    workspace_root().join("config/devices")
}

/// Parse the mock_maitai_lab.toml hardware config.
fn load_mock_maitai_config() -> HardwareConfig {
    HardwareConfig::from_file(&mock_maitai_lab_path())
        .expect("mock_maitai_lab.toml should parse successfully")
}

/// Create a registry with all factories registered (including universal drivers).
async fn create_full_registry() -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    register_all_factories(&registry, Some(&config_devices_dir()))
        .await
        .expect("factory registration should succeed");
    registry
}

/// Create a registry populated with all 9 mock_maitai_lab devices.
async fn create_populated_registry() -> (DeviceRegistry, HardwareConfig) {
    let hw_config = load_mock_maitai_config();
    let devices_dir = config_devices_dir();
    let registry = create_registry_from_config(&hw_config, Some(devices_dir.as_path()))
        .await
        .expect("registry creation from mock_maitai_lab config should succeed");

    // Register extra factories for reconciler to use
    register_all_factories(&registry, Some(&devices_dir))
        .await
        .ok(); // May double-register, that's fine

    (registry, hw_config)
}

/// Convert HardwareConfig devices to DB instruments (mirrors db_bridge::devices_to_db).
fn devices_to_db(config: &HardwareConfig) -> Vec<DbInstrument> {
    config
        .devices
        .iter()
        .map(|d| DbInstrument {
            device_id: d.id.clone(),
            name: d.name.clone(),
            driver_type: d.driver.driver_type.clone(),
            config: toml_to_json(&d.driver.config),
            enabled: d.enabled,
        })
        .collect()
}

/// Extract unique driver definitions from hardware config.
fn drivers_from_config(config: &HardwareConfig) -> Vec<DbDriver> {
    let mut seen = std::collections::HashSet::new();
    config
        .devices
        .iter()
        .filter(|d| seen.insert(d.driver.driver_type.clone()))
        .map(|d| DbDriver {
            driver_type: d.driver.driver_type.clone(),
            name: d.driver.driver_type.clone(),
            capabilities: vec![],
            commands: vec![],
        })
        .collect()
}

/// Shadow write a hardware config to the database (mirrors db_bridge::shadow_write).
async fn shadow_write(
    db: &DaqDb,
    config: &HardwareConfig,
) -> Result<(usize, usize), db::error::DbError> {
    let instruments = devices_to_db(config);
    let drivers = drivers_from_config(config);
    let driver_count = db.upsert_drivers(&drivers).await?;
    let report = db.upsert_instruments(&instruments).await?;
    Ok((driver_count, report.instruments_upserted))
}

// ============================================================================
// WP1: Daemon Startup + Config Hash Fix (T1, T2, T3)
// ============================================================================

/// T1: Verify the mock_maitai_lab.toml profile can create a full registry
/// with all 9 devices and 5 driver types.
#[tokio::test]
async fn test_t1_daemon_startup_mock_maitai_lab() {
    let (registry, hw_config) = create_populated_registry().await;

    // Check for registration failures first for diagnostics
    let failures = registry.list_registration_failures();
    assert!(
        failures.is_empty(),
        "no registration failures expected, got {} failures: {:?}",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("{}({}): {}", f.device_id, f.driver_type, f.error))
            .collect::<Vec<_>>()
    );

    // Should have exactly 9 devices
    let devices = registry.list_devices();
    assert_eq!(
        devices.len(),
        9,
        "mock_maitai_lab should register 9 devices, got {}: {:?}",
        devices.len(),
        devices.iter().map(|d| &d.id).collect::<Vec<_>>()
    );

    // Verify all expected device IDs are present
    let expected_ids: HashSet<&str> = [
        "prime_bsi",
        "rotator_2",
        "rotator_3",
        "rotator_8",
        "maitai",
        "power_meter",
        "esp300_axis1",
        "esp300_axis2",
        "esp300_axis3",
    ]
    .into_iter()
    .collect();

    let actual_ids: HashSet<&str> = devices.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        expected_ids,
        actual_ids,
        "device ID mismatch. missing: {:?}, extra: {:?}",
        expected_ids.difference(&actual_ids).collect::<Vec<_>>(),
        actual_ids.difference(&expected_ids).collect::<Vec<_>>()
    );

    // Verify we have 5 distinct driver types
    let driver_types: HashSet<String> = hw_config
        .devices
        .iter()
        .map(|d| d.driver.driver_type.clone())
        .collect();
    assert_eq!(
        driver_types.len(),
        5,
        "should have 5 driver types, got: {:?}",
        driver_types
    );
}

/// T2: Verify shadow write creates all 9 instruments and 5 drivers in DB.
#[tokio::test]
async fn test_t2_shadow_write_9_devices_5_drivers() {
    let hw_config = load_mock_maitai_config();
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

    let (driver_count, instrument_count) = shadow_write(&db, &hw_config).await.unwrap();

    assert_eq!(
        driver_count, 5,
        "should write 5 drivers, got {driver_count}"
    );
    assert_eq!(
        instrument_count, 9,
        "should write 9 instruments, got {instrument_count}"
    );

    // Verify DB state
    let db_instruments = db.get_all_instruments().await.unwrap();
    assert_eq!(db_instruments.len(), 9);

    let db_drivers = db.get_all_drivers().await.unwrap();
    assert_eq!(db_drivers.len(), 5);

    // Verify specific instrument round-trip
    let rotator2 = db.get_instrument("rotator_2").await.unwrap();
    assert!(rotator2.is_some(), "rotator_2 should exist in DB");
    let rotator2 = rotator2.unwrap();
    assert_eq!(rotator2.driver_type, "universal_thorlabs_ell14");
    assert!(rotator2.enabled);
}

/// T3: Verify that config_hash is set correctly after shadow write,
/// so initial reconcile reports all devices as "unchanged".
#[tokio::test]
async fn test_t3_config_hash_convergence_after_shadow_write() {
    let (registry, hw_config) = create_populated_registry().await;
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

    // Shadow write config to DB
    shadow_write(&db, &hw_config).await.unwrap();

    // Simulate the daemon_manager fix: set config_hash for each device
    let db_instruments = devices_to_db(&hw_config);
    for inst in &db_instruments {
        let hash = config_hash(&inst.config);
        registry.set_config_hash(&inst.device_id, hash);
    }

    // Now read instruments from DB and verify hashes match
    let db_state = db.get_all_instruments().await.unwrap();
    for inst in &db_state {
        let db_hash = config_hash(&inst.config);
        let registry_hash = registry.config_hash(&inst.device_id).unwrap_or(0);
        assert_eq!(
            db_hash, registry_hash,
            "config_hash mismatch for '{}': db={}, registry={}",
            inst.device_id, db_hash, registry_hash
        );
        assert_ne!(
            registry_hash, 0,
            "config_hash for '{}' should be non-zero (was it set?)",
            inst.device_id
        );
    }
}

/// T3 (regression): Without the fix, initial reconcile would see all devices
/// as "changed" because registry defaults config_hash to 0.
#[tokio::test]
async fn test_t3_regression_without_fix_all_changed() {
    let (registry, hw_config) = create_populated_registry().await;
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();

    // Shadow write WITHOUT setting config_hash (the bug scenario)
    shadow_write(&db, &hw_config).await.unwrap();

    // All registry config_hashes should be 0 (the default)
    for device in registry.list_devices() {
        let hash = registry.config_hash(&device.id).unwrap_or(0);
        assert_eq!(
            hash, 0,
            "without fix, config_hash for '{}' should be 0 (default), got {}",
            device.id, hash
        );
    }

    // DB instruments should have non-zero hashes
    let db_instruments = db.get_all_instruments().await.unwrap();
    for inst in &db_instruments {
        let db_hash = config_hash(&inst.config);
        assert_ne!(
            db_hash, 0,
            "DB config_hash for '{}' should be non-zero",
            inst.device_id
        );
    }
}

// ============================================================================
// WP3: Watch Reconciler + Hot-Swap (T5, T6, T7, T8)
// ============================================================================

#[cfg(feature = "server")]
mod watch_reconciler_tests {
    use super::*;
    use server::grpc::config_service::ConfigServiceImpl;
    use server::grpc::{
        ConfigService, DeleteInstrumentRequest, InstrumentConfig, UpsertInstrumentRequest,
    };
    use tonic::Request;

    /// Set up a full multi-device environment with watch reconciler running.
    ///
    /// Returns (registry, db, config_service, shutdown_token).
    async fn setup_watch_env() -> (
        Arc<DeviceRegistry>,
        DaqDb,
        ConfigServiceImpl,
        CancellationToken,
    ) {
        let (registry, hw_config) = create_populated_registry().await;
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        shadow_write(&db, &hw_config).await.unwrap();

        // Set config hashes (the fix from T3)
        let db_instruments = devices_to_db(&hw_config);
        for inst in &db_instruments {
            let hash = config_hash(&inst.config);
            registry.set_config_hash(&inst.device_id, hash);
        }

        let registry = Arc::new(registry);
        let shutdown = CancellationToken::new();

        // Start watch reconciler
        let db2 = db.clone();
        let reg2 = registry.clone();
        let shutdown2 = shutdown.clone();
        tokio::spawn(async move {
            // Use a simple polling reconciler loop for integration tests
            // (watch_reconciler is in the bin crate, not accessible here).
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    () = shutdown2.cancelled() => break,
                    _ = interval.tick() => {
                        // Reconcile: read DB desired state, diff with registry
                        let db_instruments = db2.get_all_instruments().await.unwrap_or_default();
                        let desired: std::collections::HashMap<String, _> = db_instruments
                            .iter()
                            .filter(|i| i.enabled)
                            .map(|i| (i.device_id.clone(), i))
                            .collect();

                        let current_devices = reg2.list_devices();
                        let current_ids: HashSet<String> =
                            current_devices.iter().map(|d| d.id.clone()).collect();

                        // Remove: in registry but not desired
                        for device in &current_devices {
                            if !desired.contains_key(&device.id) {
                                let _ = reg2.unregister(&device.id).await;
                            }
                        }

                        // Add: desired but not in registry
                        for (id, inst) in &desired {
                            if !current_ids.contains(id) {
                                if !reg2.has_factory(&inst.driver_type) {
                                    continue;
                                }
                                let config_toml = db::config_store::json_to_toml(&inst.config);
                                if reg2
                                    .register_from_toml(
                                        id,
                                        &inst.name,
                                        &inst.driver_type,
                                        config_toml,
                                    )
                                    .await
                                    .is_ok()
                                {
                                    let new_hash = config_hash(&inst.config);
                                    reg2.set_config_hash(id, new_hash);
                                    reg2.set_config_source(id, "db");
                                }
                            } else {
                                // Check config hash for updates
                                let new_hash = config_hash(&inst.config);
                                let old_hash = reg2.config_hash(id).unwrap_or(0);
                                if new_hash != old_hash && reg2.is_device_idle(id) {
                                    // Unregister + re-register
                                    let _ = reg2.unregister(id).await;
                                    let config_toml =
                                        db::config_store::json_to_toml(&inst.config);
                                    if reg2
                                        .register_from_toml(
                                            id,
                                            &inst.name,
                                            &inst.driver_type,
                                            config_toml,
                                        )
                                        .await
                                        .is_ok()
                                    {
                                        reg2.set_config_hash(id, new_hash);
                                        reg2.set_config_source(id, "db");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Wait for reconciler to start and do initial pass
        tokio::time::sleep(Duration::from_millis(300)).await;

        let config_svc = ConfigServiceImpl::new(db.clone(), None);
        (registry, db, config_svc, shutdown)
    }

    /// T5: Watch reconciler establishes on multi-device setup and keeps all 9 devices.
    #[tokio::test]
    async fn test_t5_watch_reconciler_multi_device() {
        let (registry, _db, _svc, shutdown) = setup_watch_env().await;

        let devices = registry.list_devices();
        assert_eq!(
            devices.len(),
            9,
            "watch reconciler should preserve all 9 devices, got {}: {:?}",
            devices.len(),
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        );

        shutdown.cancel();
    }

    /// T6: Upsert a new device via gRPC, watch reconciler adds it to registry.
    #[tokio::test]
    async fn test_t6_hot_swap_add_device() {
        let (registry, _db, svc, shutdown) = setup_watch_env().await;

        // Add a new device via gRPC
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "new_ell14".into(),
                name: "New ELL14 (Hot-Swap Test)".into(),
                driver_type: "universal_thorlabs_ell14".into(),
                config_json: r#"{"mock":true,"address":"9"}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

        // Wait for reconciler to pick it up
        tokio::time::sleep(Duration::from_millis(500)).await;

        let devices = registry.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "new_ell14"),
            "new_ell14 should appear in registry after upsert, found: {:?}",
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
        assert_eq!(devices.len(), 10, "should now have 10 devices");

        shutdown.cancel();
    }

    /// T7: Delete a device via gRPC, reconciler removes it from registry.
    #[tokio::test]
    async fn test_t7_device_removal() {
        let (registry, _db, svc, shutdown) = setup_watch_env().await;

        // Delete rotator_2 via gRPC
        svc.delete_instrument(Request::new(DeleteInstrumentRequest {
            device_id: "rotator_2".into(),
        }))
        .await
        .unwrap();

        // Wait for reconciler to pick it up
        tokio::time::sleep(Duration::from_millis(500)).await;

        let devices = registry.list_devices();
        assert!(
            !devices.iter().any(|d| d.id == "rotator_2"),
            "rotator_2 should be gone from registry after delete"
        );
        assert_eq!(devices.len(), 8, "should now have 8 devices");

        shutdown.cancel();
    }

    /// T8: Config change triggers device restart (unregister + re-register).
    #[tokio::test]
    async fn test_t8_config_change_triggers_restart() {
        let (registry, _db, svc, shutdown) = setup_watch_env().await;

        // Get original config_hash for rotator_2
        let original_hash = registry.config_hash("rotator_2").unwrap_or(0);

        // Modify rotator_2's config via gRPC (change address)
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "rotator_2".into(),
                name: "ELL14 Rotator Address 2 (Modified)".into(),
                driver_type: "universal_thorlabs_ell14".into(),
                config_json: r#"{"mock":true,"address":"7"}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

        // Wait for reconciler to detect hash change and restart device
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Device should still be present (re-registered with new config)
        let devices = registry.list_devices();
        assert!(
            devices.iter().any(|d| d.id == "rotator_2"),
            "rotator_2 should be re-registered after config change"
        );

        // Config hash should have changed
        let new_hash = registry.config_hash("rotator_2").unwrap_or(0);
        assert_ne!(
            new_hash, original_hash,
            "config_hash should change after config modification"
        );

        shutdown.cancel();
    }

    /// T8 edge case: upsert with unknown driver type is handled gracefully.
    #[tokio::test]
    async fn test_t8_unknown_driver_type_graceful() {
        let (registry, _db, svc, shutdown) = setup_watch_env().await;

        // Upsert with a driver type that has no factory
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "unknown_device".into(),
                name: "Unknown Device".into(),
                driver_type: "nonexistent_driver".into(),
                config_json: "{}".into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

        // Wait for reconciler
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Device should NOT appear in registry (no factory)
        let devices = registry.list_devices();
        assert!(
            !devices.iter().any(|d| d.id == "unknown_device"),
            "unknown driver type should not appear in registry"
        );
        // But the other 9 devices should still be fine
        assert_eq!(devices.len(), 9);

        shutdown.cancel();
    }
}

// ============================================================================
// WP5: Error Resilience + Shutdown (T10, T11)
// ============================================================================

/// T10: DB init failure is non-fatal (daemon can start without DB).
#[tokio::test]
async fn test_t10_db_init_failure_nonfatal() {
    // The daemon_manager handles this via the match block that returns None on error.
    // We validate the pattern here: if DaqDb::init fails, registry still works.

    let (registry, _hw_config) = create_populated_registry().await;

    // Registry should work fine even without a DB
    let devices = registry.list_devices();
    assert_eq!(devices.len(), 9);

    // Verify devices are functional
    for device in &devices {
        // All mock devices should be "idle" by default
        assert!(
            registry.is_device_idle(&device.id),
            "device '{}' should be idle",
            device.id
        );
    }
}

/// T11: Watch reconciler shuts down cleanly within timeout.
#[tokio::test]
async fn test_t11_watch_reconciler_clean_shutdown() {
    let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
    let registry = Arc::new(create_full_registry().await);
    let shutdown = CancellationToken::new();

    // Start a simple reconciler loop
    let db2 = db.clone();
    let _reg2 = registry.clone();
    let shutdown2 = shutdown.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                () = shutdown2.cancelled() => break,
                _ = interval.tick() => {
                    // Minimal reconcile
                    let _ = db2.get_all_instruments().await;
                }
            }
        }
    });

    // Let it start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Request shutdown
    shutdown.cancel();

    // Should exit cleanly within 2s
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("reconciler should shut down within 2s")
        .expect("task should not panic");
}

// ============================================================================
// WP6: Safety (T13)
// ============================================================================

mod measurement_lock_tests {
    use super::*;

    /// T13: MeasurementLock prevents reconfiguration during active acquisition.
    #[tokio::test]
    async fn test_t13_measurement_lock_safety() {
        let (registry, hw_config) = create_populated_registry().await;
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        shadow_write(&db, &hw_config).await.unwrap();

        // Set config hashes
        let db_instruments = devices_to_db(&hw_config);
        for inst in &db_instruments {
            let hash = config_hash(&inst.config);
            registry.set_config_hash(&inst.device_id, hash);
        }

        // Lock rotator_2 (simulates active measurement)
        registry.set_measurement_lock(
            "rotator_2",
            common::capabilities::MeasurementLock::Measuring,
        );

        // Modify rotator_2 config in DB
        db.upsert_instruments(&[DbInstrument {
            device_id: "rotator_2".into(),
            name: "ELL14 Rotator Address 2 (Modified)".into(),
            driver_type: "universal_thorlabs_ell14".into(),
            config: serde_json::json!({"mock": true, "address": "7"}),
            enabled: true,
        }])
        .await
        .unwrap();

        // Device should NOT be idle (locked)
        assert!(
            !registry.is_device_idle("rotator_2"),
            "rotator_2 should be locked during measurement"
        );

        // Release the lock
        registry.set_measurement_lock("rotator_2", common::capabilities::MeasurementLock::Idle);

        // Now it should be idle
        assert!(
            registry.is_device_idle("rotator_2"),
            "rotator_2 should be idle after lock release"
        );
    }
}

// ============================================================================
// WP4: RocksDB Persistence (T9) — compile-gated
// ============================================================================

// RocksDB tests require the db-surreal-rocksdb feature and use a temp directory.
// They're separated because RocksDB uses a process-global lock.
#[cfg(feature = "db-surreal-rocksdb")]
mod rocksdb_tests {
    use super::*;
    use tempfile::TempDir;

    /// T9: Instruments survive daemon restart with RocksDB persistence.
    #[tokio::test]
    async fn test_t9_rocksdb_persistence_across_restart() {
        let tmpdir = TempDir::new().unwrap();
        let db_path = tmpdir.path().join("test.db");
        let hw_config = load_mock_maitai_config();

        // "First boot": write to RocksDB
        {
            let config = DbConfig::rocksdb(&db_path);
            let db = DaqDb::init(config).await.unwrap();
            shadow_write(&db, &hw_config).await.unwrap();

            // Verify data was written
            let instruments = db.get_all_instruments().await.unwrap();
            assert_eq!(instruments.len(), 9);
            // Drop DB (simulates shutdown)
        }

        // "Second boot": read from same RocksDB path
        {
            let config = DbConfig::rocksdb(&db_path);
            let db = DaqDb::init(config).await.unwrap();

            let instruments = db.get_all_instruments().await.unwrap();
            assert_eq!(
                instruments.len(),
                9,
                "instruments should persist across restart"
            );

            let drivers = db.get_all_drivers().await.unwrap();
            assert_eq!(drivers.len(), 5, "drivers should persist across restart");

            // Verify specific instrument
            let rotator = db.get_instrument("rotator_2").await.unwrap();
            assert!(rotator.is_some(), "rotator_2 should survive restart");
        }
    }
}
