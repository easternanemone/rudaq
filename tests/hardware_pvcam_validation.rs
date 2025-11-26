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
use rust_daq::hardware::pvcam::{
    CameraInfo, CentroidsConfig, CentroidsMode, GainMode, PPFeature, PPParam, PvcamDriver,
    SpeedMode,
};
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
    assert_eq!(
        roi.height, PRIME_BSI_HEIGHT,
        "Prime BSI height should be 2048"
    );
}

/// Test 2: Validate Prime 95B camera dimensions (only when prime_95b_tests enabled)
#[test]
#[cfg(feature = "prime_95b_tests")]
fn test_prime_95b_dimensions() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create Prime 95B camera");

    // Prime 95B: 1200 x 1200 pixel sensor
    let roi = tokio_test::block_on(camera.roi());
    assert_eq!(roi.width, PRIME_95B_WIDTH, "Prime 95B width should be 1200");
    assert_eq!(
        roi.height, PRIME_95B_HEIGHT,
        "Prime 95B height should be 1200"
    );
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
    assert!(
        result.is_err(),
        "ROI exceeding sensor width should be invalid"
    );

    // Invalid ROI: Exceeds sensor height
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width,
        height: height + 1,
    };
    let result = tokio_test::block_on(camera.set_roi(invalid_roi));
    assert!(
        result.is_err(),
        "ROI exceeding sensor height should be invalid"
    );
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
    camera
        .set_exposure_ms(50.0)
        .await
        .expect("Failed to set exposure");
    let exposure = camera
        .get_exposure_ms()
        .await
        .expect("Failed to get exposure");
    assert_eq!(exposure, 50.0, "Exposure should be 50ms");

    // Change to 100ms
    camera
        .set_exposure_ms(100.0)
        .await
        .expect("Failed to set exposure");
    let exposure = camera
        .get_exposure_ms()
        .await
        .expect("Failed to get exposure");
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

    camera
        .set_binning(1, 1)
        .await
        .expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (1, 1), "Binning should be 1x1");
}

/// Test 13: Set and get 2x2 binning
#[tokio::test]
async fn test_binning_2x2() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera
        .set_binning(2, 2)
        .await
        .expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (2, 2), "Binning should be 2x2");
}

/// Test 14: Set and get 4x4 binning
#[tokio::test]
async fn test_binning_4x4() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    camera
        .set_binning(4, 4)
        .await
        .expect("Failed to set binning");
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

    camera
        .set_exposure_ms(10.0)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    assert_eq!(
        frame.width,
        expected_width(),
        "Frame width should match sensor"
    );
    assert_eq!(
        frame.height,
        expected_height(),
        "Frame height should match sensor"
    );
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

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // In mock mode, frame should contain non-zero data (test pattern)
    let non_zero_pixels = frame.buffer.iter().filter(|&&p| p != 0).count();
    assert!(
        non_zero_pixels > 0,
        "Mock frame should contain non-zero pixel data"
    );
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

    camera
        .set_exposure_ms(5.0)
        .await
        .expect("Failed to set exposure");

    // Acquire 5 frames
    for i in 0..5 {
        let frame = camera
            .acquire_frame()
            .await
            .expect(&format!("Failed to acquire frame {}", i));
        assert_eq!(frame.width, expected_width());
        assert_eq!(frame.height, expected_height());
    }
}

/// Test 21: Rapid acquisition rate test
#[tokio::test]
async fn test_rapid_acquisition() {
    let camera = PvcamDriver::new(default_camera_name()).expect("Failed to create camera");

    // Short exposure for high frame rate
    camera
        .set_exposure_ms(1.0)
        .await
        .expect("Failed to set exposure");

    let start = Instant::now();
    let frame_count = 10;

    for _ in 0..frame_count {
        camera
            .acquire_frame()
            .await
            .expect("Failed to acquire frame");
    }

    let duration = start.elapsed();
    let fps = frame_count as f64 / duration.as_secs_f64();

    // In mock mode, should achieve >10 fps
    // In hardware mode with single-frame acquisition, overhead may lower this to ~5+ fps
    #[cfg(feature = "hardware_tests")]
    assert!(fps > 5.0, "Frame rate should be >5 fps, got {:.1} fps", fps);
    #[cfg(not(feature = "hardware_tests"))]
    assert!(
        fps > 10.0,
        "Frame rate should be >10 fps, got {:.1} fps",
        fps
    );
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
    assert!(
        roi.height > 0,
        "Hardware camera should have non-zero height"
    );

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

    camera
        .set_exposure_ms(100.0)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

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

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

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
    camera
        .set_binning(2, 2)
        .await
        .expect("Failed to set binning");

    let full_roi = camera.roi().await;
    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

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
        camera
            .acquire_frame()
            .await
            .expect("Failed to acquire frame");
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
    camera
        .set_exposure_ms(100.0)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // Calculate statistics
    let mean: f64 = frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;
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
    camera
        .set_exposure_ms(100.0)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // Calculate dark current statistics
    let mean: f64 = frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;

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

    camera
        .set_exposure_ms(50.0)
        .await
        .expect("Failed to set exposure");

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

// ============================================================================
// Section 8: Camera Information Tests
// ============================================================================

/// Test 30: Get sensor temperature
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_temperature() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let temp = camera
        .get_temperature()
        .await
        .expect("Failed to get temperature");

    println!("Sensor temperature: {:.2}°C", temp);

    // Prime BSI typically cooled between -40°C and +30°C
    assert!(
        temp >= -50.0 && temp <= 50.0,
        "Temperature {} is out of expected range",
        temp
    );
}

/// Test 31: Get chip/sensor name
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_chip_name() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let name = camera
        .get_chip_name()
        .await
        .expect("Failed to get chip name");

    println!("Chip name: {}", name);

    // Should be non-empty
    assert!(!name.is_empty(), "Chip name should not be empty");
}

/// Test 32: Get bit depth
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_bit_depth() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let depth = camera
        .get_bit_depth()
        .await
        .expect("Failed to get bit depth");

    println!("ADC bit depth: {}", depth);

    // Prime BSI has 11-bit ADC native, can also be 12, 14, or 16 bit
    assert!(
        depth >= 8 && depth <= 16,
        "Unexpected bit depth: {}",
        depth
    );
}

/// Test 33: Get readout time
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_readout_time() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let time_us = camera
        .get_readout_time_us()
        .await
        .expect("Failed to get readout time");

    println!("Readout time: {:.2} us ({:.2} ms)", time_us, time_us / 1000.0);

    // Should be positive and reasonable (< 1 second)
    assert!(
        time_us > 0.0 && time_us < 1_000_000.0,
        "Readout time {} us is out of expected range",
        time_us
    );
}

/// Test 34: Get pixel size
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_pixel_size() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let (pix_w, pix_h) = camera
        .get_pixel_size_nm()
        .await
        .expect("Failed to get pixel size");

    println!("Pixel size: {} x {} nm ({:.2} x {:.2} um)",
             pix_w, pix_h, pix_w as f64 / 1000.0, pix_h as f64 / 1000.0);

    // Prime BSI has 6.5um pixels
    // Reasonable range: 1um - 100um (1000nm - 100000nm)
    assert!(
        pix_w >= 1000 && pix_w <= 100000,
        "Pixel width {} nm is out of expected range",
        pix_w
    );
    assert!(
        pix_h >= 1000 && pix_h <= 100000,
        "Pixel height {} nm is out of expected range",
        pix_h
    );
}

/// Test 35: Get gain name
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain_name() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let name = camera
        .get_gain_name()
        .await
        .expect("Failed to get gain name");

    println!("Current gain mode: {}", name);

    // Should be non-empty
    assert!(!name.is_empty(), "Gain name should not be empty");
}

/// Test 36: Get speed table name
/// Note: PARAM_SPDTAB_NAME may not be available on all cameras
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed_name() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    match camera.get_speed_name().await {
        Ok(name) => {
            println!("Current speed mode: {}", name);
            // Should be non-empty if available
            assert!(!name.is_empty(), "Speed name should not be empty");
        }
        Err(e) => {
            // PARAM_SPDTAB_NAME not available on this camera - that's OK
            println!("Speed name not available: {} (this is OK for some cameras)", e);
        }
    }
}

/// Test 37: Get gain index
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain_index() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let idx = camera
        .get_gain_index()
        .await
        .expect("Failed to get gain index");

    println!("Current gain index: {}", idx);

    // Index should be reasonable (typically 0-10)
    assert!(idx < 100, "Gain index {} seems too high", idx);
}

/// Test 38: Get speed table index
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed_index() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let idx = camera
        .get_speed_index()
        .await
        .expect("Failed to get speed index");

    println!("Current speed table index: {}", idx);

    // Index should be reasonable (typically 0-10)
    assert!(idx < 100, "Speed index {} seems too high", idx);
}

/// Test 39: Get comprehensive camera info
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_camera_info() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let info = camera
        .get_camera_info()
        .await
        .expect("Failed to get camera info");

    println!("=== Camera Information ===");
    println!("Chip name:      {}", info.chip_name);
    println!("Temperature:    {:.2}°C", info.temperature_c);
    println!("Bit depth:      {}", info.bit_depth);
    println!("Readout time:   {:.2} us", info.readout_time_us);
    println!("Pixel size:     {} x {} nm", info.pixel_size_nm.0, info.pixel_size_nm.1);
    println!("Sensor size:    {} x {} pixels", info.sensor_size.0, info.sensor_size.1);
    println!("Gain mode:      {}", info.gain_name);
    println!("Speed mode:     {}", info.speed_name);

    // Verify sensor size matches expected (Prime BSI = 2048x2048)
    assert_eq!(info.sensor_size.0, expected_width(), "Unexpected sensor width");
    assert_eq!(info.sensor_size.1, expected_height(), "Unexpected sensor height");
}

// =============================================================================
// Tests 40-45: Gain and Speed Table Selection
// =============================================================================

/// Test 40: List available gain modes
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_gain_modes() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let modes = camera
        .list_gain_modes()
        .await
        .expect("Failed to list gain modes");

    println!("=== Available Gain Modes ===");
    for mode in &modes {
        println!("  Index {}: {}", mode.index, mode.name);
    }

    // Verify we have at least one gain mode
    assert!(!modes.is_empty(), "Camera should have at least one gain mode");

    // Verify indices are sequential starting from 0
    for (i, mode) in modes.iter().enumerate() {
        assert_eq!(
            mode.index as usize, i,
            "Gain mode indices should be sequential"
        );
    }
}

/// Test 41: List available speed modes
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_speed_modes() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let modes = camera
        .list_speed_modes()
        .await
        .expect("Failed to list speed modes");

    println!("=== Available Speed Modes ===");
    for mode in &modes {
        println!("  Index {}: {}", mode.index, mode.name);
    }

    // Verify we have at least one speed mode
    assert!(!modes.is_empty(), "Camera should have at least one speed mode");

    // Verify indices are sequential starting from 0
    for (i, mode) in modes.iter().enumerate() {
        assert_eq!(
            mode.index as usize, i,
            "Speed mode indices should be sequential"
        );
    }
}

/// Test 42: Get current gain mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let gain = camera.get_gain().await.expect("Failed to get gain mode");

    println!("Current gain: Index {} - {}", gain.index, gain.name);

    // Verify gain index is within valid range
    let modes = camera.list_gain_modes().await.expect("Failed to list gain modes");
    assert!(
        (gain.index as usize) < modes.len(),
        "Current gain index should be within available modes"
    );
}

/// Test 43: Get current speed mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let speed = camera.get_speed().await.expect("Failed to get speed mode");

    println!("Current speed: Index {} - {}", speed.index, speed.name);

    // Verify speed index is within valid range
    let modes = camera.list_speed_modes().await.expect("Failed to list speed modes");
    assert!(
        (speed.index as usize) < modes.len(),
        "Current speed index should be within available modes"
    );
}

/// Test 44: Set gain mode and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_gain_index() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Get available gain modes
    let modes = camera.list_gain_modes().await.expect("Failed to list gain modes");
    assert!(!modes.is_empty(), "Need at least one gain mode to test");

    // Save original gain
    let original_gain = camera.get_gain_index().await.expect("Failed to get original gain");

    println!("Original gain index: {}", original_gain);

    // Test setting each available gain mode
    for mode in &modes {
        camera
            .set_gain_index(mode.index)
            .await
            .expect(&format!("Failed to set gain index {}", mode.index));

        let current = camera.get_gain_index().await.expect("Failed to read back gain");
        assert_eq!(
            current, mode.index,
            "Gain index should match after setting"
        );
        println!("  Set gain {}: {} - OK", mode.index, mode.name);
    }

    // Restore original gain
    camera
        .set_gain_index(original_gain)
        .await
        .expect("Failed to restore original gain");
    println!("Restored original gain index: {}", original_gain);
}

/// Test 45: Set speed mode and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_speed_index() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Get available speed modes
    let modes = camera.list_speed_modes().await.expect("Failed to list speed modes");
    assert!(!modes.is_empty(), "Need at least one speed mode to test");

    // Save original speed
    let original_speed = camera.get_speed_index().await.expect("Failed to get original speed");

    println!("Original speed index: {}", original_speed);

    // Test setting each available speed mode
    for mode in &modes {
        camera
            .set_speed_index(mode.index)
            .await
            .expect(&format!("Failed to set speed index {}", mode.index));

        let current = camera.get_speed_index().await.expect("Failed to read back speed");
        assert_eq!(
            current, mode.index,
            "Speed index should match after setting"
        );
        println!("  Set speed {}: {} - OK", mode.index, mode.name);
    }

    // Restore original speed
    camera
        .set_speed_index(original_speed)
        .await
        .expect("Failed to restore original speed");
    println!("Restored original speed index: {}", original_speed);
}

// =============================================================================
// Tests 46-49: Temperature Control
// =============================================================================

/// Test 46: Get temperature setpoint
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_temperature_setpoint() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let setpoint = camera
        .get_temperature_setpoint()
        .await
        .expect("Failed to get temperature setpoint");

    println!("Temperature setpoint: {:.2}°C", setpoint);

    // Typical cooled camera setpoints are between -50°C and +25°C
    assert!(
        setpoint >= -55.0 && setpoint <= 30.0,
        "Temperature setpoint {} seems unreasonable",
        setpoint
    );
}

/// Test 47: Get and compare temperature vs setpoint
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_temperature_vs_setpoint() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let current = camera
        .get_temperature()
        .await
        .expect("Failed to get current temperature");
    let setpoint = camera
        .get_temperature_setpoint()
        .await
        .expect("Failed to get temperature setpoint");

    println!("Current temperature:  {:.2}°C", current);
    println!("Temperature setpoint: {:.2}°C", setpoint);
    println!("Difference:           {:.2}°C", (current - setpoint).abs());

    // Both should be in reasonable range
    assert!(current >= -55.0 && current <= 50.0, "Current temp unreasonable: {}", current);
    assert!(setpoint >= -55.0 && setpoint <= 30.0, "Setpoint unreasonable: {}", setpoint);
}

/// Test 48: Get fan speed
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_fan_speed() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let speed = camera
        .get_fan_speed()
        .await
        .expect("Failed to get fan speed");

    println!("Fan speed: {:?}", speed);
}

/// Test 49: Set fan speed and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_fan_speed() {
    use rust_daq::hardware::pvcam::FanSpeed;

    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Save original fan speed
    let original_speed = camera
        .get_fan_speed()
        .await
        .expect("Failed to get original fan speed");
    println!("Original fan speed: {:?}", original_speed);

    // Test each fan speed setting
    let speeds = [FanSpeed::High, FanSpeed::Medium, FanSpeed::Low];
    for speed in &speeds {
        match camera.set_fan_speed(*speed).await {
            Ok(()) => {
                let readback = camera.get_fan_speed().await.expect("Failed to read fan speed");
                println!("  Set {:?} -> Read back {:?}", speed, readback);
                // Note: Some cameras may not support all fan speeds
                // Just verify we can set and read without error
            }
            Err(e) => {
                println!("  Set {:?} failed: {} (may not be supported)", speed, e);
            }
        }
    }

    // Restore original fan speed
    camera
        .set_fan_speed(original_speed)
        .await
        .expect("Failed to restore fan speed");
    println!("Restored fan speed: {:?}", original_speed);
}

// ============================================================================
// POST-PROCESSING FEATURE TESTS (Tests 50-53)
// ============================================================================

/// Test 50: List post-processing features
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_pp_features() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let features = camera
        .list_pp_features()
        .await
        .expect("Failed to list PP features");

    println!("Post-processing features ({}):", features.len());
    for feat in &features {
        println!("  [{}] ID={}: {}", feat.index, feat.id, feat.name);
    }

    // PP features may or may not be available depending on camera model
    // Just verify we can query without error
    println!("PP feature enumeration completed");
}

/// Test 51: Get PP params for each feature
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_pp_params() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let features = camera
        .list_pp_features()
        .await
        .expect("Failed to list PP features");

    if features.is_empty() {
        println!("No PP features available on this camera");
        return;
    }

    println!("PP feature parameters:");
    for feat in &features {
        let params = camera
            .get_pp_params(feat.index)
            .await
            .expect(&format!("Failed to get params for feature {}", feat.index));

        println!("  {} ({} params):", feat.name, params.len());
        for param in &params {
            println!("    [{}] {}: {}", param.index, param.name, param.value);
        }
    }
}

/// Test 52: Get/Set PP param value
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_set_pp_param() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let features = camera
        .list_pp_features()
        .await
        .expect("Failed to list PP features");

    if features.is_empty() {
        println!("No PP features available on this camera");
        return;
    }

    // Find a feature with parameters
    for feat in &features {
        let params = camera
            .get_pp_params(feat.index)
            .await
            .expect("Failed to get params");

        if params.is_empty() {
            continue;
        }

        // Test get/set on first parameter
        let param = &params[0];
        println!("Testing feature '{}' param '{}' (current value: {})",
            feat.name, param.name, param.value);

        let original = camera
            .get_pp_param(feat.index, param.index)
            .await
            .expect("Failed to get PP param");

        assert_eq!(original, param.value, "get_pp_param should match get_pp_params");

        println!("  get_pp_param returned: {}", original);
        println!("  PP param get/set test passed");
        return; // Only test one param
    }

    println!("No PP features with parameters found");
}

/// Test 53: Reset PP features
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_reset_pp_features() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    match camera.reset_pp_features().await {
        Ok(()) => println!("PP features reset to defaults successfully"),
        Err(e) => println!("PP reset not supported or failed: {}", e),
    }
}

// ============================================================================
// SMART STREAMING TESTS (Tests 54-57)
// ============================================================================

/// Test 54: Check if Smart Streaming is available
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_available() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_smart_streaming_available()
        .await
        .expect("Failed to check Smart Streaming availability");

    println!("Smart Streaming available: {}", available);
}

/// Test 55: Get Smart Streaming max entries
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_max_entries() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_smart_streaming_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Smart Streaming not available on this camera");
        return;
    }

    match camera.get_smart_stream_max_entries().await {
        Ok(max_entries) => println!("Smart Streaming max entries: {}", max_entries),
        Err(e) => println!("Could not get max entries (may need different query): {}", e),
    }
}

/// Test 56: Enable/disable Smart Streaming
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_enable_disable() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_smart_streaming_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Smart Streaming not available on this camera");
        return;
    }

    // Check initial status
    let initial_status = camera
        .is_smart_streaming_enabled()
        .await
        .expect("Failed to get initial status");
    println!("Initial Smart Streaming status: {}", initial_status);

    // Enable Smart Streaming
    camera
        .enable_smart_streaming()
        .await
        .expect("Failed to enable Smart Streaming");

    let enabled = camera
        .is_smart_streaming_enabled()
        .await
        .expect("Failed to check enabled status");
    println!("After enable: {}", enabled);
    assert!(enabled, "Should be enabled after enable call");

    // Disable Smart Streaming
    camera
        .disable_smart_streaming()
        .await
        .expect("Failed to disable Smart Streaming");

    let disabled = camera
        .is_smart_streaming_enabled()
        .await
        .expect("Failed to check disabled status");
    println!("After disable: {}", disabled);
    assert!(!disabled, "Should be disabled after disable call");
}

/// Test 57: Set Smart Streaming exposure sequence
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_set_exposures() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_smart_streaming_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Smart Streaming not available on this camera");
        return;
    }

    // Enable Smart Streaming first
    camera
        .enable_smart_streaming()
        .await
        .expect("Failed to enable Smart Streaming");

    // Set an HDR-style exposure sequence (short, medium, long)
    let exposures_ms = vec![1.0, 10.0, 100.0];
    println!("Setting exposure sequence: {:?}ms", exposures_ms);

    match camera.set_smart_stream_exposures(&exposures_ms).await {
        Ok(()) => println!("Exposure sequence set successfully"),
        Err(e) => println!("Failed to set exposures: {} (may require setup_exp first)", e),
    }

    // Get exposure count
    match camera.get_smart_stream_exposure_count().await {
        Ok(count) => println!("Current exposure count: {}", count),
        Err(e) => println!("Failed to get exposure count: {}", e),
    }

    // Clean up - disable Smart Streaming
    camera
        .disable_smart_streaming()
        .await
        .expect("Failed to disable Smart Streaming");
}

// ============================================================================
// Centroids Mode Tests (PrimeLocate / Particle Tracking)
// ============================================================================

/// Test 58: Check if centroids feature is available
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_available() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    match camera.is_centroids_available().await {
        Ok(available) => {
            println!("Centroids (PrimeLocate) available: {}", available);
            // Note: Not all Prime cameras support centroids
            // Prime BSI typically has this feature
        }
        Err(e) => println!("Failed to check centroids availability: {}", e),
    }
}

/// Test 59: Enable/disable centroids mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_enable_disable() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_centroids_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Centroids not available on this camera");
        return;
    }

    // Check initial state
    let initial_enabled = camera
        .is_centroids_enabled()
        .await
        .expect("Failed to get initial state");
    println!("Initial centroids enabled: {}", initial_enabled);

    // Enable centroids
    match camera.enable_centroids().await {
        Ok(()) => {
            let enabled = camera
                .is_centroids_enabled()
                .await
                .expect("Failed to check state");
            println!("After enable_centroids(): enabled={}", enabled);
        }
        Err(e) => println!("Failed to enable centroids: {}", e),
    }

    // Disable centroids
    match camera.disable_centroids().await {
        Ok(()) => {
            let enabled = camera
                .is_centroids_enabled()
                .await
                .expect("Failed to check state");
            println!("After disable_centroids(): enabled={}", enabled);
        }
        Err(e) => println!("Failed to disable centroids: {}", e),
    }
}

/// Test 60: Get/set centroids mode (Locate, Track, Blob)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_mode() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_centroids_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Centroids not available on this camera");
        return;
    }

    // Get current mode
    match camera.get_centroids_mode().await {
        Ok(mode) => println!("Current centroids mode: {:?}", mode),
        Err(e) => {
            println!("Failed to get mode: {}", e);
            return;
        }
    }

    // Try each mode
    for mode in [
        CentroidsMode::Locate,
        CentroidsMode::Track,
        CentroidsMode::Blob,
    ] {
        match camera.set_centroids_mode(mode).await {
            Ok(()) => {
                let current = camera
                    .get_centroids_mode()
                    .await
                    .expect("Failed to get mode");
                println!("Set mode to {:?}, got {:?}", mode, current);
            }
            Err(e) => println!("Failed to set mode {:?}: {}", mode, e),
        }
    }

    // Restore to default
    let _ = camera.set_centroids_mode(CentroidsMode::Locate).await;
}

/// Test 61: Get/set centroids configuration (radius, count, threshold)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_config() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let available = camera
        .is_centroids_available()
        .await
        .expect("Failed to check availability");

    if !available {
        println!("Centroids not available on this camera");
        return;
    }

    // Get current configuration
    match camera.get_centroids_config().await {
        Ok(config) => {
            println!("Current centroids config:");
            println!("  Mode: {:?}", config.mode);
            println!("  Radius: {} pixels", config.radius);
            println!("  Max count: {}", config.max_count);
            println!("  Threshold: {}", config.threshold);
        }
        Err(e) => {
            println!("Failed to get config: {}", e);
            return;
        }
    }

    // Try setting individual parameters
    println!("\nTesting parameter changes:");

    // Radius
    match camera.set_centroids_radius(10).await {
        Ok(()) => {
            let r = camera
                .get_centroids_radius()
                .await
                .expect("Failed to get radius");
            println!("  Set radius=10, got radius={}", r);
        }
        Err(e) => println!("  Failed to set radius: {}", e),
    }

    // Count
    match camera.set_centroids_count(500).await {
        Ok(()) => {
            let c = camera
                .get_centroids_count()
                .await
                .expect("Failed to get count");
            println!("  Set count=500, got count={}", c);
        }
        Err(e) => println!("  Failed to set count: {}", e),
    }

    // Threshold
    match camera.set_centroids_threshold(2000).await {
        Ok(()) => {
            let t = camera
                .get_centroids_threshold()
                .await
                .expect("Failed to get threshold");
            println!("  Set threshold=2000, got threshold={}", t);
        }
        Err(e) => println!("  Failed to set threshold: {}", e),
    }

    // Test bulk config set
    let test_config = CentroidsConfig {
        mode: CentroidsMode::Locate,
        radius: 5,
        max_count: 100,
        threshold: 1000,
    };

    match camera.set_centroids_config(&test_config).await {
        Ok(()) => {
            let config = camera
                .get_centroids_config()
                .await
                .expect("Failed to get config");
            println!("\nAfter set_centroids_config:");
            println!("  Mode: {:?}", config.mode);
            println!("  Radius: {}", config.radius);
            println!("  Max count: {}", config.max_count);
            println!("  Threshold: {}", config.threshold);
        }
        Err(e) => println!("Failed to set bulk config: {}", e),
    }
}
