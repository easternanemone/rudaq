//! PVCAM Continuous Streaming Hardware Tests
//!
//! These tests validate the continuous streaming implementation with real hardware.
//! Run with: cargo test --features 'instrument_photometrics,pvcam_hardware,hardware_tests' --test pvcam_streaming_test
//!
//! Requirements:
//! - Photometrics camera connected (Prime BSI or Prime 95B)
//! - PVCAM SDK installed at /opt/pvcam/sdk
//! - LD_LIBRARY_PATH includes /opt/pvcam/library/x86_64

#![cfg(all(
    feature = "instrument_photometrics",
    feature = "pvcam_hardware",
    feature = "hardware_tests"
))]

use rust_daq::hardware::capabilities::{ExposureControl, FrameProducer};
use rust_daq::hardware::pvcam::PvcamDriver;
use std::time::{Duration, Instant};

/// Test basic streaming start/stop
#[tokio::test]
async fn test_streaming_start_stop() {
    println!("=== Test: Streaming Start/Stop ===");

    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to open camera");

    // Set reasonable exposure for streaming
    camera
        .set_exposure_ms(10.0)
        .await
        .expect("Failed to set exposure");

    // Start streaming
    println!("Starting stream...");
    camera.start_stream().await.expect("Failed to start stream");
    assert!(camera.is_streaming(), "Should be streaming after start");

    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stop streaming
    println!("Stopping stream...");
    camera.stop_stream().await.expect("Failed to stop stream");
    assert!(!camera.is_streaming(), "Should not be streaming after stop");

    println!("Start/stop test PASSED");
}

/// Test streaming frame delivery
#[tokio::test]
async fn test_streaming_frame_delivery() {
    println!("=== Test: Streaming Frame Delivery ===");

    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to open camera");

    // Set 10ms exposure (~100 FPS max)
    camera
        .set_exposure_ms(10.0)
        .await
        .expect("Failed to set exposure");

    // Get frame receiver
    let mut rx = camera
        .take_frame_receiver()
        .await
        .expect("Failed to get frame receiver");

    // Start streaming
    camera.start_stream().await.expect("Failed to start stream");

    // Collect frames for 1 second
    let start = Instant::now();
    let mut frame_count = 0;
    let test_duration = Duration::from_secs(1);

    println!("Collecting frames for {:?}...", test_duration);

    while start.elapsed() < test_duration {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(frame)) => {
                frame_count += 1;
                if frame_count == 1 {
                    println!(
                        "First frame: {}x{}, {} pixels",
                        frame.width,
                        frame.height,
                        frame.buffer.len()
                    );
                }
            }
            Ok(None) => {
                println!("Channel closed");
                break;
            }
            Err(_) => {
                println!("Timeout waiting for frame");
            }
        }
    }

    // Stop streaming
    camera.stop_stream().await.expect("Failed to stop stream");

    let elapsed = start.elapsed();
    let fps = frame_count as f64 / elapsed.as_secs_f64();

    println!(
        "Received {} frames in {:?} ({:.1} FPS)",
        frame_count, elapsed, fps
    );
    println!("Camera frame counter: {}", camera.frame_count());

    // Should have received at least some frames
    assert!(frame_count > 0, "Should have received at least one frame");

    // With 10ms exposure, expect roughly 30+ FPS (accounting for overhead)
    assert!(
        fps > 10.0,
        "FPS ({:.1}) should be > 10 with 10ms exposure",
        fps
    );

    println!("Frame delivery test PASSED");
}

/// Test streaming with buffer backpressure
#[tokio::test]
async fn test_streaming_backpressure() {
    println!("=== Test: Streaming Backpressure ===");

    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to open camera");

    // Fast exposure to stress the buffer
    camera
        .set_exposure_ms(5.0)
        .await
        .expect("Failed to set exposure");

    // Don't take the receiver - let frames queue up
    // The circular buffer should handle this without stalling

    // Start streaming
    camera.start_stream().await.expect("Failed to start stream");

    // Let it run for a bit without consuming
    println!("Streaming without consuming for 500ms...");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now consume
    let mut rx = camera
        .take_frame_receiver()
        .await
        .expect("Failed to get frame receiver");

    let mut consumed = 0;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
        consumed += 1;
        if consumed >= 10 {
            break;
        }
    }

    // Stop streaming
    camera.stop_stream().await.expect("Failed to stop stream");

    println!("Consumed {} frames after delayed start", consumed);
    println!("Total frames captured: {}", camera.frame_count());

    // Should have captured frames even without immediate consumption
    assert!(camera.frame_count() > 0, "Should have captured frames");

    println!("Backpressure test PASSED");
}

/// Test streaming duration stability
#[tokio::test]
async fn test_streaming_stability() {
    println!("=== Test: Streaming Stability (10 seconds) ===");

    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to open camera");

    camera
        .set_exposure_ms(33.0) // ~30 FPS
        .await
        .expect("Failed to set exposure");

    let mut rx = camera
        .take_frame_receiver()
        .await
        .expect("Failed to get frame receiver");

    camera.start_stream().await.expect("Failed to start stream");

    let start = Instant::now();
    let test_duration = Duration::from_secs(10);
    let mut frame_count = 0;
    let mut last_report = Instant::now();
    let mut errors = 0;

    println!("Running stability test for {:?}...", test_duration);

    while start.elapsed() < test_duration {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(frame)) => {
                frame_count += 1;

                // Validate frame
                if frame.buffer.is_empty() {
                    errors += 1;
                    println!("ERROR: Empty frame at count {}", frame_count);
                }

                // Progress report every second
                if last_report.elapsed() > Duration::from_secs(1) {
                    let fps = frame_count as f64 / start.elapsed().as_secs_f64();
                    println!(
                        "  Progress: {} frames, {:.1} FPS, {} errors",
                        frame_count, fps, errors
                    );
                    last_report = Instant::now();
                }
            }
            Ok(None) => {
                println!("Channel closed unexpectedly");
                errors += 1;
                break;
            }
            Err(_) => {
                println!("Timeout waiting for frame (1s)");
                errors += 1;
            }
        }
    }

    camera.stop_stream().await.expect("Failed to stop stream");

    let elapsed = start.elapsed();
    let fps = frame_count as f64 / elapsed.as_secs_f64();

    println!("\n=== Stability Test Results ===");
    println!("Duration: {:?}", elapsed);
    println!("Frames: {}", frame_count);
    println!("FPS: {:.1}", fps);
    println!("Errors: {}", errors);
    println!("Camera frame counter: {}", camera.frame_count());

    assert!(errors == 0, "Should have no errors during stability test");
    assert!(frame_count > 100, "Should have captured >100 frames in 10s");

    println!("Stability test PASSED");
}

/// Test rapid start/stop cycling
#[tokio::test]
async fn test_streaming_rapid_cycling() {
    println!("=== Test: Rapid Start/Stop Cycling ===");

    let camera = PvcamDriver::new("PrimeBSI").expect("Failed to open camera");

    camera
        .set_exposure_ms(10.0)
        .await
        .expect("Failed to set exposure");

    let cycles = 5;
    println!("Running {} start/stop cycles...", cycles);

    for i in 0..cycles {
        // Start
        camera
            .start_stream()
            .await
            .expect(&format!("Failed to start stream on cycle {}", i));

        // Brief acquisition
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Stop
        camera
            .stop_stream()
            .await
            .expect(&format!("Failed to stop stream on cycle {}", i));

        // Brief pause
        tokio::time::sleep(Duration::from_millis(50)).await;

        println!("  Cycle {} complete", i + 1);
    }

    // Final check - should still work
    camera.start_stream().await.expect("Failed final start");
    tokio::time::sleep(Duration::from_millis(100)).await;
    camera.stop_stream().await.expect("Failed final stop");

    println!("Rapid cycling test PASSED");
}
