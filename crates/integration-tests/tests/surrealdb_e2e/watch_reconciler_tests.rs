// ============================================================================
// WP3: Watch Reconciler + Hot-Swap (T5, T6, T7, T8)
// ============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use db::config_store::config_hash;
use db::{DaqDb, DbConfig};
use server::grpc::config_service::ConfigServiceImpl;
use server::grpc::{
    ConfigService, DeleteInstrumentRequest, InstrumentConfig, UpsertInstrumentRequest,
};
use tokio_util::sync::CancellationToken;
use tonic::Request;

use super::helpers::*;

/// Set up a full multi-device environment with watch reconciler running.
///
/// Returns (registry, db, config_service, shutdown_token).
async fn setup_watch_env() -> (
    hardware::registry::DeviceRegistry,
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

    let registry = registry;
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
                    let desired: HashMap<String, _> = db_instruments
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
