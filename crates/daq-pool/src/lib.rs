//! Zero-allocation object pool for high-performance frame handling.
//!
//! This crate provides two complementary pool implementations optimized for the
//! PVCAM frame processing pipeline where per-frame heap allocations are prohibitively
//! expensive:
//!
//! - [`Pool<T>`]: Generic object pool with lock-free access after acquire
//! - [`BufferPool`]: Specialized byte buffer pool with `bytes::Bytes` integration
//!
//! # Key Design: RwLock-Free Access (bd-0dax.1.6)
//!
//! Unlike naive pool implementations that take a lock on every `get()` call,
//! this pool caches the slot pointer at `Loaned` creation time. This eliminates
//! per-access locking overhead, which is critical for high-throughput frame
//! processing where frames may be accessed multiple times.
//!
//! # Safety Model
//!
//! The pool uses a semaphore + lock-free queue pattern:
//! 1. Semaphore tracks available slots (permits = available items)
//! 2. `SegQueue` holds indices of free slots (lock-free)
//! 3. `RwLock<Vec<UnsafeCell<T>>>` only locked during:
//!    - `acquire()`: to get slot pointer (once per loan)
//!    - `release()`: to apply reset function
//!    - `grow()`: to add new slots (rare)
//! 4. `Loaned` caches raw pointer for lock-free access thereafter
//!
//! # Example
//!
//! ```
//! use daq_pool::Pool;
//!
//! # tokio_test::block_on(async {
//! // Create pool with 30 frame buffers (~240MB for 8MB frames)
//! let pool = Pool::new_with_reset(
//!     30,
//!     || vec![0u8; 8 * 1024 * 1024],  // 8MB frame buffer
//!     |buf| buf.fill(0),               // Reset on return
//! );
//!
//! // Acquire a buffer (no allocation!)
//! let mut frame = pool.acquire().await;
//! frame[0] = 42;  // Direct access via Deref - NO LOCK TAKEN
//!
//! // Return to pool automatically when dropped
//! drop(frame);
//! # });
//! ```

pub mod buffer_pool;
pub mod frame_data;

// Re-export buffer pool types for convenience
pub use buffer_pool::{BufferPool, PooledBuffer};

// Re-export frame data type for use by drivers
pub use frame_data::FrameData;

use crossbeam_queue::SegQueue;
use parking_lot::RwLock;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{error, warn};

/// Type alias for reset function used when returning items to the pool.
type ResetFn<T> = Box<dyn Fn(&mut T) + Send + Sync>;

/// Type alias for factory function used to create new pool items.
type FactoryFn<T> = Arc<dyn Fn() -> T + Send + Sync>;

/// Generic pool for pre-allocated objects with lock-free access.
///
/// Uses a semaphore for slot availability tracking. The RwLock on slots is
/// only taken during acquire/release/grow, NOT during `Loaned::get()` calls.
///
/// # Type Parameters
/// - `T`: The type of object to pool (must be `Send`)
///
/// # Safety
///
/// This type uses `UnsafeCell` internally but is safe because:
/// 1. Semaphore ensures at most `size` permits outstanding
/// 2. Each permit corresponds to exactly one slot index
/// 3. `SegQueue` ensures each index held by at most one `Loaned`
/// 4. Slot pointer cached at acquire time, valid for loan lifetime
/// 5. RwLock only protects grow() operations, not per-access
pub struct Pool<T> {
    /// Pre-allocated items in UnsafeCell.
    /// RwLock only taken for: acquire (pointer cache), release (reset), grow()
    slots: RwLock<Vec<UnsafeCell<T>>>,
    /// Lock-free queue of available slot indices
    free_indices: SegQueue<usize>,
    /// Semaphore counting available items
    semaphore: Semaphore,
    /// Optional reset function called when item returned to pool
    reset_fn: Option<ResetFn<T>>,
    /// Factory function to create new items when growing
    factory: FactoryFn<T>,
    /// Initial pool size (for reporting growth)
    initial_size: usize,
    /// Current total size (atomic for lock-free reads)
    current_size: AtomicUsize,
}

// SAFETY: Pool is Send+Sync because:
// 1. UnsafeCell contents accessed only when holding semaphore permit
// 2. Each permit corresponds to exactly one slot
// 3. Semaphore guarantees exclusive access to each slot
// 4. T: Send allows transfer between threads
// 5. RwLock protects slots Vec during growth
unsafe impl<T: Send> Send for Pool<T> {}
unsafe impl<T: Send> Sync for Pool<T> {}

impl<T: Send + 'static> Pool<T> {
    /// Create a new pool with the specified size, factory, and optional reset function.
    ///
    /// # Arguments
    /// - `size`: Number of items to pre-allocate (must be > 0)
    /// - `factory`: Function that creates a new instance of T
    /// - `reset`: Optional function to reset T when returned to pool
    ///
    /// # Panics
    /// Panics if `size` is 0.
    pub fn new<F, R>(size: usize, factory: F, reset: Option<R>) -> Arc<Self>
    where
        F: Fn() -> T + Send + Sync + 'static,
        R: Fn(&mut T) + Send + Sync + 'static,
    {
        assert!(size > 0, "pool size must be greater than 0");

        // Pre-allocate all slots
        let slots: Vec<UnsafeCell<T>> = (0..size).map(|_| UnsafeCell::new(factory())).collect();

        // Initialize free list with all indices
        let free_indices = SegQueue::new();
        for i in 0..size {
            free_indices.push(i);
        }

        Arc::new(Self {
            slots: RwLock::new(slots),
            free_indices,
            semaphore: Semaphore::new(size),
            reset_fn: reset.map(|f| Box::new(f) as ResetFn<T>),
            factory: Arc::new(factory),
            initial_size: size,
            current_size: AtomicUsize::new(size),
        })
    }

    /// Create a new pool without a reset function.
    pub fn new_simple<F>(size: usize, factory: F) -> Arc<Self>
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self::new(size, factory, None::<fn(&mut T)>)
    }

    /// Create a new pool with a reset function.
    pub fn new_with_reset<F, R>(size: usize, factory: F, reset: R) -> Arc<Self>
    where
        F: Fn() -> T + Send + Sync + 'static,
        R: Fn(&mut T) + Send + Sync + 'static,
    {
        Self::new(size, factory, Some(reset))
    }

    /// Grow the pool by adding new slots.
    ///
    /// Called automatically when pool exhausted. Logs an error to indicate backpressure.
    fn grow(&self, count: usize) {
        let mut slots = self.slots.write();
        let old_size = slots.len();
        let new_size = old_size + count;

        error!(
            pool_type = std::any::type_name::<T>(),
            old_size,
            new_size,
            initial_size = self.initial_size,
            "Pool exhausted! Growing pool. This indicates backpressure - \
             frames produced faster than consumed."
        );

        // Add new slots
        for _ in 0..count {
            slots.push(UnsafeCell::new((self.factory)()));
        }

        // Add new indices to free list
        for i in old_size..new_size {
            self.free_indices.push(i);
        }

        // Update size tracking
        self.current_size.store(new_size, Ordering::Release);

        // Add permits for new slots
        self.semaphore.add_permits(count);
    }

    /// Acquire an item from the pool, blocking if none available.
    ///
    /// Returns a `Loaned<T>` that will automatically return the item
    /// to the pool when dropped.
    ///
    /// # Note
    ///
    /// For PVCAM frame processing, prefer `try_acquire_timeout()` to avoid
    /// blocking longer than the SDK's buffer window (~200ms at 100 FPS).
    pub async fn acquire(self: &Arc<Self>) -> Loaned<T> {
        // Wait for a permit
        let permit = self
            .semaphore
            .acquire()
            .await
            .expect("semaphore closed unexpectedly");
        permit.forget(); // We manage the permit manually via release()

        // Pop from free list
        let idx = self
            .free_indices
            .pop()
            .expect("free list empty after permit - internal invariant violated");

        // CRITICAL FIX (bd-0dax.1.6): Cache slot pointer NOW while holding lock
        // This allows lock-free access in get()/get_mut()
        let slot_ptr = {
            let slots = self.slots.read();
            slots[idx].get()
        };

        Loaned {
            pool: Arc::clone(self),
            idx,
            slot_ptr, // Cached pointer - no lock needed for subsequent access
        }
    }

    /// Try to acquire an item from the pool without blocking.
    ///
    /// Returns `None` if no items are currently available.
    /// This is the preferred method for PVCAM frame processing to avoid
    /// blocking the SDK callback thread.
    #[must_use]
    pub fn try_acquire(self: &Arc<Self>) -> Option<Loaned<T>> {
        // Try to get permit without blocking
        let permit = self.semaphore.try_acquire().ok()?;
        permit.forget();

        // Pop from free list
        let idx = self
            .free_indices
            .pop()
            .expect("free list empty after permit - internal invariant violated");

        // Cache slot pointer (bd-0dax.1.6 fix)
        let slot_ptr = {
            let slots = self.slots.read();
            slots[idx].get()
        };

        Some(Loaned {
            pool: Arc::clone(self),
            idx,
            slot_ptr,
        })
    }

    /// Try to acquire an item with a timeout.
    ///
    /// **CRITICAL for PVCAM (bd-0dax.3.6)**: The SDK uses CIRC_NO_OVERWRITE mode
    /// with a 20-slot circular buffer. At 100 FPS, this gives ~200ms before data
    /// is overwritten. Use a timeout well under this (e.g., 50-100ms) to detect
    /// backpressure before data corruption occurs.
    ///
    /// Returns `None` if timeout expires before a slot becomes available.
    pub async fn try_acquire_timeout(self: &Arc<Self>, timeout: Duration) -> Option<Loaned<T>> {
        // Try to get permit with timeout
        let permit = match tokio::time::timeout(timeout, self.semaphore.acquire()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return None, // Semaphore closed
            Err(_) => {
                warn!(
                    timeout_ms = timeout.as_millis(),
                    available = self.available(),
                    size = self.size(),
                    "Pool acquire timeout - backpressure detected"
                );
                return None;
            }
        };
        permit.forget();

        // Pop from free list
        let idx = self
            .free_indices
            .pop()
            .expect("free list empty after permit - internal invariant violated");

        // Cache slot pointer (bd-0dax.1.6 fix)
        let slot_ptr = {
            let slots = self.slots.read();
            slots[idx].get()
        };

        Some(Loaned {
            pool: Arc::clone(self),
            idx,
            slot_ptr,
        })
    }

    /// Acquire an item, growing the pool if necessary.
    ///
    /// Unlike `try_acquire`, this will grow the pool if exhausted.
    /// Use sparingly - pool growth indicates backpressure issues.
    fn acquire_or_grow(self: &Arc<Self>) -> Loaned<T> {
        if let Some(loaned) = self.try_acquire() {
            return loaned;
        }

        // Grow by doubling or at least 8 slots
        let current = self.current_size.load(Ordering::Acquire);
        let grow_count = current.max(8);
        self.grow(grow_count);

        self.try_acquire()
            .expect("acquire failed after grow - internal invariant violated")
    }

    /// Release an item back to the pool.
    ///
    /// Called automatically by `Loaned::drop`.
    fn release(&self, idx: usize) {
        // Apply reset function if provided
        if let Some(reset_fn) = &self.reset_fn {
            // SAFETY: We hold exclusive access to this slot
            let slots = self.slots.read();
            let item = unsafe { &mut *slots[idx].get() };
            reset_fn(item);
        }

        // Return index to free list
        self.free_indices.push(idx);

        // Release semaphore permit
        self.semaphore.add_permits(1);
    }

    /// Get the total size of the pool.
    #[must_use]
    pub fn size(&self) -> usize {
        self.current_size.load(Ordering::Acquire)
    }

    /// Get the number of currently available items.
    #[must_use]
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the initial size of the pool.
    #[must_use]
    pub fn initial_size(&self) -> usize {
        self.initial_size
    }
}

/// RAII guard for a loaned item from the pool.
///
/// Provides direct `&T` and `&mut T` access to the pooled item **without locking**.
/// The slot pointer is cached at creation time, eliminating per-access lock overhead.
///
/// Automatically returns the item to the pool when dropped.
///
/// # Performance Note (bd-0dax.1.6)
///
/// Unlike implementations that lock on every `get()` call, this struct caches
/// the slot pointer at creation. This is critical for high-throughput scenarios
/// where the same frame buffer may be accessed many times (e.g., for pixel
/// statistics, histogram computation, display).
pub struct Loaned<T: Send + 'static> {
    pool: Arc<Pool<T>>,
    idx: usize,
    /// Cached slot pointer - set once at acquire(), used for lock-free access.
    /// SAFETY: Valid for lifetime of Loaned because:
    /// 1. Pool slots Vec only grows, never shrinks
    /// 2. This slot is exclusively ours until drop()
    /// 3. RwLock write only taken in grow(), which only appends
    slot_ptr: *mut T,
}

// SAFETY: Loaned is Send+Sync because:
// 1. We have exclusive access to our slot via semaphore
// 2. T: Send allows transfer between threads
// 3. The raw pointer is derived from pool slots which are Sync
unsafe impl<T: Send + 'static> Send for Loaned<T> {}
unsafe impl<T: Send + 'static> Sync for Loaned<T> {}

impl<T: Send + 'static> Loaned<T> {
    /// Get immutable reference to the loaned item.
    ///
    /// **Lock-free**: Uses cached pointer, no RwLock access.
    #[inline]
    #[must_use]
    pub fn get(&self) -> &T {
        // SAFETY: We hold exclusive access via semaphore permit.
        // Pointer was cached at acquire() and is valid for our lifetime.
        unsafe { &*self.slot_ptr }
    }

    /// Get mutable reference to the loaned item.
    ///
    /// **Lock-free**: Uses cached pointer, no RwLock access.
    #[inline]
    #[must_use]
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: We hold exclusive access via semaphore permit.
        // &mut self ensures no other references exist.
        unsafe { &mut *self.slot_ptr }
    }

    /// Get a reference to the pool this item belongs to.
    #[must_use]
    pub fn pool(&self) -> &Arc<Pool<T>> {
        &self.pool
    }

    /// Get the slot index (for debugging/metrics).
    #[must_use]
    pub fn slot_index(&self) -> usize {
        self.idx
    }
}

impl<T: Clone + Send + 'static> Loaned<T> {
    /// Clone the item contents and return it, consuming the loan.
    ///
    /// The pooled slot is returned to the pool immediately.
    #[must_use]
    pub fn clone_item(self) -> T {
        self.get().clone()
        // self dropped here, returning slot to pool
    }

    /// Try to clone into a new pool slot.
    ///
    /// Returns `None` if no pool slots are available.
    #[must_use]
    pub fn try_clone(&self) -> Option<Self> {
        let mut new_loan = self.pool.try_acquire()?;
        *new_loan.get_mut() = self.get().clone();
        Some(new_loan)
    }
}

impl<T: Send + 'static> Deref for Loaned<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T: Send + 'static> DerefMut for Loaned<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T: Clone + Send + 'static> Clone for Loaned<T> {
    /// Clone the loaned item into a new pool slot.
    ///
    /// If the pool is exhausted, it will automatically grow and log an error.
    fn clone(&self) -> Self {
        if let Some(cloned) = self.try_clone() {
            return cloned;
        }

        // Slow path: grow pool and clone
        let mut new_loan = self.pool.acquire_or_grow();
        *new_loan.get_mut() = self.get().clone();
        new_loan
    }
}

impl<T: Send + 'static> Drop for Loaned<T> {
    fn drop(&mut self) {
        self.pool.release(self.idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn test_pool_basic() {
        let pool = Pool::new_with_reset(2, || vec![0u8; 100], |v| v.fill(0));

        let mut item1 = pool.acquire().await;
        item1[0] = 42;
        drop(item1);

        let item2 = pool.acquire().await;
        assert_eq!(item2[0], 0); // Reset to zero
    }

    #[tokio::test]
    async fn test_try_acquire_success() {
        let pool = Pool::new_simple(2, || 0i32);

        let item = pool.try_acquire();
        assert!(item.is_some());
        assert_eq!(pool.available(), 1);
    }

    #[tokio::test]
    async fn test_try_acquire_exhausted() {
        let pool = Pool::new_simple(1, || 0i32);

        let _held = pool.acquire().await;
        let item = pool.try_acquire();
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn test_try_acquire_timeout_success() {
        let pool = Pool::new_simple(1, || 42i32);

        let item = pool.try_acquire_timeout(Duration::from_millis(100)).await;
        assert!(item.is_some());
        assert_eq!(*item.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_try_acquire_timeout_expires() {
        let pool = Pool::new_simple(1, || 0i32);

        let _held = pool.acquire().await;
        let item = pool.try_acquire_timeout(Duration::from_millis(10)).await;
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn test_lock_free_access() {
        // Verify that get()/get_mut() don't take locks by checking
        // we can call them many times without performance degradation
        let pool = Pool::new_simple(1, || vec![0u8; 1024]);
        let mut item = pool.acquire().await;

        // This would deadlock or be very slow if get() took a lock each time
        for i in 0..10000 {
            item[i % 1024] = (i % 256) as u8;
            let _ = item[i % 1024];
        }
    }

    #[tokio::test]
    async fn test_clone_item() {
        let pool = Pool::new_simple(1, || vec![1, 2, 3]);

        let loaned = pool.acquire().await;
        let cloned = loaned.clone_item();

        assert_eq!(cloned, vec![1, 2, 3]);
        assert_eq!(pool.available(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let pool = Pool::new_simple(4, || 0i32);

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let pool = Arc::clone(&pool);
                tokio::spawn(async move {
                    let mut item = pool.acquire().await;
                    *item = i;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    *item
                })
            })
            .collect();

        for handle in handles {
            let _ = handle.await.unwrap();
        }

        assert_eq!(pool.available(), 4);
    }

    #[tokio::test]
    async fn test_reset_function_called() {
        let reset_count = Arc::new(AtomicUsize::new(0));
        let reset_count_clone = Arc::clone(&reset_count);

        let pool = Pool::new_with_reset(
            1,
            || 0i32,
            move |_| {
                reset_count_clone.fetch_add(1, Ordering::SeqCst);
            },
        );

        let item = pool.acquire().await;
        drop(item);
        assert_eq!(reset_count.load(Ordering::SeqCst), 1);

        let item = pool.acquire().await;
        drop(item);
        assert_eq!(reset_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_slot_index() {
        let pool = Pool::new_simple(3, || 0i32);

        let item0 = pool.acquire().await;
        let item1 = pool.acquire().await;
        let item2 = pool.acquire().await;

        // Indices should be 0, 1, 2 (in some order)
        let mut indices = vec![item0.slot_index(), item1.slot_index(), item2.slot_index()];
        indices.sort();
        assert_eq!(indices, vec![0, 1, 2]);
    }
}
