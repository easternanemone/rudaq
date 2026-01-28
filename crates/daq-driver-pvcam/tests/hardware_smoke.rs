#![cfg(not(target_arch = "wasm32"))]
//! PVCAM Hardware Smoke Test Suite
//!
//! Comprehensive smoke tests for verifying PVCAM camera connectivity and operation.
//! Designed to run in CI/CD environments when hardware is available.
//!
//! # Environment Variables
//!
//! Required:
//! - `PVCAM_SMOKE_TEST=1` - Enable the test suite
//! - `PVCAM_VERSION` - PVCAM library version (e.g., "7.1.1.118")
//! - `PVCAM_SDK_DIR` - Path to PVCAM SDK (e.g., "/opt/pvcam/sdk")
//! - `LIBRARY_PATH` - Must include PVCAM library path for linker
//! - `LD_LIBRARY_PATH` - Must include PVCAM library path for runtime
//!
//! Optional:
//! - `PVCAM_CAMERA_NAME` - Camera name (default: "PrimeBSI")
//!
//! # Quick Setup
//!
//! ```bash
//! source /etc/profile.d/pvcam.sh
//! export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
//! export PVCAM_SMOKE_TEST=1
//! ```
//!
//! # Running
//!
//! ```bash
//! cargo test -p daq-driver-pvcam --test hardware_smoke --features pvcam_sdk -- --nocapture --test-threads=1
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `pvcam_smoke_test` | Basic connectivity, exposure, single frame |
//! | `pvcam_camera_info_test` | Camera metadata (resolution) |
//! | `pvcam_multiple_frames_test` | Acquire 5 consecutive frames |
//! | `pvcam_streaming_test` | Continuous streaming for 1 second |
//! | `pvcam_exposure_range_test` | Test various exposure times |
//! | `pvcam_frame_statistics_test` | Validate pixel data statistics |

#![cfg(feature = "pvcam_sdk")]

use common::capabilities::{ExposureControl, FrameProducer};
use daq_driver_pvcam::PvcamDriver;
use std::env;
use std::time::{Duration, Instant};

// =============================================================================
// Test Configuration
// =============================================================================

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

/// Skip test with message if smoke test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("PVCAM smoke test skipped (set PVCAM_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Test 1: Basic Smoke Test
// =============================================================================

/// PVCAM Hardware Smoke Test
///
/// This test verifies:
/// 1. SDK initialization
/// 2. Camera enumeration and detection
/// 3. Camera connection
/// 4. Basic exposure setting
/// 5. Single frame acquisition
/// 6. Proper cleanup
#[tokio::test]
async fn pvcam_smoke_test() {
    skip_if_disabled!();

    let camera_name = camera_name();
    println!("=== PVCAM Hardware Smoke Test ===");
    println!("Camera: {}", camera_name);

    // Step 1: Initialize camera
    println!("\n[1/5] Initializing camera...");
    let camera = PvcamDriver::new_async(camera_name.clone())
        .await
        .expect("Failed to create PVCAM driver - check SDK installation and camera connection");

    // Step 2: Verify camera resolution
    println!("[2/5] Querying camera resolution...");
    let (sensor_width, sensor_height) = camera.resolution();
    println!("  Sensor: {}x{}", sensor_width, sensor_height);

    assert!(sensor_width > 0, "Sensor width must be positive");
    assert!(sensor_height > 0, "Sensor height must be positive");

    // Step 3: Set short exposure
    println!("[3/5] Setting exposure to 10ms...");
    camera
        .set_exposure(0.010)
        .await
        .expect("Failed to set exposure (s)");
    let exposure = camera
        .get_exposure()
        .await
        .expect("Failed to query exposure (s)");
    let exposure_ms = exposure * 1000.0;
    println!("  Exposure: {:.3} ms", exposure_ms);

    assert!(
        (exposure_ms - 10.0).abs() < 1.0,
        "Exposure should be approximately 10ms, got {}ms",
        exposure_ms
    );

    // Step 4: Acquire single frame
    println!("[4/5] Acquiring single frame (one-shot)...");
    let start = Instant::now();
    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire single frame");

    let elapsed = start.elapsed();
    println!("  Frame received in {:?}", elapsed);
    println!("  Frame size: {}x{}", frame.width, frame.height);

    let pixels: Vec<u16> = match frame.bit_depth {
        16 => frame
            .data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect(),
        _ => frame.data.iter().map(|&b| b as u16).collect(),
    };

    println!("  Buffer size: {} pixels", pixels.len());

    assert!(frame.width > 0, "Frame width must be positive");
    assert!(frame.height > 0, "Frame height must be positive");
    assert!(
        pixels.len() == (frame.width * frame.height) as usize,
        "Pixel buffer size must match frame dimensions"
    );

    // Step 5: Cleanup
    println!("[5/5] Ensuring acquisition stopped...");
    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    // Calculate frame statistics
    let sum: u64 = pixels.iter().map(|&v| v as u64).sum();
    let mean = sum as f64 / pixels.len() as f64;
    let max_val = *pixels.iter().max().unwrap_or(&0);
    let min_val = *pixels.iter().min().unwrap_or(&0);

    println!("\n=== Frame Statistics ===");
    println!("  Mean: {:.2}", mean);
    println!("  Min: {}", min_val);
    println!("  Max: {}", max_val);

    println!("\n=== PVCAM Smoke Test PASSED ===");
}

// =============================================================================
// Test 2: Camera Info
// =============================================================================

/// Test camera info retrieval (resolution, connection)
#[tokio::test]
async fn pvcam_camera_info_test() {
    skip_if_disabled!();

    println!("=== PVCAM Camera Info Test ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    let (width, height) = camera.resolution();

    println!("Camera Info:");
    println!("  Resolution: {}x{}", width, height);
    println!("  Total pixels: {}", width * height);
    println!("  Megapixels: {:.2}", (width * height) as f64 / 1_000_000.0);

    // For Prime BSI, expect 2048x2048
    assert!(width >= 1024, "Width should be at least 1024");
    assert!(height >= 1024, "Height should be at least 1024");

    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    println!("\n=== Camera Info Test PASSED ===");
}

// =============================================================================
// Test 3: Multiple Frame Acquisition
// =============================================================================

/// Test acquiring multiple consecutive frames
#[tokio::test]
async fn pvcam_multiple_frames_test() {
    skip_if_disabled!();

    println!("=== PVCAM Multiple Frames Test ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    // Set short exposure for fast acquisition
    camera
        .set_exposure(0.005) // 5ms
        .await
        .expect("Failed to set exposure");

    let num_frames = 5;
    println!("Acquiring {} consecutive frames...", num_frames);

    let start = Instant::now();
    let mut frame_times = Vec::new();

    for i in 0..num_frames {
        let frame_start = Instant::now();
        let frame = camera
            .acquire_frame()
            .await
            .unwrap_or_else(|e| panic!("Failed to acquire frame {}: {}", i + 1, e));

        let frame_time = frame_start.elapsed();
        frame_times.push(frame_time);

        println!(
            "  Frame {}: {}x{} in {:?}",
            i + 1,
            frame.width,
            frame.height,
            frame_time
        );

        assert!(
            frame.width > 0 && frame.height > 0,
            "Frame dimensions must be positive"
        );
    }

    let total_time = start.elapsed();
    let avg_time: Duration = frame_times.iter().sum::<Duration>() / num_frames as u32;

    println!("\nSummary:");
    println!("  Total time: {:?}", total_time);
    println!("  Average frame time: {:?}", avg_time);
    println!(
        "  Effective frame rate: {:.2} fps",
        num_frames as f64 / total_time.as_secs_f64()
    );

    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    println!("\n=== Multiple Frames Test PASSED ===");
}

// =============================================================================
// Test 4: Continuous Streaming
// =============================================================================

/// Test continuous streaming for a short duration
#[tokio::test]
async fn pvcam_streaming_test() {
    skip_if_disabled!();

    println!("=== PVCAM Streaming Test ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    // Ensure clean state before starting (stop any lingering stream)
    let _ = camera.stop_stream().await;

    // Set short exposure for high frame rate
    camera
        .set_exposure(0.010) // 10ms
        .await
        .expect("Failed to set exposure");

    // Register output BEFORE starting
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    camera
        .register_primary_output(tx)
        .await
        .expect("Failed to register output");

    // Start streaming
    println!("Starting continuous streaming...");
    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    // Collect frames for ~1 second
    let stream_duration = Duration::from_secs(1);
    let start = Instant::now();
    let mut frame_count = 0u32;

    while start.elapsed() < stream_duration {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(_frame)) => {
                frame_count += 1;
            }
            Ok(None) => {
                println!("  Channel closed");
                break;
            }
            Err(_) => {
                println!("  Timeout waiting for frame");
                break;
            }
        }
    }

    let elapsed = start.elapsed();

    // Stop streaming
    println!("Stopping streaming...");
    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    println!("\nStreaming Results:");
    println!("  Duration: {:?}", elapsed);
    println!("  Frames captured: {}", frame_count);
    println!(
        "  Frame rate: {:.2} fps",
        frame_count as f64 / elapsed.as_secs_f64()
    );

    assert!(
        frame_count > 0,
        "Should have captured at least one frame during streaming"
    );

    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    println!("\n=== Streaming Test PASSED ===");
}

// =============================================================================
// Test 5: Exposure Range
// =============================================================================

/// Test setting various exposure times
#[tokio::test]
async fn pvcam_exposure_range_test() {
    skip_if_disabled!();

    println!("=== PVCAM Exposure Range Test ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    let test_exposures_ms = [1.0, 10.0, 50.0, 100.0, 500.0];

    for &exp_ms in &test_exposures_ms {
        let exp_s = exp_ms / 1000.0;
        camera
            .set_exposure(exp_s)
            .await
            .unwrap_or_else(|e| panic!("Failed to set exposure to {}ms: {}", exp_ms, e));

        let readback = camera
            .get_exposure()
            .await
            .expect("Failed to read exposure");
        let readback_ms = readback * 1000.0;

        println!("  Set: {}ms, Read: {:.3}ms", exp_ms, readback_ms);

        // Allow 10% tolerance or 1ms minimum
        let tolerance = (exp_ms * 0.1).max(1.0);
        assert!(
            (readback_ms - exp_ms).abs() < tolerance,
            "Exposure should be approximately {}ms, got {}ms",
            exp_ms,
            readback_ms
        );
    }

    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    println!("\n=== Exposure Range Test PASSED ===");
}

// =============================================================================
// Test 6: Frame Statistics
// =============================================================================

/// Test frame data statistics (validates pixel data is reasonable)
#[tokio::test]
async fn pvcam_frame_statistics_test() {
    skip_if_disabled!();

    println!("=== PVCAM Frame Statistics Test ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    // Ensure clean state before starting (stop any lingering stream)
    let _ = camera.stop_stream().await;

    // Set moderate exposure
    camera
        .set_exposure(0.020)
        .await
        .expect("Failed to set exposure");

    // Use streaming pattern for more robust callback registration
    // Register output BEFORE starting (critical for callback timing)
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    camera
        .register_primary_output(tx)
        .await
        .expect("Failed to register output");

    // Start streaming
    println!("Starting stream for frame capture...");
    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    // Small delay to ensure callback is fully registered
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Wait for first frame with longer timeout (10s) for robust callback capture
    let frame = match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        Ok(Some(frame)) => frame,
        Ok(None) => {
            let _ = camera.stop_stream().await;
            let _ = camera.close().await;
            panic!("Channel closed before receiving frame");
        }
        Err(_) => {
            let _ = camera.stop_stream().await;
            let _ = camera.close().await;
            panic!("Timed out waiting for frame (10s) - EOF callback may not be firing");
        }
    };

    // Stop streaming immediately after getting frame
    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    // Convert to u16 pixels
    let pixels: Vec<u16> = frame
        .data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Calculate statistics
    let sum: u64 = pixels.iter().map(|&v| v as u64).sum();
    let mean = sum as f64 / pixels.len() as f64;
    let min_val = *pixels.iter().min().unwrap_or(&0);
    let max_val = *pixels.iter().max().unwrap_or(&0);

    // Calculate standard deviation
    let variance: f64 = pixels
        .iter()
        .map(|&v| {
            let diff = v as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / pixels.len() as f64;
    let std_dev = variance.sqrt();

    println!("Frame: {}x{}", frame.width, frame.height);
    println!("Statistics:");
    println!("  Mean: {:.2}", mean);
    println!("  Std Dev: {:.2}", std_dev);
    println!("  Min: {}", min_val);
    println!("  Max: {}", max_val);
    println!("  Dynamic Range: {}", max_val - min_val);

    // Validate statistics are reasonable
    assert!(mean > 0.0, "Mean should be positive (not a blank frame)");
    assert!(max_val > min_val, "Should have some dynamic range");
    assert!(max_val < 65535, "Should not be saturated");

    // Explicitly close to release camera handle before next test
    let _ = camera.close().await;

    println!("\n=== Frame Statistics Test PASSED ===");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that smoke test is properly skipped when not enabled
#[test]
fn smoke_test_skip_check() {
    let enabled = smoke_test_enabled();
    if !enabled {
        println!("Smoke test correctly disabled (PVCAM_SMOKE_TEST not set)");
    } else {
        println!("Smoke test enabled via PVCAM_SMOKE_TEST=1");
    }
}
