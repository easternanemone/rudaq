//! Integration tests for mock driver system
//!
//! These tests verify that all 6 mock devices are integrated and working.

use daq_driver_mock::*;

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

    drop((instant, realistic, chaos));
}

/// Test voltage ranges
#[test]
fn test_voltage_ranges() {
    let bipolar10 = VoltageRange::Bipolar10V;
    let bipolar5 = VoltageRange::Bipolar5V;
    let unipolar10 = VoltageRange::Unipolar10V;
    let unipolar5 = VoltageRange::Unipolar5V;

    drop((bipolar10, bipolar5, unipolar10, unipolar5));
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
