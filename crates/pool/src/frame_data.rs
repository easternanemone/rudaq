//! Frame data type for zero-allocation frame handling.
//!
//! This module provides the `FrameData` type for high-performance frame processing.
//! By storing frame data in a pool, we eliminate per-frame heap allocations
//! (~8MB per frame at 100 FPS).
//!
//! # Design (bd-0dax.3, bd-0dax.4)
//!
//! `FrameData` is designed for:
//! - **Zero-allocation reuse**: Fixed-capacity pixel buffer, never shrinks
//! - **Inline metadata**: No Box allocation for metadata fields
//! - **O(1) reset**: Clears metadata, preserves buffer capacity
//!
//! # Safety
//!
//! The `copy_from_sdk` method is unsafe as it copies from raw pointers.
//! Callers must ensure the source pointer is valid and the length doesn't
//! exceed the buffer capacity.

/// Frame data stored in pool slots.
///
/// Designed for zero-allocation reuse:
/// - Fixed-capacity pixel buffer (pre-allocated, never shrinks)
/// - Inline metadata (no Box allocation)
/// - O(1) reset function (clears metadata, preserves buffer)
///
/// # Memory Layout
///
/// - `pixels`: Pre-allocated Vec<u8> with capacity set at pool creation
/// - Metadata fields: ~100 bytes inline (no heap allocation)
/// - Total per slot: ~8MB + 100 bytes for 2048x2048x16bit frames
#[derive(Debug)]
pub struct FrameData {
    // === Pixel Data (pre-allocated, fixed capacity) ===
    /// Pre-allocated pixel buffer.
    /// Capacity is fixed at pool creation.
    /// `actual_len` indicates valid data (may be < capacity).
    pub pixels: Vec<u8>,

    /// Actual bytes written this frame (may be < pixels.capacity()).
    pub actual_len: usize,

    // === Frame Identity ===
    /// Driver-assigned monotonic frame number (never resets during acquisition).
    pub frame_number: u64,

    /// Hardware frame number from SDK (for gap detection).
    /// -1 indicates unset.
    pub hw_frame_nr: i32,

    // === Dimensions (may vary if ROI changes between acquisitions) ===
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,

    // === Timing ===
    /// Capture timestamp (nanoseconds since epoch, from hardware if available).
    pub timestamp_ns: u64,

    /// Exposure time in milliseconds.
    pub exposure_ms: f64,

    // === ROI ===
    pub roi_x: u32,
    pub roi_y: u32,

    // === Extended Metadata (inline, not boxed) ===
    /// Sensor temperature in Celsius (if available).
    pub temperature_c: Option<f64>,

    /// Binning factors (x, y).
    pub binning: Option<(u16, u16)>,
}

impl FrameData {
    /// Create a new FrameData with pre-allocated buffer.
    ///
    /// # Arguments
    /// - `byte_capacity`: Size of pixel buffer to pre-allocate
    ///
    /// # Panics
    /// Panics if `byte_capacity` is 0.
    #[must_use]
    pub fn with_capacity(byte_capacity: usize) -> Self {
        assert!(byte_capacity > 0, "frame buffer capacity must be > 0");

        // Pre-allocate and zero-fill the buffer
        let pixels = vec![0u8; byte_capacity];

        Self {
            pixels,
            actual_len: 0,
            frame_number: 0,
            hw_frame_nr: -1,
            width: 0,
            height: 0,
            bit_depth: 16,
            timestamp_ns: 0,
            exposure_ms: 0.0,
            roi_x: 0,
            roi_y: 0,
            temperature_c: None,
            binning: None,
        }
    }

    /// Reset metadata for pool reuse.
    ///
    /// **Does NOT zero pixel data** - this is intentional:
    /// - Zeroing 8MB = ~4ms overhead at 1GB/s memset
    /// - Previous frame data is overwritten by next memcpy anyway
    /// - No security concern (same process)
    ///
    /// Only resets metadata fields (~100 bytes), providing O(1) reset.
    pub fn reset(&mut self) {
        self.actual_len = 0;
        self.frame_number = 0;
        self.hw_frame_nr = -1;
        self.timestamp_ns = 0;
        self.temperature_c = None;
        self.binning = None;
        // Note: pixels buffer capacity preserved, not zeroed
    }

    /// Get the valid pixel data as a slice.
    ///
    /// Returns only the bytes that were actually written this frame,
    /// not the full pre-allocated capacity.
    #[inline]
    #[must_use]
    pub fn pixel_data(&self) -> &[u8] {
        &self.pixels[..self.actual_len]
    }

    /// Get the valid pixel data as a mutable slice.
    #[inline]
    #[must_use]
    pub fn pixel_data_mut(&mut self) -> &mut [u8] {
        &mut self.pixels[..self.actual_len]
    }

    /// Get the pre-allocated buffer capacity.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.pixels.capacity()
    }

    /// Copy frame data from SDK buffer into this slot.
    ///
    /// # Safety
    ///
    /// - `src` must point to valid memory of at least `len` bytes
    /// - `len` must not exceed `self.pixels.capacity()`
    ///
    /// # Panics
    ///
    /// Panics if `len > self.pixels.capacity()`.
    #[inline]
    pub unsafe fn copy_from_sdk(&mut self, src: *const u8, len: usize) {
        assert!(
            len <= self.pixels.capacity(),
            "frame data ({} bytes) exceeds buffer capacity ({} bytes)",
            len,
            self.pixels.capacity()
        );

        std::ptr::copy_nonoverlapping(src, self.pixels.as_mut_ptr(), len);
        self.actual_len = len;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_data_creation() {
        let frame = FrameData::with_capacity(1024);
        assert_eq!(frame.capacity(), 1024);
        assert_eq!(frame.actual_len, 0);
        assert_eq!(frame.frame_number, 0);
        assert_eq!(frame.hw_frame_nr, -1);
    }

    #[test]
    fn test_frame_data_reset() {
        let mut frame = FrameData::with_capacity(1024);
        frame.actual_len = 512;
        frame.frame_number = 42;
        frame.hw_frame_nr = 100;
        frame.timestamp_ns = 123456789;
        frame.temperature_c = Some(25.0);
        frame.binning = Some((2, 2));

        frame.reset();

        assert_eq!(frame.actual_len, 0);
        assert_eq!(frame.frame_number, 0);
        assert_eq!(frame.hw_frame_nr, -1);
        assert_eq!(frame.timestamp_ns, 0);
        assert!(frame.temperature_c.is_none());
        assert!(frame.binning.is_none());
        // Capacity should be preserved
        assert_eq!(frame.capacity(), 1024);
    }

    #[test]
    fn test_copy_from_sdk() {
        let mut frame = FrameData::with_capacity(1024);
        let src_data: Vec<u8> = (0..512).map(|i| i as u8).collect();

        unsafe {
            frame.copy_from_sdk(src_data.as_ptr(), src_data.len());
        }

        assert_eq!(frame.actual_len, 512);
        assert_eq!(frame.pixel_data(), &src_data[..]);
    }

    #[test]
    #[should_panic(expected = "frame data")]
    fn test_copy_from_sdk_overflow_panics() {
        let mut frame = FrameData::with_capacity(100);
        let src_data = [0u8; 200];

        unsafe {
            frame.copy_from_sdk(src_data.as_ptr(), src_data.len());
        }
    }
}
