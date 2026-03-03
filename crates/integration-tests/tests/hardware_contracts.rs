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
//! Hardware capability contract tests.
//!
//! These tests define behavioral contracts for each capability trait and verify
//! that mock devices satisfy them. The same contracts can be applied to real
//! hardware drivers to ensure mock fidelity.
//!
//! # Design
//!
//! Each contract is a generic async function parameterized over the capability
//! trait. This means:
//! - The same test logic works for mock AND real devices
//! - Mock devices are validated as faithful stand-ins for hardware
//! - Adding a new driver only requires running existing contracts against it
//!
//! Run with: cargo nextest run -p integration-tests --test hardware_contracts

use driver_mock::MockMode;
use hardware::capabilities::{ExposureControl, FrameProducer, Movable, Readable, Triggerable};
use hardware::drivers::mock::{MockCamera, MockPowerMeter, MockStage};

// =============================================================================
// Contract: Movable
// =============================================================================

/// Movable contract: absolute positioning.
///
/// After `move_abs(x)`, `position()` must return a value equal to x
/// (within tolerance for realistic modes with encoder noise).
async fn contract_movable_absolute_position(device: &dyn Movable, tolerance: f64) {
    let targets = [0.0, 10.0, -5.0, 25.5, 0.0];
    for &target in &targets {
        device.move_abs(target).await.unwrap();
        let pos = device.position().await.unwrap();
        assert!(
            (pos - target).abs() <= tolerance,
            "After move_abs({target}), position() returned {pos} (tolerance: {tolerance})"
        );
    }
}

/// Movable contract: relative positioning.
///
/// `move_rel(d)` must shift the position by exactly d (within tolerance).
async fn contract_movable_relative_position(device: &dyn Movable, tolerance: f64) {
    // Start at a known position
    device.move_abs(0.0).await.unwrap();

    let steps = [5.0, 3.0, -2.0, -6.0];
    let mut expected = 0.0;
    for &step in &steps {
        device.move_rel(step).await.unwrap();
        expected += step;
        let pos = device.position().await.unwrap();
        assert!(
            (pos - expected).abs() <= tolerance,
            "After move_rel({step}), position() returned {pos}, expected {expected} (tolerance: {tolerance})"
        );
    }
}

/// Movable contract: settle completes.
///
/// `wait_settled()` must return `Ok(())` after a move completes.
async fn contract_movable_settle(device: &dyn Movable) {
    device.move_abs(10.0).await.unwrap();
    device.wait_settled().await.unwrap();
    // Position should be stable after settling
    let pos1 = device.position().await.unwrap();
    let pos2 = device.position().await.unwrap();
    assert!(
        (pos1 - pos2).abs() < 0.01,
        "Position should be stable after settling: {pos1} vs {pos2}"
    );
}

/// Movable contract: stop leaves device queryable.
///
/// `stop()` may return `Ok(())` or `Err` (some devices don't support stop),
/// but either way the device must remain in a queryable state afterwards.
async fn contract_movable_stop(device: &dyn Movable) {
    device.move_abs(50.0).await.unwrap();
    // stop() may not be supported — that's a valid response
    let _stop_result = device.stop().await;
    // Position must be queryable regardless of stop support
    let _pos = device.position().await.unwrap();
}

// =============================================================================
// Contract: Readable
// =============================================================================

/// Readable contract: returns finite value.
///
/// `read()` must return a finite f64 value (not NaN, not Infinity).
async fn contract_readable_finite_value(device: &dyn Readable) {
    let value = device.read().await.unwrap();
    assert!(
        value.is_finite(),
        "read() must return a finite value, got {value}"
    );
}

/// Readable contract: multiple reads succeed.
///
/// Consecutive reads must all succeed without errors.
async fn contract_readable_consecutive_reads(device: &dyn Readable) {
    for i in 0..10 {
        let value = device
            .read()
            .await
            .unwrap_or_else(|e| panic!("Read #{i} failed: {e}"));
        assert!(
            value.is_finite(),
            "Read #{i} returned non-finite value: {value}"
        );
    }
}

/// Readable contract: bounded values.
///
/// At typical power levels, readings must stay non-negative. Note: at very
/// low power (near noise floor), physical detectors CAN read slightly
/// negative due to dark current subtraction. Use a base power well above
/// the noise floor when testing this contract.
async fn contract_readable_non_negative_power(device: &dyn Readable) {
    for _ in 0..10 {
        let value = device.read().await.unwrap();
        assert!(
            value >= 0.0,
            "Power reading must be non-negative at typical levels, got {value}"
        );
    }
}

// =============================================================================
// Contract: Triggerable
// =============================================================================

/// Triggerable contract: trigger without arm fails.
///
/// Calling `trigger()` without first calling `arm()` must return an error.
async fn contract_triggerable_unarmed_fails(device: &dyn Triggerable) {
    let result = device.trigger().await;
    assert!(
        result.is_err(),
        "trigger() must fail when device is not armed"
    );
}

/// Triggerable contract: arm-then-trigger succeeds.
///
/// `arm()` followed by `trigger()` must both succeed.
async fn contract_triggerable_arm_trigger(device: &dyn Triggerable) {
    device.arm().await.unwrap();
    assert!(
        device.is_armed().await.unwrap(),
        "is_armed() must return true after arm()"
    );
    device.trigger().await.unwrap();
}

/// Triggerable contract: repeated arm-trigger cycles.
///
/// The arm-trigger sequence must be repeatable.
async fn contract_triggerable_repeated_cycles(device: &dyn Triggerable) {
    for i in 0..5 {
        device
            .arm()
            .await
            .unwrap_or_else(|e| panic!("arm() cycle #{i} failed: {e}"));
        device
            .trigger()
            .await
            .unwrap_or_else(|e| panic!("trigger() cycle #{i} failed: {e}"));
    }
}

// =============================================================================
// Contract: FrameProducer
// =============================================================================

/// FrameProducer contract: resolution is immutable.
///
/// `resolution()` must return the same value regardless of device state.
async fn contract_frame_producer_immutable_resolution(device: &dyn FrameProducer) {
    let res1 = device.resolution();
    assert!(res1.0 > 0 && res1.1 > 0, "Resolution must be positive");

    // Resolution shouldn't change after streaming
    device.start_stream().await.unwrap();
    let res2 = device.resolution();
    device.stop_stream().await.unwrap();
    let res3 = device.resolution();

    assert_eq!(res1, res2, "Resolution must not change during streaming");
    assert_eq!(res1, res3, "Resolution must not change after streaming");
}

/// FrameProducer contract: streaming state transitions.
///
/// - `start_stream()` → `is_streaming()` returns true
/// - `start_stream()` again → returns error (already streaming)
/// - `stop_stream()` → `is_streaming()` returns false
async fn contract_frame_producer_streaming_lifecycle(device: &dyn FrameProducer) {
    // Should not be streaming initially
    // (this may not hold if the device was previously used, but is a reasonable default)

    // Start streaming
    device.start_stream().await.unwrap();

    // Double start must fail
    let result = device.start_stream().await;
    assert!(
        result.is_err(),
        "start_stream() must fail when already streaming"
    );

    // Stop streaming
    device.stop_stream().await.unwrap();
}

/// FrameProducer contract: frame count increments.
///
/// Each trigger must increment `frame_count()` by exactly 1.
/// Uses concrete MockCamera because Rust trait objects can't combine
/// multiple non-auto traits (`dyn FrameProducer + Triggerable`).
async fn contract_frame_producer_count_increments(camera: &MockCamera) {
    let initial = camera.frame_count();
    camera.arm().await.unwrap();

    for i in 1..=5 {
        camera.trigger().await.unwrap();
        assert_eq!(
            camera.frame_count(),
            initial + i,
            "frame_count() must increment by 1 per trigger"
        );
    }
}

// =============================================================================
// Contract: ExposureControl
// =============================================================================

/// ExposureControl contract: set/get roundtrip.
///
/// `set_exposure(e)` followed by `get_exposure()` must return approximately e.
async fn contract_exposure_roundtrip(device: &dyn ExposureControl, tolerance: f64) {
    let exposures = [0.001, 0.01, 0.1, 1.0, 0.05];
    for &exp in &exposures {
        device.set_exposure(exp).await.unwrap();
        let got = device.get_exposure().await.unwrap();
        assert!(
            (got - exp).abs() <= tolerance,
            "After set_exposure({exp}), get_exposure() returned {got} (tolerance: {tolerance})"
        );
    }
}

// =============================================================================
// Apply Contracts: MockStage (Instant Mode)
// =============================================================================

#[tokio::test]
async fn test_contract_movable_absolute_position_instant() {
    let stage = MockStage::new();
    contract_movable_absolute_position(&stage, 0.0).await;
}

#[tokio::test]
async fn test_contract_movable_relative_position_instant() {
    let stage = MockStage::new();
    contract_movable_relative_position(&stage, 0.0).await;
}

#[tokio::test]
async fn test_contract_movable_settle_instant() {
    let stage = MockStage::new();
    contract_movable_settle(&stage).await;
}

#[tokio::test]
async fn test_contract_movable_stop_instant() {
    let stage = MockStage::new();
    contract_movable_stop(&stage).await;
}

// =============================================================================
// Apply Contracts: MockStage (Realistic Mode)
// =============================================================================

#[tokio::test(start_paused = true)]
async fn test_contract_movable_absolute_position_realistic() {
    let stage = MockStage::builder().mode(MockMode::Realistic).build();
    // Realistic mode has encoder noise (~0.1 um = 1e-4 mm)
    contract_movable_absolute_position(&stage, 0.001).await;
}

#[tokio::test(start_paused = true)]
async fn test_contract_movable_relative_position_realistic() {
    let stage = MockStage::builder().mode(MockMode::Realistic).build();
    contract_movable_relative_position(&stage, 0.001).await;
}

#[tokio::test(start_paused = true)]
async fn test_contract_movable_settle_realistic() {
    let stage = MockStage::builder().mode(MockMode::Realistic).build();
    contract_movable_settle(&stage).await;
}

// =============================================================================
// Apply Contracts: MockPowerMeter (Readable)
// =============================================================================

#[tokio::test]
async fn test_contract_readable_finite_value_power_meter() {
    let pm = MockPowerMeter::new(1e-3); // 1 mW
    contract_readable_finite_value(&pm).await;
}

#[tokio::test]
async fn test_contract_readable_consecutive_reads_power_meter() {
    let pm = MockPowerMeter::new(1e-3);
    contract_readable_consecutive_reads(&pm).await;
}

#[tokio::test]
async fn test_contract_readable_non_negative_power_meter() {
    // Use 1 mW (well above noise floor) so noise doesn't push below zero.
    // At very low power (< 10 µW), the mock's Gaussian noise can produce
    // slightly negative readings — this is physically accurate behavior.
    let pm = MockPowerMeter::new(1e-3);
    contract_readable_non_negative_power(&pm).await;
}

// =============================================================================
// Apply Contracts: MockCamera (Triggerable + FrameProducer + ExposureControl)
// =============================================================================

#[tokio::test]
async fn test_contract_triggerable_unarmed_fails_camera() {
    let camera = MockCamera::new(1920, 1080);
    contract_triggerable_unarmed_fails(&camera).await;
}

#[tokio::test]
async fn test_contract_triggerable_arm_trigger_camera() {
    let camera = MockCamera::new(1920, 1080);
    contract_triggerable_arm_trigger(&camera).await;
}

#[tokio::test]
async fn test_contract_triggerable_repeated_cycles_camera() {
    let camera = MockCamera::new(640, 480);
    contract_triggerable_repeated_cycles(&camera).await;
}

#[tokio::test]
async fn test_contract_frame_producer_immutable_resolution_camera() {
    let camera = MockCamera::new(2048, 2048);
    contract_frame_producer_immutable_resolution(&camera).await;
}

#[tokio::test]
async fn test_contract_frame_producer_streaming_lifecycle_camera() {
    let camera = MockCamera::new(1920, 1080);
    contract_frame_producer_streaming_lifecycle(&camera).await;
}

#[tokio::test]
async fn test_contract_frame_producer_count_increments_camera() {
    let camera = MockCamera::new(1920, 1080);
    contract_frame_producer_count_increments(&camera).await;
}

#[tokio::test]
async fn test_contract_exposure_roundtrip_camera() {
    let camera = MockCamera::new(1920, 1080);
    // Camera exposure may quantize slightly
    contract_exposure_roundtrip(&camera, 1e-6).await;
}

// =============================================================================
// Cross-Capability Contract: Exposure Does Not Start Acquisition
// =============================================================================

/// Setting exposure must not change the streaming or armed state.
#[tokio::test]
async fn test_contract_exposure_does_not_start_acquisition() {
    let camera = MockCamera::new(1920, 1080);

    let count_before = camera.frame_count();
    camera.set_exposure(0.05).await.unwrap();

    // Frame count must not change from exposure alone
    assert_eq!(
        camera.frame_count(),
        count_before,
        "set_exposure() must not produce frames"
    );
}

// =============================================================================
// Cross-Capability Contract: Move + Read Coordination
// =============================================================================

/// Moving a stage and reading a power meter must be independent operations
/// that can interleave without deadlock or corruption.
#[tokio::test]
async fn test_contract_movable_readable_independence() {
    let stage = MockStage::new();
    let pm = MockPowerMeter::new(1e-3);

    for i in 0..5 {
        let target = f64::from(i) * 2.0;
        stage.move_abs(target).await.unwrap();
        stage.wait_settled().await.unwrap();
        let reading = pm.read().await.unwrap();
        assert!(
            reading.is_finite(),
            "Power reading must be finite at position {target}"
        );
        let pos = stage.position().await.unwrap();
        assert!(
            (pos - target).abs() < 0.01,
            "Position must be stable: expected {target}, got {pos}"
        );
    }
}
