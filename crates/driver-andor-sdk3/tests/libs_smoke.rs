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
use driver_andor_sdk3::{AndorCamera, AndorSpectrograph, FilterPosition, SlitPort};

// =============================================================================
// Test Configuration (used by feature-gated hardware tests)
// =============================================================================

#[cfg(feature = "hardware")]
fn smoke_test_enabled() -> bool {
    std::env::var("ANDOR_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

#[cfg(all(feature = "hardware", feature = "camera"))]
fn camera_index() -> i32 {
    std::env::var("ANDOR_CAMERA_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[cfg(all(feature = "hardware", feature = "spectrograph"))]
fn spectrograph_index() -> i32 {
    std::env::var("ANDOR_SPECTROGRAPH_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[cfg(feature = "hardware")]
macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Andor smoke test skipped (set ANDOR_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

/// Try setting trigger mode from a list of preferred modes.
///
/// Not all camera models (or the SIMCAM) support all trigger mode strings.
/// This helper tries each mode in priority order and returns the first that
/// succeeds, or `None` if all fail.
#[cfg(all(feature = "hardware", feature = "camera"))]
async fn try_set_trigger_mode(camera: &AndorCamera, modes: &[&str]) -> Option<String> {
    for &mode in modes {
        match camera.set_trigger_mode(mode).await {
            Ok(()) => return Some(mode.to_string()),
            Err(e) => {
                println!("  Trigger mode '{}' not supported: {}", mode, e);
            }
        }
    }
    None
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

/// Mock triggering test — only runs without the `camera` feature.
///
/// When `camera` is enabled, `new_mock()` opens real hardware (index 0) and
/// `trigger()` calls `AT_Command("SoftwareTrigger")`, which requires the camera
/// to be in Software trigger mode and streaming. The hardware trigger tests
/// below cover the real-SDK path.
#[cfg(not(feature = "camera"))]
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
// Mock Spectrograph — Wavelength Limits [bd-p1zz]
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_wavelength_limits() {
    let spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    let limits = spectro
        .get_wavelength_limits(1)
        .await
        .expect("Failed to get wavelength limits");

    assert!(limits.min_nm < limits.max_nm, "min should be less than max");
    assert!(limits.min_nm >= 0.0, "min should be non-negative");
}

// =============================================================================
// Mock Spectrograph — Detector Offset [bd-1rfx]
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_detector_offset() {
    let spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    let offset = spectro
        .get_detector_offset()
        .await
        .expect("Failed to get detector offset");
    assert_eq!(offset, 0, "Default mock detector offset should be 0");

    spectro
        .set_detector_offset(5)
        .await
        .expect("Failed to set detector offset");
}

// =============================================================================
// Mock Spectrograph — Filter Wheel [bd-8n55]
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_filter_wheel() {
    let spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    let present = spectro
        .filter_is_present()
        .await
        .expect("Failed to check filter presence");
    assert!(present, "Mock should report filter wheel present");

    let filter = spectro
        .get_filter()
        .await
        .expect("Failed to get filter position");
    assert_eq!(filter.0, 1, "Default mock filter position should be 1");

    spectro
        .set_filter(FilterPosition(3))
        .await
        .expect("Failed to set filter position");

    let info = spectro
        .get_filter_info(FilterPosition(1))
        .await
        .expect("Failed to get filter info");
    assert!(!info.is_empty(), "Filter info should not be empty");
}

// =============================================================================
// Mock Spectrograph — Focus Mirror [bd-2in1]
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_focus_mirror() {
    let spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    let present = spectro
        .focus_mirror_is_present()
        .await
        .expect("Failed to check focus mirror presence");
    assert!(present, "Mock should report focus mirror present");

    let focus = spectro
        .get_focus_mirror()
        .await
        .expect("Failed to get focus mirror position");
    assert_eq!(focus, 0, "Default mock focus mirror should be 0");

    let max_steps = spectro
        .get_focus_mirror_max_steps()
        .await
        .expect("Failed to get focus mirror max steps");
    assert!(max_steps > 0, "Max steps should be positive");

    spectro
        .set_focus_mirror(100)
        .await
        .expect("Failed to set focus mirror");
}

// =============================================================================
// Mock Spectrograph — Multi-Port Slit Management [bd-2qd1]
// =============================================================================

#[tokio::test]
async fn mock_spectrograph_multi_port_slit() {
    let spectro = AndorSpectrograph::new_mock()
        .await
        .expect("Failed to create mock spectrograph");

    // Check presence of all slit ports
    for port in [
        SlitPort::InputSide,
        SlitPort::InputDirect,
        SlitPort::OutputSide,
        SlitPort::OutputDirect,
    ] {
        let present = spectro
            .auto_slit_is_present(port)
            .await
            .expect("Failed to check slit presence");
        assert!(present, "Mock should report slit port {:?} present", port);
    }

    // Set and get slit width on a specific port
    spectro
        .set_slit_width_port(SlitPort::InputDirect, 200.0)
        .await
        .expect("Failed to set slit width");

    let width = spectro
        .get_slit_width_port(SlitPort::InputDirect)
        .await
        .expect("Failed to get slit width");
    assert!(width > 0.0, "Slit width should be positive");

    // Reset slit
    spectro
        .reset_slit(SlitPort::InputDirect)
        .await
        .expect("Failed to reset slit");
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

    println!("  Model: {}", camera.info().model);

    // Not all camera models support all trigger mode strings — e.g., the
    // SIMCAM doesn't implement "External". Try modes in priority order.
    let trigger_mode = try_set_trigger_mode(&camera, &["External", "Software", "Internal"]).await;
    match &trigger_mode {
        Some(mode) => println!("  Trigger mode set to: {}", mode),
        None => {
            println!("  No recognized trigger modes available, skipping");
            println!("=== Hardware Camera Trigger Config Test SKIPPED ===");
            return;
        }
    };

    // iStar-specific features — only test if the camera supports them.
    let features = camera.info().features.clone();

    if features.gate_mode {
        camera
            .set_gate_mode("DDG")
            .await
            .expect("Failed to set gate mode");
        println!("  Gate mode set to DDG");
    }

    if features.mcp_gain {
        camera
            .set_mcp_gain(3600)
            .await
            .expect("Failed to set MCP gain");
        println!("  MCP gain set to 3600");
    }

    if features.ddg_output_delay {
        camera
            .set_ddg_output_delay(1300000) // 1.3µs in picoseconds
            .await
            .expect("Failed to set DDG delay");
        println!("  DDG delay set to 1.3µs");
    }

    if features.ddg_output_width {
        camera
            .set_ddg_output_width(10000000) // 10µs in picoseconds
            .await
            .expect("Failed to set DDG width");
        println!("  DDG width set to 10µs");
    }

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

    println!("  Model: {}", camera.info().model);

    let features = camera.info().features.clone();

    // Configure for external triggering (typical LIBS setup) if available,
    // otherwise fall back to Software trigger for basic trigger-sync validation.
    let trigger_mode = try_set_trigger_mode(&camera, &["External", "Software"]).await;
    match &trigger_mode {
        Some(mode) => println!("  Trigger mode set to: {}", mode),
        None => {
            println!("  No external/software trigger modes available, skipping");
            println!("=== Camera + Trigger Sync Test SKIPPED ===");
            return;
        }
    };

    if features.gate_mode {
        camera
            .set_gate_mode("DDG")
            .await
            .expect("Failed to set gate mode");
    }

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

    if features.mcp_gain {
        camera
            .set_mcp_gain(3600)
            .await
            .expect("Failed to set MCP gain");
    }
    if features.ddg_output_delay {
        camera
            .set_ddg_output_delay(1300000)
            .await
            .expect("Failed to set DDG delay");
    }
    if features.ddg_output_width {
        camera
            .set_ddg_output_width(10000000)
            .await
            .expect("Failed to set DDG width");
    }

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
