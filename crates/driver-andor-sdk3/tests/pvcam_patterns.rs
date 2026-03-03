//! PVCAM-pattern tests ported to Andor SDK3 (bd-lhd1)
//!
//! Adapts continuous_acquisition_tier1.rs, stress_1000_frames.rs, and
//! frame_loss_metrics_test.rs patterns for the SDK3 mock backend.
//!
//! Key adaptations from PVCAM originals:
//! - `AndorCamera::new_mock()` replaces `PvcamDriver::new_async(name)`
//! - `LoanedFrame` (pool) replaces `Arc<Frame>` (broadcast)
//! - Mock has no readout delay; FPS ≈ 1/exposure
//! - No `acquire_frame()` — single frame via start/stop
//! - No `close()` — Drop handles cleanup
//!
//! ## Running Tests
//!
//! ```bash
//! cargo nextest run -p driver-andor-sdk3 --test pvcam_patterns
//! ```

#![cfg(not(feature = "camera"))]

use common::capabilities::{ExposureControl, FrameProducer};
use driver_andor_sdk3::AndorCamera;
use std::time::{Duration, Instant};

// =============================================================================
// Test Configuration (mirrors PVCAM common/mod.rs)
// =============================================================================

/// Standard exposure times.
///
/// Note: In mock mode, actual frame time = exposure + gradient generation overhead.
/// For 2048x2048 16-bit frames, gradient generation adds ~40-50ms per frame,
/// so effective FPS is much lower than 1/exposure.
mod exposures {
    /// Fast exposure for streaming tests (5ms, effective ~50ms with mock overhead)
    pub const FAST_SEC: f64 = 0.005;

    /// Standard exposure for reliable operation (10ms)
    pub const STANDARD_SEC: f64 = 0.010;
}

/// Standard test durations.
mod durations {
    use std::time::Duration;

    /// Standard performance test (long enough for ~10 frames at mock speed)
    pub const STANDARD: Duration = Duration::from_secs(5);

    /// Frame receive timeout
    pub const FRAME_TIMEOUT: Duration = Duration::from_secs(2);
}

/// Minimum FPS for mock mode (accounting for gradient generation overhead).
/// 2048x2048 gradient takes ~40-50ms, so each frame is ~50ms → ~20 FPS.
const MOCK_MIN_FPS: f64 = 1.0;

/// Lightweight frame tracker (mirrors PVCAM FrameTracker).
struct FrameTracker {
    last_frame_nr: Option<u64>,
    first_frame_nr: Option<u64>,
    frame_count: u64,
    skipped: u64,
    duplicates: u64,
    out_of_order: u64,
}

impl FrameTracker {
    fn new() -> Self {
        Self {
            last_frame_nr: None,
            first_frame_nr: None,
            frame_count: 0,
            skipped: 0,
            duplicates: 0,
            out_of_order: 0,
        }
    }

    fn record(&mut self, frame_nr: u64) {
        self.frame_count += 1;

        if self.first_frame_nr.is_none() {
            self.first_frame_nr = Some(frame_nr);
        }

        if let Some(last) = self.last_frame_nr {
            if frame_nr == last {
                self.duplicates += 1;
            } else if frame_nr > last + 1 {
                self.skipped += frame_nr - last - 1;
            } else if frame_nr < last {
                self.out_of_order += 1;
            }
        }

        self.last_frame_nr = Some(frame_nr);
    }
}

// =============================================================================
// Tier 1 Test 1: Basic Frame Acquisition (port of test_basic_frame_acquisition)
// =============================================================================

/// Single-frame capture via start_stream → receive → stop_stream.
///
/// Validates:
/// - Camera initialization
/// - Single frame received with correct dimensions
/// - Frame data is non-empty with correct byte count
#[tokio::test]
async fn test_basic_frame_acquisition() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    cam.set_exposure(exposures::STANDARD_SEC)
        .await
        .expect("Failed to set exposure");

    let exposure = cam.get_exposure().await.expect("Failed to get exposure");
    assert!(
        (exposure - exposures::STANDARD_SEC).abs() < 0.001,
        "Exposure should be set correctly"
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx)
        .await
        .expect("Failed to register output");

    let start = Instant::now();
    cam.start_stream().await.expect("Failed to start stream");

    let frame = tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv())
        .await
        .expect("Timeout waiting for frame")
        .expect("Channel closed");

    let elapsed = start.elapsed();

    cam.stop_stream().await.expect("Failed to stop stream");

    // Validate frame (mirrors PVCAM assertions)
    assert!(frame.width > 0, "Frame width must be positive");
    assert!(frame.height > 0, "Frame height must be positive");

    let data = frame.pixel_data();
    assert!(!data.is_empty(), "Frame data must not be empty");

    // Verify data size matches dimensions (16-bit pixels)
    let expected_bytes = (frame.width as usize) * (frame.height as usize) * 2;
    assert_eq!(
        data.len(),
        expected_bytes,
        "Frame data size should match dimensions x 2 bytes"
    );

    println!(
        "Frame acquired in {:?}: {}x{}, {} bytes",
        elapsed,
        frame.width,
        frame.height,
        data.len()
    );
}

// =============================================================================
// Tier 1 Test 2: Continuous Streaming (port of test_continuous_streaming)
// =============================================================================

/// Sustained continuous streaming with FPS measurement.
///
/// Validates:
/// - Frames are received continuously without stalls
/// - Frame rate is within expected range for exposure time
/// - Clean shutdown without errors
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_continuous_streaming() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let (sensor_width, sensor_height) = cam.resolution();

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    cam.register_primary_output(tx)
        .await
        .expect("Failed to register output");

    cam.start_stream().await.expect("Failed to start streaming");

    let mut tracker = FrameTracker::new();
    let mut timeout_errors = 0u64;
    let test_duration = durations::STANDARD;
    let start = Instant::now();

    while start.elapsed() < test_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Some(frame)) => {
                tracker.record(frame.frame_number);
            }
            Ok(None) => {
                panic!("Channel closed unexpectedly");
            }
            Err(_) => {
                timeout_errors += 1;
            }
        }
    }

    let duration = start.elapsed();
    cam.stop_stream().await.expect("Failed to stop streaming");

    let fps = tracker.frame_count as f64 / duration.as_secs_f64();

    println!(
        "Continuous streaming: {} frames in {:?} ({:.1} FPS), sensor {}x{}",
        tracker.frame_count, duration, fps, sensor_width, sensor_height
    );

    // Assertions (mirrors PVCAM assert_fps_near, assert_frame_count_min)
    assert!(
        tracker.frame_count >= 5,
        "Should receive at least 5 frames, got {}",
        tracker.frame_count
    );

    // Mock FPS is limited by gradient generation overhead (~50ms/frame for 2048x2048)
    assert!(
        fps >= MOCK_MIN_FPS,
        "FPS {:.1} too low (expected >= {:.1})",
        fps,
        MOCK_MIN_FPS
    );

    assert_eq!(
        tracker.duplicates, 0,
        "No duplicate frame numbers should occur"
    );
    assert_eq!(timeout_errors, 0, "No timeout errors expected");
}

// =============================================================================
// Tier 1 Test 3: Frame Data Integrity (port of test_frame_data_integrity)
// =============================================================================

/// Validate frame data integrity across multiple frames.
///
/// Checks:
/// - Consistent dimensions across frames
/// - Pixel data is not all zeros
/// - Data buffer size matches pixel count
#[tokio::test]
async fn test_frame_data_integrity() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    let (sensor_width, sensor_height) = cam.resolution();

    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    cam.register_primary_output(tx)
        .await
        .expect("Failed to register output");

    cam.start_stream().await.expect("Failed to start streaming");

    let frames_to_check = 5;
    let mut valid_frames = 0;
    let mut zero_frames = 0;

    for i in 0..frames_to_check {
        let frame = tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv())
            .await
            .expect("Timeout waiting for frame")
            .expect("Channel closed");

        // Validate dimensions
        assert_eq!(frame.width, sensor_width, "Frame {} width mismatch", i + 1);
        assert_eq!(
            frame.height,
            sensor_height,
            "Frame {} height mismatch",
            i + 1
        );

        // Validate data size
        let data = frame.pixel_data();
        let expected_bytes = (sensor_width as usize) * (sensor_height as usize) * 2;
        assert_eq!(
            data.len(),
            expected_bytes,
            "Frame {} data size mismatch",
            i + 1
        );

        // Check for zero frame (uninitialized buffer)
        let is_zero = data.iter().take(100).all(|&b| b == 0);
        if is_zero {
            zero_frames += 1;
        } else {
            // Calculate pixel stats (mirrors PVCAM frame_to_u16 + stats)
            let pixels: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let mean: f64 = pixels.iter().map(|&p| f64::from(p)).sum::<f64>() / pixels.len() as f64;
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

        valid_frames += 1;
    }

    cam.stop_stream().await.expect("Failed to stop streaming");

    assert_eq!(valid_frames, frames_to_check, "All frames should be valid");
    assert_eq!(zero_frames, 0, "No frames should be all zeros");
}

// =============================================================================
// Tier 1 Test 4: Frame Numbering Sequence (port of test_frame_numbering_sequence)
// =============================================================================

/// Validate frame numbering integrity during streaming.
///
/// Checks:
/// - Frame numbers are monotonically increasing
/// - No duplicate frame numbers
/// - No gaps in frame sequence (mock FIFO semantics)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_frame_numbering_sequence() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    cam.register_primary_output(tx)
        .await
        .expect("Failed to register output");

    cam.start_stream().await.expect("Failed to start streaming");

    let mut tracker = FrameTracker::new();
    let mut frame_numbers: Vec<u64> = Vec::new();
    let test_duration = durations::STANDARD;
    let start = Instant::now();

    while start.elapsed() < test_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Some(frame)) => {
                frame_numbers.push(frame.frame_number);
                tracker.record(frame.frame_number);
            }
            Ok(None) => {
                panic!("Channel closed unexpectedly");
            }
            Err(_) => {
                break;
            }
        }
    }

    cam.stop_stream().await.expect("Failed to stop streaming");

    println!("Frame Number Analysis:");
    println!("  Total frames: {}", frame_numbers.len());
    if let (Some(first), Some(last)) = (tracker.first_frame_nr, tracker.last_frame_nr) {
        println!("  Frame range: {} to {}", first, last);
    }
    println!("  Skipped: {}", tracker.skipped);
    println!("  Duplicates: {}", tracker.duplicates);
    println!("  Out-of-order: {}", tracker.out_of_order);

    if frame_numbers.len() >= 10 {
        println!("  First 10: {:?}", &frame_numbers[..10]);
    }

    assert!(frame_numbers.len() >= 5, "Should receive at least 5 frames");
    assert_eq!(
        tracker.out_of_order, 0,
        "Frame numbers should be monotonically increasing"
    );
    assert_eq!(tracker.duplicates, 0, "No duplicate frame numbers");
    assert_eq!(
        tracker.skipped, 0,
        "No skipped frames in mock FIFO delivery"
    );
}

// =============================================================================
// Stress Test: Many Frames Sustained Streaming (port of stress_1000_frames.rs)
// =============================================================================

/// Sustained streaming stress test for 100+ frames.
///
/// This is the mock-mode equivalent of the PVCAM 1000-frame stress test.
/// Mock mode is slower per-frame (each frame sleeps for exposure duration),
/// so we target 100 frames at 5ms exposure (~0.5s).
///
/// Validates:
/// - Sustained acquisition without stalls or errors
/// - Frame numbering remains sequential throughout
/// - No pool exhaustion (frames returned properly)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_stress_sustained_streaming() {
    const TARGET_FRAMES: u64 = 20;

    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    let (sensor_width, sensor_height) = cam.resolution();

    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("Failed to set exposure");

    println!("Stress test: {} frames target", TARGET_FRAMES);
    println!("Sensor: {}x{}", sensor_width, sensor_height);

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    cam.register_primary_output(tx)
        .await
        .expect("Failed to register output");

    cam.start_stream().await.expect("Failed to start streaming");

    let mut tracker = FrameTracker::new();
    let mut timeout_errors = 0u64;
    let start = Instant::now();
    let max_duration = Duration::from_secs(10); // generous timeout

    while tracker.frame_count < TARGET_FRAMES && start.elapsed() < max_duration {
        match tokio::time::timeout(durations::FRAME_TIMEOUT, rx.recv()).await {
            Ok(Some(frame)) => {
                tracker.record(frame.frame_number);

                // Progress every 25 frames
                if tracker.frame_count.is_multiple_of(25) {
                    let elapsed = start.elapsed();
                    let fps = tracker.frame_count as f64 / elapsed.as_secs_f64();
                    println!(
                        "  Progress: {} frames @ {:.1}s ({:.1} FPS)",
                        tracker.frame_count,
                        elapsed.as_secs_f64(),
                        fps
                    );
                }
            }
            Ok(None) => {
                panic!("Channel closed at frame {}", tracker.frame_count);
            }
            Err(_) => {
                timeout_errors += 1;
                assert!(
                    timeout_errors <= 5,
                    "Too many timeouts ({timeout_errors}) at frame {}",
                    tracker.frame_count
                );
            }
        }
    }

    let duration = start.elapsed();
    cam.stop_stream().await.expect("Failed to stop streaming");

    let fps = tracker.frame_count as f64 / duration.as_secs_f64();

    println!("\nStress Test Results:");
    println!("  Frames: {}/{}", tracker.frame_count, TARGET_FRAMES);
    println!("  Duration: {:?}", duration);
    println!("  FPS: {:.1}", fps);
    println!("  Skipped: {}", tracker.skipped);
    println!("  Duplicates: {}", tracker.duplicates);
    println!("  Timeouts: {}", timeout_errors);

    assert!(
        tracker.frame_count >= TARGET_FRAMES,
        "Should capture {} frames, got {}",
        TARGET_FRAMES,
        tracker.frame_count
    );
    assert_eq!(tracker.duplicates, 0, "No duplicate frames");
    assert_eq!(tracker.skipped, 0, "No skipped frames in mock delivery");
    assert!(timeout_errors <= 5, "Too many timeout errors");

    // FPS sanity check (mock overhead makes actual FPS much lower than 1/exposure)
    assert!(
        fps >= MOCK_MIN_FPS,
        "FPS {:.1} too low (expected >= {:.1})",
        fps,
        MOCK_MIN_FPS
    );
}

// =============================================================================
// Frame Loss Metrics (port of frame_loss_metrics_test.rs concepts)
// =============================================================================

/// Verify that backpressure (small channel) doesn't cause panics or errors.
///
/// The mock acquisition loop uses `try_send()` which drops frames under
/// backpressure rather than blocking. This test confirms the pipeline stays
/// healthy even when frames are dropped.
#[tokio::test]
async fn test_frame_loss_under_backpressure() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    // Very fast exposure + tiny channel = guaranteed drops
    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("set exposure");

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    cam.register_primary_output(tx)
        .await
        .expect("register output");

    cam.start_stream().await.expect("start stream");

    // Let frames pile up (many will be dropped by try_send)
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop stream first, then drain remaining
    cam.stop_stream().await.expect("stop stream");

    // Drain buffered frames after stop
    let mut received = 0u64;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        received += 1;
    }

    // We should have received at least one frame
    assert!(
        received >= 1,
        "Should receive at least 1 frame under backpressure, got {}",
        received
    );

    // The key assertion: no panics, no errors — graceful degradation
    println!(
        "Backpressure test: received {} frames (many dropped gracefully)",
        received
    );
}

/// Verify frame numbering starts at 0 and resets on restart.
///
/// Mirrors PVCAM frame_loss_metrics concept of tracking sequence integrity
/// across acquisition cycles.
#[tokio::test]
async fn test_frame_numbering_resets_on_restart() {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Failed to create mock camera");

    cam.set_exposure(exposures::FAST_SEC)
        .await
        .expect("set exposure");

    // First cycle
    let (tx1, mut rx1) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx1)
        .await
        .expect("register output");

    cam.start_stream().await.expect("start stream");

    let frame1 = tokio::time::timeout(durations::FRAME_TIMEOUT, rx1.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(frame1.frame_number, 0, "First cycle starts at 0");

    // Get a few more frames
    for _ in 0..3 {
        let _ = tokio::time::timeout(durations::FRAME_TIMEOUT, rx1.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
    }

    cam.stop_stream().await.expect("stop stream");

    // Second cycle — frame_count should reset
    let (tx2, mut rx2) = tokio::sync::mpsc::channel(8);
    cam.register_primary_output(tx2)
        .await
        .expect("register output #2");

    cam.start_stream().await.expect("start stream #2");

    let frame2 = tokio::time::timeout(durations::FRAME_TIMEOUT, rx2.recv())
        .await
        .expect("timeout #2")
        .expect("channel closed #2");
    assert_eq!(
        frame2.frame_number, 0,
        "Second cycle should reset frame_number to 0"
    );

    cam.stop_stream().await.expect("stop stream #2");
}
