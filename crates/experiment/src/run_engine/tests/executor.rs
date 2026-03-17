//! Tests for plan execution, document emission, and engine state.

use std::sync::Arc;

use tokio::time::Duration;

use super::super::*;
use crate::plans::Count;
use crate::plans_imperative::ImperativePlan;
use common::experiment::document::Document;
use hardware::registry::DeviceRegistry;

#[tokio::test]
async fn test_engine_state_transitions() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    assert_eq!(engine.state().await, EngineState::Idle);

    // Can't pause when idle
    assert!(engine.pause().await.is_err());

    // Can't resume when idle
    assert!(engine.resume().await.is_err());
}

#[tokio::test]
async fn test_document_subscription() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let mut rx = engine.subscribe();

    // Queue a simple plan
    let plan = Box::new(Count::new(3));
    engine.queue(plan).await;

    // Start in a separate task
    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Should receive StartDoc
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout waiting for StartDoc")
        .expect("channel error receiving StartDoc");
    match doc {
        Document::Start(start) => assert_eq!(start.plan_type, "count"),
        other => panic!(
            "Expected Start document, got {:?}",
            std::mem::discriminant(&other)
        ),
    }

    // Should receive Manifest document after Start (bd-ib06)
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout waiting for Manifest")
        .expect("channel error receiving Manifest");
    match doc {
        Document::Manifest(manifest) => {
            assert_eq!(manifest.plan_type, "count");
            assert!(manifest.system_info.contains_key("software_version"));
        }
        other => panic!(
            "Expected Manifest document, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

/// Test that Wait command can be interrupted by abort (bd-lnoi)
#[tokio::test]
async fn test_wait_interruptible_by_abort() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Create a plan with a long wait (60 seconds - would block if not interruptible)
    let plan = Box::new(ImperativePlan::wait(60.0));
    engine.queue(plan).await;

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Wait for engine to start (receive StartDoc)
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout waiting for StartDoc")
        .expect("channel error receiving StartDoc");
    assert!(
        matches!(doc, Document::Start(_)),
        "Expected Start document, got {:?}",
        std::mem::discriminant(&doc)
    );

    // Give the Wait command time to start executing
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request abort - should take effect within 200ms (2 check cycles)
    let abort_start = tokio::time::Instant::now();
    engine
        .abort("Test abort")
        .await
        .expect("Abort should succeed");

    // Wait for StopDoc with abort status
    let doc = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match rx.recv().await {
                Ok(Document::Stop(stop)) => return stop,
                Ok(_) => {} // Skip other documents
                Err(err) => panic!("Channel closed before StopDoc: {err}"),
            }
        }
    })
    .await;

    let abort_elapsed = abort_start.elapsed();

    assert!(doc.is_ok(), "Should receive StopDoc within 500ms");
    let stop = doc.unwrap();
    assert_eq!(stop.exit_status, "abort", "Exit status should be 'abort'");

    // Verify abort was fast (< 500ms, well under the 60s wait)
    // The chunked sleep checks every 100ms, so abort should complete in ~200ms max
    assert!(
        abort_elapsed < Duration::from_millis(500),
        "Abort took too long: {:?} (expected < 500ms)",
        abort_elapsed
    );
}

/// Test that normal Wait still works correctly (bd-lnoi)
#[tokio::test]
async fn test_wait_completes_normally() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Create a plan with a short wait
    let wait_duration = 0.2; // 200ms
    let plan = Box::new(ImperativePlan::wait(wait_duration));
    engine.queue(plan).await;

    let start_time = tokio::time::Instant::now();

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Wait for StopDoc
    let stop_doc = tokio::time::timeout(Duration::from_secs(2), async {
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

    let elapsed = start_time.elapsed();

    // Should complete successfully
    assert_eq!(stop_doc.exit_status, "success");

    // Timing should be approximately correct (within 100ms tolerance)
    // The wait should take at least wait_duration and not much longer
    let expected_min = Duration::from_secs_f64(wait_duration);
    let expected_max = Duration::from_secs_f64(wait_duration + 0.15); // Allow 150ms overhead

    assert!(
        elapsed >= expected_min,
        "Wait completed too fast: {:?} (expected >= {:?})",
        elapsed,
        expected_min
    );
    assert!(
        elapsed < expected_max,
        "Wait took too long: {:?} (expected < {:?})",
        elapsed,
        expected_max
    );
}

#[tokio::test]
async fn test_state_returns_correct_value() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let state = engine.state().await;
    assert!(matches!(state, EngineState::Idle));
}

#[tokio::test]
async fn test_engine_abort_when_idle() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let result = engine.abort("Test abort").await;
    // Aborting when idle should return an error
    assert!(result.is_err(), "abort() on idle engine should return Err");
    assert_eq!(
        engine.state().await,
        EngineState::Idle,
        "Engine should remain Idle"
    );
}

#[tokio::test]
async fn test_manifest_contains_plan_info() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);
    let mut rx = engine.subscribe();

    let plan = Box::new(Count::new(1));
    engine.queue(plan).await;

    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Skip StartDoc
    let _ = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;

    // Get Manifest
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(doc.is_ok());
    if let Ok(Ok(Document::Manifest(manifest))) = doc {
        assert_eq!(manifest.plan_type, "count");
        assert!(manifest.system_info.contains_key("software_version"));
        assert!(manifest.system_info.contains_key("hostname"));
    } else {
        panic!("Expected Manifest document");
    }
}

#[tokio::test]
async fn test_subscribe_multiple_times() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let mut rx1 = engine.subscribe();
    let mut rx2 = engine.subscribe();

    let plan = Box::new(Count::new(1));
    engine.queue(plan).await;

    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Both subscriptions should receive documents
    let doc1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv()).await;
    let doc2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv()).await;

    assert!(doc1.is_ok());
    assert!(doc2.is_ok());
}

#[test]
fn test_engine_state_enum_display() {
    assert_eq!(format!("{:?}", EngineState::Idle), "Idle");
    assert_eq!(format!("{:?}", EngineState::Running), "Running");
    assert_eq!(format!("{:?}", EngineState::Paused), "Paused");
}
