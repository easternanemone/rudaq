//! Integration Tests - Hardware Scripting Bindings
//!
//! Tests the complete integration between Rhai scripts and hardware devices.
//! Validates that async hardware operations can be safely called from
//! synchronous scripts using the async→sync bridge.

use rust_daq::hardware::capabilities::Movable; // Needed for .position().await
use rust_daq::hardware::mock::{MockCamera, MockStage};
use rust_daq::scripting::{CameraHandle, ScriptHost, StageHandle};
use std::sync::Arc;
use tokio::runtime::Handle;

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_movement_from_script() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());
    let stage_handle = StageHandle {
        driver: stage.clone(),
    };

    let mut scope = rhai::Scope::new();
    scope.push("stage", stage_handle.clone());

    let script = r#"
        stage.move_abs(10.0);
        let pos = stage.position();
        pos
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<f64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 10.0);

    // Verify the underlying driver state
    assert_eq!(stage.position().await.unwrap(), 10.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_relative_movement() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });

    let script = r#"
        stage.move_abs(5.0);
        stage.move_rel(3.0);
        stage.move_rel(-2.0);
        stage.position()
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<f64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 6.0); // 5.0 + 3.0 - 2.0
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stage_wait_settled() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });

    // Test that wait_settled can be called without error
    let script = r#"
        stage.move_abs(15.0);
        stage.wait_settled();
        "success"
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<String>(&mut scope, script)
        .unwrap();
    assert_eq!(result, "success");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_camera_trigger_from_script() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let camera = Arc::new(MockCamera::new(1920, 1080));
    let camera_handle = CameraHandle {
        driver: camera.clone(),
    };

    let mut scope = rhai::Scope::new();
    scope.push("camera", camera_handle);

    let script = r#"
        camera.arm();
        camera.trigger();
        let res = camera.resolution();
        res[0]
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<i64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 1920);

    // Verify frame was captured
    assert_eq!(camera.frame_count().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_camera_resolution_access() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let camera = Arc::new(MockCamera::new(640, 480));

    let mut scope = rhai::Scope::new();
    scope.push("camera", CameraHandle { driver: camera });

    let script = r#"
        let res = camera.resolution();
        let width = res[0];
        let height = res[1];
        width * height
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<i64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 640 * 480);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_device_script() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });
    scope.push("camera", CameraHandle { driver: camera });

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

    let result = host
        .engine_mut()
        .eval_with_scope::<f64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 8.0); // Final position: 4 * 2.0
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scan_with_settle_and_trigger() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });
    scope.push(
        "camera",
        CameraHandle {
            driver: camera.clone(),
        },
    );

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

    let result = host
        .engine_mut()
        .eval_with_scope::<f64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 10.0);

    // Should have captured 11 frames (0-10 inclusive)
    assert_eq!(camera.frame_count().await, 11);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sleep_function_in_script() {
    let mut host = ScriptHost::with_hardware(Handle::current());

    let start = std::time::Instant::now();
    let script = "sleep(0.05)"; // 50ms

    host.engine_mut().eval::<()>(script).unwrap();

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() >= 45); // Allow some tolerance
    assert!(elapsed.as_millis() <= 100);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_complex_workflow() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });
    scope.push(
        "camera",
        CameraHandle {
            driver: camera.clone(),
        },
    );

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

    let result = host
        .engine_mut()
        .eval_with_scope::<f64>(&mut scope, script)
        .unwrap();
    assert_eq!(result, 0.0);

    // Verify 3 frames captured during scan
    assert_eq!(camera.frame_count().await, 3);
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "Camera trigger failed")]
async fn test_error_handling_unarmed_camera() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    let mut scope = rhai::Scope::new();
    scope.push("camera", CameraHandle { driver: camera });

    // Trigger without arming should cause panic
    let script = "camera.trigger()";

    // This will panic with "Camera trigger failed: ..."
    let _result = host.engine_mut().eval_with_scope::<()>(&mut scope, script);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_triggers_same_arm() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    let mut scope = rhai::Scope::new();
    scope.push(
        "camera",
        CameraHandle {
            driver: camera.clone(),
        },
    );

    let script = r#"
        camera.arm();
        camera.trigger();
        camera.trigger();
        camera.trigger();
        "success"
    "#;

    let result = host
        .engine_mut()
        .eval_with_scope::<String>(&mut scope, script)
        .unwrap();
    assert_eq!(result, "success");

    // MockCamera stays armed, should have 3 frames
    assert_eq!(camera.frame_count().await, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_safety_limit_respected() {
    let mut host = ScriptHost::with_hardware(Handle::current());
    let stage = Arc::new(MockStage::new());

    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage });

    // Script with too many operations (>10000)
    let script = r#"
        for i in 0..20000 {
            let dummy = i * 2;
        }
    "#;

    let result = host.engine_mut().eval_with_scope::<()>(&mut scope, script);
    assert!(result.is_err());

    // Script should be terminated due to safety limit
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(err_msg.contains("terminated"), "Expected 'terminated' in error, got: {}", err_msg);
}
