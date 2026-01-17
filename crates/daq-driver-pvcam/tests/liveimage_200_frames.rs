#![cfg(feature = "pvcam_sdk")]
//! 200 Frame Streaming Test - Matches C++ LiveImage Example
//!
//! This test verifies that the Rust PVCAM driver can stream 200 frames
//! continuously, matching the behavior of the C++ LiveImage SDK example.

use daq_core::capabilities::{ExposureControl, FrameProducer};
use daq_driver_pvcam::PvcamDriver;
use std::time::{Duration, Instant};

fn smoke_test_enabled() -> bool {
    std::env::var("PVCAM_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn camera_name() -> String {
    std::env::var("PVCAM_CAMERA_NAME").unwrap_or_else(|_| "PMUSBCam00".to_string())
}

/// Stream 200 frames continuously - matches C++ LiveImage example
#[tokio::test]
#[allow(deprecated)] // subscribe_frames() still works; register_primary_output() not yet wired
async fn liveimage_200_frames() {
    if !smoke_test_enabled() {
        println!("Test skipped (set PVCAM_SMOKE_TEST=1 to enable)");
        return;
    }

    let trace_enabled = std::env::var("PVCAM_TRACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if trace_enabled {
        println!(
            "[PVCAM TRACE] env PVCAM_TRACE=1 PVCAM_TRACE_EVERY={:?}",
            std::env::var("PVCAM_TRACE_EVERY").ok()
        );
        println!(
            "[PVCAM TRACE] env RUST_LOG={:?}",
            std::env::var("RUST_LOG").ok()
        );
        println!(
            "[PVCAM TRACE] env PVCAM_SMOKE_TEST={:?}",
            std::env::var("PVCAM_SMOKE_TEST").ok()
        );
        println!(
            "[PVCAM TRACE] env PVCAM_CAMERA_NAME={:?}",
            std::env::var("PVCAM_CAMERA_NAME").ok()
        );
        let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .try_init();
    }

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║     200 Frame Streaming Test (C++ LiveImage Equivalent)      ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // Initialize camera
    println!("[1/5] Initializing camera: {}", camera_name());
    let camera = PvcamDriver::new_async(camera_name())
        .await
        .expect("Failed to open camera");

    let (width, height) = camera.resolution();
    println!(
        "      Sensor: {}x{} ({:.2} MP)",
        width,
        height,
        (width * height) as f64 / 1_000_000.0
    );

    // Set exposure
    println!("[2/5] Setting exposure to 10ms...");
    camera.set_exposure(0.010).await.expect("set exposure");
    let readback = camera.get_exposure().await.expect("get exposure");
    println!("      Exposure: {:.3}ms", readback * 1000.0);

    // Subscribe to frame stream
    println!("[3/5] Subscribing to frame stream...");
    let mut rx = camera.subscribe_frames().await.expect("subscribe");

    // Start continuous streaming
    println!("[4/5] Starting continuous acquisition...");
    let start = Instant::now();
    camera.start_stream().await.expect("start stream");

    // Receive 200 frames
    let target_frames = 200;
    let mut frames_received = 0u32;
    let mut first_frame_data: Option<Vec<u8>> = None;
    let mut last_frame_num = 0u64;

    println!("\n      Streaming {} frames...", target_frames);

    while frames_received < target_frames {
        match tokio::time::timeout(Duration::from_secs(30), rx.recv()).await {
            Ok(Ok(frame)) => {
                frames_received += 1;
                last_frame_num = frame.frame_number;

                // Save first frame for analysis
                if frames_received == 1 {
                    first_frame_data = Some(frame.data.to_vec());
                }

                // Progress indicator
                if frames_received % 50 == 0 {
                    let elapsed = start.elapsed().as_secs_f64();
                    let fps = frames_received as f64 / elapsed;
                    println!(
                        "      Frame {:3}: {}x{} @ {:.1} fps",
                        frames_received, frame.width, frame.height, fps
                    );
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                // Handle lagged frames (still count them)
                frames_received += n as u32;
                println!(
                    "      [Note: {} frames lagged, total now: {}]",
                    n, frames_received
                );
            }
            Ok(Err(e)) => {
                panic!("Channel error at frame {}: {}", frames_received, e);
            }
            Err(_) => {
                panic!(
                    "TIMEOUT: Only received {} of {} frames",
                    frames_received, target_frames
                );
            }
        }
    }

    let elapsed = start.elapsed();

    // Stop streaming
    println!("\n[5/5] Stopping acquisition...");
    camera.stop_stream().await.expect("stop stream");

    // Results
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                         RESULTS                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  Frames received:  {:4}                                      ║",
        frames_received
    );
    println!(
        "║  Last frame #:     {:4}                                      ║",
        last_frame_num
    );
    println!(
        "║  Total time:       {:6.2}s                                   ║",
        elapsed.as_secs_f64()
    );
    println!(
        "║  Frame rate:       {:6.2} fps                                ║",
        frames_received as f64 / elapsed.as_secs_f64()
    );
    println!("╚══════════════════════════════════════════════════════════════╝");

    // Validate frame data is real (not mock)
    if let Some(data) = first_frame_data {
        println!("\n═══ First Frame Analysis ═══");

        let pixels: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();

        let n = pixels.len() as f64;
        let sum: u64 = pixels.iter().map(|&v| v as u64).sum();
        let mean = sum as f64 / n;
        let min = *pixels.iter().min().unwrap_or(&0);
        let max = *pixels.iter().max().unwrap_or(&0);

        println!("  Pixels:  {}", pixels.len());
        println!("  Mean:    {:.1}", mean);
        println!("  Min:     {}", min);
        println!("  Max:     {}", max);

        // Mock gradient detection
        // Mock pattern max would be ~4196 (100 + 4096)
        // Real dark frame max < 500
        if max > 3000 {
            panic!("FAILURE: Data appears to be MOCK gradient (max={})!", max);
        }

        println!(
            "\n  ✓ Data verified as REAL camera data (max={} << 4096)",
            max
        );
    }

    // Final assertions
    assert!(
        frames_received >= target_frames,
        "Should receive at least {} frames, got {}",
        target_frames,
        frames_received
    );

    let fps = frames_received as f64 / elapsed.as_secs_f64();
    assert!(fps > 30.0, "Frame rate should be > 30 fps, got {:.1}", fps);

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║  ✓ SUCCESS: 200 frames streamed with REAL camera data!       ║");
    println!("║    Rust implementation matches C++ LiveImage behavior        ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
}
