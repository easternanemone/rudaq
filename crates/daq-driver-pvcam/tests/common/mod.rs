//! Shared test utilities for PVCAM continuous acquisition validation tests.
//!
//! This module provides reusable components for testing:
//! - `TestCamera`: Wrapper for camera setup/teardown with common operations
//! - `TestStats`: Statistics collection for test result analysis
//! - `FrameValidator`: Frame data integrity validation
//! - Assertion helpers with tolerance support for performance tests
//!
//! See: `docs/architecture/adr-pvcam-continuous-acquisition.md` for background.

#![allow(dead_code)] // Utilities may not all be used in every test file

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

// Re-export Frame type for convenience
#[cfg(feature = "pvcam_hardware")]
pub use daq_core::capabilities::Frame;

/// Test statistics collected during continuous acquisition validation.
#[derive(Debug, Clone)]
pub struct TestStats {
    /// Total test duration
    pub duration: Duration,
    /// Number of frames received
    pub frame_count: u64,
    /// Calculated frames per second
    pub fps: f64,
    /// Expected frame count based on exposure time
    pub expected_frames: u64,
    /// Number of frames skipped (gaps in frame numbering)
    pub skipped_frames: u64,
    /// Number of duplicate frame numbers detected
    pub duplicate_frames: u64,
    /// Number of timeout errors (no frame received within threshold)
    pub timeout_errors: u64,
    /// Number of channel errors (broadcast channel issues)
    pub channel_errors: u64,
    /// First frame number received
    pub first_frame_nr: Option<i32>,
    /// Last frame number received
    pub last_frame_nr: Option<i32>,
}

impl TestStats {
    /// Create new empty stats
    pub fn new() -> Self {
        Self {
            duration: Duration::ZERO,
            frame_count: 0,
            fps: 0.0,
            expected_frames: 0,
            skipped_frames: 0,
            duplicate_frames: 0,
            timeout_errors: 0,
            channel_errors: 0,
            first_frame_nr: None,
            last_frame_nr: None,
        }
    }

    /// Calculate FPS from duration and frame count
    pub fn calculate_fps(&mut self) {
        if self.duration.as_secs_f64() > 0.0 {
            self.fps = self.frame_count as f64 / self.duration.as_secs_f64();
        }
    }

    /// Calculate expected frames based on exposure time
    pub fn calculate_expected(&mut self, exposure_ms: f64) {
        let theoretical_fps = 1000.0 / exposure_ms;
        self.expected_frames = (theoretical_fps * self.duration.as_secs_f64()) as u64;
    }

    /// Get frame loss percentage
    pub fn frame_loss_pct(&self) -> f64 {
        if self.expected_frames == 0 {
            return 0.0;
        }
        let received = self.frame_count as f64;
        let expected = self.expected_frames as f64;
        ((expected - received) / expected * 100.0).max(0.0)
    }

    /// Check if test had any errors
    pub fn has_errors(&self) -> bool {
        self.timeout_errors > 0 || self.channel_errors > 0 || self.duplicate_frames > 0
    }

    /// Print summary to stdout
    pub fn print_summary(&self, test_name: &str) {
        println!("\n=== {} Results ===", test_name);
        println!("Duration: {:?}", self.duration);
        println!("Frames received: {}", self.frame_count);
        println!("FPS: {:.1}", self.fps);
        if self.expected_frames > 0 {
            println!("Expected frames: {} ({:.1}% loss)",
                     self.expected_frames, self.frame_loss_pct());
        }
        if self.skipped_frames > 0 {
            println!("Skipped frames: {}", self.skipped_frames);
        }
        if self.duplicate_frames > 0 {
            println!("Duplicate frames: {} (ERROR)", self.duplicate_frames);
        }
        if self.timeout_errors > 0 {
            println!("Timeout errors: {}", self.timeout_errors);
        }
        if self.channel_errors > 0 {
            println!("Channel errors: {}", self.channel_errors);
        }
        if let (Some(first), Some(last)) = (self.first_frame_nr, self.last_frame_nr) {
            println!("Frame range: {} - {}", first, last);
        }
    }
}

impl Default for TestStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame statistics collector for tracking frame numbering during tests.
#[derive(Debug)]
pub struct FrameTracker {
    last_frame_nr: Option<i32>,
    first_frame_nr: Option<i32>,
    frame_count: u64,
    skipped: u64,
    duplicates: u64,
}

impl FrameTracker {
    /// Create new frame tracker
    pub fn new() -> Self {
        Self {
            last_frame_nr: None,
            first_frame_nr: None,
            frame_count: 0,
            skipped: 0,
            duplicates: 0,
        }
    }

    /// Record a frame and track numbering anomalies
    #[cfg(feature = "pvcam_hardware")]
    pub fn record_frame(&mut self, frame: &Frame) {
        self.record_frame_nr(frame.frame_number as i32);
    }

    /// Record a frame number directly (for low-level tests)
    pub fn record_frame_nr(&mut self, frame_nr: i32) {
        self.frame_count += 1;

        if self.first_frame_nr.is_none() {
            self.first_frame_nr = Some(frame_nr);
        }

        if let Some(last) = self.last_frame_nr {
            if frame_nr == last {
                self.duplicates += 1;
            } else if frame_nr > last + 1 {
                // Frames were skipped (unexpected under FIFO retrieval)
                self.skipped += (frame_nr - last - 1) as u64;
            }
            // Note: frame_nr < last would be a serious error (out of order)
        }

        self.last_frame_nr = Some(frame_nr);
    }

    /// Export statistics to TestStats
    pub fn export_to_stats(&self, stats: &mut TestStats) {
        stats.frame_count = self.frame_count;
        stats.skipped_frames = self.skipped;
        stats.duplicate_frames = self.duplicates;
        stats.first_frame_nr = self.first_frame_nr;
        stats.last_frame_nr = self.last_frame_nr;
    }
}

impl Default for FrameTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame data validator for integrity checks.
#[derive(Debug)]
pub struct FrameValidator {
    expected_width: u32,
    expected_height: u32,
    expected_pixel_count: usize,
    bytes_per_pixel: usize,
}

impl FrameValidator {
    /// Create validator with expected dimensions (assumes 16-bit pixels)
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_bit_depth(width, height, 16)
    }

    /// Create validator with specific bit depth
    pub fn with_bit_depth(width: u32, height: u32, bit_depth: u32) -> Self {
        let bytes_per_pixel = ((bit_depth + 7) / 8) as usize; // Round up to bytes
        Self {
            expected_width: width,
            expected_height: height,
            expected_pixel_count: (width * height) as usize,
            bytes_per_pixel,
        }
    }

    /// Create validator for 256x256 test ROI (common in probe tests)
    pub fn for_test_roi() -> Self {
        Self::new(256, 256)
    }

    /// Create validator for full Prime BSI sensor
    pub fn for_prime_bsi() -> Self {
        Self::new(2048, 2048)
    }

    /// Expected data size in bytes
    pub fn expected_bytes(&self) -> usize {
        self.expected_pixel_count * self.bytes_per_pixel
    }

    /// Validate a frame's dimensions and data
    #[cfg(feature = "pvcam_hardware")]
    pub fn validate(&self, frame: &Frame) -> Result<(), String> {
        if frame.width != self.expected_width {
            return Err(format!(
                "Width mismatch: expected {}, got {}",
                self.expected_width, frame.width
            ));
        }

        if frame.height != self.expected_height {
            return Err(format!(
                "Height mismatch: expected {}, got {}",
                self.expected_height, frame.height
            ));
        }

        let expected_bytes = self.expected_bytes();
        if frame.data.len() != expected_bytes {
            return Err(format!(
                "Data size mismatch: expected {} bytes ({} pixels × {} bytes/pixel), got {} bytes",
                expected_bytes, self.expected_pixel_count, self.bytes_per_pixel, frame.data.len()
            ));
        }

        if frame.data.is_empty() {
            return Err("Frame data is empty".to_string());
        }

        Ok(())
    }

    /// Check if frame appears to be all zeros (uninitialized buffer)
    #[cfg(feature = "pvcam_hardware")]
    pub fn is_zero_frame(frame: &Frame) -> bool {
        // Check first 100 pixels for efficiency
        let check_count = frame.data.len().min(100);
        frame.data.iter().take(check_count).all(|&p| p == 0)
    }
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Assert that actual FPS is within tolerance of expected FPS.
///
/// # Arguments
/// * `actual` - Measured FPS
/// * `expected` - Target FPS
/// * `tolerance_pct` - Allowed deviation as percentage (e.g., 10.0 for ±10%)
/// * `context` - Test context for error message
///
/// # Panics
/// Panics if actual FPS is outside the tolerance range.
pub fn assert_fps_near(actual: f64, expected: f64, tolerance_pct: f64, context: &str) {
    let tolerance = expected * (tolerance_pct / 100.0);
    let min_fps = expected - tolerance;
    let max_fps = expected + tolerance;

    assert!(
        actual >= min_fps && actual <= max_fps,
        "{}: FPS {:.1} outside tolerance range [{:.1}, {:.1}] (expected {:.1} ±{:.0}%)",
        context,
        actual,
        min_fps,
        max_fps,
        expected,
        tolerance_pct
    );
}

/// Assert that frame count meets minimum requirement.
///
/// # Arguments
/// * `actual` - Actual frame count
/// * `expected_min` - Minimum acceptable frame count
/// * `context` - Test context for error message
///
/// # Panics
/// Panics if actual count is below minimum.
pub fn assert_frame_count_min(actual: u64, expected_min: u64, context: &str) {
    assert!(
        actual >= expected_min,
        "{}: Frame count {} below minimum {} required",
        context,
        actual,
        expected_min
    );
}

/// Assert that no duplicate frame numbers were detected.
///
/// # Arguments
/// * `duplicates` - Count of duplicate frames detected
/// * `context` - Test context for error message
///
/// # Panics
/// Panics if any duplicates were detected.
pub fn assert_no_duplicate_frames(duplicates: u64, context: &str) {
    assert_eq!(
        duplicates, 0,
        "{}: Detected {} duplicate frame numbers (should be 0)",
        context, duplicates
    );
}

/// Assert that error count is within acceptable limit.
///
/// # Arguments
/// * `errors` - Total error count
/// * `max_errors` - Maximum acceptable errors
/// * `context` - Test context for error message
///
/// # Panics
/// Panics if error count exceeds limit.
pub fn assert_errors_within_limit(errors: u64, max_errors: u64, context: &str) {
    assert!(
        errors <= max_errors,
        "{}: Error count {} exceeds maximum {} allowed",
        context,
        errors,
        max_errors
    );
}

/// Assert that frame loss is within acceptable percentage.
///
/// # Arguments
/// * `stats` - Test statistics containing frame count and expected frames
/// * `max_loss_pct` - Maximum acceptable frame loss percentage
/// * `context` - Test context for error message
///
/// # Panics
/// Panics if frame loss exceeds limit.
pub fn assert_frame_loss_within_limit(stats: &TestStats, max_loss_pct: f64, context: &str) {
    let loss = stats.frame_loss_pct();
    assert!(
        loss <= max_loss_pct,
        "{}: Frame loss {:.1}% exceeds maximum {:.1}% allowed",
        context,
        loss,
        max_loss_pct
    );
}

// ============================================================================
// Test Duration Helpers
// ============================================================================

/// Standard test durations for consistency across tests.
pub mod durations {
    use std::time::Duration;

    /// Quick functional test (1 second)
    pub const QUICK: Duration = Duration::from_secs(1);

    /// Standard performance test (5 seconds)
    pub const STANDARD: Duration = Duration::from_secs(5);

    /// Extended stability test (10 seconds)
    pub const EXTENDED: Duration = Duration::from_secs(10);

    /// Long stability test (30 seconds)
    pub const LONG: Duration = Duration::from_secs(30);

    /// Frame receive timeout (how long to wait for a single frame)
    pub const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

    /// Stall detection timeout (indicates buffer stall if exceeded)
    pub const STALL_TIMEOUT: Duration = Duration::from_secs(2);
}

/// Standard exposure times for test consistency.
pub mod exposures {
    /// Fast exposure for high FPS tests (10ms = ~100 FPS)
    pub const FAST_MS: f64 = 10.0;
    pub const FAST_SEC: f64 = 0.010;

    /// Standard exposure for reliable operation (100ms = ~10 FPS)
    pub const STANDARD_MS: f64 = 100.0;
    pub const STANDARD_SEC: f64 = 0.100;

    /// Slow exposure for timing tests (500ms = ~2 FPS)
    pub const SLOW_MS: f64 = 500.0;
    pub const SLOW_SEC: f64 = 0.500;
}

// ============================================================================
// PVCAM Error Helper (for low-level tests)
// ============================================================================

/// Get PVCAM SDK error message.
#[cfg(feature = "pvcam_hardware")]
pub fn get_pvcam_error() -> String {
    use pvcam_sys::*;
    use std::ffi::CStr;

    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Get PVCAM SDK error code.
#[cfg(feature = "pvcam_hardware")]
pub fn get_pvcam_error_code() -> i16 {
    unsafe { pvcam_sys::pl_error_code() }
}
