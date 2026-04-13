// ============================================================================
// WP6: Safety (T13)
// ============================================================================

use db::config_store::{DbInstrument, config_hash};
use db::{DaqDb, DbConfig};

use super::helpers::*;

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
