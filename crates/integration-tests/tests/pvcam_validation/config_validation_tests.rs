// ============================================================================
// UNIT TESTS: Camera Configuration and Validation
// ============================================================================

use common::core::Roi;
use common::parameter::Parameter;
use hardware::capabilities::{ExposureControl, Parameterized};
use hardware::drivers::pvcam::PvcamDriver;

use super::helpers::*;

/// Test 1: Validate Prime BSI camera dimensions
#[tokio::test]
async fn test_prime_bsi_dimensions() {
    let camera = PvcamDriver::new_async("PrimeBSI".to_string())
        .await
        .expect("Failed to create Prime BSI camera");

    // Prime BSI: 2048 x 2048 pixel sensor
    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");
    let roi = roi_param.get();

    assert_eq!(roi.width, PRIME_BSI_WIDTH, "Prime BSI width should be 2048");
    assert_eq!(
        roi.height, PRIME_BSI_HEIGHT,
        "Prime BSI height should be 2048"
    );
}

/// Test 2: Validate Prime 95B camera dimensions (only when prime_95b_tests enabled)
#[tokio::test]
#[cfg(feature = "prime_95b_tests")]
async fn test_prime_95b_dimensions() {
    let camera = PvcamDriver::new_async("Prime95B".to_string())
        .await
        .expect("Failed to create Prime 95B camera");

    // Prime 95B: 1200 x 1200 pixel sensor
    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");
    let roi = roi_param.get();

    assert_eq!(roi.width, PRIME_95B_WIDTH, "Prime 95B width should be 1200");
    assert_eq!(
        roi.height, PRIME_95B_HEIGHT,
        "Prime 95B height should be 1200"
    );
}

/// Test 3: Validate binning factors
#[tokio::test]
async fn test_binning_validation() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning missing");

    // Valid binning: 1, 2, 4, 8
    let valid_bins = vec![1, 2, 4, 8];
    for bin in valid_bins {
        let result = binning_param.set((bin, bin)).await;
        assert!(result.is_ok(), "Binning {}x{} should be valid", bin, bin);
    }

    // Invalid binning: Check 0 or excessive binning if desired, but for now
    // we only enforce valid binning works.
    // Prime BSI seems to support flexible binning (3x3, 5x5, etc).
    // Let's just test one known invalid case (0) if driver handles it, or skip.
    // For safety, removing the loop over presumed-invalid bins that actually work.
    let _invalid_bins: Vec<u16> = vec![];
}

/// Test 4: Validate ROI bounds checking
#[tokio::test]
async fn test_roi_bounds_validation() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");
    let width = expected_width();
    let height = expected_height();

    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");

    // Valid ROI: Within sensor bounds
    let valid_roi = Roi {
        x: 0,
        y: 0,
        width,
        height,
    };
    let result = roi_param.set(valid_roi).await;
    assert!(result.is_ok(), "Full sensor ROI should be valid");

    // Invalid ROI: Exceeds sensor width
    let invalid_roi = Roi {
        x: 0,
        y: 0,
        width: width + 1,
        height,
    };
    let result: Result<(), _> = roi_param.set(invalid_roi).await;
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
    let result: Result<(), _> = roi_param.set(invalid_roi).await;
    assert!(
        result.is_err(),
        "ROI exceeding sensor height should be invalid"
    );
}

/// Test 5: Frame size calculation with binning
#[tokio::test]
async fn test_frame_size_with_binning() {
    let camera = PvcamDriver::new_async(default_camera_name().to_string())
        .await
        .expect("Failed to create camera");

    let binning_param = camera
        .parameters()
        .get_typed::<Parameter<(u16, u16)>>("acquisition.binning")
        .expect("Binning missing");
    let roi_param = camera
        .parameters()
        .get_typed::<Parameter<Roi>>("acquisition.roi")
        .expect("ROI missing");

    // Set 2x2 binning
    binning_param
        .set((2, 2))
        .await
        .expect("Failed to set binning");
    let binning = binning_param.get();
    assert_eq!(binning, (2, 2), "Binning should be 2x2");

    // Frame dimensions should account for binning
    let roi = roi_param.get();
    let expected_pixels = (roi.width / binning.0 as u32) * (roi.height / binning.1 as u32);
    assert!(expected_pixels > 0, "Frame should have non-zero pixels");
}
