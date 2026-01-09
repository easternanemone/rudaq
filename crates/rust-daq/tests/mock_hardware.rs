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
//! Integration tests for mock hardware implementations
//!
//! These tests verify that MockStage and MockCamera correctly implement
//! the capability traits and exhibit realistic behavior.

use rust_daq::hardware::capabilities::{FrameProducer, Movable, Triggerable};
use rust_daq::hardware::mock::{MockCamera, MockStage};
use std::time::Instant;

// =============================================================================
// MockStage Tests
// =============================================================================

#[tokio::test]
async fn test_mock_stage_movement() {
    let stage = MockStage::new();

    // Test absolute movement
    stage.move_abs(10.0).await.unwrap();
    assert_eq!(stage.position().await.unwrap(), 10.0);

    // Test relative movement
    stage.move_rel(5.0).await.unwrap();
    assert_eq!(stage.position().await.unwrap(), 15.0);

    // Test negative relative movement
    stage.move_rel(-3.0).await.unwrap();
    assert_eq!(stage.position().await.unwrap(), 12.0);
}

#[tokio::test(start_paused = true)]
async fn test_mock_stage_timing() {
    use tokio::time::Instant;

    let stage = MockStage::new();

    // Measure time to move 20mm at 10mm/sec with paused time for determinism
    let start = Instant::now();
    stage.move_abs(20.0).await.unwrap();
    let elapsed = start.elapsed();

    println!("20mm move took: {:?} (simulated time)", elapsed);

    // With start_paused, time is deterministic: 20mm at 10mm/sec = exactly 2000ms
    assert_eq!(
        elapsed.as_millis(),
        2000,
        "Expected exactly 2000ms for 20mm move at 10mm/sec, got {}ms",
        elapsed.as_millis()
    );
}

#[tokio::test(start_paused = true)]
async fn test_mock_stage_settle_timing() {
    use tokio::time::Instant;

    let stage = MockStage::new();

    // Quick move first (100ms simulated for 1mm at 10mm/sec)
    stage.move_abs(1.0).await.unwrap();

    // Measure settle time with paused time for determinism
    let start = Instant::now();
    stage.wait_settled().await.unwrap();
    let elapsed = start.elapsed();

    println!("Settle took: {:?} (simulated time)", elapsed);

    // With start_paused, time is deterministic: settle time is exactly 50ms
    assert_eq!(
        elapsed.as_millis(),
        50,
        "Expected exactly 50ms settle time, got {}ms",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn test_mock_stage_multiple_moves() {
    let stage = MockStage::new();

    // Perform multiple moves in sequence
    for i in 1..=5 {
        let target = i as f64 * 2.0;
        stage.move_abs(target).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), target);
    }
}

// =============================================================================
// MockCamera Tests
// =============================================================================

#[tokio::test]
async fn test_mock_camera_trigger() {
    let camera = MockCamera::new(1920, 1080);

    // Must arm before trigger
    camera.arm().await.unwrap();
    camera.trigger().await.unwrap();

    assert_eq!(camera.resolution(), (1920, 1080));
}

#[tokio::test]
async fn test_mock_camera_unarmed_trigger_fails() {
    let camera = MockCamera::new(640, 480);

    // Should fail without arming
    let result = camera.trigger().await;
    assert!(
        result.is_err(),
        "Trigger should fail when camera is not armed"
    );
}

#[tokio::test]
async fn test_mock_camera_frame_count() {
    let camera = MockCamera::new(1920, 1080);

    camera.arm().await.unwrap();

    // Capture 5 frames
    for i in 1..=5 {
        camera.trigger().await.unwrap();
        assert_eq!(camera.frame_count(), i);
    }
}

#[tokio::test(start_paused = true)]
async fn test_mock_camera_trigger_timing() {
    use tokio::time::Instant;

    let camera = MockCamera::new(1920, 1080);

    camera.arm().await.unwrap();

    // Measure trigger time with paused time for deterministic results
    let start = Instant::now();
    camera.trigger().await.unwrap();
    let elapsed = start.elapsed();

    println!("Frame readout took: {:?} (simulated time)", elapsed);

    // With start_paused, time is deterministic - MockCamera uses 33ms frame readout
    assert_eq!(
        elapsed.as_millis(),
        33,
        "Expected exactly 33ms frame readout in simulated time, got {}ms",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn test_mock_camera_streaming() {
    let camera = MockCamera::new(1920, 1080);

    // Start streaming
    camera.start_stream().await.unwrap();
    assert!(camera.is_streaming().await);

    // Cannot start while already streaming
    let result = camera.start_stream().await;
    assert!(result.is_err());

    // Stop streaming
    camera.stop_stream().await.unwrap();
    assert!(!camera.is_streaming().await);
}

#[tokio::test]
async fn test_mock_camera_resolutions() {
    let cameras = [
        MockCamera::new(1920, 1080),
        MockCamera::new(640, 480),
        MockCamera::new(3840, 2160),
    ];

    assert_eq!(cameras[0].resolution(), (1920, 1080));
    assert_eq!(cameras[1].resolution(), (640, 480));
    assert_eq!(cameras[2].resolution(), (3840, 2160));
}

// =============================================================================
// Combined Hardware Tests
// =============================================================================

#[tokio::test]
async fn test_synchronized_stage_camera() {
    let stage = MockStage::new();
    let camera = MockCamera::new(1920, 1080);

    // Simulate a simple scan: move stage, trigger camera at each position
    let positions = [0.0, 5.0, 10.0, 15.0, 20.0];

    camera.arm().await.unwrap();

    for (i, &pos) in positions.iter().enumerate() {
        // Move stage
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        // Capture frame
        camera.trigger().await.unwrap();

        // Verify state
        assert_eq!(stage.position().await.unwrap(), pos);
        assert_eq!(camera.frame_count(), (i + 1) as u64);
    }

    println!("Scan complete: {} positions acquired", positions.len());
}

#[tokio::test]
async fn test_parallel_hardware_operations() {
    let stage1 = MockStage::new();
    let stage2 = MockStage::new();
    let camera = MockCamera::new(1920, 1080);

    // Start parallel operations
    let stage1_task = tokio::spawn(async move {
        stage1.move_abs(20.0).await.unwrap();
        stage1.position().await.unwrap()
    });

    let stage2_task = tokio::spawn(async move {
        stage2.move_abs(15.0).await.unwrap();
        stage2.position().await.unwrap()
    });

    let camera_task = tokio::spawn(async move {
        camera.arm().await.unwrap();
        for _ in 0..3 {
            camera.trigger().await.unwrap();
        }
        camera.frame_count()
    });

    // Wait for all tasks
    let stage1_pos = stage1_task.await.unwrap();
    let stage2_pos = stage2_task.await.unwrap();
    let frame_count = camera_task.await.unwrap();

    assert_eq!(stage1_pos, 20.0);
    assert_eq!(stage2_pos, 15.0);
    assert_eq!(frame_count, 3);
}
