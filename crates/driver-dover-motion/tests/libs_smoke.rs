#![cfg(not(target_arch = "wasm32"))]
//! Dover Motion Driver Smoke Tests
//!
//! Tests basic functionality of the Dover Motion axis driver.
//!
//! # Environment Variables
//!
//! Required:
//! - `DOVER_MOTION_SMOKE_TEST=1` - Enable hardware tests
//!
//! Optional:
//! - `DOVER_CONFIG_PATH` - Path to Dover Motion device config (default: simulated)
//! - `DOVER_AXIS_NAME` - Axis to test (default: "X")
//!
//! # Usage
//!
//! ```bash
//! # Mock mode (no hardware)
//! cargo nextest run -p driver-dover-motion
//!
//! # Hardware mode
//! export DOVER_MOTION_SMOKE_TEST=1
//! export DOVER_CONFIG_PATH="C:\\ProgramData\\Dover Motion\\SmartStage.xml"
//! cargo nextest run --profile libs-hardware --features hardware -p driver-dover-motion
//! ```

use common::capabilities::{Movable, Parameterized, TriggerOnPosition};
use driver_dover_motion::DoverAxisDriver;
use std::env;

// =============================================================================
// Test Configuration
// =============================================================================

fn smoke_test_enabled() -> bool {
    env::var("DOVER_MOTION_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn config_path() -> String {
    env::var("DOVER_CONFIG_PATH").unwrap_or_else(|_| "mock://smartstage".to_string())
}

fn axis_name() -> String {
    env::var("DOVER_AXIS_NAME").unwrap_or_else(|_| "X".to_string())
}

macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Dover Motion smoke test skipped (set DOVER_MOTION_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Mock Mode Tests (Always Run)
// =============================================================================

#[tokio::test]
async fn mock_driver_initialization() {
    println!("=== Dover Motion Mock Initialization Test ===");

    // This test always runs with mock hardware
    let driver = DoverAxisDriver::new_async("mock://device", "X", "USB")
        .await
        .expect("Failed to create mock driver");

    // Verify parameters are accessible
    let params = driver.parameters();
    assert!(
        params.get("position").is_some(),
        "Position parameter should exist"
    );

    println!("=== Mock Initialization Test PASSED ===");
}

#[tokio::test]
async fn mock_basic_motion() {
    println!("=== Dover Motion Mock Basic Motion Test ===");

    let driver = DoverAxisDriver::new_async("mock://device", "X", "USB")
        .await
        .expect("Failed to create mock driver");

    // Test absolute move
    driver
        .move_abs(10.0)
        .await
        .expect("Failed to move absolute");

    // Test relative move
    driver.move_rel(5.0).await.expect("Failed to move relative");

    // Test position query
    let pos = driver.position().await.expect("Failed to get position");
    println!("  Current position: {} mm", pos);

    // Test stop
    driver.stop().await.expect("Failed to stop");

    println!("=== Mock Basic Motion Test PASSED ===");
}

#[tokio::test]
async fn mock_trigger_on_position() {
    println!("=== Dover Motion Mock TOP Test ===");

    let driver = DoverAxisDriver::new_async("mock://device", "X", "USB")
        .await
        .expect("Failed to create mock driver");

    // Test TOP enable
    driver
        .enable_top(
            0.0,   // start_position
            100.0, // end_position
            1.0,   // increment
            false, // bidirectional
            1000,  // pulse_width_ns
        )
        .await
        .expect("Failed to enable TOP");

    // Verify TOP is enabled
    let enabled = driver
        .is_top_enabled()
        .await
        .expect("Failed to query TOP state");
    assert!(enabled, "TOP should be enabled");

    // Test TOP disable
    driver.disable_top().await.expect("Failed to disable TOP");

    let disabled = driver
        .is_top_enabled()
        .await
        .expect("Failed to query TOP state");
    assert!(!disabled, "TOP should be disabled");

    println!("=== Mock TOP Test PASSED ===");
}

// =============================================================================
// Hardware Tests (Gated by Environment Variable)
// =============================================================================

#[cfg(feature = "hardware")]
#[tokio::test]
async fn hardware_device_connection() {
    skip_if_disabled!();

    let path = config_path();
    let axis = axis_name();

    println!("=== Dover Motion Hardware Connection Test ===");
    println!("Config path: {}", path);
    println!("Axis: {}", axis);

    let driver = DoverAxisDriver::new_async(&path, &axis, "USB")
        .await
        .expect("Failed to connect to Dover Motion device");

    // Verify we can read position
    let pos = driver.position().await.expect("Failed to get position");
    println!("  Current position: {} mm", pos);

    println!("=== Hardware Connection Test PASSED ===");
}

#[cfg(feature = "hardware")]
#[tokio::test]
async fn hardware_small_move() {
    skip_if_disabled!();

    let path = config_path();
    let axis = axis_name();

    println!("=== Dover Motion Hardware Small Move Test ===");

    let driver = DoverAxisDriver::new_async(&path, &axis, "USB")
        .await
        .expect("Failed to connect");

    // Get current position
    let start_pos = driver
        .position()
        .await
        .expect("Failed to get start position");
    println!("  Start position: {} mm", start_pos);

    // Make a small relative move (0.1mm)
    driver.move_rel(0.1).await.expect("Failed to move");

    // Wait for motion to settle
    driver
        .wait_settled()
        .await
        .expect("Failed to wait for settle");

    // Check final position
    let end_pos = driver.position().await.expect("Failed to get end position");
    println!("  End position: {} mm", end_pos);

    // Verify motion occurred (within tolerance)
    let delta = (end_pos - start_pos - 0.1).abs();
    assert!(
        delta < 0.01,
        "Position change should be ~0.1mm, got delta = {}",
        delta
    );

    // Return to start position
    driver
        .move_abs(start_pos)
        .await
        .expect("Failed to return to start");
    driver
        .wait_settled()
        .await
        .expect("Failed to wait for return");

    println!("=== Hardware Small Move Test PASSED ===");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_top_parameters() {
    let driver = DoverAxisDriver::new_async("mock://device", "X", "USB")
        .await
        .expect("Failed to create driver");

    // Test invalid increment (must be positive)
    let result = driver.enable_top(0.0, 100.0, -1.0, false, 1000).await;
    assert!(result.is_err(), "Negative increment should fail");

    // Test invalid pulse width (must be 50-204,800 ns)
    let result = driver.enable_top(0.0, 100.0, 1.0, false, 25).await;
    assert!(result.is_err(), "Pulse width < 50ns should fail");

    // Test invalid pulse width (must be multiple of 50ns)
    let result = driver.enable_top(0.0, 100.0, 1.0, false, 75).await;
    assert!(
        result.is_err(),
        "Pulse width not multiple of 50ns should fail"
    );
}
