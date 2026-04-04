// ============================================================================
// WP1: Daemon Startup + Config Hash Fix (T1, T2, T3)
// ============================================================================

use std::collections::HashSet;

use db::config_store::config_hash;
use db::{DaqDb, DbConfig};

use super::helpers::*;

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
        "should have 5 driver types, got: {driver_types:?}"
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
