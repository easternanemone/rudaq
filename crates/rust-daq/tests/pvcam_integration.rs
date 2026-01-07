#![allow(
    clippy::unwrap_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    missing_docs
)]
use daq_experiment::run_engine::RunEngine;
use daq_hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};
use std::sync::Arc;

#[tokio::test]
async fn test_pvcam_with_run_engine() -> anyhow::Result<()> {
    // 1. Initialize Registry
    let mut registry = DeviceRegistry::new();

    // 2. Register Mock PVCAM
    registry
        .register(DeviceConfig {
            id: "camera".into(),
            name: "Prime BSI Mock".into(),
            driver: DriverType::Pvcam {
                camera_name: "MockCamera".into(), // Mock mode uses any name or specific mock flag?
                                                  // Feature "mock" is enabled in dev-dependencies
            },
        })
        .await?;

    // 3. Initialize RunEngine
    let registry_arc = Arc::new(registry);
    let run_engine = Arc::new(RunEngine::new(registry_arc.clone()));

    // 4. Subscribe to events
    let mut _rx = run_engine.subscribe();

    // 5. Verify driver from registry
    // DeviceRegistry is now internally thread-safe via DashMap (bd-834p)
    let _camera = registry_arc
        .get_frame_producer("camera")
        .expect("Camera not found");

    // Check if we can start stream (conceptually)
    // camera.start_stream(...);

    Ok(())
}
