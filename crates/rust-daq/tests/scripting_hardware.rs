#![cfg(not(target_arch = "wasm32"))]
//! Integration Tests - Hardware Scripting Bindings
//!
//! Tests the complete integration between Rhai scripts and hardware devices.
//! Validates that async hardware operations can be safely called from
//! synchronous scripts using the async→sync bridge.

use rust_daq::hardware::capabilities::{FrameProducer, Movable};
use rust_daq::hardware::mock::{MockCamera, MockStage};
use rust_daq::scripting::{CameraHandle, RhaiEngine, ScriptEngine, ScriptValue, StageHandle};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_movement_from_script() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());
    let stage_handle = StageHandle {
        driver: stage.clone(),
        data_tx: None,
    };

    engine
        .set_global("stage", ScriptValue::new(stage_handle.clone()))
        .unwrap();

    let script = r#"
        stage.move_abs(10.0);
        let pos = stage.position();
        pos
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<f64>().unwrap(), 10.0);

    // Verify the underlying driver state
    assert_eq!(stage.position().await.unwrap(), 10.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_relative_movement() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage,
                data_tx: None,
            }),
        )
        .unwrap();

    let script = r#"
        stage.move_abs(5.0);
        stage.move_rel(3.0);
        stage.move_rel(-2.0);
        stage.position()
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<f64>().unwrap(), 6.0); // 5.0 + 3.0 - 2.0
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_wait_settled() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage,
                data_tx: None,
            }),
        )
        .unwrap();

    // Test that wait_settled can be called without error
    let script = r#"
        stage.move_abs(15.0);
        stage.wait_settled();
        "success"
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<String>().unwrap(), "success");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_camera_trigger_from_script() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let camera = Arc::new(MockCamera::new(1920, 1080));
    let camera_handle = CameraHandle {
        driver: camera.clone(),
        data_tx: None,
    };

    engine
        .set_global("camera", ScriptValue::new(camera_handle))
        .unwrap();

    let script = r#"
        camera.arm();
        camera.trigger();
        let res = camera.resolution();
        res[0]
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<i64>().unwrap(), 1920);

    // Verify frame was captured
    assert_eq!(camera.frame_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_camera_resolution_access() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let camera = Arc::new(MockCamera::new(640, 480));

    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera,
                data_tx: None,
            }),
        )
        .unwrap();

    let script = r#"
        let res = camera.resolution();
        let width = res[0];
        let height = res[1];
        width * height
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<i64>().unwrap(), 640 * 480);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_device_script() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage,
                data_tx: None,
            }),
        )
        .unwrap();
    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera,
                data_tx: None,
            }),
        )
        .unwrap();

    let script = r#"
        // Simple scan experiment
        camera.arm();  // Must arm before triggering
        for i in 0..5 {
            let pos = i * 2.0;
            stage.move_abs(pos);
            camera.trigger();
        }
        stage.position()
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<f64>().unwrap(), 8.0); // Final position: 4 * 2.0
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scan_with_settle_and_trigger() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage,
                data_tx: None,
            }),
        )
        .unwrap();
    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera.clone(),
                data_tx: None,
            }),
        )
        .unwrap();

    let script = r#"
        camera.arm();

        // Scan from 0 to 10mm in 1mm steps
        for i in 0..11 {
            stage.move_abs(i * 1.0);
            stage.wait_settled();
            camera.trigger();
        }

        stage.position()
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<f64>().unwrap(), 10.0);

    // Should have captured 11 frames (0-10 inclusive)
    assert_eq!(camera.frame_count(), 11);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sleep_function_in_script() {
    let mut engine = RhaiEngine::with_hardware().unwrap();

    let start = std::time::Instant::now();
    let script = "sleep(0.05)"; // 50ms

    engine.execute_script(script).await.unwrap();

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() >= 45); // Allow some tolerance
    assert!(elapsed.as_millis() <= 100);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_complex_workflow() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage.clone(),
                data_tx: None,
            }),
        )
        .unwrap();
    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera.clone(),
                data_tx: None,
            }),
        )
        .unwrap();

    // Realistic scientific workflow
    let script = r#"
        // Setup
        camera.arm();
        let res = camera.resolution();
        print("Camera resolution: " + res[0] + "x" + res[1]);

        // Calibration scan
        print("Starting calibration scan...");
        for i in 0..3 {
            let pos = i * 5.0;
            print("Moving to " + pos + "mm");
            stage.move_abs(pos);
            stage.wait_settled();
            camera.trigger();
        }

        // Return home
        stage.move_abs(0.0);
        stage.wait_settled();

        let final_pos = stage.position();
        print("Scan complete, position: " + final_pos);
        final_pos
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<f64>().unwrap(), 0.0);

    // Verify 3 frames captured during scan
    assert_eq!(camera.frame_count(), 3);
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "Camera trigger failed")]
async fn test_error_handling_unarmed_camera() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let camera = Arc::new(MockCamera::new(1920, 1080));

    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera,
                data_tx: None,
            }),
        )
        .unwrap();

    // Trigger without arming should cause panic (or error propagated from bindings)
    let script = "camera.trigger()";

    // Rhai binding usually panics or returns error.
    // The original test expected panic. Let's see if RhaiEngine catches it as an Error or if it panics.
    // If the binding panics, the task might panic.
    // `RhaiEngine` runs in `spawn_blocking`. If it panics, `await` might return JoinError.
    // However, `should_panic` expects the test thread to panic.
    // If the panic happens in a spawned task, it might not propagate unless we unwrap the JoinResult properly.
    // `RhaiEngine::execute_script` maps JoinError to ScriptError::AsyncError.
    // So it likely won't panic the test thread directly, it will return an Err.
    // BUT, the original code was `host.engine_mut().eval...` which ran on the same thread (mostly).
    // `RhaiEngine` runs on a separate thread.
    // If the binding panics, `execute_script` returns Err(AsyncError(Task join error: ... panic ...)).
    // So I should probably assert it returns Err and check the message, OR rely on `should_panic` if I unwrap() the result.
    // Let's try unwrapping the result. If the task panicked, the unwrap will panic with the error message.
    engine.execute_script(script).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_triggers_same_arm() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let camera = Arc::new(MockCamera::new(1920, 1080));

    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera.clone(),
                data_tx: None,
            }),
        )
        .unwrap();

    let script = r#"
        camera.arm();
        camera.trigger();
        camera.trigger();
        camera.trigger();
        "success"
    "#;

    let result = engine.execute_script(script).await.unwrap();
    assert_eq!(result.downcast::<String>().unwrap(), "success");

    // MockCamera stays armed, should have 3 frames
    assert_eq!(camera.frame_count(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_safety_limit_respected() {
    let mut engine = RhaiEngine::with_hardware().unwrap();
    let stage = Arc::new(MockStage::new());

    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage,
                data_tx: None,
            }),
        )
        .unwrap();

    // Script with too many operations (>10000)
    let script = r#"
        for i in 0..20000 {
            let dummy = i * 2;
        }
    "#;

    let result = engine.execute_script(script).await;
    assert!(result.is_err());

    // Script should be terminated due to safety limit
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Safety limit exceeded") || err_msg.contains("terminated"),
        "Expected 'Safety limit exceeded' or 'terminated' in error, got: {}",
        err_msg
    );
}
