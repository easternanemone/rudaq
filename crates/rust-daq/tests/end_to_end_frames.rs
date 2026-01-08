#![cfg(all(feature = "instrument_photometrics", not(target_arch = "wasm32")))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    missing_docs
)]

use daq_core::experiment::document::Document;
use daq_experiment::plans::Count;
use daq_experiment::run_engine::RunEngine;
use daq_hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
use std::sync::Arc;
use tokio::time::Duration;

#[tokio::test]
async fn test_end_to_end_frames() -> anyhow::Result<()> {
    // 1. Setup DeviceRegistry with Mock Pvcam
    let mut registry = DeviceRegistry::new();
    registry
        .register(DeviceConfig {
            id: "camera".into(),
            name: "Mock Camera".into(),
            driver: DriverType::Pvcam {
                camera_name: "MockCam".into(),
            },
        })
        .await?;
    let registry_arc = Arc::new(registry);

    // 2. Setup RunEngine
    let run_engine = Arc::new(RunEngine::new(registry_arc.clone()));

    // 3. Subscribe to documents
    let mut doc_rx = run_engine.subscribe();

    // 4. Capture documents in background
    let captured_docs = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let captured_clone = captured_docs.clone();

    let _capture_handle = tokio::spawn(async move {
        while let Ok(doc) = doc_rx.recv().await {
            captured_clone.lock().await.push(doc);
        }
    });

    // 5. Execute Plan
    // Setup 3 frames
    let plan = Box::new(Count::new(3).with_detector("camera"));
    run_engine.queue(plan).await;
    run_engine.start().await?;

    // Wait slightly for emission
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 6. Verify Data
    let docs = captured_docs.lock().await;
    println!("Captured {} documents", docs.len());

    // Verify we have EventDocs with camera data
    let event_docs: Vec<_> = docs
        .iter()
        .filter_map(|d| {
            if let Document::Event(e) = d {
                Some(e)
            } else {
                None
            }
        })
        .collect();

    assert!(
        event_docs.len() >= 3,
        "Expected at least 3 events, got {}",
        event_docs.len()
    );

    for (i, event) in event_docs.iter().enumerate() {
        // Check for "camera" in arrays (binary data)
        if let Some(data) = event.arrays.get("camera") {
            println!("Event {}: Found camera frame of {} bytes", i, data.len());
            // Mock driver usually produces 2048*2048*2 bytes = 8MB/frame
            // But verify it's not empty
            assert!(!data.is_empty(), "Frame data should not be empty");
        } else {
            panic!("Event {} missing 'camera' array data", i);
        }
    }

    Ok(())
}
