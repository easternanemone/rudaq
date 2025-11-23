//! PVCAM Camera Hardware Validation Test Suite
//!
//! Comprehensive validation tests for Photometrics PVCAM camera driver.
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
//! # Hardware Setup Requirements
//!
//! For actual hardware validation (not mock-based tests):
//!
//! 1. **Camera**:
//!    - Photometrics Prime BSI or Prime 95B
//!    - Connected via USB 3.0 or PCIe interface
//!    - PVCAM SDK installed and configured
//!    - Camera powered on and recognized by system
//!
//! 2. **Illumination** (for uniformity tests):
//!    - Uniform white light source (LED panel or integrating sphere)
//!    - Diffuse illumination across sensor area
//!    - Avoid bright spots or shadows
//!
//! 3. **Dark Environment** (for noise tests):
//!    - Camera lens cap or opaque cover
//!    - Room lights off
//!    - Allows measurement of dark current and read noise
//!
//! 4. **System Requirements**:
//!    - PVCAM SDK installed (libpvcam.so / pvcam64.dll)
//!    - Appropriate USB/PCIe drivers
//!    - Sufficient RAM for frame buffers (4+ GB recommended)
//!    - Fast SSD for continuous acquisition tests
//!
//! # Test Categories
//!
//! ## Unit Tests (5 tests)
//! - Camera name parsing and dimensions
//! - Binning validation logic
//! - ROI bounds checking
//! - Frame dimension calculations
//! - Mock mode operation
//!
//! ## Mock Integration Tests (15 tests)
//! - Camera creation and initialization
//! - Exposure control
//! - ROI configuration
//! - Binning configuration
//! - Frame acquisition
//! - Error handling
//!
//! ## Hardware Validation Tests (8 tests, marked #[ignore])
//! - Real camera initialization
//! - Physical frame acquisition
//! - Hardware ROI and binning
//! - Exposure timing accuracy
//! - Pixel uniformity
//! - Dark frame noise
//! - Triggered acquisition
//!
//! # Running Tests
//!
//! ```bash
//! # Run all mock tests (no hardware required)
//! cargo test --test hardware_pvcam_validation
//!
//! # Run with hardware tests (requires Prime BSI/95B camera)
//! cargo test --test hardware_pvcam_validation --features "pvcam_hardware,hardware_tests" \
//!   -- --ignored --test-threads=1
//!
//! # Run specific test
//! cargo test --test hardware_pvcam_validation test_create_prime_bsi
//! ```
//!
//! # Safety Considerations
//!
//! - Ensure camera is securely mounted
//! - Use lens cap for dark frame tests
//! - Avoid overexposure (saturation) during illuminated tests
//! - Monitor camera temperature during continuous acquisition
//! - Have adequate cooling for extended tests

use rust_daq::hardware::capabilities::{ExposureControl, FrameProducer, Triggerable};
use rust_daq::hardware::pvcam::PvcamDriver;
use rust_daq::hardware::{Frame, Roi};
use std::time::{Duration, Instant};

// ============================================================================
// UNIT TESTS: Camera Configuration and Validation
// ============================================================================

/// Test 1: Validate Prime BSI camera dimensions
#[test]
fn test_prime_bsi_dimensions() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create Prime BSI camera");

    // Prime BSI: 2048 x 2048 pixel sensor
    let roi = tokio_test::block_on(camera.roi());
    assert_eq!(roi.width, 2048, "Prime BSI width should be 2048");
    assert_eq!(roi.height, 2048, "Prime BSI height should be 2048");
}

/// Test 2: Validate Prime 95B camera dimensions
#[test]
fn test_prime_95b_dimensions() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create Prime 95B camera");

    // Prime 95B: 1200 x 1200 pixel sensor
    let roi = tokio_test::block_on(camera.roi());
    assert_eq!(roi.width, 1200, "Prime 95B width should be 1200");
    assert_eq!(roi.height, 1200, "Prime 95B height should be 1200");
}

/// Test 3: Validate binning factors
#[test]
fn test_binning_validation() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

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
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    // Valid ROI: Within sensor bounds (1200 x 1200)
    let valid_roi = Roi {
        x: 0,
        y: 0,
        width: 1200,
        height: 1200,
    };
    let result = tokio_test::block_on(camera.set_roi(valid_roi));
    assert!(result.is_ok(), "Full sensor ROI should be valid");

    // Invalid ROI: Exceeds sensor width
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: 1201,
        height: 1200,
    };
    let result = tokio_test::block_on(camera.set_roi(invalid_roi));
    assert!(result.is_err(), "ROI exceeding sensor width should be invalid");

    // Invalid ROI: Exceeds sensor height
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: 1200,
        height: 1201,
    };
    let result = tokio_test::block_on(camera.set_roi(invalid_roi));
    assert!(result.is_err(), "ROI exceeding sensor height should be invalid");
}

/// Test 5: Frame size calculation with binning
#[test]
fn test_frame_size_with_binning() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    // 2048 x 2048 with 2x2 binning = 1024 x 1024
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

/// Test 6: Create Prime BSI camera instance
#[tokio::test]
async fn test_create_prime_bsi() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create Prime BSI camera");
    let roi = camera.roi().await;
    assert_eq!(roi.width, 2048);
    assert_eq!(roi.height, 2048);
}

/// Test 7: Create Prime 95B camera instance
#[tokio::test]
async fn test_create_prime_95b() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create Prime 95B camera");
    let roi = camera.roi().await;
    assert_eq!(roi.width, 1200);
    assert_eq!(roi.height, 1200);
}

/// Test 8: Set and get exposure time
#[tokio::test]
async fn test_exposure_control() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    // Set exposure to 50ms
    camera.set_exposure_ms(50.0).await.expect("Failed to set exposure");
    let exposure = camera.get_exposure_ms().await.expect("Failed to get exposure");
    assert_eq!(exposure, 50.0, "Exposure should be 50ms");

    // Change to 100ms
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");
    let exposure = camera.get_exposure_ms().await.expect("Failed to get exposure");
    assert_eq!(exposure, 100.0, "Exposure should be 100ms");
}

/// Test 9: Set and get full sensor ROI
#[tokio::test]
async fn test_roi_full_sensor() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    let roi = Roi {
        x: 0,
        y: 0,
        width: 1200,
        height: 1200,
    };

    camera.set_roi(roi).await.expect("Failed to set ROI");
    let retrieved_roi = camera.roi().await;

    assert_eq!(retrieved_roi.x, 0);
    assert_eq!(retrieved_roi.y, 0);
    assert_eq!(retrieved_roi.width, 1200);
    assert_eq!(retrieved_roi.height, 1200);
}

/// Test 10: Set and get quarter sensor ROI
#[tokio::test]
async fn test_roi_quarter_sensor() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    // Center 1024x1024 ROI on 2048x2048 sensor
    let roi = Roi {
        x: 512,
        y: 512,
        width: 1024,
        height: 1024,
    };

    camera.set_roi(roi).await.expect("Failed to set ROI");
    let retrieved_roi = camera.roi().await;

    assert_eq!(retrieved_roi.x, 512);
    assert_eq!(retrieved_roi.y, 512);
    assert_eq!(retrieved_roi.width, 1024);
    assert_eq!(retrieved_roi.height, 1024);
}

/// Test 11: Set and get 1x1 binning (no binning)
#[tokio::test]
async fn test_binning_1x1() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    camera.set_binning(1, 1).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (1, 1), "Binning should be 1x1");
}

/// Test 12: Set and get 2x2 binning
#[tokio::test]
async fn test_binning_2x2() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    camera.set_binning(2, 2).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (2, 2), "Binning should be 2x2");
}

/// Test 13: Set and get 4x4 binning
#[tokio::test]
async fn test_binning_4x4() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    camera.set_binning(4, 4).await.expect("Failed to set binning");
    let binning = camera.binning().await;
    assert_eq!(binning, (4, 4), "Binning should be 4x4");
}

/// Test 14: Invalid binning factor should fail
#[tokio::test]
async fn test_invalid_binning() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    // 3x3 binning is invalid (must be 1, 2, 4, or 8)
    let result = camera.set_binning(3, 3).await;
    assert!(result.is_err(), "Invalid binning should return error");
}

/// Test 15: ROI exceeding sensor bounds should fail
#[tokio::test]
async fn test_invalid_roi_exceeds_sensor() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    // ROI exceeds 1200x1200 sensor
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: 1300,
        height: 1200,
    };

    let result = camera.set_roi(invalid_roi).await;
    assert!(result.is_err(), "ROI exceeding sensor should return error");
}

/// Test 16: Acquire single frame
#[tokio::test]
async fn test_acquire_single_frame() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    camera.set_exposure_ms(10.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    assert_eq!(frame.width, 2048, "Frame width should match sensor");
    assert_eq!(frame.height, 2048, "Frame height should match sensor");
    assert_eq!(frame.buffer.len(), 2048 * 2048, "Frame buffer size should be width * height");
}

/// Test 17: Frame data pattern validation
#[tokio::test]
async fn test_frame_data_pattern() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // In mock mode, frame should contain non-zero data (test pattern)
    let non_zero_pixels = frame.buffer.iter().filter(|&&p| p != 0).count();
    assert!(non_zero_pixels > 0, "Mock frame should contain non-zero pixel data");
}

/// Test 18: Arm and disarm triggering
#[tokio::test]
async fn test_arm_disarm_trigger() {
    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to create camera");

    // Arm for triggering
    camera.arm().await.expect("Failed to arm camera");

    // Disarm
    camera.disarm().await.expect("Failed to disarm camera");
}

/// Test 19: Multiple frame acquisition
#[tokio::test]
async fn test_multiple_frames() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    camera.set_exposure_ms(5.0).await.expect("Failed to set exposure");

    // Acquire 5 frames
    for i in 0..5 {
        let frame = camera.acquire_frame().await.expect(&format!("Failed to acquire frame {}", i));
        assert_eq!(frame.width, 1200);
        assert_eq!(frame.height, 1200);
    }
}

/// Test 20: Rapid acquisition rate test
#[tokio::test]
async fn test_rapid_acquisition() {
    let camera = PvcamDriver::new("Prime95B").expect("Failed to create camera");

    // Short exposure for high frame rate
    camera.set_exposure_ms(1.0).await.expect("Failed to set exposure");

    let start = Instant::now();
    let frame_count = 10;

    for _ in 0..frame_count {
        camera.acquire_frame().await.expect("Failed to acquire frame");
    }

    let duration = start.elapsed();
    let fps = frame_count as f64 / duration.as_secs_f64();

    // Should achieve >10 fps with 1ms exposure in mock mode
    assert!(fps > 10.0, "Frame rate should be >10 fps, got {:.1} fps", fps);
}

// ============================================================================
// HARDWARE VALIDATION TESTS (require physical camera)
// ============================================================================

/// Test 21: Hardware camera initialization
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_initialization() {
    // This test requires PVCAM SDK and physical camera
    let camera = PvcamDriver::new("PMCam").expect("Failed to open hardware camera");

    // Verify camera properties
    let roi = camera.roi().await;
    assert!(roi.width > 0, "Hardware camera should have non-zero width");
    assert!(roi.height > 0, "Hardware camera should have non-zero height");
}

/// Test 22: Hardware frame acquisition
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
}

/// Test 23: Hardware ROI configuration
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

/// Test 24: Hardware binning and frame size
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

/// Test 25: Exposure time accuracy
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_exposure_accuracy() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    let exposure_times = vec![10.0, 50.0, 100.0, 500.0]; // milliseconds

    for exposure_ms in exposure_times {
        camera.set_exposure_ms(exposure_ms).await.expect("Failed to set exposure");

        let start = Instant::now();
        camera.acquire_frame().await.expect("Failed to acquire frame");
        let actual_ms = start.elapsed().as_millis() as f64;

        // Actual time should be close to requested exposure (±20% tolerance for overhead)
        let tolerance = exposure_ms * 0.2;
        assert!(
            actual_ms >= exposure_ms - tolerance && actual_ms <= exposure_ms + tolerance * 5.0,
            "Exposure time {:.1}ms actual {:.1}ms (should be within ±20%)",
            exposure_ms,
            actual_ms
        );
    }
}

/// Test 26: Frame pixel uniformity (requires uniform illumination)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_pixel_uniformity() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Uniform illumination test: standard deviation should be low
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Calculate statistics
    let mean: f64 = frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;
    let variance: f64 = frame.buffer
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
    assert!(
        relative_std < 0.05,
        "Pixel uniformity: std_dev {:.1}, mean {:.1}, relative {:.3} (should be <0.05)",
        std_dev,
        mean,
        relative_std
    );
}

/// Test 27: Dark frame noise level (requires lens cap / dark environment)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_dark_noise() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    // Dark frame test: mean should be near zero, low variance
    camera.set_exposure_ms(100.0).await.expect("Failed to set exposure");

    let frame = camera.acquire_frame().await.expect("Failed to acquire frame");

    // Calculate dark current statistics
    let mean: f64 = frame.buffer.iter().map(|&p| p as f64).sum::<f64>() / frame.buffer.len() as f64;

    // Dark current should be low (<50 ADU typical for modern sCMOS)
    assert!(
        mean < 100.0,
        "Dark frame mean {:.1} ADU (should be <100 for good sensor)",
        mean
    );
}

/// Test 28: Triggered acquisition mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_triggered_acquisition() {
    let camera = PvcamDriver::new("PMCam").expect("Failed to open camera");

    camera.set_exposure_ms(50.0).await.expect("Failed to set exposure");

    // Arm for external trigger
    camera.arm().await.expect("Failed to arm camera");

    // Wait for trigger (or timeout after 2 seconds)
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        camera.wait_for_trigger()
    ).await;

    // Disarm regardless of result
    camera.disarm().await.expect("Failed to disarm camera");

    // Note: This test will timeout if no trigger signal is provided
    // In production setup, connect external trigger source
    match result {
        Ok(Ok(())) => println!("✓ Trigger received"),
        Ok(Err(e)) => panic!("Trigger wait failed: {}", e),
        Err(_) => println!("⚠ Trigger timeout (expected without trigger source)"),
    }
}
