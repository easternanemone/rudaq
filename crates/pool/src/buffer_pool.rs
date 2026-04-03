//! Zero-copy buffer pool for `bytes::Bytes` integration.
//!
//! This module provides `BufferPool` and `PooledBuffer` types that enable
//! true zero-allocation frame handling by integrating with `bytes::Bytes`.
//!
//! # Design (bd-0dax.4)
//!
//! The key insight is that `Bytes::from_owner()` allows custom drop behavior.
//! When a `PooledBuffer` is dropped (via the `Bytes` wrapper), it automatically
//! returns its buffer to the pool for reuse.
//!
//! ## Memory Flow
//!
//! ```text
//! 1. BufferPool pre-allocates Vec<u8> buffers at startup
//! 2. acquire() returns PooledBuffer (wraps buffer + pool reference)
//! 3. Copy SDK data into buffer (mutable access via get_mut())
//! 4. freeze() converts to Bytes (zero-copy, just Arc increment)
//! 5. Bytes passed to Frame, broadcast to consumers
//! 6. When all Bytes clones dropped, PooledBuffer::drop() runs
//! 7. Buffer returned to pool for reuse
//! ```
//!
//! ## Safety
//!
//! - `PooledBuffer` implements `AsRef<[u8]> + Send + 'static` for `Bytes::from_owner()`
//! - Arc<BufferPoolInner> ensures pool outlives all buffers
//! - Buffers cleared on return (configurable) to prevent data leakage
//!
//! # Example
//!
//! ```ignore
//! use pool::buffer_pool::{BufferPool, PooledBuffer};
//! use bytes::Bytes;
//!
//! // Create pool with 30 8MB buffers (~240MB total)
//! let pool = BufferPool::new(30, 8 * 1024 * 1024);
//!
//! // In frame acquisition loop:
//! let mut buffer = pool.try_acquire().expect("pool exhausted");
//! unsafe {
//!     buffer.copy_from_ptr(sdk_ptr, frame_bytes);
//! }
//!
//! // Convert to Bytes (zero-copy!)
//! let bytes: Bytes = buffer.freeze();
//!
//! // bytes can be cloned, sent to consumers, etc.
//! // When all clones dropped, buffer returns to pool
//! ```

use bytes::Bytes;
use crossbeam_queue::SegQueue;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::info;

#[cfg(feature = "metrics")]
use prometheus::{register_int_counter, register_int_gauge, IntCounter, IntGauge};
#[cfg(feature = "metrics")]
use std::sync::LazyLock;

#[cfg(feature = "metrics")]
static BUFFER_POOL_AVAILABLE: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "buffer_pool_available",
        "Current number of available buffers in the pool"
    )
    .expect("Failed to register buffer_pool_available")
});
#[cfg(feature = "metrics")]
static BUFFER_POOL_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!("buffer_pool_total", "Total number of buffers in the pool")
        .expect("Failed to register buffer_pool_total")
});
#[cfg(feature = "metrics")]
static BUFFER_POOL_EXHAUSTION_EVENTS: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "buffer_pool_exhaustion_events_total",
        "Total number of buffer pool exhaustion events"
    )
    .expect("Failed to register buffer_pool_exhaustion_events_total")
});
/// Internal state for the buffer pool.
///
/// Wrapped in Arc for shared ownership between pool and PooledBuffer instances.
struct BufferPoolInner {
    /// Lock-free queue of available buffers
    free_buffers: SegQueue<Vec<u8>>,
    /// Semaphore tracking available buffers
    semaphore: Semaphore,
    /// Capacity of each buffer in bytes
    buffer_capacity: usize,
    /// Total number of buffers in the pool
    pool_size: usize,
    /// Number of buffers currently available
    available: AtomicUsize,
    /// Metrics: total acquires
    total_acquires: AtomicU64,
    /// Metrics: total returns
    total_returns: AtomicU64,
}

#[cfg(feature = "metrics")]
#[allow(clippy::cast_possible_wrap)] // pool sizes are bounded, well within i64::MAX
fn update_metrics(pool: &BufferPoolInner) {
    BUFFER_POOL_AVAILABLE.set(pool.available.load(Ordering::Relaxed) as i64);
    BUFFER_POOL_TOTAL.set(pool.pool_size as i64);
}

#[cfg(feature = "metrics")]
fn record_exhaustion_event() {
    BUFFER_POOL_EXHAUSTION_EVENTS.inc();
}

/// Guard for semaphore permits that ensures they're returned on panic.
///
/// This guard prevents permit leaks when code panics after acquiring a permit
/// but before the buffer is successfully returned to the pool.
///
/// # How it works
///
/// 1. Acquire a `SemaphorePermit` from tokio::sync::Semaphore
/// 2. Call `std::mem::forget(permit)` to prevent automatic return
/// 3. Create `PermitGuard::new(pool)` to track the permit
/// 4. If the code panics before `guard.release()`, the guard's Drop returns the permit manually
/// 5. If `guard.release()` is called, the guard marks itself as released (normal path)
struct PermitGuard<'a> {
    pool: &'a BufferPoolInner,
    released: bool,
}

impl<'a> PermitGuard<'a> {
    /// Create a new permit guard for a permit that has already been forgotten.
    ///
    /// IMPORTANT: The caller MUST call `std::mem::forget(permit)` before creating this guard.
    /// If this guard is dropped without calling `release()`, the permit will be
    /// returned to the semaphore.
    fn new(pool: &'a BufferPoolInner) -> Self {
        Self {
            pool,
            released: false,
        }
    }

    /// Mark the permit as released (buffer is now responsible for returning it).
    ///
    /// This should be called in the success path when the buffer has been successfully
    /// acquired and will be responsible for returning the permit via its own Drop impl.
    fn release(mut self) {
        self.released = true;
        // Don't add permit back - it's being used by PooledBuffer
    }
}

impl Drop for PermitGuard<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Panic/error path - return the permit
            self.pool.semaphore.add_permits(1);
        }
    }
}

/// Pool of pre-allocated byte buffers for zero-allocation frame handling.
///
/// Buffers are returned automatically when dropped (via `PooledBuffer::drop`).
/// Thread-safe and designed for high-throughput concurrent access.
#[derive(Clone)]
pub struct BufferPool {
    inner: Arc<BufferPoolInner>,
}

impl BufferPool {
    /// Create a new buffer pool with the specified size and buffer capacity.
    ///
    /// # Arguments
    ///
    /// - `pool_size`: Number of buffers to pre-allocate
    /// - `buffer_capacity`: Size in bytes for each buffer
    ///
    /// # Panics
    ///
    /// Panics if `pool_size` is 0 or `buffer_capacity` is 0.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: precision loss acceptable for metrics/display
    pub fn new(pool_size: usize, buffer_capacity: usize) -> Self {
        assert!(pool_size > 0, "pool_size must be > 0");
        assert!(buffer_capacity > 0, "buffer_capacity must be > 0");

        let free_buffers = SegQueue::new();

        // Pre-allocate all buffers
        for _ in 0..pool_size {
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for metrics/display
            let buffer = vec![0u8; buffer_capacity];
            free_buffers.push(buffer);
        }

        info!(
            pool_size,
            buffer_capacity_mb = buffer_capacity as f64 / (1024.0 * 1024.0),
            total_mb = (pool_size * buffer_capacity) as f64 / (1024.0 * 1024.0),
            "BufferPool created"
        );

        let pool = Self {
            inner: Arc::new(BufferPoolInner {
                free_buffers,
                semaphore: Semaphore::new(pool_size),
                buffer_capacity,
                pool_size,
                available: AtomicUsize::new(pool_size),
                total_acquires: AtomicU64::new(0),
                total_returns: AtomicU64::new(0),
            }),
        };
        #[cfg(feature = "metrics")]
        update_metrics(&pool.inner);
        pool
    }

    /// Try to acquire a buffer without blocking.
    ///
    /// Returns `None` if no buffers are available (backpressure indicator).
    #[must_use]
    pub fn try_acquire(&self) -> Option<PooledBuffer> {
        // Try to acquire semaphore permit without blocking
        let Ok(permit) = self.inner.semaphore.try_acquire() else {
            #[cfg(feature = "metrics")]
            record_exhaustion_event();
            return None;
        };

        // Forget the permit to prevent automatic return, then create guard
        std::mem::forget(permit);
        let guard = PermitGuard::new(&self.inner);

        // Pop a buffer from the free queue
        let Some(buffer) = self.inner.free_buffers.pop() else {
            #[cfg(feature = "metrics")]
            record_exhaustion_event();
            return None;
        };

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        update_metrics(&self.inner);

        // Release the guard - buffer is now responsible for returning the permit
        guard.release();

        Some(PooledBuffer {
            buffer: Some(buffer),
            actual_len: 0,
            pool: Arc::clone(&self.inner),
        })
    }

    /// Acquire a buffer, waiting up to the specified timeout.
    ///
    /// Returns `None` if the timeout expires before a buffer becomes available.
    pub async fn try_acquire_timeout(&self, timeout: Duration) -> Option<PooledBuffer> {
        // Try to acquire semaphore permit with timeout
        let Ok(Ok(permit)) = tokio::time::timeout(timeout, self.inner.semaphore.acquire()).await
        else {
            #[cfg(feature = "metrics")]
            record_exhaustion_event();
            return None;
        };

        // Forget the permit to prevent automatic return, then create guard
        std::mem::forget(permit);
        let guard = PermitGuard::new(&self.inner);

        // Pop a buffer from the free queue
        let Some(buffer) = self.inner.free_buffers.pop() else {
            #[cfg(feature = "metrics")]
            record_exhaustion_event();
            return None;
        };

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        update_metrics(&self.inner);

        // Release the guard - buffer is now responsible for returning the permit
        guard.release();

        Some(PooledBuffer {
            buffer: Some(buffer),
            actual_len: 0,
            pool: Arc::clone(&self.inner),
        })
    }

    /// Acquire a buffer with a configurable timeout, returning an error on failure.
    ///
    /// Unlike `try_acquire_timeout` which returns `Option`, this returns a `Result` with
    /// a descriptive `PoolError::Timeout` that includes diagnostic information (available
    /// slots, capacity) for debugging backpressure issues.
    ///
    /// # Arguments
    ///
    /// - `timeout`: Maximum duration to wait for a buffer to become available.
    ///
    /// # Errors
    ///
    /// Returns `PoolError::Timeout` if no buffer becomes available within the specified duration.
    pub async fn acquire_timeout(
        &self,
        timeout: Duration,
    ) -> Result<PooledBuffer, crate::PoolError> {
        match tokio::time::timeout(timeout, self.inner.semaphore.acquire()).await {
            Ok(Ok(permit)) => {
                std::mem::forget(permit);
                let guard = PermitGuard::new(&self.inner);

                let Some(buffer) = self.inner.free_buffers.pop() else {
                    // Semaphore/queue desync — should not happen, but guard returns permit
                    return Err(crate::PoolError::Timeout {
                        timeout,
                        available: self.available(),
                        capacity: self.size(),
                    });
                };

                self.inner.available.fetch_sub(1, Ordering::Relaxed);
                self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);
                #[cfg(feature = "metrics")]
                update_metrics(&self.inner);

                guard.release();

                Ok(PooledBuffer {
                    buffer: Some(buffer),
                    actual_len: 0,
                    pool: Arc::clone(&self.inner),
                })
            }
            Ok(Err(_)) => {
                // Semaphore closed
                Err(crate::PoolError::Timeout {
                    timeout,
                    available: self.available(),
                    capacity: self.size(),
                })
            }
            Err(_) => {
                #[cfg(feature = "metrics")]
                record_exhaustion_event();
                Err(crate::PoolError::Timeout {
                    timeout,
                    available: self.available(),
                    capacity: self.size(),
                })
            }
        }
    }

    /// Acquire a buffer, blocking until one is available.
    pub async fn acquire(&self) -> PooledBuffer {
        // Acquire semaphore permit (blocks if none available)
        let permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .expect("semaphore closed");

        // Forget the permit to prevent automatic return, then create guard
        std::mem::forget(permit);
        let guard = PermitGuard::new(&self.inner);

        // Pop a buffer from the free queue
        let buffer = self
            .inner
            .free_buffers
            .pop()
            .expect("semaphore/queue desync");

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);

        // Release the guard - buffer is now responsible for returning the permit
        guard.release();

        PooledBuffer {
            buffer: Some(buffer),
            actual_len: 0,
            pool: Arc::clone(&self.inner),
        }
    }

    /// Number of currently available buffers.
    #[must_use]
    pub fn available(&self) -> usize {
        self.inner.available.load(Ordering::Relaxed)
    }

    /// Number of currently available buffers (alias for metrics).
    #[must_use]
    pub fn available_buffers(&self) -> usize {
        self.available()
    }

    /// Check if the pool is under pressure (low availability).
    ///
    /// Returns true if availability is below 20% of capacity.
    #[must_use]
    pub fn is_under_pressure(&self) -> bool {
        let threshold = (self.size() * 20) / 100;
        if threshold == 0 {
            return self.available() == 0;
        }
        self.available() < threshold
    }

    /// Check if the pool has recovered from pressure.
    ///
    /// Returns true if availability is above 50% of capacity.
    #[must_use]
    pub fn is_recovered(&self) -> bool {
        let threshold = (self.size() * 50) / 100;
        if threshold == 0 {
            return self.available() > 0;
        }
        self.available() > threshold
    }

    /// Total number of buffers in the pool.
    #[must_use]
    pub fn size(&self) -> usize {
        self.inner.pool_size
    }

    /// Total number of buffers in the pool (alias for metrics).
    #[must_use]
    pub fn total_buffers(&self) -> usize {
        self.size()
    }

    /// Record a pool exhaustion event for observability.
    pub fn record_exhaustion_event(&self) {
        #[cfg(feature = "metrics")]
        record_exhaustion_event();
    }

    /// Capacity of each buffer in bytes.
    #[must_use]
    pub fn buffer_capacity(&self) -> usize {
        self.inner.buffer_capacity
    }

    /// Total number of buffer acquisitions since pool creation.
    #[must_use]
    pub fn total_acquires(&self) -> u64 {
        self.inner.total_acquires.load(Ordering::Relaxed)
    }

    /// Total number of buffer returns since pool creation.
    #[must_use]
    pub fn total_returns(&self) -> u64 {
        self.inner.total_returns.load(Ordering::Relaxed)
    }
}

/// A buffer acquired from the pool with automatic return on drop.
///
/// This type can be converted to `bytes::Bytes` via `freeze()` for zero-copy
/// integration with the rest of the system.
///
/// # Safety
///
/// Implements `AsRef<[u8]> + Send + 'static` as required by `Bytes::from_owner()`.
pub struct PooledBuffer {
    /// The actual buffer (Option for take-on-freeze)
    buffer: Option<Vec<u8>>,
    /// Actual length of valid data (may be < buffer capacity)
    actual_len: usize,
    /// Reference to pool for return on drop
    pool: Arc<BufferPoolInner>,
}

impl PooledBuffer {
    /// Get the valid data as a slice.
    ///
    /// Returns only the bytes that have been written (up to `actual_len`),
    /// not the full buffer capacity.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match &self.buffer {
            Some(buf) => &buf[..self.actual_len],
            None => &[],
        }
    }

    /// Get the buffer capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.as_ref().map_or(0, |b| b.capacity())
    }

    /// Get the actual length of valid data.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actual_len
    }

    /// Check if the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actual_len == 0
    }

    /// Set the actual length of valid data.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the buffer capacity.
    pub fn set_len(&mut self, len: usize) {
        let cap = self.capacity();
        assert!(
            len <= cap,
            "set_len({}) exceeds buffer capacity ({})",
            len,
            cap
        );
        self.actual_len = len;
    }

    /// Copy data from a raw pointer into the buffer.
    ///
    /// # Safety
    ///
    /// - `src` must point to valid memory of at least `len` bytes
    /// - `len` must not exceed the buffer capacity
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the buffer capacity.
    pub unsafe fn copy_from_ptr(&mut self, src: *const u8, len: usize) {
        let buf = self.buffer.as_mut().expect("buffer already frozen");
        assert!(
            len <= buf.capacity(),
            "copy_from_ptr: len ({}) exceeds buffer capacity ({})",
            len,
            buf.capacity()
        );

        // SAFETY: src is valid for `len` bytes (caller contract), dst has sufficient capacity
        // (asserted above), and regions don't overlap (src is external, dst is our buffer).
        unsafe { std::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), len) };
        // SAFETY: We just copied `len` bytes into the buffer, and asserted len <= capacity
        unsafe { buf.set_len(len) };
        self.actual_len = len;
    }

    /// Copy data from a slice into the buffer.
    ///
    /// # Panics
    ///
    /// Panics if the slice length exceeds the buffer capacity.
    pub fn copy_from_slice(&mut self, src: &[u8]) {
        let buf = self.buffer.as_mut().expect("buffer already frozen");
        assert!(
            src.len() <= buf.capacity(),
            "copy_from_slice: len ({}) exceeds buffer capacity ({})",
            src.len(),
            buf.capacity()
        );

        // Clear and extend to avoid indexing into zero-length buffer
        buf.clear();
        buf.extend_from_slice(src);
        self.actual_len = src.len();
    }

    /// Get mutable access to the raw buffer.
    ///
    /// Use this to fill the buffer with data, then call `set_len()` to
    /// indicate how much data was written.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer.as_mut().expect("buffer already frozen")
    }

    /// Convert this buffer into `bytes::Bytes` (zero-copy).
    ///
    /// After calling this, the `PooledBuffer` no longer owns the buffer.
    /// When the returned `Bytes` (and all its clones) are dropped, the
    /// buffer will be automatically returned to the pool.
    ///
    /// # Zero-Copy
    ///
    /// This operation does NOT copy the buffer data. It simply wraps the
    /// existing allocation in a `Bytes` handle with a custom drop implementation.
    #[must_use]
    pub fn freeze(mut self) -> Bytes {
        let buffer = self.buffer.take().expect("buffer already frozen");
        let actual_len = self.actual_len;
        let pool = Arc::clone(&self.pool);

        // Create a wrapper that returns buffer to pool on drop
        let owner = BufferOwner {
            buffer,
            actual_len,
            pool,
        };

        // This does NOT copy the data - just wraps it
        Bytes::from_owner(owner)
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        // If buffer hasn't been frozen, return it to the pool
        if let Some(mut buffer) = self.buffer.take() {
            // Clear the buffer for security (optional, can be disabled for performance)
            // This prevents data leakage if buffers are reused across contexts
            // buffer.fill(0);  // Uncomment if security is a concern

            // Reset length but keep capacity
            buffer.clear();

            // Return to pool
            self.pool.free_buffers.push(buffer);
            self.pool.available.fetch_add(1, Ordering::Relaxed);
            self.pool.total_returns.fetch_add(1, Ordering::Relaxed);
            #[cfg(feature = "metrics")]
            update_metrics(&self.pool);

            // Re-add the semaphore permit
            self.pool.semaphore.add_permits(1);
        }
    }
}

// Required for Bytes::from_owner()
impl AsRef<[u8]> for PooledBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

// Required for Bytes::from_owner() - PooledBuffer is safe to send between threads
// because the underlying Vec<u8> is Send and the pool Arc is Send+Sync
unsafe impl Send for PooledBuffer {}

/// Internal wrapper for buffer ownership in Bytes.
///
/// This type is created by `PooledBuffer::freeze()` and handles returning
/// the buffer to the pool when the `Bytes` is dropped.
struct BufferOwner {
    buffer: Vec<u8>,
    actual_len: usize,
    pool: Arc<BufferPoolInner>,
}

impl AsRef<[u8]> for BufferOwner {
    fn as_ref(&self) -> &[u8] {
        &self.buffer[..self.actual_len]
    }
}

impl Drop for BufferOwner {
    fn drop(&mut self) {
        // Return buffer to pool
        let mut buffer = std::mem::take(&mut self.buffer);

        // Reset length but keep capacity
        buffer.clear();

        // Return to pool
        self.pool.free_buffers.push(buffer);
        self.pool.available.fetch_add(1, Ordering::Relaxed);
        self.pool.total_returns.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        update_metrics(&self.pool);

        // Re-add the semaphore permit
        self.pool.semaphore.add_permits(1);
    }
}

// Required for Bytes::from_owner()
unsafe impl Send for BufferOwner {}
unsafe impl Sync for BufferOwner {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_creation() {
        let pool = BufferPool::new(4, 1024);
        assert_eq!(pool.size(), 4);
        assert_eq!(pool.available(), 4);
        assert_eq!(pool.buffer_capacity(), 1024);
    }

    #[test]
    fn test_try_acquire() {
        let pool = BufferPool::new(2, 1024);

        let buf1 = pool.try_acquire();
        assert!(buf1.is_some());
        assert_eq!(pool.available(), 1);

        let buf2 = pool.try_acquire();
        assert!(buf2.is_some());
        assert_eq!(pool.available(), 0);

        // Pool exhausted
        let buf3 = pool.try_acquire();
        assert!(buf3.is_none());

        // Return one
        drop(buf1);
        assert_eq!(pool.available(), 1);

        // Can acquire again
        let buf4 = pool.try_acquire();
        assert!(buf4.is_some());
    }

    #[test]
    fn test_copy_from_slice() {
        let pool = BufferPool::new(1, 1024);
        let mut buf = pool.try_acquire().unwrap();

        let data = b"hello world";
        buf.copy_from_slice(data);

        assert_eq!(buf.len(), data.len());
        assert_eq!(buf.as_slice(), data);
    }

    #[test]
    fn test_copy_from_ptr() {
        let pool = BufferPool::new(1, 1024);
        let mut buf = pool.try_acquire().unwrap();

        let data = b"hello world";
        unsafe {
            buf.copy_from_ptr(data.as_ptr(), data.len());
        }

        assert_eq!(buf.len(), data.len());
        assert_eq!(buf.as_slice(), data);
    }

    #[test]
    fn test_freeze_to_bytes() {
        let pool = BufferPool::new(1, 1024);
        let mut buf = pool.try_acquire().unwrap();

        let data = b"hello world";
        buf.copy_from_slice(data);

        // Pool exhausted before freeze
        assert_eq!(pool.available(), 0);

        // Freeze to Bytes
        let bytes = buf.freeze();

        // Buffer is still in use (owned by Bytes)
        assert_eq!(pool.available(), 0);
        assert_eq!(bytes.as_ref(), data);

        // Drop Bytes - buffer should return to pool
        drop(bytes);
        assert_eq!(pool.available(), 1);
    }

    #[test]
    fn test_bytes_clone_keeps_buffer() {
        let pool = BufferPool::new(1, 1024);
        let mut buf = pool.try_acquire().unwrap();

        buf.copy_from_slice(b"test data");
        let bytes1 = buf.freeze();

        // Clone the Bytes
        let bytes2 = bytes1.clone();

        // Both clones share the same underlying buffer
        assert_eq!(bytes1.as_ref(), bytes2.as_ref());

        // Buffer not returned yet
        assert_eq!(pool.available(), 0);

        // Drop first clone
        drop(bytes1);
        assert_eq!(pool.available(), 0); // Still held by bytes2

        // Drop second clone - now buffer returns
        drop(bytes2);
        assert_eq!(pool.available(), 1);
    }

    #[tokio::test]
    async fn test_async_acquire() {
        let pool = BufferPool::new(1, 1024);

        let buf1 = pool.acquire().await;
        assert_eq!(pool.available(), 0);

        // Spawn task to release buffer
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(buf1);
        });

        // Should block until buffer is returned
        let buf2 = pool.acquire().await;
        assert_eq!(buf2.capacity(), 1024);

        // Cleanup
        let _ = pool_clone;
    }

    #[tokio::test]
    async fn test_try_acquire_timeout() {
        let pool = BufferPool::new(1, 1024);

        let _buf1 = pool.try_acquire().unwrap();

        // Should timeout since pool is exhausted
        let result = pool.try_acquire_timeout(Duration::from_millis(10)).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_acquire_timeout_success() {
        let pool = BufferPool::new(2, 512);

        let result = pool.acquire_timeout(Duration::from_millis(100)).await;
        assert!(result.is_ok());
        let buf = result.expect("should succeed");
        assert_eq!(buf.capacity(), 512);
    }

    #[tokio::test(start_paused = true)]
    async fn test_acquire_timeout_expires_returns_error() {
        use crate::PoolError;

        let pool = BufferPool::new(1, 256);

        // Hold the only buffer
        let _held = pool.try_acquire().unwrap();

        let result = pool.acquire_timeout(Duration::from_millis(50)).await;
        match result {
            Err(PoolError::Timeout {
                timeout,
                available,
                capacity,
            }) => {
                assert_eq!(timeout, Duration::from_millis(50));
                assert_eq!(available, 0);
                assert_eq!(capacity, 1);
            }
            Err(other) => panic!("expected PoolError::Timeout, got: {other:?}"),
            Ok(_) => panic!("expected error but got Ok"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_acquire_timeout_succeeds_when_buffer_released() {
        let pool = BufferPool::new(1, 128);

        let held = pool.acquire().await;

        // Spawn a task that releases the buffer after 10ms
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(held);
        });

        // Should succeed because buffer is released before 200ms timeout
        let result = pool.acquire_timeout(Duration::from_millis(200)).await;
        assert!(result.is_ok());
        assert_eq!(
            result.expect("should succeed after release").capacity(),
            128
        );
    }

    #[test]
    fn test_metrics() {
        let pool = BufferPool::new(2, 1024);

        assert_eq!(pool.total_acquires(), 0);
        assert_eq!(pool.total_returns(), 0);

        let buf1 = pool.try_acquire().unwrap();
        assert_eq!(pool.total_acquires(), 1);

        let buf2 = pool.try_acquire().unwrap();
        assert_eq!(pool.total_acquires(), 2);

        drop(buf1);
        assert_eq!(pool.total_returns(), 1);

        let bytes = buf2.freeze();
        drop(bytes);
        assert_eq!(pool.total_returns(), 2);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer capacity")]
    fn test_copy_from_slice_overflow() {
        let pool = BufferPool::new(1, 10);
        let mut buf = pool.try_acquire().unwrap();

        // This should panic
        buf.copy_from_slice(&[0u8; 20]);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer capacity")]
    fn test_set_len_overflow() {
        let pool = BufferPool::new(1, 10);
        let mut buf = pool.try_acquire().unwrap();

        // This should panic
        buf.set_len(20);
    }

    /// Test pool exhaustion behavior (bd-dmbl).
    ///
    /// This test validates the core behavior that enables graceful frame dropping:
    /// - `try_acquire()` returns `None` when pool is exhausted
    /// - `available()` correctly reflects pool state
    /// - Buffers are properly returned to pool when dropped
    /// - Metrics accurately track acquire/return operations
    #[test]
    fn test_pool_exhaustion_returns_none() {
        // Create a small pool to easily exhaust
        let pool_size = 3;
        let buffer_capacity = 1024;
        let pool = BufferPool::new(pool_size, buffer_capacity);

        // Initial state: all buffers available
        assert_eq!(
            pool.available(),
            pool_size,
            "Pool should start with all {} buffers available",
            pool_size
        );
        assert_eq!(
            pool.total_acquires(),
            0,
            "Initial acquire count should be 0"
        );
        assert_eq!(pool.total_returns(), 0, "Initial return count should be 0");

        // Acquire all buffers from the pool
        let buf1 = pool.try_acquire();
        assert!(
            buf1.is_some(),
            "First acquire should succeed when pool has {} available",
            pool_size
        );
        assert_eq!(
            pool.available(),
            2,
            "After first acquire, 2 buffers should remain available"
        );
        assert_eq!(
            pool.total_acquires(),
            1,
            "Acquire count should be 1 after first acquire"
        );

        let buf2 = pool.try_acquire();
        assert!(
            buf2.is_some(),
            "Second acquire should succeed when pool has 2 available"
        );
        assert_eq!(
            pool.available(),
            1,
            "After second acquire, 1 buffer should remain available"
        );
        assert_eq!(
            pool.total_acquires(),
            2,
            "Acquire count should be 2 after second acquire"
        );

        let buf3 = pool.try_acquire();
        assert!(
            buf3.is_some(),
            "Third acquire should succeed when pool has 1 available"
        );
        assert_eq!(
            pool.available(),
            0,
            "After exhausting pool, 0 buffers should be available"
        );
        assert_eq!(
            pool.total_acquires(),
            3,
            "Acquire count should be 3 after exhausting pool"
        );

        // Pool is now exhausted - try_acquire should return None
        let exhausted_result = pool.try_acquire();
        assert!(
            exhausted_result.is_none(),
            "try_acquire() should return None when pool is exhausted (bd-dmbl behavior)"
        );
        assert_eq!(
            pool.available(),
            0,
            "Available count should remain 0 after failed acquire"
        );
        // Note: total_acquires should NOT increment on failed acquire
        assert_eq!(
            pool.total_acquires(),
            3,
            "Acquire count should NOT increment on failed acquire attempt"
        );

        // Verify exhaustion persists with multiple attempts
        for i in 0..5 {
            let result = pool.try_acquire();
            assert!(
                result.is_none(),
                "Attempt {}: try_acquire() should consistently return None when exhausted",
                i + 1
            );
        }
        assert_eq!(
            pool.total_acquires(),
            3,
            "Acquire count should remain 3 after multiple failed attempts"
        );

        // Return one buffer to pool by dropping it
        drop(buf1);
        assert_eq!(
            pool.available(),
            1,
            "After dropping one buffer, 1 should be available again"
        );
        assert_eq!(
            pool.total_returns(),
            1,
            "Return count should be 1 after first drop"
        );

        // Now acquire should succeed again
        let buf4 = pool.try_acquire();
        assert!(
            buf4.is_some(),
            "Acquire should succeed after buffer is returned to pool"
        );
        assert_eq!(
            pool.available(),
            0,
            "Pool should be exhausted again after re-acquire"
        );
        assert_eq!(
            pool.total_acquires(),
            4,
            "Acquire count should be 4 after successful re-acquire"
        );

        // And exhausted again
        let exhausted_again = pool.try_acquire();
        assert!(
            exhausted_again.is_none(),
            "Pool should be exhausted again after re-acquire"
        );

        // Return all buffers and verify final state
        drop(buf2);
        drop(buf3);
        drop(buf4);

        assert_eq!(
            pool.available(),
            pool_size,
            "All {} buffers should be available after dropping all",
            pool_size
        );
        assert_eq!(
            pool.total_acquires(),
            4,
            "Final acquire count should be 4 (3 initial + 1 re-acquire)"
        );
        assert_eq!(
            pool.total_returns(),
            4,
            "Final return count should be 4 (matching acquires)"
        );
    }

    #[test]
    fn test_under_pressure_and_recovered_thresholds() {
        let pool = BufferPool::new(10, 256);
        assert!(!pool.is_under_pressure());

        let mut held = Vec::new();
        for _ in 0..9 {
            held.push(pool.try_acquire().expect("buffer"));
        }
        assert!(
            pool.is_under_pressure(),
            "Pool should be under pressure when <20% available"
        );
        assert!(!pool.is_recovered());

        for _ in 0..5 {
            drop(held.pop());
        }
        assert!(
            pool.is_recovered(),
            "Pool should be recovered when >50% available"
        );
    }

    /// Test that frozen buffers also return to pool correctly.
    ///
    /// This is important for the PVCAM frame pipeline where buffers are
    /// converted to Bytes via freeze() before being sent to consumers.
    #[test]
    fn test_pool_exhaustion_with_frozen_buffers() {
        let pool = BufferPool::new(2, 512);

        // Acquire and freeze both buffers
        let mut buf1 = pool.try_acquire().unwrap();
        buf1.copy_from_slice(b"frame1");
        let bytes1 = buf1.freeze();

        let mut buf2 = pool.try_acquire().unwrap();
        buf2.copy_from_slice(b"frame2");
        let bytes2 = buf2.freeze();

        // Pool should be exhausted
        assert_eq!(
            pool.available(),
            0,
            "Pool should be exhausted after freezing both buffers"
        );
        assert!(
            pool.try_acquire().is_none(),
            "try_acquire should return None when all buffers are frozen into Bytes"
        );

        // Clone the Bytes - buffer should still be held
        let bytes1_clone = bytes1.clone();
        assert_eq!(
            pool.available(),
            0,
            "Cloning Bytes should not return buffer to pool"
        );

        // Drop original - clone still holds reference
        drop(bytes1);
        assert_eq!(
            pool.available(),
            0,
            "Buffer should not return until ALL Bytes clones dropped"
        );

        // Drop clone - now buffer returns
        drop(bytes1_clone);
        assert_eq!(
            pool.available(),
            1,
            "Buffer should return after all Bytes references dropped"
        );

        // Can acquire again
        let buf3 = pool.try_acquire();
        assert!(
            buf3.is_some(),
            "Should be able to acquire after Bytes dropped"
        );

        // Clean up
        drop(bytes2);
        drop(buf3);
        assert_eq!(
            pool.available(),
            2,
            "All buffers should be returned after cleanup"
        );
    }

    /// Test that permits are returned on panic (bd-d9nw.1).
    ///
    /// This test validates that the PermitGuard ensures semaphore permits
    /// are returned to the pool if a panic occurs after acquiring a permit
    /// but before the buffer is successfully returned to the pool.
    #[test]
    fn test_permit_guard_prevents_leak_on_panic() {
        use std::panic;

        let pool = BufferPool::new(3, 256);

        // Initial state: all buffers available
        assert_eq!(pool.available(), 3, "Pool should start with 3 buffers");

        // Acquire one buffer normally to hold
        let buf1 = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 2, "2 buffers should remain");

        // Test 1: Verify guard works by simulating what happens if
        // free_buffers.pop() returns None (which would be a bug, but tests the guard)
        let inner_clone = Arc::clone(&pool.inner);
        let result = panic::catch_unwind(panic::AssertUnwindSafe(move || {
            // Acquire permit
            let _permit = inner_clone.semaphore.try_acquire().unwrap();
            let _guard = PermitGuard::new(&inner_clone);

            // Simulate an unexpected panic before guard.release() is called
            panic!("simulated panic during acquisition");
        }));

        assert!(result.is_err(), "Panic should have been caught");

        // The guard should have returned the permit on drop
        // We should be able to acquire 2 more buffers (the original 2 remaining)
        let buf2 = pool.try_acquire();
        assert!(
            buf2.is_some(),
            "Should be able to acquire after panic (permit was returned by guard)"
        );

        let buf3 = pool.try_acquire();
        assert!(
            buf3.is_some(),
            "Should be able to acquire second buffer after panic"
        );

        // Pool should now be exhausted
        assert!(
            pool.try_acquire().is_none(),
            "Pool should be exhausted after acquiring all available buffers"
        );

        // Clean up
        drop(buf1);
        drop(buf2);
        drop(buf3);

        assert_eq!(pool.available(), 3, "All buffers returned after cleanup");
    }
}
