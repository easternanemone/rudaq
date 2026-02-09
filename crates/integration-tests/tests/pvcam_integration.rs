#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
use experiment::run_engine::RunEngine;
use hardware::registry::DeviceRegistry;
use std::sync::Arc;

#[tokio::test]
async fn test_pvcam_with_run_engine() -> anyhow::Result<()> {
    // 1. Initialize Registry
    let registry = DeviceRegistry::new();

    // 2. Register Mock PVCAM
    #[cfg(feature = "pvcam")]
    {
        use driver_pvcam::PvcamFactory;
        registry.register_factory(Box::new(PvcamFactory));

        registry
            .register_from_toml(
                "camera",
                "Prime BSI Mock",
                "pvcam",
                toml::toml! {
                    camera_name = "MockCamera"
                }
                .into(),
            )
            .await?;
    }

    #[cfg(not(feature = "pvcam"))]
    {
        println!("Skipping test: pvcam feature not enabled");
        return Ok(());
    }

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
