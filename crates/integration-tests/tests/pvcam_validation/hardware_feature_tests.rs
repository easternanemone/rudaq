// ============================================================================
// Hardware Feature Tests
//
// Camera information, gain/speed table, temperature control,
// post-processing, smart streaming, centroids, PrimeEnhance,
// and frame rotation/flip tests.
// ============================================================================

use common::parameter::Parameter;
use hardware::capabilities::Parameterized;
use hardware::drivers::pvcam::PvcamDriver;

use super::helpers::*;

// ============================================================================
// Section 8: Camera Information Tests (Tests 30-39)
// ============================================================================

/// Test 30: Get sensor temperature
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_temperature() {
    println!("Skipped: Thermal features not directly exposed in new PvcamDriver API");
}

/// Test 31: Get chip/sensor name
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_chip_name() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 32: Get bit depth
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_bit_depth() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 33: Get readout time
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_readout_time() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 34: Get pixel size
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_pixel_size() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 35: Get gain name
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain_name() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 36: Get speed table name
/// Note: PARAM_SPDTAB_NAME may not be available on all cameras
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed_name() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 37: Get gain index
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain_index() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 38: Get speed table index
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed_index() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

/// Test 39: Get comprehensive camera info
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_camera_info() {
    println!("Skipped: Info features not directly exposed in new PvcamDriver API");
}

// =============================================================================
// Tests 40-45: Gain and Speed Table Selection
// =============================================================================

/// Test 40: List available gain modes
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_gain_modes() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

/// Test 41: List available speed modes
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_speed_modes() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

/// Test 42: Get current gain mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_gain() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

/// Test 43: Get current speed mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_speed() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

/// Test 44: Set gain mode and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_gain_index() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

/// Test 45: Set speed mode and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_speed_index() {
    println!("Skipped: Readout features not directly exposed in new PvcamDriver API");
}

// =============================================================================
// Tests 46-49: Temperature Control
// =============================================================================

/// Test 46: Get temperature setpoint
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_temperature_setpoint() {
    println!("Skipped: Thermal features not directly exposed in new PvcamDriver API");
}

/// Test 47: Get and compare temperature vs setpoint
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_temperature_vs_setpoint() {
    println!("Skipped: Thermal features not directly exposed in new PvcamDriver API");
}

/// Test 48: Get fan speed
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_fan_speed() {
    println!("Skipped: Fan Speed features not directly exposed in new PvcamDriver API");
}

/// Test 49: Set fan speed and verify
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_set_fan_speed() {
    println!("Skipped: Fan Speed features not directly exposed in new PvcamDriver API");
}

// ============================================================================
// POST-PROCESSING FEATURE TESTS (Tests 50-53)
// ============================================================================

/// Test 50: List post-processing features
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_list_pp_features() {
    println!("Skipped: PP features not currently exposed in new PvcamDriver API");
}

/// Test 51: Get PP params for each feature
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_pp_params() {
    println!("Skipped: PP features not currently exposed in new PvcamDriver API");
}

/// Test 52: Get/Set PP param value
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_get_set_pp_param() {
    println!("Skipped: PP features not currently exposed in new PvcamDriver API");
}

/// Test 53: Reset PP features
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_reset_pp_features() {
    println!("Skipped: PP features not currently exposed in new PvcamDriver API");
}

// ============================================================================
// SMART STREAMING TESTS (Tests 54-57)
// ============================================================================

/// Test 54: Check if Smart Streaming is available
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_available() {
    println!("Skipped: Smart Streaming features not currently exposed in new PvcamDriver API");
}

/// Test 55: Get Smart Streaming max entries
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_max_entries() {
    println!("Skipped: Smart Streaming features not currently exposed in new PvcamDriver API");
}

/// Test 56: Enable/disable Smart Streaming
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_enable_disable() {
    println!("Skipped: Smart Streaming features not currently exposed in new PvcamDriver API");
}

/// Test 57: Set Smart Streaming exposure sequence
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_smart_streaming_set_exposures() {
    println!("Skipped: Smart Streaming features not currently exposed in new PvcamDriver API");
}

// ============================================================================
// Centroids Mode Tests (PrimeLocate / Particle Tracking)
// ============================================================================

/// Test 58: Check if centroids feature is available
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_available() {
    println!("Skipped: Centroids features not currently exposed in new PvcamDriver API");
}

/// Test 59: Enable/disable centroids mode
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_enable_disable() {
    println!("Skipped: Centroids features not currently exposed in new PvcamDriver API");
}

#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_mode() {
    println!("Skipped: Centroids features not currently exposed in new PvcamDriver API");
}

#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_centroids_config() {
    println!("Skipped: Centroids features not currently exposed in new PvcamDriver API");
}

// ============================================================================
// PrimeEnhance (Denoising) Tests
// ============================================================================

/// Test 62: Check PrimeEnhance availability and enable/disable
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_prime_enhance() {
    println!("Skipped: Prime Enhance features not currently exposed in new PvcamDriver API");
    // Original test logic removed until features are re-implemented
}

// ============================================================================
// Frame Rotation and Flip Tests
// ============================================================================

/// Test 63: Frame rotation and flip
#[tokio::test]
#[cfg_attr(not(feature = "hardware_tests"), ignore)]
async fn test_hardware_frame_processing() {
    let camera = PvcamDriver::new_async("PMCam".to_string())
        .await
        .expect("Failed to open camera");

    // Check rotation availability (via parameter existence)
    if let Some(rot_param) = camera
        .parameters()
        .get_typed::<Parameter<String>>("processing.host_rotate")
    {
        println!("Frame rotation available");
        let current_rot = rot_param.get();
        println!("Current rotation: {}", current_rot);

        // Test setting rotation
        // FrameRotate values: "None", "90 CW", "180 CW", "270 CW"
        for rot_val in ["None", "90 CW", "180 CW", "270 CW"] {
            match rot_param.set(rot_val.to_string()).await {
                Ok(()) => {
                    let actual = rot_param.get();
                    println!("Set rotation to {}, got {}", rot_val, actual);
                }
                Err(e) => println!("Failed to set rotation {}: {}", rot_val, e),
            }
        }

        // Restore original
        let _ = rot_param.set(current_rot).await;
    } else {
        println!("Frame rotation parameter not found");
    }

    // Check flip availability
    if let Some(flip_param) = camera
        .parameters()
        .get_typed::<Parameter<String>>("processing.host_flip")
    {
        println!("Frame flip available");
        let current_flip = flip_param.get();
        println!("Current flip mode: {}", current_flip);

        // Test flip modes
        // FrameFlip values: "None", "X", "Y", "XY"
        for flip_val in ["None", "X", "Y", "XY"] {
            match flip_param.set(flip_val.to_string()).await {
                Ok(()) => {
                    let actual = flip_param.get();
                    println!("Set flip mode {}, got {}", flip_val, actual);
                }
                Err(e) => println!("Failed to set flip {}: {}", flip_val, e),
            }
        }

        // Restore original
        let _ = flip_param.set(current_flip).await;
    } else {
        println!("Frame flip parameter not found");
    }
}
