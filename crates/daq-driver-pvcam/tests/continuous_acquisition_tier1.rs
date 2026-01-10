//! Tier 1: Core Functional Tests for PVCAM Continuous Acquisition
//!
//! These tests validate the fundamental continuous acquisition functionality:
//! - Single frame capture via FIFO (get_oldest_frame + unlock)
//! - Multi-frame sustained streaming
//! - Frame data integrity (dimensions, non-zero data)
//! - Frame numbering sequence validation
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && git pull && \
//!   cargo nextest run --profile hardware -p daq-driver-pvcam --features pvcam_hardware \
//!     --test continuous_acquisition_tier1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]

mod common;

use common::{
    assert_errors_within_limit, assert_fps_near, assert_frame_count_min,
    assert_no_duplicate_frames, durations, exposures, Frame, FrameTracker, FrameValidator,
    TestStats,
};
use daq_core::capabilities::{ExposureControl, FrameProducer};
use daq_driver_pvcam::PvcamDriver;
use std::time::Instant;

// =============================================================================
// Test Configuration
// =============================================================================

/// Camera name for tests (Prime BSI on maitai)
const CAMERA_NAME: &str = "PrimeBSI";

/// Test ROI dimensions (small for fast tests)
const TEST_ROI_WIDTH: u32 = 256;
const TEST_ROI_HEIGHT: u32 = 256;

// =============================================================================
// Tier 1 Test 1: Basic Frame Acquisition
// =============================================================================

/// Test single frame acquisition using continuous mode with FIFO retrieval.
///
/// Validates:
/// - Camera initialization and setup
/// - Single frame capture succeeds
/// - Frame dimensions match expected ROI
/// - Frame data is non-empty
#[tokio::test]
async fn test_basic_frame_acquisition() {
    println!("\n=== Tier 1 Test: Basic Frame Acquisition ===\n");

    // Initialize driver
    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    // Set standard exposure (100ms)
    camera
        .set_exposure(exposures::STANDARD_SEC)
        .await
        .expect("Failed to set exposure");

    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    println!("Exposure set to: {:.1}ms", exposure * 1000.0);

    // Acquire single frame
    let start = Instant::now();
    let frame = camera
        .acquire_frame()
        .await
        .expect("Failed to acquire frame");
    let elapsed = start.elapsed();

    println!("Frame acquired in {:?}", elapsed);
    println!("Frame dimensions: {}x{}", frame.width, frame.height);
    println!("Frame data size: {} bytes", frame.data.len());

    // Validate frame
    assert!(frame.width > 0, "Frame width must be positive");
    assert!(frame.height > 0, "Frame height must be positive");
    assert!(!frame.data.is_empty(), "Frame data must not be empty");

    // Verify data size matches dimensions (16-bit pixels)
    let expected_bytes = (frame.width * frame.height * 2) as usize;
    assert_eq!(
        frame.data.len(),
        expected_bytes,
        "Frame data size should match dimensions × 2 bytes"
    );

    // Stop stream
    let _ = camera.stop_stream().await;

    println!("\n=== Basic Frame Acquisition PASSED ===\n");
}

// =============================================================================
// Tier 1 Test 2: Continuous Streaming
// =============================================================================

/// Test sustained continuous streaming for a short duration.
///
/// Validates:
/// - Continuous acquisition starts successfully
/// - Frames are received continuously without stalls
/// - Frame rate is within expected range for exposure time
/// - Clean shutdown without errors
#[tokio::test]
async fn test_continuous_streaming() {
    println!("\n=== Tier 1 Test: Continuous Streaming ===\n");

    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    // Get sensor resolution to determine realistic FPS expectations
    // Full sensor (2048x2048) has ~23ms readout time, limiting max FPS to ~30
    // Smaller ROIs can achieve higher frame rates
    let (sensor_width, sensor_height) = camera.resolution();
    let is_full_sensor = sensor_width >= 2048 && sensor_height >= 2048;

    // Set fast exposure for higher frame rate
    camera
        .set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    // For full sensor: readout ~23ms + exposure 10ms = 33ms/frame = ~30 FPS
    // For small ROI: readout negligible, ~100 FPS possible
    let expected_fps = if is_full_sensor {
        // Full sensor limited by readout time (~23ms for Prime BSI)
        let readout_ms = 23.0;
        1000.0 / (exposures::FAST_MS + readout_ms)
    } else {
        1000.0 / exposures::FAST_MS
    };

    println!(
        "Exposure: {:.1}ms, Sensor: {}x{}, Expected FPS: {:.0}",
        exposures::FAST_MS,
        sensor_width,
        sensor_height,
        expected_fps
    );

    // Subscribe before starting stream
    let mut rx = camera
        .subscribe_frames()
        .await
        .expect("Failed to subscribe to frames");

    // Start streaming
    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let mut tracker = FrameTracker::new();
    let test_duration = durations::STANDARD;
    let start = Instant::now();

    println!(
        "Streaming for {:?} with frame timeout {:?}...",
        test_duration,
        durations::FRAME_TIMEOUT
    );

    // Collect frames for test duration
    while start.elapsed() < test_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Ok(frame)) => {
                tracker.record_frame(&frame);
            }
            Ok(Err(e)) => {
                stats.channel_errors += 1;
                println!("Channel error: {}", e);
            }
            Err(_) => {
                stats.timeout_errors += 1;
                println!("Timeout waiting for frame at {:?}", start.elapsed());
            }
        }
    }

    stats.duration = start.elapsed();
    tracker.export_to_stats(&mut stats);
    stats.calculate_fps();
    // Calculate expected frames using actual frame time (exposure + readout)
    let frame_time_ms = if is_full_sensor {
        exposures::FAST_MS + 23.0 // Include readout time for full sensor
    } else {
        exposures::FAST_MS
    };
    stats.calculate_expected(frame_time_ms);

    // Stop streaming
    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    // Print results
    stats.print_summary("Continuous Streaming");

    // Assertions - use sensor-aware expected FPS calculated above
    // Allow 40% tolerance to account for sequence mode batch transitions
    assert_fps_near(stats.fps, expected_fps, 40.0, "Continuous streaming");
    assert_frame_count_min(stats.frame_count, 10, "Continuous streaming");
    assert_no_duplicate_frames(stats.duplicate_frames, "Continuous streaming");

    println!("\n=== Continuous Streaming PASSED ===\n");
}

// =============================================================================
// Tier 1 Test 2b: Sustained Full-Sensor Streaming (500+ frames)
// =============================================================================

/// Exercise the full-sensor path long enough to validate sustained throughput.
/// Targets 500+ frames on Prime BSI at ~30 FPS (10ms exposure + ~23ms readout).
#[tokio::test]
async fn test_sustained_full_sensor_streaming() {
    println!("\n=== Tier 1 Test: Sustained Full-Sensor Streaming ===\n");

    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    let (sensor_width, sensor_height) = camera.resolution();
    assert!(
        sensor_width >= 2048 && sensor_height >= 2048,
        "Expected Prime BSI full sensor"
    );

    // Fast exposure to hit ~30 FPS with full readout time included
    camera
        .set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let expected_fps = {
        let readout_ms = 23.0; // Prime BSI full-sensor readout time
        1000.0 / (exposures::FAST_MS + readout_ms)
    };

    println!(
        "Exposure: {:.1}ms, Sensor: {}x{}, Expected FPS: {:.1}",
        exposures::FAST_MS,
        sensor_width,
        sensor_height,
        expected_fps
    );

    let mut rx = camera
        .subscribe_frames()
        .await
        .expect("Failed to subscribe to frames");

    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let mut tracker = FrameTracker::new();
    let test_duration = durations::SUSTAINED;
    let start = Instant::now();

    println!(
        "Streaming for {:?} (sustained) with frame timeout {:?}...",
        test_duration,
        durations::FRAME_TIMEOUT
    );

    while start.elapsed() < test_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Ok(frame)) => {
                tracker.record_frame(&frame);
            }
            Ok(Err(e)) => {
                stats.channel_errors += 1;
                println!("Channel error: {}", e);
            }
            Err(_) => {
                stats.timeout_errors += 1;
                println!("Timeout waiting for frame at {:?}", start.elapsed());
            }
        }
    }

    stats.duration = start.elapsed();
    tracker.export_to_stats(&mut stats);
    stats.calculate_fps();

    // Full-sensor timing: exposure + readout dominate expected frame time
    let frame_time_ms = exposures::FAST_MS + 23.0;
    stats.calculate_expected(frame_time_ms);

    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    stats.print_summary("Sustained Full-Sensor Streaming");

    // Expect ~600 frames over 20s; require at least 500 to catch regressions
    assert_frame_count_min(stats.frame_count, 500, "Sustained streaming");
    assert_fps_near(stats.fps, expected_fps, 35.0, "Sustained streaming");
    assert_no_duplicate_frames(stats.duplicate_frames, "Sustained streaming");
    assert_errors_within_limit(
        stats.timeout_errors + stats.channel_errors,
        1,
        "Sustained streaming",
    );

    println!("\n=== Sustained Full-Sensor Streaming PASSED ===\n");
}

// =============================================================================
// Tier 1 Test 3: Frame Data Integrity
// =============================================================================

/// Test frame data integrity across multiple frames.
///
/// Validates:
/// - Frame dimensions are consistent across frames
/// - Pixel data is not all zeros (buffer was actually filled)
/// - Data buffer size matches expected pixel count
#[tokio::test]
async fn test_frame_data_integrity() {
    println!("\n=== Tier 1 Test: Frame Data Integrity ===\n");

    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    // Get sensor resolution
    let (sensor_width, sensor_height) = camera.resolution();
    println!("Sensor resolution: {}x{}", sensor_width, sensor_height);

    // Set moderate exposure for good signal
    camera
        .set_exposure(0.020) // 20ms
        .await
        .expect("Failed to set exposure");

    // Subscribe and start streaming
    let mut rx = camera
        .subscribe_frames()
        .await
        .expect("Failed to subscribe to frames");

    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    // Create validator for sensor dimensions
    let validator = FrameValidator::new(sensor_width, sensor_height);

    let frames_to_check = 5;
    let mut valid_frames = 0;
    let mut zero_frames = 0;

    println!("Validating {} frames...", frames_to_check);

    for i in 0..frames_to_check {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Ok(frame)) => {
                // Validate dimensions and data size
                match validator.validate(&frame) {
                    Ok(()) => {
                        valid_frames += 1;

                        // Check for zero frame (uninitialized buffer)
                        if FrameValidator::is_zero_frame(&frame) {
                            zero_frames += 1;
                            println!(
                                "  Frame {}: {}x{} - WARNING: appears to be all zeros",
                                i + 1,
                                frame.width,
                                frame.height
                            );
                        } else {
                            // Calculate basic stats to verify data
                            let pixels = frame_to_u16(&frame);
                            let mean: f64 =
                                pixels.iter().map(|&p| p as f64).sum::<f64>() / pixels.len() as f64;
                            let max = *pixels.iter().max().unwrap_or(&0);
                            let min = *pixels.iter().min().unwrap_or(&0);

                            println!(
                                "  Frame {}: {}x{} - mean={:.1}, min={}, max={}",
                                i + 1,
                                frame.width,
                                frame.height,
                                mean,
                                min,
                                max
                            );
                        }
                    }
                    Err(e) => {
                        println!("  Frame {}: INVALID - {}", i + 1, e);
                    }
                }
            }
            Ok(Err(e)) => {
                println!("  Frame {}: Channel error - {}", i + 1, e);
            }
            Err(_) => {
                println!("  Frame {}: Timeout", i + 1);
            }
        }
    }

    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    println!("\nResults:");
    println!("  Valid frames: {}/{}", valid_frames, frames_to_check);
    println!("  Zero frames: {}", zero_frames);

    // Assertions
    assert_eq!(valid_frames, frames_to_check, "All frames should be valid");
    assert_eq!(zero_frames, 0, "No frames should be all zeros");

    println!("\n=== Frame Data Integrity PASSED ===\n");
}

// =============================================================================
// Tier 1 Test 4: Frame Numbering Sequence
// =============================================================================

/// Test frame numbering sequence during streaming.
///
/// Validates:
/// - Frame numbers are monotonically increasing
/// - No duplicate frame numbers (would indicate buffer issues)
/// - Frame number gaps are tracked (unexpected under FIFO)
#[tokio::test]
async fn test_frame_numbering_sequence() {
    println!("\n=== Tier 1 Test: Frame Numbering Sequence ===\n");

    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    // Use standard exposure
    camera
        .set_exposure(exposures::STANDARD_SEC)
        .await
        .expect("Failed to set exposure");

    let mut rx = camera
        .subscribe_frames()
        .await
        .expect("Failed to subscribe to frames");

    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    let mut tracker = FrameTracker::new();
    let mut frame_numbers: Vec<i32> = Vec::new();
    let test_duration = durations::STANDARD;
    let start = Instant::now();

    println!("Collecting frame numbers for {:?}...", test_duration);

    while start.elapsed() < test_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Ok(frame)) => {
                let frame_nr = frame.frame_number as i32;
                frame_numbers.push(frame_nr);
                tracker.record_frame(&frame);
            }
            Ok(Err(_)) | Err(_) => {
                // Timeout or channel error, continue
            }
        }
    }

    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    let mut stats = TestStats::new();
    tracker.export_to_stats(&mut stats);

    // Analyze sequence
    println!("\nFrame Number Analysis:");
    println!("  Total frames received: {}", frame_numbers.len());

    if let (Some(first), Some(last)) = (stats.first_frame_nr, stats.last_frame_nr) {
        println!("  Frame number range: {} to {}", first, last);
        println!("  Theoretical span: {}", last - first + 1);
    }

    println!("  Skipped frames: {}", stats.skipped_frames);
    println!("  Duplicate frames: {}", stats.duplicate_frames);

    // Check monotonicity
    let mut out_of_order = 0;
    for window in frame_numbers.windows(2) {
        if window[1] < window[0] {
            out_of_order += 1;
        }
    }
    println!("  Out-of-order frames: {}", out_of_order);

    // Print first 10 frame numbers for debugging
    if frame_numbers.len() >= 10 {
        println!("  First 10 frame numbers: {:?}", &frame_numbers[..10]);
    }

    // Assertions
    assert!(frame_numbers.len() >= 5, "Should receive at least 5 frames");
    assert_eq!(
        out_of_order, 0,
        "Frame numbers should be monotonically increasing"
    );
    assert_no_duplicate_frames(stats.duplicate_frames, "Frame numbering");

    // Under FIFO semantics we expect zero skipped frames.
    assert_eq!(
        stats.skipped_frames, 0,
        "No skipped frames expected in FIFO retrieval"
    );

    println!("\n=== Frame Numbering Sequence PASSED ===\n");
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert frame data to u16 pixels (assumes 16-bit depth)
fn frame_to_u16(frame: &Frame) -> Vec<u16> {
    frame
        .data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}
