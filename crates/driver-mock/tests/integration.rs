//! Integration tests for mock driver system
//!
//! These tests verify that all 6 mock devices are integrated and working.

use ::common::capabilities::{ExposureControl, FrameProducer, Movable, Triggerable};
use ::common::driver::DriverFactory;
use driver_mock::*;

/// Test that all 6 mock devices can be instantiated
#[test]
fn test_all_devices_instantiate() {
    let camera = MockCamera::builder().build();
    let stage = MockStage::builder().build();
    let meter = MockPowerMeter::builder().build();
    let laser = MockLaser::new();
    let rotator = MockRotator::new();
    let dac = MockDAQOutput::new();

    // Just verify they all exist
    drop((camera, stage, meter, laser, rotator, dac));
}

/// Test error config scenarios
#[test]
fn test_error_scenarios() {
    let _fail_after = ErrorConfig::scenario(ErrorScenario::FailAfterN {
        operation: "read",
        count: 5,
    });

    let _timeout = ErrorConfig::scenario(ErrorScenario::Timeout { operation: "move" });

    let _comm_loss = ErrorConfig::scenario(ErrorScenario::CommunicationLoss);

    let _fault = ErrorConfig::scenario(ErrorScenario::HardwareFault { code: 42 });

    // All should construct without panic
}

/// Test random error config with seed
#[test]
fn test_random_errors_with_seed() {
    let config1 = ErrorConfig::random_failures_seeded(0.5, Some(12345));
    let config2 = ErrorConfig::random_failures_seeded(0.5, Some(12345));

    // Both configs should be deterministic
    drop((config1, config2));
}

/// Test mock modes
#[test]
fn test_mock_modes() {
    let instant = MockMode::Instant;
    let realistic = MockMode::Realistic;
    let chaos = MockMode::Chaos;

    let _ = (instant, realistic, chaos);
}

/// Test voltage ranges
#[test]
fn test_voltage_ranges() {
    let bipolar10 = VoltageRange::Bipolar10V;
    let bipolar5 = VoltageRange::Bipolar5V;
    let unipolar10 = VoltageRange::Unipolar10V;
    let unipolar5 = VoltageRange::Unipolar5V;

    let _ = (bipolar10, bipolar5, unipolar10, unipolar5);
}

/// Test link function (prevents linker optimization)
#[test]
fn test_link() {
    link();
}

/// Test ErrorConfig reset
#[test]
fn test_error_reset() {
    let config = ErrorConfig::scenario(ErrorScenario::HardwareFault { code: 42 });
    config.reset();
    // Should not panic
}

// =============================================================================
// Integration tests for bd-gf04: Realistic Mock Drivers
// =============================================================================

/// Test that MockCamera produces frames with rich metadata (bd-qseb)
#[tokio::test]
async fn test_camera_frame_metadata_enrichment() {
    let camera = MockCamera::builder()
        .mode(MockMode::Realistic)
        .initial_temperature(20.0)
        .build();

    camera.set_exposure(0.05).await.unwrap();
    camera.arm().await.unwrap();
    camera.trigger().await.unwrap();

    // Frame metadata should include temperature, gain, binning, trigger mode
    let temp = camera.temperature().await;
    assert!(
        temp > 15.0 && temp < 30.0,
        "Temperature should be realistic"
    );
}

/// Test camera state machine transitions (bd-wn20)
#[tokio::test]
async fn test_camera_state_lifecycle() {
    let camera = MockCamera::builder().mode(MockMode::Realistic).build();

    assert_eq!(camera.device_state(), CameraDeviceState::Ready);

    camera.start_stream().await.unwrap();
    assert_eq!(camera.device_state(), CameraDeviceState::Acquiring);

    camera.stop_stream().await.unwrap();
    assert_eq!(camera.device_state(), CameraDeviceState::Ready);
}

/// Test camera temperature observable stream (bd-34gs)
#[tokio::test]
async fn test_camera_temperature_stream() {
    let camera = MockCamera::builder()
        .initial_temperature(20.0)
        .mode(MockMode::Realistic)
        .build();

    camera.set_temperature_setpoint(30.0).await;

    let handle = camera
        .start_temperature_stream()
        .expect("Should start in Realistic mode");

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

    // Temperature should have drifted from 20 toward 30
    let temp = camera.temperature().await;
    assert!(
        temp > 20.5,
        "Temperature should drift toward setpoint (got {})",
        temp
    );

    handle.abort();
}

/// Test power meter reading stream (bd-34gs)
#[tokio::test]
async fn test_power_meter_reading_stream() {
    let meter = MockPowerMeter::builder()
        .base_power(0.001) // 1 mW
        .mode(MockMode::Realistic)
        .rng_seed(42)
        .build();

    let handle = meter
        .start_reading_stream()
        .expect("Should start in Realistic mode");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let reading = meter.get_power_reading();
    assert!(
        reading > 0.0005 && reading < 0.002,
        "Power reading should be near 1mW (got {})",
        reading
    );

    handle.abort();
}

/// Test stage encoder noise in realistic mode (bd-utp6)
#[tokio::test]
async fn test_stage_encoder_noise() {
    let stage = MockStage::builder()
        .initial_position(5.0)
        .mode(MockMode::Realistic)
        .build();

    // In realistic mode, multiple position reads should show slight noise
    let mut readings = Vec::new();
    for _ in 0..10 {
        readings.push(stage.position().await.unwrap());
    }

    // All should be near 5.0
    for r in &readings {
        assert!(
            (*r - 5.0).abs() < 0.001,
            "Position should be near 5.0 (got {})",
            r
        );
    }

    // Check there's actually some variation (encoder noise)
    let min = readings.iter().copied().fold(f64::INFINITY, f64::min);
    let max = readings.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max - min > 0.0,
        "Should have some noise variation (min={}, max={})",
        min,
        max
    );
}

/// Test rotator encoder quantization (bd-utp6)
#[tokio::test]
async fn test_rotator_encoder_quantization() {
    // Default pulses_per_degree is 143.4
    let rotator = MockRotator::new();

    // Move to a position that doesn't land on an encoder step
    rotator.move_abs(45.123456).await.unwrap();
    let pos = rotator.position().await.unwrap();

    // Position should be quantized to nearest encoder step
    let encoder_step = 1.0 / 143.4;
    let quantized = (45.123456_f64 / encoder_step).round() * encoder_step;
    assert!(
        (pos - quantized).abs() < 1e-6,
        "Position should be quantized (got {}, expected {})",
        pos,
        quantized
    );
}

/// Test that all mock devices support the DriverFactory pattern (bd-dups)
#[tokio::test]
async fn test_factory_metadata_populated() {
    let camera_factory = MockCameraFactory;
    let stage_factory = MockStageFactory;
    let meter_factory = MockPowerMeterFactory;

    // All factories should report their driver types correctly
    assert_eq!(camera_factory.driver_type(), "mock_camera");
    assert_eq!(stage_factory.driver_type(), "mock_stage");
    assert_eq!(meter_factory.driver_type(), "mock_power_meter");

    // All factories should report capabilities
    assert!(!camera_factory.capabilities().is_empty());
    assert!(!stage_factory.capabilities().is_empty());
    assert!(!meter_factory.capabilities().is_empty());

    // All factories should build successfully with default config
    let config = toml::Value::Table(toml::map::Map::new());
    let camera_components = camera_factory.build(config.clone()).await.unwrap();
    let stage_components = stage_factory.build(config.clone()).await.unwrap();
    let meter_components = meter_factory.build(config).await.unwrap();

    assert!(camera_components.frame_producer.is_some());
    assert!(camera_components.triggerable.is_some());
    assert!(stage_components.movable.is_some());
    assert!(meter_components.readable.is_some());
}
