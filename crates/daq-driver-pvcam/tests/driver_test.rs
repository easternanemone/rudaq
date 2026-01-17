//! Integration tests for PvcamDriver
//!
//! Tests the high-level driver interface including:
//! - Async driver creation
//! - Parameter access
//! - Frame acquisition (mock mode)
//! - Streaming (mock mode)
//!
//! ## Running Tests
//!
//! ```bash
//! # Mock mode tests
//! cargo test -p daq-driver-pvcam --test driver_test
//!
//! # Hardware tests
//! cargo test -p daq-driver-pvcam --test driver_test --features "pvcam_sdk,hardware_tests"
//! ```

use daq_core::capabilities::{ExposureControl, FrameProducer, Parameterized, Triggerable};
use daq_driver_pvcam::PvcamDriver;
use std::time::Duration;

// =============================================================================
// Mock Mode Driver Tests
// =============================================================================

#[cfg(not(feature = "pvcam_sdk"))]
mod mock_driver {
    use super::*;

    #[tokio::test]
    async fn create_driver_mock() {
        let driver = PvcamDriver::new_async("MockCamera".to_string()).await;
        assert!(driver.is_ok(), "Should create driver in mock mode");
    }

    #[tokio::test]
    async fn driver_resolution() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();
        let (width, height) = driver.resolution();

        // Mock mode should return 2048x2048
        assert_eq!(width, 2048);
        assert_eq!(height, 2048);
    }

    #[tokio::test]
    async fn driver_exposure_control() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();

        // Set exposure to 50ms
        driver.set_exposure(0.050).await.unwrap();

        // Read back
        let exposure = driver.get_exposure().await.unwrap();
        assert!((exposure - 0.050).abs() < 0.001, "Exposure should be 50ms");
    }

    #[tokio::test]
    async fn driver_arm_trigger() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();

        // Initially not armed
        let armed = driver.is_armed().await.unwrap();
        assert!(!armed, "Should not be armed initially");

        // Arm
        driver.arm().await.unwrap();
        let armed = driver.is_armed().await.unwrap();
        assert!(armed, "Should be armed after arm()");

        // Trigger (should not error)
        driver.trigger().await.unwrap();
    }

    #[tokio::test]
    async fn driver_parameters() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();
        let params = driver.parameters();

        // Should have registered parameters
        let names = params.names();
        assert!(
            names.contains(&"acquisition.exposure_ms"),
            "Should have acquisition.exposure_ms parameter"
        );
        assert!(
            names.contains(&"acquisition.roi"),
            "Should have acquisition.roi parameter"
        );
        assert!(
            names.contains(&"acquisition.binning"),
            "Should have acquisition.binning parameter"
        );
        assert!(
            names.contains(&"thermal.temperature"),
            "Should have thermal.temperature parameter"
        );
    }

    #[tokio::test]
    #[allow(deprecated)] // subscribe_frames() still works; register_primary_output() not yet wired
    async fn driver_streaming_mock() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();

        // Set short exposure for fast mock frames
        driver.set_exposure(0.010).await.unwrap();

        // Not streaming initially
        let streaming = driver.is_streaming().await.unwrap();
        assert!(!streaming, "Should not be streaming initially");

        // Start streaming
        driver.start_stream().await.unwrap();

        let streaming = driver.is_streaming().await.unwrap();
        assert!(streaming, "Should be streaming after start_stream()");

        // Subscribe and receive a frame
        // NOTE: Using deprecated subscribe_frames() because register_primary_output()
        // is not yet implemented in the streaming loop (frames aren't sent to primary_tx)
        if let Some(mut rx) = driver.subscribe_frames().await {
            tokio::select! {
                frame = rx.recv() => {
                    let frame = frame.expect("Should receive frame");
                    assert!(frame.width > 0);
                    assert!(frame.height > 0);
                    println!("Received mock frame: {}x{}", frame.width, frame.height);
                }
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    panic!("Timed out waiting for frame");
                }
            }
        }

        // Stop streaming
        driver.stop_stream().await.unwrap();

        let streaming = driver.is_streaming().await.unwrap();
        assert!(!streaming, "Should not be streaming after stop_stream()");
    }

    #[tokio::test]
    async fn driver_frame_count() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();

        // Initially zero
        assert_eq!(driver.frame_count(), 0);

        // Set short exposure
        driver.set_exposure(0.005).await.unwrap();

        // Start streaming
        driver.start_stream().await.unwrap();

        // Wait for some frames
        tokio::time::sleep(Duration::from_millis(50)).await;

        let count = driver.frame_count();
        println!("Frame count after 50ms: {}", count);

        // Should have some frames
        assert!(count >= 1, "Should have received at least 1 frame");

        driver.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn driver_acquire_single_frame() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .unwrap();

        // Set exposure
        driver.set_exposure(0.010).await.unwrap();

        // Acquire single frame
        let frame = driver.acquire_frame().await.unwrap();

        assert!(frame.width > 0);
        assert!(frame.height > 0);
        println!("Acquired single frame: {}x{}", frame.width, frame.height);
    }
}

// =============================================================================
// Hardware Driver Tests
// =============================================================================

#[cfg(all(feature = "pvcam_sdk", feature = "hardware_tests"))]
mod hardware_driver {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use tracing_subscriber::EnvFilter;

    lazy_static::lazy_static! {
        static ref CAMERA_LOCK: Mutex<()> = Mutex::new(());
        static ref LOG_INIT: () = {
            let _ = tracing_subscriber::fmt()
                .with_test_writer()
                .with_env_filter(EnvFilter::new("debug,pvcam_sys=trace"))
                .try_init();
        };
    }

    #[tokio::test]
    async fn hardware_create_driver() {
        let _ = *LOG_INIT;
        let _lock = CAMERA_LOCK.lock().unwrap();

        let driver = PvcamDriver::new_async("pvcamUSB_0".to_string()).await;
        assert!(
            driver.is_ok(),
            "Should create driver with real hardware: {:?}",
            driver.err()
        );
    }

    #[tokio::test]
    async fn hardware_resolution() {
        let _lock = CAMERA_LOCK.lock().unwrap();

        let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
            .await
            .unwrap();
        let (width, height) = driver.resolution();

        // Prime BSI is 2048x2048
        assert_eq!(width, 2048, "Width should be 2048");
        assert_eq!(height, 2048, "Height should be 2048");
    }

    #[tokio::test]
    async fn hardware_exposure_control() {
        let _lock = CAMERA_LOCK.lock().unwrap();

        let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
            .await
            .unwrap();

        // Set exposure to 100ms
        driver.set_exposure(0.100).await.unwrap();

        // Read back
        let exposure = driver.get_exposure().await.unwrap();
        assert!(
            (exposure - 0.100).abs() < 0.01,
            "Exposure should be ~100ms, got {}",
            exposure
        );
    }

    #[tokio::test]
    #[allow(deprecated)] // subscribe_frames() still works; register_primary_output() not yet wired
    async fn hardware_stream_frames() {
        let _lock = CAMERA_LOCK.lock().unwrap();

        let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
            .await
            .unwrap();

        // Set short exposure
        driver.set_exposure(0.010).await.unwrap();

        // Start streaming
        driver.start_stream().await.unwrap();
        println!("Streaming started");

        // Subscribe and receive frames
        // NOTE: Using deprecated subscribe_frames() because register_primary_output()
        // is not yet implemented in the streaming loop (frames aren't sent to primary_tx)
        if let Some(mut rx) = driver.subscribe_frames().await {
            let mut received = 0;
            let start = std::time::Instant::now();

            while received < 5 && start.elapsed() < Duration::from_secs(30) {
                tokio::select! {
                    frame = rx.recv() => {
                        if let Ok(frame) = frame {
                            received += 1;
                            println!("Frame {}: {}x{}", received, frame.width, frame.height);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }

            assert!(
                received >= 3,
                "Should receive at least 3 frames, got {}",
                received
            );
        }

        // Stop streaming
        driver.stop_stream().await.unwrap();
        println!("Streaming stopped, total frames: {}", driver.frame_count());
    }

    /// Test 29: Full driver 200-frame streaming test
    ///
    /// This uses the actual PvcamDriver (not isolation tests) to stream 200 frames.
    /// If this fails at ~19 frames while isolation tests pass, the issue is in
    /// something unique to the full driver path.
    #[tokio::test]
    #[allow(deprecated)] // subscribe_frames() still works; register_primary_output() not yet wired
    async fn hardware_stream_200_frames() {
        let _lock = CAMERA_LOCK.lock().unwrap();
        let _ = *LOG_INIT;

        const TARGET_FRAMES: usize = 200;

        println!("\n=== DRIVER TEST: 200 Frame Streaming ===");
        println!("Uses actual PvcamDriver::start_stream()");
        println!("Target: {} frames", TARGET_FRAMES);
        println!();

        let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
            .await
            .expect("Should create driver");

        // Set 100ms exposure (same as isolation tests)
        driver.set_exposure(0.100).await.unwrap();
        println!("[OK] Exposure set to 100ms");

        // Ensure clear mode is PreExposure
        if let Some(param) = driver.parameters().get("acquisition.clear_mode") {
            param.set_json(json!("PreExposure")).unwrap();
            println!("[OK] Clear mode set to PreExposure");
        }

        // Ensure trigger mode is Timed
        if let Some(param) = driver.parameters().get("acquisition.trigger_mode") {
            param.set_json(json!("Timed")).unwrap();
            println!("[OK] Trigger mode set to Timed");
        }

        // Start streaming
        println!("[SETUP] Starting stream...");
        driver.start_stream().await.unwrap();
        println!("[OK] Stream started");

        // Subscribe and receive frames
        println!(
            "\n=== FRAME ACQUISITION (target: {} frames) ===\n",
            TARGET_FRAMES
        );

        let mut received = 0usize;
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(60); // 60 seconds should be plenty for 200 frames at 100ms

        if let Some(mut rx) = driver.subscribe_frames().await {
            while received < TARGET_FRAMES && start.elapsed() < timeout {
                tokio::select! {
                    frame = rx.recv() => {
                        if let Ok(frame) = frame {
                            received += 1;
                            if received <= 25 || received % 50 == 0 {
                                println!(
                                    "[FRAME {}] {}x{}, frame_nr={}",
                                    received, frame.width, frame.height, frame.frame_number
                                );
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        // Check if streaming stopped
                        if !driver.is_streaming().await.unwrap_or(false) {
                            println!("[ERROR] Streaming stopped unexpectedly at frame {}", received);
                            break;
                        }
                    }
                }
            }
        }

        let total_time = start.elapsed();
        driver.stop_stream().await.unwrap();

        println!("\n=== DRIVER TEST COMPLETE ===\n");
        println!("Frames received: {}/{}", received, TARGET_FRAMES);
        println!("Time: {:?}", total_time);
        println!("Driver frame_count: {}", driver.frame_count());

        if received >= TARGET_FRAMES {
            println!("RESULT: Full driver streaming is working!");
        } else {
            println!(
                "RESULT: Full driver failed at {} frames (target was {})",
                received, TARGET_FRAMES
            );
        }

        assert!(
            received >= TARGET_FRAMES,
            "Expected {} frames, got {}. Full driver has issue at frame {}!",
            TARGET_FRAMES,
            received,
            received + 1
        );
    }
}
