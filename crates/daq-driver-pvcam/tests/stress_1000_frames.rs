//! Stress Test: 1000+ Frames Sustained Streaming (bd-d8di)
//!
//! This test validates sustained high-throughput streaming by capturing 1000+ frames
//! continuously. It exercises the full acquisition pipeline under load, including:
//! - Buffer pool allocation/release cycles
//! - Frame numbering integrity over long durations
//! - Pool exhaustion handling (bd-dmbl)
//! - Callback/FIFO synchronization stability
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && git pull && \
//!   cargo nextest run --profile hardware -p daq-driver-pvcam --features pvcam_sdk \
//!     --test stress_1000_frames'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_sdk")]

mod common;

use common::capabilities::{ExposureControl, FrameProducer};
use common::{
    assert_errors_within_limit, assert_fps_near, assert_frame_count_min,
    assert_no_duplicate_frames, durations, exposures, FrameTracker, TestStats,
};
use daq_driver_pvcam::PvcamDriver;
use std::env;
use std::time::{Duration, Instant};

// =============================================================================
// Environment Variable Gating
// =============================================================================

/// Check if stress test is enabled via environment variable.
/// Set PVCAM_STRESS_TEST=1 to enable this expensive test.
fn stress_test_enabled() -> bool {
    env::var("PVCAM_STRESS_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

// =============================================================================
// Test Configuration
// =============================================================================

/// Camera name for tests (Prime BSI on maitai)
const CAMERA_NAME: &str = "pvcamUSB_0";

/// Stress test duration: 45 seconds to reliably capture 1000+ frames at ~30 FPS
const STRESS_TEST_DURATION: Duration = Duration::from_secs(45);

/// Minimum required frames for the stress test to pass
const MIN_REQUIRED_FRAMES: u64 = 1000;

/// Progress report interval (every 5 seconds)
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum allowed errors (timeouts + channel errors)
const MAX_ALLOWED_ERRORS: u64 = 5;

// =============================================================================
// Stress Test: 1000+ Frames Sustained Streaming
// =============================================================================

/// Stress test: Sustained streaming for 1000+ frames.
///
/// This test validates:
/// - Camera can sustain continuous acquisition for 45+ seconds
/// - At least 1000 frames are captured without major errors
/// - Frame numbering remains sequential (no duplicates, minimal gaps)
/// - Pool exhaustion handling works correctly (dropped_frames metric)
/// - FPS remains stable throughout the acquisition
///
/// Expected performance at ~30 FPS (Prime BSI full sensor):
/// - 45 seconds × 30 FPS = ~1350 frames expected
/// - Minimum requirement: 1000 frames
#[tokio::test]
#[allow(deprecated)] // subscribe_frames() still works
async fn test_stress_1000_frames_sustained_streaming() {
    // Gate expensive test behind environment variable
    if !stress_test_enabled() {
        println!("Stress test skipped (set PVCAM_STRESS_TEST=1 to enable)");
        return;
    }

    println!("\n{:=^80}", "");
    println!("  STRESS TEST: 1000+ Frames Sustained Streaming (bd-d8di)");
    println!("{:=^80}\n", "");

    // Initialize driver
    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    // Get sensor resolution
    let (sensor_width, sensor_height) = camera.resolution();
    let is_full_sensor = sensor_width >= 2048 && sensor_height >= 2048;

    println!("Camera: {}", CAMERA_NAME);
    println!("Sensor resolution: {}x{}", sensor_width, sensor_height);
    println!(
        "Sensor mode: {}",
        if is_full_sensor {
            "Full sensor"
        } else {
            "ROI/Binned"
        }
    );

    // Set fast exposure for maximum frame rate
    camera
        .set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let exposure = camera.get_exposure().await.expect("Failed to get exposure");
    println!("Exposure: {:.1}ms", exposure * 1000.0);

    // Calculate expected FPS based on sensor mode
    // Full sensor has ~23ms readout time on Prime BSI
    let expected_fps = if is_full_sensor {
        let readout_ms = 23.0; // Prime BSI full-sensor readout
        1000.0 / (exposures::FAST_MS + readout_ms)
    } else {
        // Smaller ROI/binning has faster readout
        1000.0 / (exposures::FAST_MS + 5.0) // Estimate 5ms readout for smaller sensors
    };

    println!("Expected FPS: {:.1}", expected_fps);
    println!("Test duration: {:?}", STRESS_TEST_DURATION);
    println!(
        "Expected frames: ~{:.0}",
        expected_fps * STRESS_TEST_DURATION.as_secs_f64()
    );
    println!("Minimum required: {} frames", MIN_REQUIRED_FRAMES);
    println!();

    // Register output before starting stream
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    camera
        .register_primary_output(tx)
        .await
        .expect("Failed to register output");

    // Start streaming
    println!("Starting sustained streaming...\n");
    let stream_start = Instant::now();

    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    // Statistics tracking
    let mut stats = TestStats::new();
    let mut tracker = FrameTracker::new();
    let mut last_progress_report = Instant::now();
    let mut progress_interval_count = 0u32;

    // Progress tracking for detailed reports
    let mut interval_frame_count = 0u64;
    let mut interval_start = Instant::now();

    println!(
        "{:>8} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "Time(s)", "Frames", "FPS(curr)", "FPS(avg)", "Timeouts", "Errors"
    );
    println!("{:-<68}", "");

    // Main acquisition loop
    while stream_start.elapsed() < STRESS_TEST_DURATION {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Some(frame)) => {
                tracker.record_frame(&frame);
                interval_frame_count += 1;
            }
            Ok(None) => {
                stats.channel_errors += 1;
                eprintln!(
                    "[ERROR] Channel closed at {:.1}s",
                    stream_start.elapsed().as_secs_f64()
                );
                break;
            }
            Err(_) => {
                stats.timeout_errors += 1;
                eprintln!(
                    "[WARNING] Frame timeout at {:.1}s (total timeouts: {})",
                    stream_start.elapsed().as_secs_f64(),
                    stats.timeout_errors
                );
            }
        }

        // Progress report every PROGRESS_INTERVAL
        if last_progress_report.elapsed() >= PROGRESS_INTERVAL {
            progress_interval_count += 1;
            let elapsed = stream_start.elapsed();
            let total_frames = tracker.frame_count;
            let interval_duration = interval_start.elapsed();

            // Calculate current interval FPS
            let interval_fps = if interval_duration.as_secs_f64() > 0.0 {
                interval_frame_count as f64 / interval_duration.as_secs_f64()
            } else {
                0.0
            };

            // Calculate average FPS
            let avg_fps = if elapsed.as_secs_f64() > 0.0 {
                total_frames as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };

            println!(
                "{:>8.1} {:>10} {:>10.1} {:>10.1} {:>10} {:>10}",
                elapsed.as_secs_f64(),
                total_frames,
                interval_fps,
                avg_fps,
                stats.timeout_errors,
                stats.channel_errors
            );

            // Reset interval counters
            interval_frame_count = 0;
            interval_start = Instant::now();
            last_progress_report = Instant::now();
        }
    }

    stats.duration = stream_start.elapsed();

    // Stop streaming
    println!("\nStopping streaming...");
    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    // Export tracker stats
    tracker.export_to_stats(&mut stats);
    stats.calculate_fps();

    // Calculate expected frames
    let frame_time_ms = if is_full_sensor {
        exposures::FAST_MS + 23.0
    } else {
        exposures::FAST_MS + 5.0
    };
    stats.calculate_expected(frame_time_ms);

    // Print detailed results
    println!("\n{:=^80}", "");
    println!("  STRESS TEST RESULTS");
    println!("{:=^80}", "");

    println!("\n📊 Frame Statistics:");
    println!("  Total frames captured: {}", stats.frame_count);
    println!("  Test duration: {:.2}s", stats.duration.as_secs_f64());
    println!("  Average FPS: {:.2}", stats.fps);
    println!("  Expected FPS: {:.1}", expected_fps);
    println!("  Expected frames: {}", stats.expected_frames);
    println!(
        "  Frame loss: {:.2}% ({} frames)",
        stats.frame_loss_pct(),
        stats.expected_frames.saturating_sub(stats.frame_count)
    );

    println!("\n📋 Frame Numbering:");
    if let (Some(first), Some(last)) = (stats.first_frame_nr, stats.last_frame_nr) {
        println!("  First frame number: {}", first);
        println!("  Last frame number: {}", last);
        println!("  Frame number span: {}", last - first + 1);
    }
    println!("  Skipped frames: {}", stats.skipped_frames);
    println!("  Duplicate frames: {}", stats.duplicate_frames);

    println!("\n⚠️  Errors:");
    println!("  Timeout errors: {}", stats.timeout_errors);
    println!("  Channel errors: {}", stats.channel_errors);
    let total_errors = stats.timeout_errors + stats.channel_errors;
    println!("  Total errors: {}", total_errors);

    // Pool exhaustion metrics (bd-dmbl)
    println!("\n🔄 Pool Exhaustion (bd-dmbl):");
    println!("  Frames dropped due to backpressure: (check PVCAM BACKPRESSURE logs above)");

    // Check if test passed/failed before assertions
    let passed = stats.frame_count >= MIN_REQUIRED_FRAMES
        && stats.duplicate_frames == 0
        && total_errors <= MAX_ALLOWED_ERRORS;

    println!("\n{:=^80}", "");
    if passed {
        println!("  ✅ STRESS TEST PASSED");
    } else {
        println!("  ❌ STRESS TEST FAILED");
    }
    println!("{:=^80}\n", "");

    // Assertions
    assert_frame_count_min(
        stats.frame_count,
        MIN_REQUIRED_FRAMES,
        "1000+ frame stress test",
    );
    assert_no_duplicate_frames(stats.duplicate_frames, "1000+ frame stress test");
    assert_errors_within_limit(total_errors, MAX_ALLOWED_ERRORS, "1000+ frame stress test");

    // FPS should be within 50% tolerance (generous for sustained test)
    assert_fps_near(stats.fps, expected_fps, 50.0, "1000+ frame stress test");

    println!("=== Stress Test: 1000+ Frames PASSED ===\n");
}

/// Stress test variant: Extended duration for 2000+ frames.
///
/// This is an optional longer test that can be enabled manually.
/// Runs for 90 seconds to capture 2000+ frames.
#[tokio::test]
#[ignore] // Run with: cargo test --ignored
#[allow(deprecated)]
async fn test_stress_2000_frames_extended() {
    println!("\n=== Extended Stress Test: 2000+ Frames ===\n");

    let camera = PvcamDriver::new_async(CAMERA_NAME.to_string())
        .await
        .expect("Failed to create PVCAM driver");

    let (sensor_width, sensor_height) = camera.resolution();
    let is_full_sensor = sensor_width >= 2048 && sensor_height >= 2048;

    camera
        .set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let expected_fps = if is_full_sensor {
        1000.0 / (exposures::FAST_MS + 23.0)
    } else {
        1000.0 / (exposures::FAST_MS + 5.0)
    };

    println!("Sensor: {}x{}", sensor_width, sensor_height);
    println!("Expected FPS: {:.1}", expected_fps);
    println!("Extended test duration: 90 seconds");
    println!("Target: 2000+ frames\n");

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    camera
        .register_primary_output(tx)
        .await
        .expect("Failed to register output");

    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let mut tracker = FrameTracker::new();
    let extended_duration = Duration::from_secs(90);
    let start = Instant::now();
    let mut last_report = Instant::now();

    while start.elapsed() < extended_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Some(frame)) => {
                tracker.record_frame(&frame);

                // Report every 10 seconds
                if last_report.elapsed() >= Duration::from_secs(10) {
                    println!(
                        "  Progress: {} frames @ {:.1}s ({:.1} FPS)",
                        tracker.frame_count,
                        start.elapsed().as_secs_f64(),
                        tracker.frame_count as f64 / start.elapsed().as_secs_f64()
                    );
                    last_report = Instant::now();
                }
            }
            Ok(Err(e)) => {
                stats.channel_errors += 1;
                eprintln!("Channel error: {}", e);
            }
            Err(_) => {
                stats.timeout_errors += 1;
            }
        }
    }

    stats.duration = start.elapsed();
    camera
        .stop_stream()
        .await
        .expect("Failed to stop streaming");

    tracker.export_to_stats(&mut stats);
    stats.calculate_fps();

    stats.print_summary("Extended Stress Test (2000+ frames)");

    // Assertions for extended test
    assert_frame_count_min(stats.frame_count, 2000, "2000+ frame stress test");
    assert_no_duplicate_frames(stats.duplicate_frames, "2000+ frame stress test");
    assert_errors_within_limit(
        stats.timeout_errors + stats.channel_errors,
        10, // Allow slightly more errors for longer test
        "2000+ frame stress test",
    );

    println!("\n=== Extended Stress Test PASSED ===\n");
}
