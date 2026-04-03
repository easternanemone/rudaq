//! SimCam-based integration tests for AndorCamera
//!
//! Exercises the complete acquisition pipeline through the mock backend:
//! - `register_primary_output()` → `start_stream()` → receive `LoanedFrame` → verify metadata
//! - Observer notification via `TapRegistry`
//! - Frame pool return semantics
//! - Start/stop lifecycle correctness
//!
//! ## Running Tests
//!
//! ```bash
//! # Mock mode (default — no hardware needed)
//! cargo nextest run -p driver-andor-sdk3 --test simcam_integration
//! ```

use common::capabilities::{ExposureControl, FrameObserver, FrameProducer, ObserverHandle};
use common::data::FrameView;
use driver_andor_sdk3::AndorCamera;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Helper: create a mock camera with given exposure (seconds).
async fn mock_camera_with_exposure(exposure_s: f64) -> AndorCamera {
    let cam = AndorCamera::new_mock()
        .await
        .expect("Should create mock camera");
    cam.set_exposure(exposure_s)
        .await
        .expect("Should set exposure");
    cam
}

// =============================================================================
// Pooled Frame Delivery (register_primary_output → start_stream → receive)
// =============================================================================

#[cfg(not(feature = "camera"))]
mod pooled_delivery {
    use super::*;

    #[tokio::test]
    async fn receive_single_frame() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout waiting for frame")
            .expect("channel closed unexpectedly");

        assert_eq!(frame.width, 2048);
        assert_eq!(frame.height, 2048);
        assert_eq!(frame.bit_depth, 16);
        assert!(
            frame.frame_number == 0,
            "first frame should be frame_number 0"
        );
        assert!(frame.timestamp_ns > 0, "timestamp should be non-zero");
        assert!(frame.exposure_ms > 0.0, "exposure_ms should be positive");

        cam.stop_stream().await.expect("stop_stream");
    }

    #[tokio::test]
    async fn receive_multiple_frames_sequential() {
        let cam = mock_camera_with_exposure(0.005).await;
        const N: u64 = 5;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        let mut received = Vec::new();
        for _ in 0..N {
            let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
            received.push(frame);
        }

        cam.stop_stream().await.expect("stop_stream");

        // Verify monotonic frame numbers
        for (i, frame) in received.iter().enumerate() {
            assert_eq!(
                frame.frame_number, i as u64,
                "frame {} should have frame_number {}",
                i, i
            );
        }

        // Verify timestamps advance
        for pair in received.windows(2) {
            assert!(
                pair[1].timestamp_ns >= pair[0].timestamp_ns,
                "timestamps should advance: {} >= {}",
                pair[1].timestamp_ns,
                pair[0].timestamp_ns
            );
        }
    }

    #[tokio::test]
    async fn frame_metadata_matches_configuration() {
        let exposure_s = 0.010;
        let cam = mock_camera_with_exposure(exposure_s).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        cam.stop_stream().await.expect("stop_stream");

        // exposure_ms should match configured exposure (within tolerance)
        let expected_ms = exposure_s * 1000.0;
        assert!(
            (frame.exposure_ms - expected_ms).abs() < 0.1,
            "exposure_ms ({}) should match configured ({} ms)",
            frame.exposure_ms,
            expected_ms
        );

        // Dimensions should match mock sensor
        let (w, h) = cam.resolution();
        assert_eq!(frame.width, w);
        assert_eq!(frame.height, h);
    }

    #[tokio::test]
    async fn pixel_data_non_empty_and_correct_size() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        cam.stop_stream().await.expect("stop_stream");

        let data = frame.pixel_data();
        assert!(!data.is_empty(), "pixel_data() should not be empty");

        // For 16-bit 2048x2048: 2048 * 2048 * 2 = 8_388_608 bytes
        let expected_len = (frame.width as usize) * (frame.height as usize) * 2;
        assert_eq!(
            data.len(),
            expected_len,
            "pixel data length should be width * height * 2"
        );
    }

    #[tokio::test]
    async fn pixel_data_contains_gradient_pattern() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        cam.stop_stream().await.expect("stop_stream");

        let data = frame.pixel_data();

        // Verify the gradient pattern: value = (x + y + offset) % 65535
        // For frame_number 0, offset = 0 % 100 = 0
        let offset = (frame.frame_number % 100) as u16;

        // Check a few pixels at known positions
        // Pixel at (0, 0): value = (0 + 0 + offset) % 65535
        let val_00 = u16::from_le_bytes([data[0], data[1]]);
        assert_eq!(
            val_00,
            (u32::from(offset) % 65535) as u16,
            "pixel (0,0) should match gradient formula"
        );

        // Pixel at (1, 0): value = (1 + 0 + offset) % 65535
        let val_10 = u16::from_le_bytes([data[2], data[3]]);
        assert_eq!(
            val_10,
            ((1 + u32::from(offset)) % 65535) as u16,
            "pixel (1,0) should match gradient formula"
        );

        // Pixel at (0, 1): value = (0 + 1 + offset) % 65535
        let stride = frame.width as usize * 2;
        let val_01 = u16::from_le_bytes([data[stride], data[stride + 1]]);
        assert_eq!(
            val_01,
            ((1 + u32::from(offset)) % 65535) as u16,
            "pixel (0,1) should match gradient formula"
        );
    }

    #[tokio::test]
    async fn frame_pool_returns_after_drop() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Receive frames and immediately drop them (return to pool)
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
            drop(frame); // Explicitly return to pool
        }

        cam.stop_stream().await.expect("stop_stream");
        // If pool return is broken, we would have run out of buffers above
    }
}

// =============================================================================
// Observer (TapRegistry) Notification
// =============================================================================

#[cfg(not(feature = "camera"))]
mod observer_notification {
    use super::*;

    /// Shared state between the observer and the test.
    struct ObserverState {
        count: AtomicU64,
        last_frame_number: AtomicU64,
    }

    impl ObserverState {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                count: AtomicU64::new(0),
                last_frame_number: AtomicU64::new(u64::MAX),
            })
        }

        fn count(&self) -> u64 {
            self.count.load(Ordering::Relaxed)
        }

        fn last_frame_number(&self) -> u64 {
            self.last_frame_number.load(Ordering::Relaxed)
        }
    }

    /// Observer that increments shared counters on each frame.
    struct CountingObserver {
        state: Arc<ObserverState>,
    }

    impl CountingObserver {
        fn new(state: Arc<ObserverState>) -> Self {
            Self { state }
        }
    }

    impl FrameObserver for CountingObserver {
        fn on_frame(&self, frame: &FrameView<'_>) {
            self.state.count.fetch_add(1, Ordering::Relaxed);
            self.state
                .last_frame_number
                .store(frame.frame_number, Ordering::Relaxed);
        }

        fn name(&self) -> &'static str {
            "counting_observer"
        }
    }

    #[tokio::test]
    async fn observer_receives_frames() {
        let cam = mock_camera_with_exposure(0.005).await;

        let state = ObserverState::new();
        let _handle = cam
            .register_observer(Box::new(CountingObserver::new(state.clone())))
            .await
            .expect("register_observer");

        // Also register primary output (mock loop sends to both)
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Wait for a few frames via primary channel
        for _ in 0..3 {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
        }

        cam.stop_stream().await.expect("stop_stream");

        // Observer should have been called at least as many times
        assert!(
            state.count() >= 3,
            "observer should have received >= 3 frames, got {}",
            state.count()
        );
    }

    #[tokio::test]
    async fn observer_frame_numbers_match_primary() {
        let cam = mock_camera_with_exposure(0.005).await;

        let state = ObserverState::new();
        let _handle = cam
            .register_observer(Box::new(CountingObserver::new(state.clone())))
            .await
            .expect("register_observer");

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Receive 5 frames from primary
        let mut last_primary_nr = 0;
        for _ in 0..5 {
            let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
            last_primary_nr = frame.frame_number;
        }

        cam.stop_stream().await.expect("stop_stream");

        // Observer's last_frame_number should match or exceed primary's
        assert!(
            state.last_frame_number() >= last_primary_nr,
            "observer last_frame_number ({}) should match primary ({})",
            state.last_frame_number(),
            last_primary_nr
        );
    }

    #[tokio::test]
    async fn unregister_observer_stops_notifications() {
        let cam = mock_camera_with_exposure(0.005).await;

        let state = ObserverState::new();
        let handle: ObserverHandle = cam
            .register_observer(Box::new(CountingObserver::new(state.clone())))
            .await
            .expect("register_observer");

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Receive a couple of frames
        for _ in 0..2 {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
        }

        let count_before = state.count();
        assert!(count_before >= 2, "should have >= 2 before unregister");

        // Unregister observer
        cam.unregister_observer(handle)
            .await
            .expect("unregister_observer");

        // Receive more frames
        for _ in 0..3 {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
        }

        cam.stop_stream().await.expect("stop_stream");

        let count_after = state.count();
        // After unregister, count should not increase (or only marginally
        // if there was a race between unregister and notification)
        assert!(
            count_after <= count_before + 1,
            "observer should stop receiving after unregister: before={}, after={}",
            count_before,
            count_after
        );
    }

    #[tokio::test]
    async fn multiple_observers_all_notified() {
        let cam = mock_camera_with_exposure(0.005).await;

        let state_a = ObserverState::new();
        let state_b = ObserverState::new();

        let _handle_a = cam
            .register_observer(Box::new(CountingObserver::new(state_a.clone())))
            .await
            .expect("register observer A");
        let _handle_b = cam
            .register_observer(Box::new(CountingObserver::new(state_b.clone())))
            .await
            .expect("register observer B");

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        for _ in 0..3 {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
        }

        cam.stop_stream().await.expect("stop_stream");

        assert!(
            state_a.count() >= 3,
            "observer A should receive >= 3, got {}",
            state_a.count()
        );
        assert!(
            state_b.count() >= 3,
            "observer B should receive >= 3, got {}",
            state_b.count()
        );
    }
}

// =============================================================================
// Start / Stop Lifecycle
// =============================================================================

#[cfg(not(feature = "camera"))]
mod lifecycle {
    use super::*;

    #[tokio::test]
    async fn double_start_rejected() {
        let cam = mock_camera_with_exposure(0.005).await;

        cam.start_stream().await.expect("first start_stream");

        // Second start should fail
        let result = cam.start_stream().await;
        assert!(result.is_err(), "double start_stream should return error");

        cam.stop_stream().await.expect("stop_stream");
    }

    #[tokio::test]
    async fn stop_without_start_is_ok() {
        let cam = mock_camera_with_exposure(0.005).await;

        // stop_stream on non-streaming camera should be no-op
        cam.stop_stream()
            .await
            .expect("stop_stream without start should be Ok");
    }

    #[tokio::test]
    async fn start_stop_start_cycle() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        // First cycle
        cam.start_stream().await.expect("start_stream #1");

        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout #1")
            .expect("channel closed #1");
        assert_eq!(frame.frame_number, 0, "first cycle starts at frame 0");

        cam.stop_stream().await.expect("stop_stream #1");

        // Re-register output for second cycle (old channel may be stale)
        let (tx2, mut rx2) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx2)
            .await
            .expect("register_primary_output #2");

        // Second cycle
        cam.start_stream().await.expect("start_stream #2");

        let frame2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("timeout #2")
            .expect("channel closed #2");
        // frame_count is reset to 0 on start_stream, so frame_number starts at 0 again
        assert_eq!(
            frame2.frame_number, 0,
            "second cycle should reset frame_number to 0"
        );

        cam.stop_stream().await.expect("stop_stream #2");
    }

    #[tokio::test]
    async fn channel_closes_after_stop() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Receive one frame
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        cam.stop_stream().await.expect("stop_stream");

        // Drain any buffered frames, then the channel should yield None or timeout
        loop {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Some(_)) => {}  // buffered frame, keep draining
                Ok(None) => break, // channel closed
                Err(_) => break,   // timeout — no more frames
            }
        }
    }
}

// =============================================================================
// Backpressure Handling
// =============================================================================

#[cfg(not(feature = "camera"))]
mod backpressure {
    use super::*;

    #[tokio::test]
    async fn small_channel_drops_gracefully() {
        let cam = mock_camera_with_exposure(0.002).await;

        // Very small channel — will fill up quickly
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Wait for several frames to be produced (many will be dropped)
        tokio::time::sleep(Duration::from_millis(50)).await;

        // We should still be able to receive at least one frame
        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert!(
            frame.width > 0,
            "should receive valid frame despite backpressure"
        );

        cam.stop_stream().await.expect("stop_stream");
    }

    #[tokio::test]
    #[ignore = "SimCam backpressure test times out in resource-constrained CI (180s × 4 retries)"]
    async fn stream_continues_after_slow_consumer() {
        let cam = mock_camera_with_exposure(0.005).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        cam.register_primary_output(tx)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream");

        // Simulate slow consumer: receive one, wait, then receive more
        let frame1 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        // Slow consumer delay — frames may be dropped
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Drain buffered frame
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(10), rx.recv()).await {}

        // Should still receive new frames
        let frame2 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout after slow consumer")
            .expect("channel closed after slow consumer");

        assert!(
            frame2.frame_number > frame1.frame_number,
            "new frame ({}) should come after first frame ({})",
            frame2.frame_number,
            frame1.frame_number
        );

        cam.stop_stream().await.expect("stop_stream");
    }
}

// =============================================================================
// Exposure Change Between Streams
// =============================================================================

#[cfg(not(feature = "camera"))]
mod exposure_change {
    use super::*;

    #[tokio::test]
    async fn exposure_change_reflected_in_frame_metadata() {
        let cam = AndorCamera::new_mock()
            .await
            .expect("Should create mock camera");

        // Stream 1: 10ms exposure
        cam.set_exposure(0.010).await.expect("set exposure 10ms");

        let (tx1, mut rx1) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx1)
            .await
            .expect("register_primary_output");

        cam.start_stream().await.expect("start_stream #1");

        let frame1 = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
            .await
            .expect("timeout #1")
            .expect("channel closed #1");

        cam.stop_stream().await.expect("stop_stream #1");

        assert!(
            (frame1.exposure_ms - 10.0).abs() < 0.5,
            "frame1 exposure_ms ({}) should be ~10ms",
            frame1.exposure_ms
        );

        // Stream 2: 50ms exposure
        cam.set_exposure(0.050).await.expect("set exposure 50ms");

        let (tx2, mut rx2) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx2)
            .await
            .expect("register_primary_output #2");

        cam.start_stream().await.expect("start_stream #2");

        let frame2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("timeout #2")
            .expect("channel closed #2");

        cam.stop_stream().await.expect("stop_stream #2");

        assert!(
            (frame2.exposure_ms - 50.0).abs() < 0.5,
            "frame2 exposure_ms ({}) should be ~50ms",
            frame2.exposure_ms
        );
    }
}
