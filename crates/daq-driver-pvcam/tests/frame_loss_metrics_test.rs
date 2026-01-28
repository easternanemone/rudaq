//! Integration tests for frame loss metrics (bd-dmbl, bd-ek9n.3)
//!
//! Tests the frame loss tracking API on PvcamAcquisition:
//! - `frame_loss_stats()` returns (lost_frames, discontinuity_events, dropped_frames)
//! - `dropped_frame_count()` returns just the dropped_frames count
//! - `reset_frame_loss_metrics()` clears all counters
//!
//! ## Running Tests
//!
//! ```bash
//! # Mock mode tests (no hardware required)
//! cargo test -p daq-driver-pvcam --test frame_loss_metrics_test
//! ```

use common::parameter::Parameter;
use std::sync::atomic::Ordering;

// Import the acquisition module to access PvcamAcquisition
// Note: We access this through the public API since the struct is pub
use daq_driver_pvcam::components::acquisition::PvcamAcquisition;

// =============================================================================
// Frame Loss Metrics Tests
// =============================================================================

/// Test that frame_loss_stats() returns correct initial values.
///
/// All counters should be zero when a new PvcamAcquisition is created.
#[test]
fn test_frame_loss_stats_initial_values() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    // frame_loss_stats() returns (lost_frames, discontinuity_events, dropped_frames)
    let (lost, discontinuities, dropped) = acquisition.frame_loss_stats();

    assert_eq!(lost, 0, "lost_frames should be 0 initially");
    assert_eq!(
        discontinuities, 0,
        "discontinuity_events should be 0 initially"
    );
    assert_eq!(dropped, 0, "dropped_frames should be 0 initially");
}

/// Test that dropped_frame_count() returns correct initial value.
#[test]
fn test_dropped_frame_count_initial_value() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    let dropped = acquisition.dropped_frame_count();
    assert_eq!(dropped, 0, "dropped_frame_count should be 0 initially");
}

/// Test that frame loss counters can be incremented and read correctly.
///
/// This simulates what happens during actual acquisition when:
/// - Frames are lost due to buffer overflow (lost_frames)
/// - Gaps are detected in frame sequence (discontinuity_events)
/// - Frames are dropped due to pool exhaustion (dropped_frames)
#[test]
fn test_frame_loss_counters_increment() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    // Simulate frame loss events by incrementing the atomic counters
    // These are public fields on PvcamAcquisition
    acquisition.lost_frames.fetch_add(5, Ordering::SeqCst);
    acquisition
        .discontinuity_events
        .fetch_add(2, Ordering::SeqCst);
    acquisition.dropped_frames.fetch_add(3, Ordering::SeqCst);

    // Verify frame_loss_stats() reflects the updates
    let (lost, discontinuities, dropped) = acquisition.frame_loss_stats();
    assert_eq!(lost, 5, "lost_frames should be 5 after incrementing");
    assert_eq!(
        discontinuities, 2,
        "discontinuity_events should be 2 after incrementing"
    );
    assert_eq!(dropped, 3, "dropped_frames should be 3 after incrementing");

    // Verify dropped_frame_count() convenience method
    let dropped_count = acquisition.dropped_frame_count();
    assert_eq!(dropped_count, 3, "dropped_frame_count should be 3");
}

/// Test that reset_frame_loss_metrics() clears all counters.
///
/// This is called at the start of each new acquisition to reset statistics.
#[test]
fn test_reset_frame_loss_metrics() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    // Set some non-zero values
    acquisition.lost_frames.fetch_add(10, Ordering::SeqCst);
    acquisition
        .discontinuity_events
        .fetch_add(5, Ordering::SeqCst);
    acquisition.dropped_frames.fetch_add(7, Ordering::SeqCst);

    // Verify they're non-zero
    let (lost, discontinuities, dropped) = acquisition.frame_loss_stats();
    assert_eq!(lost, 10);
    assert_eq!(discontinuities, 5);
    assert_eq!(dropped, 7);

    // Reset all metrics
    acquisition.reset_frame_loss_metrics();

    // Verify all counters are back to zero
    let (lost, discontinuities, dropped) = acquisition.frame_loss_stats();
    assert_eq!(lost, 0, "lost_frames should be 0 after reset");
    assert_eq!(
        discontinuities, 0,
        "discontinuity_events should be 0 after reset"
    );
    assert_eq!(dropped, 0, "dropped_frames should be 0 after reset");

    // Also verify the convenience method
    assert_eq!(
        acquisition.dropped_frame_count(),
        0,
        "dropped_frame_count should be 0 after reset"
    );
}

/// Test that multiple increments accumulate correctly.
///
/// Simulates sustained backpressure where many frames are dropped.
#[test]
fn test_dropped_frames_accumulation() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    // Simulate 100 dropped frames (as would happen during sustained backpressure)
    for _ in 0..100 {
        acquisition.dropped_frames.fetch_add(1, Ordering::SeqCst);
    }

    assert_eq!(
        acquisition.dropped_frame_count(),
        100,
        "Should accumulate 100 dropped frames"
    );

    // Verify it's also reflected in frame_loss_stats()
    let (_, _, dropped) = acquisition.frame_loss_stats();
    assert_eq!(
        dropped, 100,
        "frame_loss_stats should show 100 dropped frames"
    );
}

/// Test that frame_loss_stats() returns consistent values.
///
/// The tuple order should always be (lost_frames, discontinuity_events, dropped_frames).
#[test]
fn test_frame_loss_stats_tuple_order() {
    let streaming = Parameter::new("test.streaming", false);
    let buffer_mode = Parameter::new("test.buffer_mode", "Overwrite".to_string());
    let acquisition = PvcamAcquisition::new(streaming, buffer_mode);

    // Set distinct values to verify tuple order
    acquisition.lost_frames.store(111, Ordering::SeqCst);
    acquisition
        .discontinuity_events
        .store(222, Ordering::SeqCst);
    acquisition.dropped_frames.store(333, Ordering::SeqCst);

    let stats = acquisition.frame_loss_stats();

    // Verify tuple order: (lost_frames, discontinuity_events, dropped_frames)
    assert_eq!(stats.0, 111, "First element should be lost_frames (111)");
    assert_eq!(
        stats.1, 222,
        "Second element should be discontinuity_events (222)"
    );
    assert_eq!(stats.2, 333, "Third element should be dropped_frames (333)");
}
