//! Frame pool for zero-allocation frame handling in PVCAM acquisition.
//!
//! This module provides pool factory functions for high-performance frame processing.
//! By pre-allocating frame buffers, we eliminate per-frame heap allocations
//! (~8MB per frame at 100 FPS).
//!
//! # Design (bd-0dax.3, bd-0dax.4)
//!
//! The PVCAM SDK controls when circular buffer slots are reused. We MUST copy
//! frame data from the SDK buffer before calling `pl_exp_unlock_oldest_frame()`.
//!
//! - **Before**: Copy into freshly allocated `Vec<u8>` (8 MB allocation per frame)
//! - **After**: Copy into pre-allocated pool slot (0 allocation after warmup)
//!
//! # Frame Data Location
//!
//! `FrameData` is defined in `daq_pool::FrameData` for sharing across crates.
//! This avoids coupling `daq-server` to `daq-driver-pvcam`.
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
//! use daq_pool::FrameData;
//!
//! // Create pool matching SDK buffer count (30 slots default, ~240MB for 8MB frames)
//! let pool = create_frame_pool(30, 8 * 1024 * 1024);
//!
//! // In frame loop: acquire slot, copy, return to pool on drop
//! let mut frame = pool.try_acquire().expect("pool exhausted");
//! unsafe {
//!     frame.get_mut().copy_from_sdk(sdk_ptr, frame_bytes);
//! }
//! ```

use daq_pool::{Loaned, Pool};
use std::sync::Arc;

// Re-export FrameData from daq_pool for backwards compatibility
pub use daq_pool::FrameData;

/// Default pool size: 30 frames provides ~300ms headroom at 100 FPS.
///
/// This matches the SDK's typical 20-slot circular buffer with 50% margin
/// for consumer latency (storage writes, GUI updates, gRPC transmission).
pub const DEFAULT_POOL_SIZE: usize = 30;

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

    // Note: FrameData unit tests are in daq_pool::frame_data::tests

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
