//! Tests for push-based engine state broadcast (bd-sz76).

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::time::Duration;

use super::super::*;
use crate::plans::Count;
use hardware::registry::DeviceRegistry;

/// Receive the next state change with a timeout, ignoring lag errors.
async fn recv_state(rx: &mut broadcast::Receiver<EngineState>) -> EngineState {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(state)) => return state,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {} // retry
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                panic!("state broadcast channel closed unexpectedly")
            }
            Err(elapsed) => panic!("timeout waiting for state change: {elapsed}"),
        }
    }
}

#[tokio::test]
async fn subscribe_state_receives_running_then_idle() {
    let registry = DeviceRegistry::new();
    let engine = Arc::new(RunEngine::new(registry));

    let mut state_rx = engine.subscribe_state();

    engine.queue(Box::new(Count::new(2))).await;

    let engine_for_task = engine.clone();
    let handle = tokio::spawn(async move { engine_for_task.start().await });

    assert_eq!(recv_state(&mut state_rx).await, EngineState::Running);
    assert_eq!(recv_state(&mut state_rx).await, EngineState::Idle);

    // Verify the plan task completed successfully
    handle
        .await
        .expect("task panicked")
        .expect("plan execution failed");
}

#[tokio::test]
async fn subscribe_state_receives_aborting_then_idle() {
    let registry = DeviceRegistry::new();
    let engine = Arc::new(RunEngine::new(registry));

    let mut state_rx = engine.subscribe_state();

    // Use a long-running plan with delay to ensure it's still running when we abort
    engine
        .queue(Box::new(Count::new(100).with_delay(0.1)))
        .await;

    let engine_for_task = engine.clone();
    let handle = tokio::spawn(async move { engine_for_task.start().await });

    assert_eq!(recv_state(&mut state_rx).await, EngineState::Running);

    engine
        .abort("test abort")
        .await
        .expect("abort should succeed");

    assert_eq!(recv_state(&mut state_rx).await, EngineState::Aborting);
    assert_eq!(recv_state(&mut state_rx).await, EngineState::Idle);

    // Plan task should complete (aborted, but not panicked)
    let _ = handle.await.expect("task panicked");
}

#[tokio::test]
async fn subscribe_state_no_subscribers_does_not_panic() {
    let registry = DeviceRegistry::new();
    let engine = Arc::new(RunEngine::new(registry));

    engine.queue(Box::new(Count::new(1))).await;

    let engine_for_task = engine.clone();
    let handle = tokio::spawn(async move { engine_for_task.start().await });

    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("timeout running plan")
        .expect("task panicked");

    assert!(
        result.is_ok(),
        "plan should complete without state subscribers"
    );
}
