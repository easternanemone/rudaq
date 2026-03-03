// ============================================================================
// MOCK INTEGRATION TESTS: Camera Operations
// ============================================================================

use common::core::Roi;
use common::parameter::Parameter;
use hardware::capabilities::{
    ExposureControl, Frame, FrameProducer, Parameterized, Readable, Triggerable,
};
use hardware::drivers::pvcam::PvcamDriver;
use std::time::Instant;

use super::helpers::*;

/// Test 6: Create default camera instance
#[tokio::test]
async fn test_create_default_camera() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI param missing");
    let roi = roi_param.get();
    assert_eq!(roi.width, expected_width());
    assert_eq!(roi.height, expected_height());
}

/// Test 7: Create Prime BSI camera instance explicitly
#[tokio::test]
async fn test_create_prime_bsi() {
    let camera = PvcamDriver::new_async("PrimeBSI".to_string())
        .await
        .expect("Failed to create Prime BSI camera");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI param missing");
    let roi = roi_param.get();
    assert_eq!(roi.width, PRIME_BSI_WIDTH);
    assert_eq!(roi.height, PRIME_BSI_HEIGHT);
}

/// Test 8: Create Prime 95B camera instance (only when prime_95b_tests enabled)
#[tokio::test]
#[cfg(feature = "prime_95b_tests")]
async fn test_create_prime_95b() {
    let camera = PvcamDriver::new_async("Prime95B".to_string())
        .await
        .expect("Failed to create Prime 95B camera");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI param missing");
    let roi = roi_param.get();
    assert_eq!(roi.width, PRIME_95B_WIDTH);
    assert_eq!(roi.height, PRIME_95B_HEIGHT);
}

/// Test 9: Set and get exposure time
#[tokio::test]
async fn test_exposure_control() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    // Set exposure to 50ms
    camera
        .set_exposure(0.050)
        .await
        .expect("Failed to set exposure");
    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    assert!((exposure - 0.050).abs() < 1e-6, "Exposure should be 50ms");

    // Change to 100ms
    camera
        .set_exposure(0.100)
        .await
        .expect("Failed to set exposure");
    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    assert!((exposure - 0.100).abs() < 1e-6, "Exposure should be 100ms");
}

/// Test 10: Set and get full sensor ROI
#[tokio::test]
async fn test_roi_full_sensor() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let roi = Roi {
        x: 0,
        y: 0,
        width: expected_width(),
        height: expected_height(),
    };

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI parameter not found");
    let result: Result<(), _> = roi_param.set(roi).await;
    result.expect("Failed to set ROI");
    let retrieved_roi = roi_param.get();

    assert_eq!(retrieved_roi.x, 0);
    assert_eq!(retrieved_roi.y, 0);
    assert_eq!(retrieved_roi.width, expected_width());
    assert_eq!(retrieved_roi.height, expected_height());
}

/// Test 11: Set and get quarter sensor ROI
#[tokio::test]
async fn test_roi_quarter_sensor() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let w = expected_width();
    let h = expected_height();

    // Center quarter ROI
    let roi = Roi {
        x: w / 4,
        y: h / 4,
        width: w / 2,
        height: h / 2,
    };

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI parameter not found");

    let result: Result<(), _> = roi_param.set(roi).await;
    result.expect("Failed to set ROI");
    let retrieved_roi = roi_param.get();

    assert_eq!(retrieved_roi.x, w / 4);
    assert_eq!(retrieved_roi.y, h / 4);
    assert_eq!(retrieved_roi.width, w / 2);
    assert_eq!(retrieved_roi.height, h / 2);
}

/// Test 12: Set and get 1x1 binning (no binning)
#[tokio::test]
async fn test_binning_1x1() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning parameter not found");
    let result: Result<(), _> = binning_param.set((1, 1)).await;
    result.expect("Failed to set binning");

    let binning = binning_param.get();
    assert_eq!(binning, (1, 1), "Binning should be 1x1");
}

/// Test 13: Set and get 2x2 binning
#[tokio::test]
async fn test_binning_2x2() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning parameter not found");
    let result: Result<(), _> = binning_param.set((2, 2)).await;
    result.expect("Failed to set binning");

    let binning = binning_param.get();
    assert_eq!(binning, (2, 2), "Binning should be 2x2");
}

/// Test 14: Set and get 4x4 binning
#[tokio::test]
async fn test_binning_4x4() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning parameter not found");
    let result: Result<(), _> = binning_param.set((4, 4)).await;
    result.expect("Failed to set binning");

    let binning = binning_param.get();
    assert_eq!(binning, (4, 4), "Binning should be 4x4");
}

/// Test 15: Invalid binning factor should fail
#[tokio::test]
async fn test_invalid_binning() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning parameter not found");

    // 3x3 binning is invalid (must be 1, 2, 4, or 8)
    let result: Result<(), _> = binning_param.set((3, 3)).await;
    assert!(result.is_err(), "Invalid binning should return error");
}

/// Test 16: ROI exceeding sensor bounds should fail
#[tokio::test]
async fn test_invalid_roi_exceeds_sensor() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI parameter not found");

    // ROI exceeds sensor
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: expected_width() + 100,
        height: expected_height(),
    };

    let result: Result<(), _> = roi_param.set(invalid_roi).await;
    assert!(result.is_err(), "ROI exceeding sensor should return error");
}

/// Test 17: Acquire single frame
#[tokio::test]
async fn test_acquire_single_frame() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    camera
        .set_exposure(0.010)
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
        frame.data.len(),
        (expected_width() * expected_height() * 2) as usize,
        "Frame buffer size should be width * height * 2 (16-bit)"
    );
}

/// Test 18: Frame data pattern validation
#[tokio::test]
async fn test_frame_data_pattern() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");

    // In mock mode, frame should contain non-zero data (test pattern)
    let non_zero_pixels = frame.data.iter().filter(|&&p| p != 0).count();
    assert!(
        non_zero_pixels > 0,
        "Mock frame should contain non-zero pixel data"
    );
}

/// Test 19: Arm and disarm triggering
#[tokio::test]
async fn test_arm_disarm_trigger() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    // Arm for triggering
    camera.arm().await.expect("Failed to arm camera");

    // Disarm
    let armed_param = camera
        .parameters()
        .get_typed::<Parameter<bool>>("acquisition.armed")
        .expect("Armed parameter not found");

    let result: Result<(), _> = armed_param.set(false).await;
    result.expect("Failed to disarm camera");
}

/// Test 20: Multiple frame acquisition
#[tokio::test]
async fn test_multiple_frames() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    camera
        .set_exposure(0.005)
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
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    // Short exposure for high frame rate
    camera
        .set_exposure(0.001)
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
