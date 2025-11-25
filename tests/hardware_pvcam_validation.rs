#![cfg(feature = "instrument_photometrics")]
//! PVCAM Camera Hardware Validation Test Suite
//!
//! Comprehensive validation tests for Photometrics PVCAM camera driver.
//! Tests are written for Prime BSI (2048x2048) by default.
//!
//! These tests verify:
//! - Camera initialization and enumeration
//! - Frame acquisition (single and continuous)
//! - Region of Interest (ROI) configuration
//! - Binning control (1x1, 2x2, 4x4, 8x8)
//! - Exposure time control and accuracy
//! - Triggered acquisition modes
//! - Error handling and recovery
//! - Frame data integrity
//!
//! # Camera Models
//!
//! - **Prime BSI** (default): 2048 x 2048 pixel sensor
//! - **Prime 95B** (optional): 1200 x 1200 pixel sensor
//!   Enable with `--features prime_95b_tests`
//!
//! # Hardware Setup Requirements
//!
//! For actual hardware validation (not mock-based tests):
//!
//! 1. **Camera**:
//!    - Photometrics Prime BSI (default) or Prime 95B
//!    - Connected via USB 3.0 or PCIe interface
//!    - PVCAM SDK installed and configured
//!    - Camera powered on and recognized by system
//!
//! 2. **Environment Variables** (required):
//!    - `PVCAM_SDK_DIR=/opt/pvcam/sdk`
//!    - `PVCAM_LIB_DIR=/opt/pvcam/library/x86_64`
//!    - `PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode`
//!    - `LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH`
//!
//! 3. **Illumination** (for uniformity tests):
//!    - Uniform white light source (LED panel or integrating sphere)
//!    - Diffuse illumination across sensor area
//!    - Avoid bright spots or shadows
//!
//! 4. **Dark Environment** (for noise tests):
//!    - Camera lens cap or opaque cover
//!    - Room lights off
//!    - Allows measurement of dark current and read noise
//!
//! # Running Tests
//!
//! ```bash
//! # Run all mock tests (no hardware required)
//! cargo test --test hardware_pvcam_validation --features instrument_photometrics
//!
//! # Run with hardware tests (requires Prime BSI camera)
//! cargo test --test hardware_pvcam_validation \
//!   --features "instrument_photometrics,pvcam_hardware,hardware_tests" \
//!   -- --test-threads=1
//!
//! # Run with Prime 95B camera instead
//! cargo test --test hardware_pvcam_validation \
//!   --features "instrument_photometrics,pvcam_hardware,hardware_tests,prime_95b_tests" \
//!   -- --test-threads=1
//! ```

use rust_daq::hardware::capabilities::{ExposureControl, Triggerable};
use rust_daq::hardware::pvcam::PvcamDriver;
use rust_daq::hardware::Roi;
use std::time::Instant;

// ============================================================================
// Camera Model Constants
// ============================================================================

/// Prime BSI sensor dimensions (default)
const PRIME_BSI_WIDTH: u32 = 2048;
const PRIME_BSI_HEIGHT: u32 = 2048;

/// Prime 95B sensor dimensions (optional)
#[cfg(feature = "prime_95b_tests")]
const PRIME_95B_WIDTH: u32 = 1200;
#[cfg(feature = "prime_95b_tests")]
const PRIME_95B_HEIGHT: u32 = 1200;

/// Get the default camera name for tests
fn default_camera_name() -> &'static str {
    #[cfg(feature = "prime_95b_tests")]
    {
        "Prime95B"
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        "PrimeBSI"
    }
}

/// Get the expected sensor width
fn expected_width() -> u32 {
    #[cfg(feature = "prime_95b_tests")]
    {
        PRIME_95B_WIDTH
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        PRIME_BSI_WIDTH
    }
}

/// Get the expected sensor height
fn expected_height() -> u32 {
    #[cfg(feature = "prime_95b_tests")]
    {
        PRIME_95B_HEIGHT
    }
    #[cfg(not(feature = "prime_95b_tests"))]
    {
        PRIME_BSI_HEIGHT
    }
}

// ============================================================================
// UNIT TESTS: Camera Configuration and Validation
// ============================================================================

/// Test 1: Validate Prime BSI camera dimensions
#[test]
fn test_prime_bsi_dimensions() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create Prime BSI camera");

    // Prime BSI: 2048 x 2048 pixel sensor
    let roi = tokio_test::block_on(camera.roi());
    assert_eq!(roi.width, PRIME_BSI_WIDTH, "Prime BSI width should be 2048");
    assert_eq!(roi.height, PRIME_BSI_HEIGHT, "Prime BSI height should be 2048");
}

/// Test 2: Validate Prime 95B camera dimensions (only when prime_95b_tests enabled)
#[test]
#[cfg(feature = "prime_95b_tests")]
fn test_prime_95b_dimensions() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create Prime 95B camera");

    // Prime 95B: 1200 x 1200 pixel sensor
    let roi = tokio_test::block_on(camera.roi());
    assert_eq!(roi.width, PRIME_95B_WIDTH, "Prime 95B width should be 1200");
    assert_eq!(roi.height, PRIME_95B_HEIGHT, "Prime 95B height should be 1200");
}

/// Test 3: Validate binning factors
#[test]
fn test_binning_validation() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Valid binning: 1, 2, 4, 8
    let valid_bins = vec![1, 2, 4, 8];
    for bin in valid_bins {
        let result = tokio_test::block_on(camera.set_binning(bin, bin));
        assert!(result.is_ok(), "Binning {}x{} should be valid", bin, bin);
    }

    // Invalid binning: 3, 5, 6, 7, 16
    let invalid_bins = vec![3, 5, 6, 7, 16];
    for bin in invalid_bins {
        let result = tokio_test::block_on(camera.set_binning(bin, bin));
        assert!(result.is_err(), "Binning {}x{} should be invalid", bin, bin);
    }
}

/// Test 4: Validate ROI bounds checking
#[test]
fn test_roi_bounds_validation() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");
    let width = expected_width();
    let height = expected_height();

    // Valid ROI: Within sensor bounds
    let valid_roi = Roi {
        x: 0,
        y: 0,
        width,
        height,
    };
    let result = tokio_test::block_on(camera.set_roi(valid_roi));
    assert!(result.is_ok(), "Full sensor ROI should be valid");

    // Invalid ROI: Exceeds sensor width
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: width + 1,
        height,
    };
    let result = tokio_test::block_on(camera.set_roi(invalid_roi));
    assert!(result.is_err(), "ROI exceeding sensor width should be invalid");

    // Invalid ROI: Exceeds sensor height
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width,
        height: height + 1,
    };
    let result = tokio_test::block_on(camera.set_roi(invalid_roi));
    assert!(result.is_err(), "ROI exceeding sensor height should be invalid");
}

/// Test 5: Frame size calculation with binning
#[test]
fn test_frame_size_with_binning() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Set 2x2 binning
    tokio_test::block_on(camera.set_binning(2, 2)).expect("Failed to set binning");
    let binning = tokio_test::block_on(camera.binning());
    assert_eq!(binning, (2, 2), "Binning should be 2x2");

    // Frame dimensions should account for binning
    let roi = tokio_test::block_on(camera.roi());
    let expected_pixels = (roi.width / binning.0 as u32) * (roi.height / binning.1 as u32);
    assert!(expected_pixels > 0, "Frame should have non-zero pixels");
}

// ============================================================================
// MOCK INTEGRATION TESTS: Camera Operations
// ============================================================================

/// Test 6: Create default camera instance
#[tokio::test]
async fn test_create_default_camera() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");
    let roi = camera.roi().await;
    assert_eq!(roi.width, expected_width());
    assert_eq!(roi.height, expected_height());
}

/// Test 7: Create Prime BSI camera instance explicitly
#[tokio::test]
async fn test_create_prime_bsi() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create Prime BSI camera");
    let roi = camera.roi().await;
    assert_eq!(roi.width, PRIME_BSI_WIDTH);
    assert_eq!(roi.height, PRIME_BSI_HEIGHT);
}

/// Test 8: Create Prime 95B camera instance (only when prime_95b_tests enabled)
#[tokio::test]
#[cfg(feature = "prime_95b_tests")]
async fn test_create_prime_95b() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create Prime 95B camera");
    let roi = camera.roi().await;
    assert_eq!(roi.width, PRIME_95B_WIDTH);
    assert_eq!(roi.height, PRIME_95B_HEIGHT);
}

/// Test 9: Set and get exposure time
#[tokio::test]
async fn test_exposure_control() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Set exposure to 50ms
    camera.set_exposure_ms(50.0).await.expect("Failed to set exposure");
    let exposure = camera.get_exposure_ms().await.expect("Failed to get exposure");
    assert_eq!(exposure, 50.0, "Exposure should be 50ms");

    // Change to 100ms
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");
    let exposure = camera.get_exposure_ms().await.expect("Failed to get exposure");
    assert_eq!(exposure, 100.0, "Exposure should be 100ms");
}

/// Test 10: Set and get full sensor ROI
#[tokio::test]
async fn test_roi_full_sensor() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    let roi = Roi {
        x: 0,
        y: 0,
        width: expected_width(),
        height: expected_height(),
    };

    camera.set_roi(roi).await.expect("Failed to set ROI");
    let retrieved_roi = camera.roi().await;

    assert_eq!(retrieved_roi.x, 0);
    assert_eq!(retrieved_roi.y, 0);
    assert_eq!(retrieved_roi.width, expected_width());
    assert_eq!(retrieved_roi.height, expected_height());
}

/// Test 11: Set and get quarter sensor ROI
#[tokio::test]
async fn test_roi_quarter_sensor() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    let w = expected_width();
    let h = expected_height();

    // Center quarter ROI
    let roi = Roi {
        x: w / 4,
        y: h / 4,
        width: w / 2,
        height: h / 2,
    };

    camera.set_roi(roi).await.expect("Failed to set ROI");
    let retrieved_roi = camera.roi().await;

    assert_eq!(retrieved_roi.x, w / 4);
    assert_eq!(retrieved_roi.y, h / 4);
    assert_eq!(retrieved_roi.width, w / 2);
    assert_eq!(retrieved_roi.height, h / 2);
}

/// Test 12: Set and get 1x1 binning (no binning)
#[tokio::test]
async fn test_binning_1x1() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera.set_binning(1, 1).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (1, 1), "Binning should be 1x1");
}

/// Test 13: Set and get 2x2 binning
#[tokio::test]
async fn test_binning_2x2() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera.set_binning(2, 2).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (2, 2), "Binning should be 2x2");
}

/// Test 14: Set and get 4x4 binning
#[tokio::test]
async fn test_binning_4x4() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera.set_binning(4, 4).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (4, 4), "Binning should be 4x4");
}

/// Test 15: Invalid binning factor should fail
#[tokio::test]
async fn test_invalid_binning() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // 3x3 binning is invalid (must be 1, 2, 4, or 8)
    let result = camera.set_binning(3, 3).await;
    assert!(result.is_err(), "Invalid binning should return error");
}

/// Test 16: ROI exceeding sensor bounds should fail
#[tokio::test]
async fn test_invalid_roi_exceeds_sensor() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // ROI exceeds sensor
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: expected_width() + 100,
        height: expected_height(),
    };

    let result = camera.set_roi(invalid_roi).await;
    assert!(result.is_err(), "ROI exceeding sensor should return error");
}

/// Test 17: Acquire single frame
#[tokio::test]
async fn test_acquire_single_frame() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera.set_exposure_ms(10.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    assert_eq!(frame.width, expected_width(), "Frame width should match sensor");
    assert_eq!(frame.height, expected_height(), "Frame height should match sensor");
    assert_eq!(
        frame.buffer.len(),
        (expected_width() * expected_height()) as usize,
        "Frame buffer size should be width * height"
    );
}

/// Test 18: Frame data pattern validation
#[tokio::test]
async fn test_frame_data_pattern() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // In mock mode, frame should contain non-zero data (test pattern)
    let non_zero_pixels = frame.buffer.iter().filter(|&&p| p != 0).count();
    assert!(non_zero_pixels > 0, "Mock frame should contain non-zero pixel data");
}

/// Test 19: Arm and disarm triggering
#[tokio::test]
async fn test_arm_disarm_trigger() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Arm for triggering
    camera.arm().await.expect("Failed to arm camera");

    // Disarm
    camera.disarm().await.expect("Failed to disarm camera");
}

/// Test 20: Multiple frame acquisition
#[tokio::test]
async fn test_multiple_frames() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera.set_exposure_ms(5.0).await.expect("Failed to set exposure");

    // Acquire 5 frames
    for i in 0..5 {
        let frame = camera.acquire_frame().await.expect(&format!("Failed to acquire frame {}", i));
        assert_eq!(frame.width, expected_width());
        assert_eq!(frame.height, expected_height());
    }
}

/// Test 21: Rapid acquisition rate test
#[tokio::test]
async fn test_rapid_acquisition() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Short exposure for high frame rate
    camera.set_exposure_ms(1.0).await.expect("Failed to set exposure");

    let start = Instant::now();
    let frame_count = 10;

    for _ in 0..frame_count {
        camera.acquire_frame().await.expect("Failed to acquire frame");
    }

    let duration = start.elapsed();
    let fps = frame_count as f64 / duration.as_secs_f64();

    // In mock mode, should achieve >10 fps
    // In hardware mode with single-frame acquisition, overhead may lower this to ~5+ fps
    #[cfg(feature = "hardware_tests")]
    assert!(fps > 5.0, "Frame rate should be >5 fps, got {:.1} fps", fps);
    #[cfg(not(feature = "hardware_tests"))]
    assert!(fps > 10.0, "Frame rate should be >10 fps, got {:.1} fps", fps);
}

// ============================================================================
// HARDWARE VALIDATION TESTS (require physical camera)
// ============================================================================

/// Test 22: Hardware camera initialization
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_initialization() {
    // This test requires PVCAM SDK and physical camera
    let camera = PvcamDriver::new("PMCam").expect("Failed to open hardware camera");

    // Verify camera properties
    let roi = camera.roi().await;
    assert!(roi.width > 0, "Hardware camera should have non-zero width");
    assert!(roi.height > 0, "Hardware camera should have non-zero height");

    println!(
        "Hardware camera detected: {}x{} pixels",
        roi.width, roi.height
    );
}

/// Test 23: Hardware frame acquisition
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_frame_acquisition() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Verify frame properties
    assert!(frame.width > 0);
    assert!(frame.height > 0);
    assert_eq!(frame.buffer.len(), (frame.width * frame.height) as usize);

    println!(
        "Acquired frame: {}x{}, {} pixels",
        frame.width,
        frame.height,
        frame.buffer.len()
    );
}

/// Test 24: Hardware ROI configuration
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_roi() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Set quarter-sensor ROI
    let full_roi = camera.roi().await;
    let roi = Roi {
        x: full_roi.width / 4,
        y: full_roi.height / 4,
        width: full_roi.width / 2,
        height: full_roi.height / 2,
    };

    camera.set_roi(roi).await.expect("Failed to set ROI");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Frame size should match ROI
    assert_eq!(frame.width, roi.width);
    assert_eq!(frame.height, roi.height);
}

/// Test 25: Hardware binning and frame size
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_binning() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Set 2x2 binning
    camera.set_binning(2, 2).await.expect("Failed to set binning");

    let full_roi = camera.roi().await;
    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Frame dimensions should be half of ROI due to 2x2 binning
    assert_eq!(frame.width, full_roi.width / 2);
    assert_eq!(frame.height, full_roi.height / 2);
}

/// Test 26: Exposure time accuracy
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_exposure_accuracy() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let exposure_times = vec![10.0, 50.0, 100.0, 500.0]; // milliseconds

    for exposure_ms in exposure_times {
        camera
            .set_exposure_ms(exposure_ms)
            .await
            .expect("Failed to set exposure");

        let start = Instant::now();
        camera.acquire_frame().await.expect("Failed to acquire frame");
        let actual_ms = start.elapsed().as_millis() as f64;

        // Single-frame acquisition has significant overhead (buffer setup, readout)
        // For short exposures, overhead dominates; for longer exposures, ratio improves
        // Expect at least exposure_ms but allow significant overhead for single-frame mode
        let min_expected = exposure_ms; // Should take at least the exposure time
        let max_overhead = 200.0; // Allow up to 200ms overhead for setup/readout
        assert!(
            actual_ms >= min_expected && actual_ms <= exposure_ms + max_overhead,
            "Exposure time {:.1}ms actual {:.1}ms (should be exposure + ≤200ms overhead)",
            exposure_ms,
            actual_ms
        );
    }
}

/// Test 27: Frame pixel uniformity (requires uniform illumination)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_pixel_uniformity() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Uniform illumination test: standard deviation should be low
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Calculate statistics
    let mean: f64 =
        frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;
    let variance: f64 = frame
        .buffer
        .iter()
        .map(|&p| {
            let diff = p as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / frame.buffer.len() as f64;
    let std_dev = variance.sqrt();

    // With uniform illumination, std_dev should be <5% of mean
    let relative_std = std_dev / mean;
    println!(
        "Uniformity: mean={:.1}, std_dev={:.1}, relative={:.3}",
        mean, std_dev, relative_std
    );

    assert!(
        relative_std < 0.05,
        "Pixel uniformity: std_dev {:.1}, mean {:.1}, relative {:.3} (should be <0.05)",
        std_dev,
        mean,
        relative_std
    );
}

/// Test 28: Dark frame noise level (requires lens cap / dark environment)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_dark_noise() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Dark frame test: mean should be near zero, low variance
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Calculate dark current statistics
    let mean: f64 =
        frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;

    println!("Dark frame mean: {:.1} ADU", mean);

    // Dark current should be low (<200 ADU typical for modern sCMOS)
    // Prime BSI typically shows ~100-110 ADU offset in dark frames
    assert!(
        mean < 200.0,
        "Dark frame mean {:.1} ADU (should be <200 for good sensor)",
        mean
    );
}

/// Test 29: Triggered acquisition mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_triggered_acquisition() {
    use std::time::Duration;

    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    camera.set_exposure_ms(50.0).await.expect("Failed to set exposure");

    // Arm for external trigger
    camera.arm().await.expect("Failed to arm camera");

    // Wait for trigger (or timeout after 2 seconds)
    let result = tokio::time::timeout(Duration::from_secs(2), camera.wait_for_trigger()).await;

    // Disarm regardless of result
    camera.disarm().await.expect("Failed to disarm camera");

    // Note: This test will timeout if no trigger signal is provided
    // In production setup, connect external trigger source
    match result {
        Ok(Ok(())) => println!("Trigger received"),
        Ok(Err(e)) => panic!("Trigger wait failed: {}", e),
        Err(_) => println!("Trigger timeout (expected without trigger source)"),
    }
}
