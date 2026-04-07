//! Tests for the orphan-plan watchdog.

use std::sync::Arc;

use tokio::time::Duration;

use super::super::*;
use crate::plans::Count;
use common::experiment::document::Document;
use hardware::registry::DeviceRegistry;

/// Test that the watchdog aborts an engine stuck in Running with no activity (bd-c9z1)
///
/// Uses direct state manipulation to avoid complex timing interactions
/// with the plan execution loop. Uses multi-thread runtime for real-time testing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_watchdog_aborts_orphaned_running_plan() {
    let registry = DeviceRegistry::new();
    let mut engine_raw = RunEngine::new(registry);
    // Timeout = 200ms, check interval = max(20ms, 1s) = 1s
    engine_raw.set_watchdog_timeout(Duration::from_millis(200));
    let engine = Arc::new(engine_raw);

    // Directly set state to Running to simulate an orphaned plan
    *engine.state.write().await = EngineState::Running;

    // Spawn the watchdog
    let watchdog_handle = engine.spawn_watchdog();

    // Wait for the check interval (1s) + margin for the watchdog to fire
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The watchdog should have called abort(), transitioning to Aborting.
    let state = engine.state().await;
    assert!(
        state == EngineState::Aborting || state == EngineState::Idle,
        "Watchdog should have triggered abort, got state: {state}"
    );

    watchdog_handle.abort();
}

/// Test that the watchdog aborts a Paused engine with no activity (bd-c9z1)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_watchdog_aborts_orphaned_paused_plan() {
    let registry = DeviceRegistry::new();
    let mut engine_raw = RunEngine::new(registry);
    engine_raw.set_watchdog_timeout(Duration::from_millis(200));
    let engine = Arc::new(engine_raw);

    // Directly set state to Paused to simulate an orphaned paused plan
    *engine.state.write().await = EngineState::Paused;

    let watchdog_handle = engine.spawn_watchdog();

    // Wait for the check interval (1s) + margin
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let state = engine.state().await;
    assert!(
        state == EngineState::Aborting || state == EngineState::Idle,
        "Watchdog should have triggered abort on paused engine, got state: {state}"
    );

    watchdog_handle.abort();
}

/// Test that the watchdog does NOT abort a plan with ongoing activity (bd-c9z1)
#[tokio::test(start_paused = true)]
async fn test_watchdog_does_not_abort_active_plan() {
    let registry = DeviceRegistry::new();
    let mut engine_raw = RunEngine::new(registry);
    // 2-second timeout
    engine_raw.set_watchdog_timeout(Duration::from_secs(2));
    let engine = Arc::new(engine_raw);

    // Queue a plan with 5 events (Count emits events rapidly)
    let plan = Box::new(Count::new(5));
    engine.queue(plan).await;

    // Spawn the watchdog
    let watchdog_handle = engine.spawn_watchdog();

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    let mut rx = engine.subscribe();

    // Collect all documents until StopDoc
    let stop_doc = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(Document::Stop(stop)) => return stop,
                Ok(_) => {}
                Err(err) => panic!("Channel closed before StopDoc: {err}"),
            }
        }
    })
    .await
    .expect("Should receive StopDoc");

    // Plan should complete successfully (watchdog should NOT have fired)
    assert_eq!(
        stop_doc.exit_status, "success",
        "Active plan should complete successfully, not be aborted by watchdog"
    );

    watchdog_handle.abort();
}

/// Test that touch_activity resets the watchdog timer (bd-c9z1)
#[tokio::test(start_paused = true)]
async fn test_touch_activity_prevents_watchdog() {
    let registry = DeviceRegistry::new();
    let mut engine_raw = RunEngine::new(registry);
    engine_raw.set_watchdog_timeout(Duration::from_secs(2));
    let engine = Arc::new(engine_raw);

    // Manually set state to Running to simulate an active plan
    *engine.state.write().await = EngineState::Running;

    let watchdog_handle = engine.spawn_watchdog();

    // Keep touching activity every 1 second (under the 2s timeout)
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        engine.touch_activity().await;
    }

    // Engine should still be Running (watchdog did not fire)
    assert_eq!(
        engine.state().await,
        EngineState::Running,
        "Watchdog should not fire when activity is refreshed"
    );

    watchdog_handle.abort();
    // Reset state to avoid panic on drop
    *engine.state.write().await = EngineState::Idle;
}
