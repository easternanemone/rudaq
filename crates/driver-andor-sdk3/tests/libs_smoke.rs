#![cfg(not(target_arch = "wasm32"))]
//! Andor SDK3 Driver Smoke Tests
//!
//! Tests basic functionality of the Andor iStar camera and Shamrock spectrograph drivers.
//!
//! # Environment Variables
//!
//! Required:
//! - `ANDOR_SMOKE_TEST=1` - Enable hardware tests
//!
//! Optional:
//! - `ANDOR_CAMERA_INDEX` - Camera index to test (default: 0)
//! - `ANDOR_SPECTROGRAPH_INDEX` - Spectrograph index to test (default: 0)
//!
//! # Usage
//!
//! ```bash
//! # Mock mode (no hardware)
//! cargo nextest run -p driver-andor-sdk3
//!
//! # Hardware mode (requires Andor SDK3 installed on Windows)
//! export ANDOR_SMOKE_TEST=1
//! cargo nextest run --profile libs-hardware --features hardware,camera -p driver-andor-sdk3
//! ```

use common::capabilities::{ExposureControl, FrameProducer, Triggerable};
use driver_andor_sdk3::{AndorCamera, AndorSpectrograph};
use std::env;

// =============================================================================
// Test Configuration
// =============================================================================

fn smoke_test_enabled() -> bool {
    env::var("ANDOR_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn camera_index() -> i32 {
    env::var("ANDOR_CAMERA_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn spectrograph_index() -> i32 {
    env::var("ANDOR_SPECTROGRAPH_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Andor smoke test skipped (set ANDOR_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Mock Camera Tests (Always Run)
// =============================================================================

#[tokio::test]
async fn mock_camera_initialization() {
    println!("=== Andor Camera Mock Initialization Test ===");

    // Create mock camera
    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    // Verify resolution
    let (width, height) = camera.resolution();
    println!("  Resolution: {}x{}", width, height);
    assert!(width > 0, "Width should be positive");
    assert!(height > 0, "Height should be positive");

    println!("=== Mock Camera Initialization Test PASSED ===");
}

#[tokio::test]
async fn mock_camera_exposure_control() {
    println!("=== Andor Camera Mock Exposure Control Test ===");

    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    // Set exposure
    camera
        .set_exposure(0.001)
        .await
        .expect("Failed to set exposure");

    // Get exposure
    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    println!("  Exposure: {} seconds", exposure);
    assert!((exposure - 0.001).abs() < 1e-6, "Exposure should be ~1ms");

    println!("=== Mock Exposure Control Test PASSED ===");
}

#[tokio::test]
async fn mock_camera_triggering() {
    println!("=== Andor Camera Mock Triggering Test ===");

    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    // Arm camera
    camera.arm().await.expect("Failed to arm camera");

    // Verify armed state
    let is_armed = camera.is_armed().await.unwrap_or(false);
    assert!(is_armed, "Camera should be armed");

    // Send software trigger
    camera.trigger().await.expect("Failed to trigger camera");

    println!("=== Mock Triggering Test PASSED ===");
}

#[tokio::test]
async fn mock_camera_frame_producer() {
    println!("=== Andor Camera Mock Frame Producer Test ===");

    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    // Start streaming
    camera.start_stream().await.expect("Failed to start stream");

    // Stop streaming
    camera.stop_stream().await.expect("Failed to stop stream");

    println!("=== Mock Frame Producer Test PASSED ===");
}

// =============================================================================
// Mock Spectrograph Tests
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_initialization() {
    println!("=== Andor Spectrograph Mock Initialization Test ===");

    let _spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    // Verify device responds
    println!("  Mock spectrograph created successfully");

    println!("=== Mock Spectrograph Initialization Test PASSED ===");
}

// =============================================================================
// Hardware Camera Tests (Gated by Environment Variable)
// =============================================================================

#[cfg(all(feature = "hardware", feature = "camera"))]
#[tokio::test]
async fn hardware_camera_connection() {
    skip_if_disabled!();

    let index = camera_index();

    println!("=== Andor Camera Hardware Connection Test ===");
    println!("Camera index: {}", index);

    let camera = AndorCamera::new_async(index)
        .await
        .expect("Failed to connect to Andor camera");

    // Get camera info
    let (width, height) = camera.resolution();
    println!("  Resolution: {}x{}", width, height);

    // Get model name
    let info = camera.info();
    println!("  Model: {}", info.model);
    println!("  Serial: {}", info.serial_number);

    println!("=== Hardware Camera Connection Test PASSED ===");
}

#[cfg(all(feature = "hardware", feature = "camera"))]
#[tokio::test]
async fn hardware_camera_exposure() {
    skip_if_disabled!();

    let index = camera_index();

    println!("=== Andor Camera Hardware Exposure Test ===");

    let camera = AndorCamera::new_async(index)
        .await
        .expect("Failed to connect");

    // Set exposure to 1.5ms (typical LIBS integration time)
    camera
        .set_exposure(0.0015)
        .await
        .expect("Failed to set exposure");

    // Verify exposure
    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    println!("  Exposure: {} seconds", exposure);
    assert!(
        (exposure - 0.0015).abs() < 0.0001,
        "Exposure should be ~1.5ms"
    );

    println!("=== Hardware Camera Exposure Test PASSED ===");
}

#[cfg(all(feature = "hardware", feature = "camera"))]
#[tokio::test]
async fn hardware_camera_trigger_config() {
    skip_if_disabled!();

    let index = camera_index();

    println!("=== Andor Camera Hardware Trigger Config Test ===");

    let camera = AndorCamera::new_async(index)
        .await
        .expect("Failed to connect");

    // Set external trigger mode
    camera
        .set_trigger_mode("External")
        .await
        .expect("Failed to set trigger mode");

    // Set gate mode to DDG
    camera
        .set_gate_mode("DDG")
        .await
        .expect("Failed to set gate mode");

    // Set MCP gain (for intensified cameras)
    camera
        .set_mcp_gain(3600)
        .await
        .expect("Failed to set MCP gain");

    // Set DDG timing
    camera
        .set_ddg_output_delay(1300000) // 1.3µs in picoseconds
        .await
        .expect("Failed to set DDG delay");

    camera
        .set_ddg_output_width(10000000) // 10µs in picoseconds
        .await
        .expect("Failed to set DDG width");

    println!("  Trigger configuration set successfully");

    println!("=== Hardware Camera Trigger Config Test PASSED ===");
}

// =============================================================================
// Hardware Spectrograph Tests
// =============================================================================

#[cfg(all(feature = "hardware", feature = "spectrograph"))]
#[tokio::test]
async fn hardware_spectrograph_connection() {
    skip_if_disabled!();

    let index = spectrograph_index();

    println!("=== Andor Spectrograph Hardware Connection Test ===");
    println!("Spectrograph index: {}", index);

    let spectro = AndorSpectrograph::new_async(index)
        .await
        .expect("Failed to connect to spectrograph");

    // Get number of gratings
    let num_gratings = spectro
        .num_gratings()
        .await
        .expect("Failed to get number of gratings");
    println!("  Number of gratings: {}", num_gratings);

    assert!(num_gratings > 0, "Should have at least one grating");

    println!("=== Hardware Spectrograph Connection Test PASSED ===");
}

#[cfg(all(feature = "hardware", feature = "spectrograph"))]
#[tokio::test]
async fn hardware_spectrograph_wavelength() {
    skip_if_disabled!();

    let index = spectrograph_index();

    println!("=== Andor Spectrograph Hardware Wavelength Test ===");

    let spectro = AndorSpectrograph::new_async(index)
        .await
        .expect("Failed to connect");

    // Get current wavelength
    let wavelength = spectro
        .get_wavelength()
        .await
        .expect("Failed to get wavelength");
    println!("  Current wavelength: {} nm", wavelength);

    // Set wavelength (use same value to avoid moving grating)
    spectro
        .set_wavelength(wavelength)
        .await
        .expect("Failed to set wavelength");

    println!("=== Hardware Spectrograph Wavelength Test PASSED ===");
}

// =============================================================================
// Multi-Device Coordination Tests
// =============================================================================

#[cfg(all(feature = "hardware", feature = "camera"))]
#[tokio::test]
async fn hardware_camera_and_trigger_sync() {
    skip_if_disabled!();

    println!("=== Andor Camera + Trigger Synchronization Test ===");
    println!("This test verifies camera can be configured for triggered acquisition");

    let camera = AndorCamera::new_async(camera_index())
        .await
        .expect("Failed to connect");

    // Configure for external triggering (typical LIBS setup)
    camera
        .set_trigger_mode("External")
        .await
        .expect("Failed to set trigger mode");
    camera
        .set_gate_mode("DDG")
        .await
        .expect("Failed to set gate mode");

    // Query dynamic exposure range — limits change with trigger/gate mode
    let (exp_min, exp_max) = camera
        .get_exposure_range()
        .await
        .expect("Failed to get exposure range");
    println!("  Exposure range: {exp_min}..{exp_max} s");

    // Use minimum + 10% headroom (1.5ms may be out of range in External mode)
    let exposure = (exp_min * 1.1).min(exp_max);
    println!("  Using exposure: {exposure} s");
    camera
        .set_exposure(exposure)
        .await
        .expect("Failed to set exposure");

    camera
        .set_mcp_gain(3600)
        .await
        .expect("Failed to set MCP gain");
    camera
        .set_ddg_output_delay(1300000)
        .await
        .expect("Failed to set DDG delay");
    camera
        .set_ddg_output_width(10000000)
        .await
        .expect("Failed to set DDG width");

    // Arm camera
    camera.arm().await.expect("Failed to arm camera");

    // In a real LIBS system, this is where:
    // 1. Dover Motion stage would move to position
    // 2. Dover Motion TOP would trigger camera
    // 3. Camera would gate MCP and acquire spectrum

    println!("  Camera configured and armed for triggered acquisition");
    println!("  Ready for external trigger (e.g., from Dover Motion TOP)");

    println!("=== Camera + Trigger Sync Test PASSED ===");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_exposure() {
    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create camera");

    // Test exposure too small (typically min is ~1µs)
    let result = camera.set_exposure(0.0).await;
    // In mock mode, this should return an error
    assert!(
        result.is_err(),
        "Setting exposure to 0.0 should return an error"
    );
}

#[tokio::test]
async fn test_trigger_without_arm() {
    let camera = AndorCamera::new_mock()
        .await
        .expect("Failed to create camera");

    // Try to trigger without arming
    // Mock camera should handle this gracefully by returning an error
    let result = camera.trigger().await;
    assert!(
        result.is_err(),
        "Triggering without arming should return an error"
    );
}
