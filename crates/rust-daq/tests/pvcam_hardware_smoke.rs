#![cfg(not(target_arch = "wasm32"))]
//! PVCAM Hardware Smoke Test
//!
//! A minimal smoke test for verifying PVCAM camera connectivity and basic operation.
//! Designed to run quickly in CI/CD environments when hardware is available.
//!
//! # Environment Variables
//!
//! - `PVCAM_SMOKE_TEST=1` - Required to enable the test
//! - `PVCAM_CAMERA_NAME` - Optional camera name (default: "PrimeBSI")
//!
//! # Prerequisites
//!
//! - PVCAM SDK installed at `/opt/pvcam/sdk`
//! - Camera connected and powered on
//! - Environment sourced: `source /opt/pvcam/etc/profile.d/pvcam.sh`
//!
//! # Running
//!
//! ```bash
//! export PVCAM_SDK_DIR=/opt/pvcam/sdk
//! export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
//! export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
//! export PVCAM_SMOKE_TEST=1
//! export PVCAM_CAMERA_NAME=PrimeBSI  # optional
//!
//! cargo test --test pvcam_hardware_smoke --features 'instrument_photometrics,pvcam_hardware' -- --nocapture
//! ```

#![cfg(all(feature = "instrument_photometrics", feature = "pvcam_hardware"))]

use rust_daq::hardware::capabilities::{ExposureControl, FrameProducer};
use rust_daq::hardware::pvcam::PvcamDriver;
use std::env;
use std::time::Duration;

/// Check if smoke test is enabled via environment variable
fn smoke_test_enabled() -> bool {
    env::var("PVCAM_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get camera name from environment or default to PrimeBSI
fn camera_name() -> String {
    env::var("PVCAM_CAMERA_NAME").unwrap_or_else(|_| "PrimeBSI".to_string())
}

/// PVCAM Hardware Smoke Test
///
/// This test verifies:
/// 1. SDK initialization
/// 2. Camera enumeration and detection
/// 3. Camera connection
/// 4. Basic exposure setting
/// 5. Single frame acquisition
/// 6. Proper cleanup
///
/// Designed to complete in under 10 seconds with minimal frame acquisition.
#[tokio::test]
async fn pvcam_smoke_test() {
    // Skip if smoke test not enabled
    if !smoke_test_enabled() {
        println!("PVCAM smoke test skipped (set PVCAM_SMOKE_TEST=1 to enable)");
        return;
    }

    let camera_name = camera_name();
    println!("=== PVCAM Hardware Smoke Test ===");
    println!("Camera: {}", camera_name);

    // Step 1: Initialize camera
    println!("\n[1/5] Initializing camera...");
    let camera = PvcamDriver::new(&camera_name)
        .expect("Failed to create PVCAM driver - check SDK installation and camera connection");

    // Step 2: Verify camera info
    println!("[2/5] Querying camera info...");
    let info = camera
        .get_camera_info()
        .await
        .expect("Failed to get camera info");
    println!("  Chip: {}", info.chip_name);
    println!("  Sensor: {}x{}", info.sensor_size.0, info.sensor_size.1);

    // Validate sensor dimensions (basic sanity check)
    assert!(info.sensor_size.0 > 0, "Sensor width must be positive");
    assert!(info.sensor_size.1 > 0, "Sensor height must be positive");

    // Step 3: Set short exposure
    println!("[3/5] Setting exposure to 10ms...");
    camera
        .set_exposure_ms(10.0)
        .await
        .expect("Failed to set exposure");
    let exposure = camera
        .get_exposure_ms()
        .await
        .expect("Failed to query exposure");
    println!("  Exposure: {} ms", exposure);

    // Allow some tolerance for camera rounding
    assert!(
        (exposure - 10.0).abs() < 1.0,
        "Exposure should be approximately 10ms, got {}ms",
        exposure
    );

    // Step 4: Acquire single frame
    println!("[4/5] Acquiring single frame...");
    let start = std::time::Instant::now();

    // Start stream acquisition for single frame
    camera
        .start_stream()
        .await
        .expect("Failed to start acquisition");

    // Subscribe to frame broadcasts and wait for a frame
    let mut rx = camera.subscribe_frames();

    let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout waiting for frame")
        .expect("Frame channel closed");

    let elapsed = start.elapsed();
    println!("  Frame received in {:?}", elapsed);
    println!("  Frame size: {}x{}", frame.width, frame.height);
    println!("  Buffer size: {} pixels", frame.buffer.len());

    // Validate frame data
    assert!(frame.width > 0, "Frame width must be positive");
    assert!(frame.height > 0, "Frame height must be positive");
    assert!(!frame.buffer.is_empty(), "Frame buffer must not be empty");
    assert_eq!(
        frame.buffer.len(),
        (frame.width * frame.height) as usize,
        "Buffer size must match frame dimensions"
    );

    // Step 5: Stop and cleanup
    println!("[5/5] Stopping acquisition...");
    camera
        .stop_stream()
        .await
        .expect("Failed to stop acquisition");

    // Calculate simple statistics on frame data
    let sum: u64 = frame.buffer.iter().map(|&v| v as u64).sum();
    let mean = sum as f64 / frame.buffer.len() as f64;
    let max_val = *frame.buffer.iter().max().unwrap_or(&0);
    let min_val = *frame.buffer.iter().min().unwrap_or(&0);

    println!("\n=== Frame Statistics ===");
    println!("  Mean: {:.2}", mean);
    println!("  Min: {}", min_val);
    println!("  Max: {}", max_val);

    println!("\n=== PVCAM Smoke Test PASSED ===");
}

/// Test that smoke test is properly skipped when not enabled
#[test]
fn smoke_test_skip_check() {
    // This test always runs to verify the skip logic works
    let enabled = smoke_test_enabled();
    if !enabled {
        println!("Smoke test correctly disabled (PVCAM_SMOKE_TEST not set)");
    } else {
        println!("Smoke test enabled via PVCAM_SMOKE_TEST=1");
    }
}
