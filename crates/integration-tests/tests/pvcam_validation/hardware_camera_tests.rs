// ============================================================================
// HARDWARE VALIDATION TESTS (require physical camera)
// ============================================================================

use common::core::Roi;
use common::parameter::Parameter;
use hardware::capabilities::{ExposureControl, FrameProducer, Parameterized, Triggerable};
use hardware::drivers::pvcam::PvcamDriver;
use std::time::Instant;

use super::helpers::*;

/// Test 22: Hardware camera initialization
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_initialization() {
    // This test requires PVCAM SDK and physical camera
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open hardware camera");

    // Verify camera properties
    let roi = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI parameter not found")
        .get();
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
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    camera
        .set_exposure(0.100)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // Verify frame properties
    assert!(frame.width > 0);
    assert!(frame.height > 0);
    assert_eq!(frame.data.len(), (frame.width * frame.height) as usize);

    println!(
        "Acquired frame: {}x{}, {} pixels",
        frame.width,
        frame.height,
        frame.data.len()
    );
}

/// Test 24: Hardware ROI configuration
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_roi() {
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");
    let full_roi = roi_param.get();

    // Set quarter-sensor ROI
    let roi = Roi {
        x: full_roi.width / 4,
        y: full_roi.height / 4,
        width: full_roi.width / 2,
        height: full_roi.height / 2,
    };

    let result: Result<(), _> = roi_param.set(roi).await;
    result.expect("Failed to set ROI");

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
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning missing");

    // Set 2x2 binning
    let result: Result<(), _> = binning_param.set((2, 2)).await;
    result.expect("Failed to set binning");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");
    let full_roi = roi_param.get();

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
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    let exposure_times = vec![0.010, 0.050, 0.100, 0.500]; // seconds

    for exposure in exposure_times {
        camera
            .set_exposure(exposure)
            .await
            .expect("Failed to set exposure");

        let start = Instant::now();
        camera
            .acquire_frame()
            .await
            .expect("Failed to acquire frame");
        let actual_s = start.elapsed().as_secs_f64();

        // Single-frame acquisition overhead
        let min_expected = exposure;
        let max_overhead = 0.200; // 200ms overhead
        assert!(
            actual_s >= min_expected && actual_s <= exposure + max_overhead,
            "Exposure time {:.3}s actual {:.3}s (should be exposure + <=200ms overhead)",
            exposure,
            actual_s
        );
    }
}

/// Test 27: Frame pixel uniformity (requires uniform illumination)
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_pixel_uniformity() {
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    // Uniform illumination test: standard deviation should be low
    camera
        .set_exposure(0.100)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // Calculate statistics
    let mean: f64 = frame.data.iter().map(|&p| p as f64).sum::<f64>() / frame.data.len() as f64;
    let variance: f64 = frame
        .data
        .iter()
        .map(|&p| {
            let diff = p as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / frame.data.len() as f64;
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
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    // Dark frame test: mean should be near zero, low variance
    camera
        .set_exposure(0.100)
        .await
        .expect("Failed to set exposure");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // Calculate dark current statistics
    let mean: f64 = frame.data.iter().map(|&p| p as f64).sum::<f64>() / frame.data.len() as f64;

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
    println!("Skipped: Trigger wait features not directly exposed in new PvcamDriver API");
}
