use daq_core::experiment::document::Document;
use daq_experiment::plans::Count;
use daq_experiment::run_engine::RunEngine;
use daq_hardware::registry::DeviceRegistry;
use std::sync::Arc;
use tokio::time::Duration;

#[tokio::test]
async fn test_run_uid_consistency() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);
    let mut rx = engine.subscribe();

    let plan = Box::new(Count::new(1));
    let queued_uid = engine.queue(plan).await;

    // Start engine in background
    let engine_clone = Arc::new(engine);
    tokio::spawn(async move {
        engine_clone.start().await.unwrap();
    });

    // Capture StartDoc
    let doc = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("Timeout waiting for doc")
        .expect("Receive error");

    if let Document::Start(start_doc) = doc {
        assert_eq!(
            start_doc.uid, queued_uid,
            "StartDoc UID should match queued UID"
        );
    } else {
        panic!("First document was not StartDoc");
    }
}
