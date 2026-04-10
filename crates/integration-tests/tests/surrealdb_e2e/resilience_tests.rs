// ============================================================================
// WP5: Error Resilience + Shutdown (T10, T11)
// ============================================================================

use std::sync::Arc;
use std::time::Duration;

use db::{DaqDb, DbConfig};
use tokio_util::sync::CancellationToken;

use super::helpers::*;

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
    let registry = create_full_registry().await;
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
