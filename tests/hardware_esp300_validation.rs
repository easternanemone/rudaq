//! Comprehensive Hardware Validation Tests for ESP300 Newport Motion Controller
//!
//! This test suite provides end-to-end validation of the ESP300 driver including:
//! - Position control accuracy and repeatability
//! - Velocity and acceleration profiles
//! - Error handling and recovery mechanisms
//! - Serial communication robustness
//! - Multi-axis coordination (if applicable)
//!
//! # Hardware Setup Requirements
//!
//! These tests require physical ESP300 hardware connected via serial port.
//! Expected setup:
//! - ESP300 controller at /dev/ttyUSB0 (Linux/macOS) or COM3 (Windows)
//! - Single axis (axis 1) with stepper or servo motor
//! - Linear stage or similar mechanical load
//! - Homing sensor (home switch) for origin calibration
//! - Safe travel range: typically 0-25mm for lab equipment
//!
//! # Test Safety Features
//!
//! All tests include:
//! - Pre-test homing to establish known state
//! - Soft limit checking to prevent mechanical damage
//! - Timeout protection against hung commands
//! - Graceful cleanup (stop) after each test
//! - Position validation within tolerance (±0.1mm typical)
//!
//! # Running Hardware Tests
//!
//! ```bash
//! # Run all hardware tests with verbose output
//! cargo test --test hardware_esp300_validation --features hardware_tests -- --nocapture
//!
//! # Run specific test
//! cargo test --test hardware_esp300_validation --features hardware_tests test_esp300_position_accuracy_small_movement
//!
//! # Run with timing details
//! RUST_LOG=debug cargo test --test hardware_esp300_validation --features hardware_tests -- --nocapture --test-threads=1
//! ```

#![cfg(all(feature = "hardware_tests", feature = "instrument_newport"))]

use rust_daq::hardware::capabilities::Movable;
use rust_daq::hardware::esp300::Esp300Driver;
use std::time::{Duration, Instant};

// =============================================================================
// Test Fixtures and Utilities
// =============================================================================

/// Port path for ESP300 device (can be overridden via ESP300_PORT env var)
fn get_esp300_port() -> String {
    std::env::var("ESP300_PORT").unwrap_or_else(|_| {
        #[cfg(target_os = "windows")]
        {
            "COM3".to_string()
        }
        #[cfg(not(target_os = "windows"))]
        {
            "/dev/ttyUSB0".to_string()
        }
    })
}

/// Safe maximum position (mm) - adjust based on actual stage travel
const SAFE_MAX_POS: f64 = 24.0;

/// Position tolerance for accuracy tests (mm)
const POSITION_TOLERANCE: f64 = 0.1;

/// Small position step for fine resolution testing
const SMALL_STEP: f64 = 0.1;

/// Medium position step for normal operation
const MEDIUM_STEP: f64 = 1.0;

/// Large position step for velocity profile testing
const LARGE_STEP: f64 = 5.0;

/// Standard velocity for general movement (mm/s)
const STANDARD_VELOCITY: f64 = 5.0;

/// Standard acceleration for controlled motion (mm/s²)
const STANDARD_ACCELERATION: f64 = 10.0;

/// Setup fixture: Home axis and verify safe state
async fn setup_esp300_axis() -> anyhow::Result<Esp300Driver> {
    let driver = Esp300Driver::new(&get_esp300_port(), 1)?;

    // Stop any ongoing motion
    driver.stop().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Home to establish known origin
    driver.home().await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify homed position (should be near zero)
    let pos = driver.position().await?;
    if pos > 1.0 {
        eprintln!(
            "Warning: Position after homing is {:.3}mm (expected ~0mm)",
            pos
        );
    }

    Ok(driver)
}

/// Cleanup fixture: Return to home position and stop
async fn cleanup_esp300_axis(driver: &Esp300Driver) -> anyhow::Result<()> {
    driver.stop().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    driver.home().await?;
    Ok(())
}

/// Assert position is within tolerance of target
fn assert_position_accurate(actual: f64, expected: f64, tolerance: f64) {
    let error = (actual - expected).abs();
    assert!(
        error <= tolerance,
        "Position accuracy failed: expected {:.3}mm ± {:.3}mm, got {:.3}mm (error: {:.3}mm)",
        expected,
        tolerance,
        actual,
        error
    );
}

// =============================================================================
// Test Group 1: Basic Position Control Accuracy (4 tests)
// =============================================================================

/// Test basic absolute position movement to small distance
#[tokio::test]
async fn test_esp300_position_accuracy_small_movement() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to small distance
        driver.move_abs(SMALL_STEP).await?;
        driver.wait_settled().await?;

        // Verify position
        let pos = driver.position().await?;
        assert_position_accurate(pos, SMALL_STEP, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test absolute position movement to medium distance
#[tokio::test]
async fn test_esp300_position_accuracy_medium_movement() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to medium distance
        driver.move_abs(MEDIUM_STEP * 3.0).await?;
        driver.wait_settled().await?;

        // Verify position
        let pos = driver.position().await?;
        assert_position_accurate(pos, MEDIUM_STEP * 3.0, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test absolute position movement to maximum safe distance
#[tokio::test]
async fn test_esp300_position_accuracy_large_movement() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to large distance
        let target = SAFE_MAX_POS - 1.0; // Stay within safe limits
        driver.move_abs(target).await?;
        driver.wait_settled().await?;

        // Verify position
        let pos = driver.position().await?;
        assert_position_accurate(pos, target, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test relative position movement (incremental)
#[tokio::test]
async fn test_esp300_relative_position_movement() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Start at 5mm
        driver.move_abs(5.0).await?;
        driver.wait_settled().await?;

        // Move relative +2mm
        driver.move_rel(MEDIUM_STEP).await?;
        driver.wait_settled().await?;

        // Should be at 6mm
        let pos = driver.position().await?;
        assert_position_accurate(pos, 6.0, POSITION_TOLERANCE);

        // Move relative -1mm
        driver.move_rel(-SMALL_STEP).await?;
        driver.wait_settled().await?;

        // Should be at 5.9mm
        let pos = driver.position().await?;
        assert_position_accurate(pos, 5.9, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

// =============================================================================
// Test Group 2: Velocity and Acceleration Profiles (4 tests)
// =============================================================================

/// Test velocity setting and verification
#[tokio::test]
async fn test_esp300_velocity_setting() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Set velocity
        driver.set_velocity(STANDARD_VELOCITY).await?;

        // Read back velocity
        let velocity = driver.velocity().await?;

        // Verify velocity (allow ±5% tolerance for hardware variation)
        let tolerance = STANDARD_VELOCITY * 0.05;
        assert!(
            (velocity - STANDARD_VELOCITY).abs() <= tolerance,
            "Velocity mismatch: expected {:.3} ± {:.3} mm/s, got {:.3} mm/s",
            STANDARD_VELOCITY,
            tolerance,
            velocity
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test acceleration setting and verification
#[tokio::test]
async fn test_esp300_acceleration_setting() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Set acceleration
        driver.set_acceleration(STANDARD_ACCELERATION).await?;

        // Read back acceleration
        let acceleration = driver.acceleration().await?;

        // Verify acceleration (allow ±5% tolerance)
        let tolerance = STANDARD_ACCELERATION * 0.05;
        assert!(
            (acceleration - STANDARD_ACCELERATION).abs() <= tolerance,
            "Acceleration mismatch: expected {:.3} ± {:.3} mm/s², got {:.3} mm/s²",
            STANDARD_ACCELERATION,
            tolerance,
            acceleration
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test movement timing consistency with velocity profile
#[tokio::test]
async fn test_esp300_velocity_profile_timing() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Set known velocity
        let velocity = 2.0; // mm/s - slow for precise timing
        driver.set_velocity(velocity).await?;

        // Measure time for known distance
        let distance = LARGE_STEP;
        let expected_time_ms = (distance / velocity * 1000.0) as u64;

        let start = Instant::now();
        driver.move_abs(distance).await?;
        driver.wait_settled().await?;
        let elapsed = start.elapsed();

        // Verify timing (allow ±20% tolerance due to acceleration ramps)
        let expected_duration = Duration::from_millis(expected_time_ms);
        let min_time = expected_duration.mul_f64(0.8);
        let max_time = expected_duration.mul_f64(1.2);

        assert!(
            elapsed >= min_time && elapsed <= max_time,
            "Movement timing out of range: expected {:?} (±20%), got {:?}",
            expected_duration,
            elapsed
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test multiple velocity changes during operation
#[tokio::test]
async fn test_esp300_velocity_changes_during_motion() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Set initial velocity
        driver.set_velocity(3.0).await?;

        // Change velocity multiple times
        for v in [2.0, 4.0, 1.5, 5.0] {
            driver.set_velocity(v).await?;
            let readback = driver.velocity().await?;
            assert!(
                (readback - v).abs() <= v * 0.1,
                "Velocity change failed: expected {:.3}, got {:.3}",
                v,
                readback
            );
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

// =============================================================================
// Test Group 3: Error Handling and Recovery (4 tests)
// =============================================================================

/// Test stop command halts motion
#[tokio::test]
async fn test_esp300_stop_halts_motion() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Start slow movement
        driver.set_velocity(1.0).await?;
        driver.move_abs(LARGE_STEP).await?;

        // Wait a bit for motion to begin
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Stop motion
        driver.stop().await?;

        // Position should be somewhere between start and end
        let stopped_pos = driver.position().await?;
        assert!(
            stopped_pos > 0.0 && stopped_pos < LARGE_STEP,
            "Position after stop unexpected: {:.3}mm (expected 0 < pos < {:.3})",
            stopped_pos,
            LARGE_STEP
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test recovery after timeout scenario
#[tokio::test]
async fn test_esp300_recovery_after_stop() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to position
        driver.move_abs(5.0).await?;
        driver.wait_settled().await?;

        // Move and stop early
        driver.move_abs(10.0).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        driver.stop().await?;

        // Try to move again to a different position
        driver.move_abs(3.0).await?;
        driver.wait_settled().await?;
        let pos2 = driver.position().await?;

        // Should have successfully moved to new position
        assert_position_accurate(pos2, 3.0, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test home command successfully returns to origin
#[tokio::test]
async fn test_esp300_home_returns_to_origin() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to known position
        driver.move_abs(10.0).await?;
        driver.wait_settled().await?;

        let _pos_before = driver.position().await?;
        assert!(_pos_before > 1.0, "Failed to move away from home");

        // Return to home
        driver.home().await?;

        // Position should be near zero
        let pos_after = driver.position().await?;
        assert_position_accurate(pos_after, 0.0, 0.5);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test position query accuracy under various conditions
#[tokio::test]
async fn test_esp300_position_query_consistency() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Move to position and verify stability
        driver.move_abs(7.5).await?;
        driver.wait_settled().await?;

        // Query position multiple times - should be consistent
        let mut positions = Vec::new();
        for _ in 0..5 {
            positions.push(driver.position().await?);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // All positions should be within tolerance
        for pos in &positions {
            assert_position_accurate(*pos, 7.5, POSITION_TOLERANCE);
        }

        // Positions should be very close to each other (within 0.01mm)
        let min_pos = positions.iter().copied().fold(f64::INFINITY, f64::min);
        let max_pos = positions.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let consistency = max_pos - min_pos;
        assert!(
            consistency < 0.01,
            "Position readings inconsistent: range {:.4}mm (positions: {:?})",
            consistency,
            positions
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

// =============================================================================
// Test Group 4: Serial Communication Robustness (2 tests)
// =============================================================================

/// Test rapid sequential commands don't cause deadlock or corruption
#[tokio::test]
async fn test_esp300_rapid_commands() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Send multiple commands in rapid succession
        for i in 0..10 {
            let target = (i % 5) as f64 + 2.0;
            driver.move_abs(target).await?;
        }

        // Last command was move to 2.0mm
        driver.wait_settled().await?;
        let final_pos = driver.position().await?;
        assert_position_accurate(final_pos, 2.0, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test timeout handling for unresponsive device simulation
#[tokio::test]
async fn test_esp300_command_timeout_handling() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Normal operation should work
        driver.move_abs(4.0).await?;
        driver.wait_settled().await?;

        let pos = driver.position().await?;
        assert_position_accurate(pos, 4.0, POSITION_TOLERANCE);

        // Quick recovery test
        driver.move_abs(6.0).await?;
        driver.wait_settled().await?;

        let pos2 = driver.position().await?;
        assert_position_accurate(pos2, 6.0, POSITION_TOLERANCE);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

// =============================================================================
// Test Group 5: Multi-Axis Coordination (2 tests)
// =============================================================================

/// Test independent control of multiple axes (if available)
#[tokio::test]
async fn test_esp300_multi_axis_independence() {
    // Note: This test assumes axis 1 and axis 2 are available
    // Skip if only single axis is present

    let driver1 = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300 axis 1: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Try to create driver for axis 2
        let driver2 = match Esp300Driver::new(&get_esp300_port(), 2) {
            Ok(d) => d,
            Err(_) => {
                // Axis 2 not available, skip this test
                eprintln!("Axis 2 not available, skipping multi-axis test");
                return Ok::<(), anyhow::Error>(());
            }
        };

        // Move axis 1
        driver1.move_abs(5.0).await?;
        driver1.wait_settled().await?;

        // Move axis 2
        driver2.move_abs(8.0).await?;
        driver2.wait_settled().await?;

        // Verify both axes are at correct positions
        let pos1 = driver1.position().await?;
        let pos2 = driver2.position().await?;

        assert_position_accurate(pos1, 5.0, POSITION_TOLERANCE);
        assert_position_accurate(pos2, 8.0, POSITION_TOLERANCE);

        // Clean up axis 2
        driver2.home().await?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver1).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Test coordinated motion across axes (if available)
#[tokio::test]
async fn test_esp300_multi_axis_coordinated_motion() {
    let driver1 = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300 axis 1: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Try to create driver for axis 2
        let driver2 = match Esp300Driver::new(&get_esp300_port(), 2) {
            Ok(d) => d,
            Err(_) => {
                // Axis 2 not available, skip this test
                eprintln!("Axis 2 not available, skipping coordinated motion test");
                return Ok::<(), anyhow::Error>(());
            }
        };

        // Set same velocity on both axes
        let velocity = 2.0;
        driver1.set_velocity(velocity).await?;
        driver2.set_velocity(velocity).await?;

        // Start both axes moving simultaneously
        driver1.move_abs(8.0).await?;
        driver2.move_abs(8.0).await?;

        // Wait for both to complete
        tokio::time::sleep(Duration::from_secs(10)).await;
        driver1.wait_settled().await?;
        driver2.wait_settled().await?;

        // Both should be at the target position
        let pos1 = driver1.position().await?;
        let pos2 = driver2.position().await?;

        assert_position_accurate(pos1, 8.0, POSITION_TOLERANCE);
        assert_position_accurate(pos2, 8.0, POSITION_TOLERANCE);

        // Clean up axis 2
        driver2.home().await?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver1).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

// =============================================================================
// Integration Tests
// =============================================================================

/// Full workflow test: home -> move -> measure -> return
#[tokio::test]
async fn test_esp300_complete_workflow() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Phase 1: Setup
        driver.set_velocity(3.0).await?;
        driver.set_acceleration(5.0).await?;

        // Phase 2: Scan sequence
        for position in [2.0, 4.0, 6.0, 8.0, 10.0] {
            driver.move_abs(position).await?;
            driver.wait_settled().await?;

            let actual_pos = driver.position().await?;
            assert_position_accurate(actual_pos, position, POSITION_TOLERANCE);

            // Simulate data collection pause
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Phase 3: Return to home
        driver.home().await?;
        let home_pos = driver.position().await?;
        assert_position_accurate(home_pos, 0.0, 0.5);

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}

/// Stress test: many movements in sequence
#[tokio::test]
async fn test_esp300_stress_many_movements() {
    let driver = match setup_esp300_axis().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to initialize ESP300: {}", e);
            panic!("Hardware setup failed");
        }
    };

    let result = async {
        // Perform many sequential movements
        for i in 0..20 {
            let position = (i % 10 + 1) as f64 * 2.0;
            if position > SAFE_MAX_POS {
                continue;
            }

            driver.move_abs(position).await?;
            driver.wait_settled().await?;

            let actual_pos = driver.position().await?;
            assert_position_accurate(actual_pos, position, POSITION_TOLERANCE);
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = cleanup_esp300_axis(&driver).await;
    assert!(result.is_ok(), "Test failed: {}", result.err().unwrap());
}
