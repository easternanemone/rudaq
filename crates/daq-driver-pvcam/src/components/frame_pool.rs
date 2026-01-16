//! Frame pool for zero-allocation frame handling in PVCAM acquisition.
//!
//! This module provides the `FrameData` type and pool factory functions for
//! high-performance frame processing. By pre-allocating frame buffers, we
//! eliminate per-frame heap allocations (~8MB per frame at 100 FPS).
//!
//! # Design (bd-0dax.3)
//!
//! The PVCAM SDK controls when circular buffer slots are reused. We MUST copy
//! frame data from the SDK buffer before calling `pl_exp_unlock_oldest_frame()`.
//!
//! - **Before**: Copy into freshly allocated `Vec<u8>` (8 MB allocation per frame)
//! - **After**: Copy into pre-allocated pool slot (0 allocation after warmup)
//!
//! # Safety
//!
//! The `FramePool` uses the `daq_pool::Pool` which provides:
//! - Semaphore-based slot tracking
//! - Lock-free access after acquisition (bd-0dax.1.6 RwLock fix)
//! - Configurable timeout for backpressure detection (bd-0dax.3.6)
//!
//! # Example
//!
//! ```ignore
//! use crate::components::frame_pool::{create_frame_pool, FramePool, LoanedFrame};
//!
//! // Create pool matching SDK buffer count (30 slots default, ~240MB for 8MB frames)
//! let pool = create_frame_pool(30, 8 * 1024 * 1024);
//!
//! // In frame loop: acquire slot, copy, return to pool on drop
//! let mut frame = pool.try_acquire().expect("pool exhausted");
//! unsafe {
//!     std::ptr::copy_nonoverlapping(sdk_ptr, frame.pixels.as_mut_ptr(), frame_bytes);
//! }
//! frame.actual_len = frame_bytes;
//! ```

use daq_pool::{Loaned, Pool};
use std::sync::Arc;

/// Default pool size: 30 frames provides ~300ms headroom at 100 FPS.
///
/// This matches the SDK's typical 20-slot circular buffer with 50% margin
/// for consumer latency (storage writes, GUI updates, gRPC transmission).
pub const DEFAULT_POOL_SIZE: usize = 30;

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
// Type Aliases
// ============================================================================

/// Pool of pre-allocated frame data slots.
pub type FramePool = Arc<Pool<FrameData>>;

/// Loaned frame from pool (auto-returns on drop).
pub type LoanedFrame = Loaned<FrameData>;

// ============================================================================
// Factory Functions
// ============================================================================

/// Create a frame pool with the specified size and buffer capacity.
///
/// # Arguments
///
/// - `pool_size`: Number of frame slots to pre-allocate
/// - `frame_capacity`: Byte capacity per frame buffer
///
/// # Pool Sizing Guidance (bd-0dax.3.7)
///
/// | SDK Buffer | Recommended Pool | Memory Usage | Rationale |
/// |------------|------------------|--------------|-----------|
/// | 20 frames  | 30 frames        | 240 MB       | 50% headroom for consumer latency |
/// | 32 frames  | 48 frames        | 384 MB       | 50% headroom for consumer latency |
///
/// Default of 30 slots covers ~300ms of frames at 100 FPS, sufficient for:
/// - Storage write latency (~10-50ms)
/// - GUI update latency (~16ms at 60 FPS)
/// - Network latency (~10-100ms)
/// - Occasional pipeline stalls (~200ms)
///
/// # Example
///
/// ```ignore
/// // Create pool for 2048x2048x16bit frames (~8MB each)
/// let frame_bytes = 2048 * 2048 * 2;
/// let pool = create_frame_pool(30, frame_bytes);
/// ```
#[must_use]
pub fn create_frame_pool(pool_size: usize, frame_capacity: usize) -> FramePool {
    tracing::info!(
        pool_size,
        frame_capacity_mb = frame_capacity as f64 / (1024.0 * 1024.0),
        total_mb = (pool_size * frame_capacity) as f64 / (1024.0 * 1024.0),
        "Creating frame pool"
    );

    Pool::new_with_reset(
        pool_size,
        move || FrameData::with_capacity(frame_capacity),
        FrameData::reset,
    )
}

/// Create a frame pool with default size and specified buffer capacity.
///
/// Uses `DEFAULT_POOL_SIZE` (30 frames) for the pool size.
#[must_use]
pub fn create_default_frame_pool(frame_capacity: usize) -> FramePool {
    create_frame_pool(DEFAULT_POOL_SIZE, frame_capacity)
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
        let src_data = vec![0u8; 200];

        unsafe {
            frame.copy_from_sdk(src_data.as_ptr(), src_data.len());
        }
    }

    #[tokio::test]
    async fn test_frame_pool_creation() {
        let pool = create_frame_pool(4, 1024);
        assert_eq!(pool.size(), 4);
        assert_eq!(pool.available(), 4);
    }

    #[tokio::test]
    async fn test_frame_pool_acquire_release() {
        let pool = create_frame_pool(2, 1024);

        let frame1 = pool.acquire().await;
        assert_eq!(pool.available(), 1);
        assert_eq!(frame1.capacity(), 1024);

        drop(frame1);
        assert_eq!(pool.available(), 2);
    }

    #[tokio::test]
    async fn test_frame_pool_reset_on_release() {
        let pool = create_frame_pool(1, 1024);

        // Acquire and modify
        let mut frame = pool.acquire().await;
        frame.get_mut().frame_number = 42;
        frame.get_mut().actual_len = 512;
        drop(frame);

        // Acquire again - should be reset
        let frame2 = pool.acquire().await;
        assert_eq!(frame2.frame_number, 0);
        assert_eq!(frame2.actual_len, 0);
    }

    #[tokio::test]
    async fn test_frame_pool_try_acquire() {
        let pool = create_frame_pool(1, 1024);

        let frame1 = pool.try_acquire();
        assert!(frame1.is_some());

        let frame2 = pool.try_acquire();
        assert!(frame2.is_none()); // Pool exhausted
    }

    #[tokio::test]
    async fn test_frame_pool_timeout() {
        use std::time::Duration;

        let pool = create_frame_pool(1, 1024);
        let _held = pool.acquire().await;

        // Should timeout since pool is exhausted
        let result = pool.try_acquire_timeout(Duration::from_millis(10)).await;
        assert!(result.is_none());
    }
}
