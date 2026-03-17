//! Tests for plan queue operations.

use std::sync::Arc;

use tokio::time::Duration;

use super::super::*;
use crate::plans::Count;
use common::experiment::document::Document;
use hardware::registry::DeviceRegistry;

#[tokio::test]
async fn test_queue_plan() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let plan = Box::new(Count::new(5));
    let _run_uid = engine.queue(plan).await;

    assert_eq!(engine.queue_len().await, 1);
}

#[tokio::test]
async fn test_clear_queue() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let plan1 = Box::new(Count::new(5));
    let plan2 = Box::new(Count::new(3));

    engine.queue(plan1).await;
    engine.queue(plan2).await;
    assert_eq!(engine.queue_len().await, 2);

    engine.clear_queue().await;
    assert_eq!(engine.queue_len().await, 0);
}

#[tokio::test]
async fn test_multiple_plan_execution() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Queue and execute first plan
    let plan1 = Box::new(Count::new(2));
    engine.queue(plan1).await;

    let engine_clone = engine.clone();
    let task1 = tokio::spawn(async move { engine_clone.start().await });
    let _ = task1.await;

    // Queue and execute second plan
    let plan2 = Box::new(Count::new(3));
    engine.queue(plan2).await;

    let engine_clone = engine.clone();
    let task2 = tokio::spawn(async move { engine_clone.start().await });
    let _ = task2.await;

    // Count StartDoc/StopDoc documents
    let mut start_count = 0;
    let mut stop_count = 0;

    while let Ok(doc_result) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
        match doc_result {
            Ok(Document::Start(_)) => start_count += 1,
            Ok(Document::Stop(_)) => stop_count += 1,
            Err(_) => break,
            _ => {}
        }
    }

    assert_eq!(start_count, 2, "Should execute 2 plans");
    assert_eq!(stop_count, 2, "Should complete 2 plans");
}
