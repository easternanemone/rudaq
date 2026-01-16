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
//! use daq_pool::buffer_pool::{BufferPool, PooledBuffer};
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
    pub fn new(pool_size: usize, buffer_capacity: usize) -> Self {
        assert!(pool_size > 0, "pool_size must be > 0");
        assert!(buffer_capacity > 0, "buffer_capacity must be > 0");

        let free_buffers = SegQueue::new();

        // Pre-allocate all buffers
        for _ in 0..pool_size {
            let buffer = vec![0u8; buffer_capacity];
            free_buffers.push(buffer);
        }

        info!(
            pool_size,
            buffer_capacity_mb = buffer_capacity as f64 / (1024.0 * 1024.0),
            total_mb = (pool_size * buffer_capacity) as f64 / (1024.0 * 1024.0),
            "BufferPool created"
        );

        Self {
            inner: Arc::new(BufferPoolInner {
                free_buffers,
                semaphore: Semaphore::new(pool_size),
                buffer_capacity,
                pool_size,
                available: AtomicUsize::new(pool_size),
                total_acquires: AtomicU64::new(0),
                total_returns: AtomicU64::new(0),
            }),
        }
    }

    /// Try to acquire a buffer without blocking.
    ///
    /// Returns `None` if no buffers are available (backpressure indicator).
    #[must_use]
    pub fn try_acquire(&self) -> Option<PooledBuffer> {
        // Try to acquire semaphore permit without blocking
        let permit = self.inner.semaphore.try_acquire().ok()?;

        // Pop a buffer from the free queue
        let buffer = self.inner.free_buffers.pop()?;

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);

        // Forget the permit - we'll re-add it when buffer is returned
        std::mem::forget(permit);

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
        let permit = tokio::time::timeout(timeout, self.inner.semaphore.acquire())
            .await
            .ok()?
            .ok()?;

        // Pop a buffer from the free queue
        let buffer = self.inner.free_buffers.pop()?;

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);

        // Forget the permit - we'll re-add it when buffer is returned
        std::mem::forget(permit);

        Some(PooledBuffer {
            buffer: Some(buffer),
            actual_len: 0,
            pool: Arc::clone(&self.inner),
        })
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

        // Pop a buffer from the free queue
        let buffer = self
            .inner
            .free_buffers
            .pop()
            .expect("semaphore/queue desync");

        // Update metrics
        self.inner.available.fetch_sub(1, Ordering::Relaxed);
        self.inner.total_acquires.fetch_add(1, Ordering::Relaxed);

        // Forget the permit - we'll re-add it when buffer is returned
        std::mem::forget(permit);

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

    /// Total number of buffers in the pool.
    #[must_use]
    pub fn size(&self) -> usize {
        self.inner.pool_size
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

        std::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), len);
        // SAFETY: We just copied `len` bytes into the buffer, and asserted len <= capacity
        buf.set_len(len);
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
}
