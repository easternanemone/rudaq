#![cfg(not(target_arch = "wasm32"))]
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
//! Device disconnect and fault-injection tests.
//!
//! These tests verify that the system degrades gracefully when devices fail
//! mid-operation. Uses the MockDevice error injection system to simulate:
//! - Communication loss (device goes offline)
//! - Hardware faults (device reports error code)
//! - Operation failures after N successes
//! - Timeout on specific operations
//!
//! Run with: cargo nextest run -p integration-tests --test fault_injection

use driver_mock::MockMode;
use driver_mock::common::{ErrorConfig, ErrorScenario};
use hardware::capabilities::{ExposureControl, FrameProducer, Movable, Readable, Triggerable};
use hardware::drivers::mock::{MockCamera, MockPowerMeter, MockStage};

// =============================================================================
// Stage: Communication Loss Mid-Scan
// =============================================================================

/// A stage that loses communication after 3 moves should fail on the 4th
/// move but not panic or corrupt state.
#[tokio::test]
async fn test_stage_communication_loss_mid_scan() {
    let stage = MockStage::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "move",
            count: 3,
        }))
        .build();

    // First 3 moves should succeed
    for i in 1..=3 {
        let target = f64::from(i) * 5.0;
        stage
            .move_abs(target)
            .await
            .unwrap_or_else(|e| panic!("Move {i} should succeed but failed: {e}"));
    }

    // 4th move should fail with an error, not a panic
    let result = stage.move_abs(20.0).await;
    assert!(
        result.is_err(),
        "Move 4 should fail after 3 successful operations"
    );
    let err = result.unwrap_err();
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("Injected failure") || err_msg.contains("failure"),
        "Error should indicate injected failure, got: {err_msg}"
    );

    // Position should still be queryable (last known good state)
    // The position query itself might also fail if it uses the same error config,
    // but the system should not panic
    let _pos_result = stage.position().await;
}

/// After a communication loss, subsequent operations should also fail
/// (the device stays "disconnected").
#[tokio::test]
async fn test_stage_persistent_communication_loss() {
    let stage = MockStage::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::CommunicationLoss))
        .build();

    // First operation triggers the communication loss
    let result1 = stage.move_abs(10.0).await;
    assert!(
        result1.is_err(),
        "First move should trigger communication loss"
    );

    // All subsequent operations should also fail
    let result2 = stage.move_abs(20.0).await;
    assert!(
        result2.is_err(),
        "Move after communication loss should also fail"
    );

    let result3 = stage.move_rel(5.0).await;
    assert!(
        result3.is_err(),
        "Relative move after communication loss should fail"
    );
}

// =============================================================================
// Camera: Error Config Resilience
// =============================================================================

/// MockCamera error injection is soft (logging-only in the streaming path).
/// Verify that the trigger path remains functional even when error_config
/// is set — the camera should not panic or corrupt state.
#[tokio::test]
async fn test_camera_trigger_resilient_to_error_config() {
    let camera = MockCamera::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "frame_capture",
            count: 2,
        }))
        .build();

    // Arm and trigger should succeed — error_config is soft on trigger path
    camera.arm().await.unwrap();
    camera.trigger().await.unwrap();
    assert_eq!(camera.frame_count(), 1);

    camera.trigger().await.unwrap();
    assert_eq!(camera.frame_count(), 2);

    // Even after exceeding the error count, trigger doesn't hard-fail
    // because MockCamera error injection is logging-only in this path.
    // The important contract: no panics, no state corruption.
    camera.trigger().await.unwrap();
    assert_eq!(camera.frame_count(), 3);
}

/// A camera with a hardware fault error_config should still have queryable
/// resolution and not panic during arm/trigger operations.
#[tokio::test]
async fn test_camera_resolution_survives_hardware_fault_config() {
    let camera = MockCamera::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::HardwareFault {
            code: 0xDEAD,
        }))
        .build();

    // Resolution should always be queryable regardless of error_config
    let (w, h) = camera.resolution();
    assert!(
        w > 0 && h > 0,
        "Resolution should be valid with fault config"
    );

    // Arm should succeed — error_config is soft
    camera.arm().await.unwrap();

    // Trigger should not panic (error_config is soft/logging-only)
    let _result = camera.trigger().await;

    // Resolution still valid after trigger attempt
    let (w2, h2) = camera.resolution();
    assert_eq!((w, h), (w2, h2), "Resolution must not change after fault");
}

// =============================================================================
// Power Meter: Intermittent Failures
// =============================================================================

/// A power meter that fails after N reads should return errors without
/// corrupting the measurement pipeline.
#[tokio::test]
async fn test_power_meter_fails_after_n_reads() {
    let pm = MockPowerMeter::builder()
        .base_power(1e-3)
        .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "read",
            count: 5,
        }))
        .build();

    // First 5 reads should succeed and return valid values
    for i in 1..=5 {
        let value = pm
            .read()
            .await
            .unwrap_or_else(|e| panic!("Read {i} should succeed: {e}"));
        assert!(value.is_finite(), "Read {i} should return finite value");
    }

    // 6th read should fail
    let result = pm.read().await;
    assert!(result.is_err(), "Read 6 should fail after 5 successes");
}

/// Communication loss on a power meter should prevent all subsequent reads.
#[tokio::test]
async fn test_power_meter_communication_loss() {
    let pm = MockPowerMeter::builder()
        .base_power(1e-3)
        .error_config(ErrorConfig::scenario(ErrorScenario::CommunicationLoss))
        .build();

    // First read triggers communication loss
    let result = pm.read().await;
    assert!(
        result.is_err(),
        "First read should trigger communication loss"
    );

    // Subsequent reads should also fail
    for _ in 0..5 {
        assert!(
            pm.read().await.is_err(),
            "Reads should stay failed after loss"
        );
    }
}

// =============================================================================
// Cross-Device: Partial System Failure
// =============================================================================

/// When one device in a multi-device system fails, the other devices
/// should continue to function normally. This tests that device failures
/// are isolated.
#[tokio::test]
async fn test_partial_system_failure_isolation() {
    // Stage will fail after 2 moves
    let stage = MockStage::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "move",
            count: 2,
        }))
        .build();

    // Camera is healthy
    let camera = MockCamera::new(1920, 1080);

    // Power meter is healthy
    let pm = MockPowerMeter::new(1e-3);

    // First scan point: all devices work
    stage.move_abs(10.0).await.unwrap();
    stage.wait_settled().await.unwrap();
    camera.arm().await.unwrap();
    camera.trigger().await.unwrap();
    let _reading = pm.read().await.unwrap();

    // Second scan point: all devices still work
    stage.move_abs(20.0).await.unwrap();
    stage.wait_settled().await.unwrap();
    camera.trigger().await.unwrap();
    let _reading = pm.read().await.unwrap();

    // Third scan point: stage fails, but camera and power meter should continue
    let stage_result = stage.move_abs(30.0).await;
    assert!(stage_result.is_err(), "Stage should fail on 3rd move");

    // Camera should still work independently
    camera.trigger().await.unwrap();
    assert_eq!(camera.frame_count(), 3, "Camera should have 3 frames");

    // Power meter should still work independently
    let reading = pm.read().await.unwrap();
    assert!(reading.is_finite(), "Power meter should still read");
}

/// When a stage fails mid-scan, the camera should be able to continue
/// capturing at the last known position. This tests the "degraded mode"
/// operation pattern.
#[tokio::test]
async fn test_degraded_scan_continues_at_last_position() {
    let stage = MockStage::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
            operation: "move",
            count: 3,
        }))
        .build();
    let camera = MockCamera::new(1920, 1080);

    camera.arm().await.unwrap();

    let mut positions_captured = Vec::new();
    let scan_targets = [0.0, 5.0, 10.0, 15.0, 20.0];

    for &target in &scan_targets {
        match stage.move_abs(target).await {
            Ok(()) => {
                stage.wait_settled().await.unwrap();
                camera.trigger().await.unwrap();
                positions_captured.push(target);
            }
            Err(_) => {
                // Stage failed — continue capturing at last known position
                // (graceful degradation)
                camera.trigger().await.unwrap();
                // Record the last successful position
                if let Some(&last) = positions_captured.last() {
                    positions_captured.push(last);
                }
            }
        }
    }

    // Should have captured a frame at every scan point (some at degraded positions)
    assert_eq!(
        positions_captured.len(),
        scan_targets.len(),
        "Should capture at every scan point, even if degraded"
    );
    assert_eq!(
        camera.frame_count(),
        scan_targets.len() as u64,
        "Should have one frame per scan point"
    );

    // First 3 should be at correct positions, last 2 at degraded position
    assert_eq!(positions_captured[0], 0.0);
    assert_eq!(positions_captured[1], 5.0);
    assert_eq!(positions_captured[2], 10.0);
    // Positions 3 and 4 should be at the last good position (10.0)
    assert_eq!(
        positions_captured[3], 10.0,
        "Degraded capture should use last position"
    );
    assert_eq!(
        positions_captured[4], 10.0,
        "Degraded capture should use last position"
    );
}

// =============================================================================
// Timeout Injection
// =============================================================================

/// A stage with a timeout on the "move" operation should return an error
/// rather than hanging indefinitely.
#[tokio::test]
async fn test_stage_timeout_on_move() {
    let stage = MockStage::builder()
        .error_config(ErrorConfig::scenario(ErrorScenario::Timeout {
            operation: "move",
        }))
        .build();

    let result = stage.move_abs(10.0).await;
    assert!(result.is_err(), "Move should fail with timeout");

    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Timeout") || err_msg.contains("timeout"),
        "Error should indicate timeout, got: {err_msg}"
    );
}
