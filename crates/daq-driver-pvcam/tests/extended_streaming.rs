#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_sdk")]

//! Extended streaming test to reproduce ~50 frame halt issue

use daq_core::capabilities::{ExposureControl, FrameProducer};
use daq_driver_pvcam::PvcamDriver;
use std::env;
use std::time::{Duration, Instant};

fn smoke_test_enabled() -> bool {
    env::var("PVCAM_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn camera_name() -> String {
    env::var("PVCAM_CAMERA_NAME").unwrap_or_else(|_| "PrimeBSI".to_string())
}

/// Extended streaming test - runs for 5 seconds to reproduce ~50 frame halt
#[tokio::test]
#[allow(deprecated)]
async fn extended_streaming_test() {
    if !smoke_test_enabled() {
        println!("Extended streaming test skipped (set PVCAM_SMOKE_TEST=1 to enable)");
        return;
    }

    println!("=== Extended Streaming Test (5 seconds) ===");

    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to create PVCAM driver");

    // Ensure clean state
    let _ = camera.stop_stream().await;

    // Set 10ms exposure (~100 fps theoretical max)
    camera
        .set_exposure(0.010)
        .await
        .expect("Failed to set exposure");

    // Subscribe BEFORE starting stream
    let mut rx = camera
        .subscribe_frames()
        .await
        .expect("Failed to subscribe to frame stream");

    // Start streaming
    println!("Starting continuous streaming for 5 seconds...");
    camera
        .start_stream()
        .await
        .expect("Failed to start streaming");

    // Collect frames for 5 seconds
    let stream_duration = Duration::from_secs(5);
    let start = Instant::now();
    let mut frame_count = 0u32;
    let mut last_report = Instant::now();

    while start.elapsed() < stream_duration {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(_frame)) => {
                frame_count += 1;
                // Report every second
                if last_report.elapsed() >= Duration::from_secs(1) {
                    println!(
                        "  Frames so far: {} (@ {:.1} fps)",
                        frame_count,
                        frame_count as f64 / start.elapsed().as_secs_f64()
                    );
                    last_report = Instant::now();
                }
            }
            Ok(Err(e)) => {
                println!("  ERROR: Receive error at frame {}: {}", frame_count, e);
                break;
            }
            Err(_) => {
                println!(
                    "  ERROR: Timeout waiting for frame after {} frames",
                    frame_count
                );
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

    println!("\n=== Extended Streaming Results ===");
    println!("  Duration: {:?}", elapsed);
    println!("  Frames captured: {}", frame_count);
    println!(
        "  Frame rate: {:.2} fps",
        frame_count as f64 / elapsed.as_secs_f64()
    );

    // At 46 fps for 5 seconds, we expect ~230 frames
    // If we only got ~50 frames, the test FAILS
    assert!(
        frame_count > 100,
        "Should have captured at least 100 frames in 5 seconds, got {}",
        frame_count
    );

    let _ = camera.close().await;
    println!("\n=== Extended Streaming Test PASSED ===");
}
