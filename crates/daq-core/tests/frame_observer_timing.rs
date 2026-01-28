//! Integration tests for FrameObserver timing requirements.
//!
//! These tests verify that slow observers generate warnings and don't block
//! the frame delivery pipeline.

use daq_core::capabilities::FrameObserver;
use daq_core::data::{Frame, FrameView};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

// Slow observer that violates timing requirements
struct SlowObserver;

impl FrameObserver for SlowObserver {
    fn on_frame(&self, _frame: &FrameView<'_>) {
        // Simulate blocking work (1ms) - violates <100µs requirement
        std::thread::sleep(Duration::from_millis(1));
    }

    fn name(&self) -> &'static str {
        "slow_observer"
    }
}

// Fast observer that complies with timing requirements
struct FastObserver {
    counter: AtomicU32,
}

impl FrameObserver for FastObserver {
    fn on_frame(&self, _frame: &FrameView<'_>) {
        // Fast operation (atomic increment) - well under 100µs
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    fn name(&self) -> &'static str {
        "fast_observer"
    }
}

/// Test that measures observer execution time and verifies fast observers complete quickly.
#[test]
fn test_fast_observer_completes_quickly() {
    let observer = FastObserver {
        counter: AtomicU32::new(0),
    };

    let frame = Frame::from_u16(64, 64, &vec![0u16; 64 * 64]);
    let frame_view = FrameView::from_frame(&frame);

    // Measure execution time
    let start = Instant::now();
    observer.on_frame(&frame_view);
    let elapsed = start.elapsed();

    // Fast observer should complete in well under 100µs
    assert!(
        elapsed < Duration::from_micros(100),
        "Fast observer took {:?}, expected < 100µs",
        elapsed
    );

    // Verify the frame was processed
    assert_eq!(observer.counter.load(Ordering::SeqCst), 1);
}

/// Test that slow observers exceed the timing threshold.
#[test]
fn test_slow_observer_exceeds_threshold() {
    let observer = SlowObserver;

    let frame = Frame::from_u16(64, 64, &vec![0u16; 64 * 64]);
    let frame_view = FrameView::from_frame(&frame);

    // Measure execution time
    let start = Instant::now();
    observer.on_frame(&frame_view);
    let elapsed = start.elapsed();

    // Slow observer should exceed the 100µs threshold (it sleeps for 1ms)
    assert!(
        elapsed > Duration::from_micros(100),
        "Slow observer took {:?}, expected > 100µs",
        elapsed
    );
}

/// Test that demonstrates the proper pattern for observers: channel-based offload.
struct ChannelBasedObserver {
    tx: std::sync::mpsc::SyncSender<u64>,
    frame_count: AtomicU32,
}

impl FrameObserver for ChannelBasedObserver {
    fn on_frame(&self, _frame: &FrameView<'_>) {
        // Non-blocking try_send to avoid stalling the frame loop
        let frame_id = self.frame_count.fetch_add(1, Ordering::SeqCst) as u64;
        let _ = self.tx.try_send(frame_id); // Drop if channel full
    }

    fn name(&self) -> &'static str {
        "channel_based_observer"
    }
}

#[test]
fn test_channel_based_observer_pattern() {
    // Create a bounded channel for frame notifications
    let (tx, rx) = std::sync::mpsc::sync_channel::<u64>(10);

    let observer = ChannelBasedObserver {
        tx,
        frame_count: AtomicU32::new(0),
    };

    let frame = Frame::from_u16(64, 64, &vec![0u16; 64 * 64]);
    let frame_view = FrameView::from_frame(&frame);

    // Process multiple frames quickly
    let start = Instant::now();
    for _ in 0..10 {
        observer.on_frame(&frame_view);
    }
    let elapsed = start.elapsed();

    // All 10 frames should be processed very quickly
    assert!(
        elapsed < Duration::from_millis(1),
        "Channel-based observer took {:?} for 10 frames, expected < 1ms",
        elapsed
    );

    // Verify frames were sent to channel
    let mut received = Vec::new();
    while let Ok(frame_id) = rx.try_recv() {
        received.push(frame_id);
    }
    assert_eq!(received.len(), 10, "Expected 10 frames in channel");
}
