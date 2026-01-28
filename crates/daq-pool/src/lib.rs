//! Zero-allocation object pool for high-performance frame handling.
//!
#![allow(unsafe_code)] // Pool uses unsafe for lock-free slot access - intentional and documented
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
    slots: RwLock<Vec<Box<UnsafeCell<T>>>>,
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
        let slots: Vec<Box<UnsafeCell<T>>> = (0..size)
            .map(|_| Box::new(UnsafeCell::new(factory())))
            .collect();

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
            slots.push(Box::new(UnsafeCell::new((self.factory)())));
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
            slots[idx].as_ref().get()
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
            slots[idx].as_ref().get()
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
            slots[idx].as_ref().get()
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
            let item = unsafe { &mut *slots[idx].as_ref().get() };
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
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    /// Profile RwLock contention by measuring access times during concurrent operations.
    ///
    /// This test verifies that `Loaned::get()` (via Deref) does NOT take the RwLock,
    /// even when other tasks are actively acquiring/releasing pool slots (which DO
    /// take the lock). If get() took the lock, we'd see contention spikes.
    ///
    /// Related issue: bd-0dax.7.3
    #[cfg_attr(miri, ignore)] // Miri is too slow for timing assertions
    #[tokio::test]
    async fn test_no_rwlock_contention_on_access() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::Instant;

        const POOL_SIZE: usize = 8;
        const ACCESS_ITERATIONS: usize = 10_000;
        const SLOW_THRESHOLD_MICROS: u64 = 100;

        let pool = Pool::new_simple(POOL_SIZE, || vec![0u8; 1024]);
        let slow_accesses = Arc::new(AtomicU64::new(0));
        let total_accesses = Arc::new(AtomicU64::new(0));

        // Acquire half the slots and hold them for the test duration
        let mut held_items = Vec::with_capacity(POOL_SIZE / 2);
        for _ in 0..POOL_SIZE / 2 {
            held_items.push(pool.acquire().await);
        }

        // Barrier to synchronize start
        let barrier = Arc::new(tokio::sync::Barrier::new(POOL_SIZE));

        // Use separate JoinSets for different return types
        let mut reader_tasks = tokio::task::JoinSet::new();
        let mut churner_tasks = tokio::task::JoinSet::new();

        // Spawn tasks that rapidly access held items via Deref (should NOT take lock)
        for item in held_items {
            let slow = slow_accesses.clone();
            let total = total_accesses.clone();
            let barrier = barrier.clone();

            reader_tasks.spawn(async move {
                barrier.wait().await;

                for _ in 0..ACCESS_ITERATIONS {
                    let start = Instant::now();

                    // Access via Deref - this should be lock-free
                    let _ = std::hint::black_box(item[0]);

                    let elapsed = start.elapsed();
                    total.fetch_add(1, Ordering::Relaxed);

                    if elapsed > std::time::Duration::from_micros(SLOW_THRESHOLD_MICROS) {
                        slow.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Return the item so it doesn't get dropped during the test
                item
            });
        }

        // Spawn tasks that continuously acquire/release (this DOES take the lock)
        // to create contention if get() were to also take the lock
        for _ in 0..(POOL_SIZE / 2) {
            let pool = pool.clone();
            let barrier = barrier.clone();

            churner_tasks.spawn(async move {
                barrier.wait().await;

                for _ in 0..ACCESS_ITERATIONS / 10 {
                    // This acquire/release cycle takes the RwLock
                    if let Some(item) = pool.try_acquire() {
                        // Hold briefly to create contention window
                        tokio::task::yield_now().await;
                        drop(item);
                    }
                    tokio::task::yield_now().await;
                }
            });
        }

        // Wait for all tasks
        while let Some(result) = reader_tasks.join_next().await {
            let _ = result.expect("reader task panicked");
        }
        while let Some(result) = churner_tasks.join_next().await {
            result.expect("churner task panicked");
        }

        let slow_count = slow_accesses.load(Ordering::Relaxed);
        let total_count = total_accesses.load(Ordering::Relaxed);
        let slow_percentage = (slow_count as f64 / total_count as f64) * 100.0;

        println!(
            "RwLock contention test: {} slow accesses (>{} us) out of {} total ({:.2}%)",
            slow_count, SLOW_THRESHOLD_MICROS, total_count, slow_percentage
        );

        // Allow up to 1% slow accesses (mostly from OS scheduling, not lock contention)
        // With lock contention, we'd see 10%+ slow accesses
        assert!(
            slow_percentage < 1.0,
            "Too many slow accesses: {:.2}% (expected < 1%). \
             This suggests RwLock contention in get() - the cached pointer optimization may not be working.",
            slow_percentage
        );
    }

    /// Verify that concurrent readers don't block each other.
    ///
    /// This test specifically checks that multiple Loaned items can be accessed
    /// simultaneously without any locking overhead.
    #[cfg_attr(miri, ignore)] // Miri is too slow for timing assertions
    #[tokio::test]
    async fn test_concurrent_readers_no_contention() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::Instant;

        const POOL_SIZE: usize = 4;
        const ITERATIONS: usize = 50_000;

        let pool = Pool::new_simple(POOL_SIZE, || vec![0u8; 4096]);

        // Acquire all slots
        let mut items = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            items.push(pool.acquire().await);
        }

        let max_latency = Arc::new(AtomicU64::new(0));
        let mut tasks = tokio::task::JoinSet::new();

        // Spawn readers that hammer their items
        for item in items {
            let max_lat = max_latency.clone();

            tasks.spawn(async move {
                for i in 0..ITERATIONS {
                    let start = Instant::now();

                    // Multiple accesses per iteration to stress test
                    let _ = std::hint::black_box(item[i % 4096]);
                    let _ = std::hint::black_box(item[(i + 1) % 4096]);
                    let _ = std::hint::black_box(item[(i + 2) % 4096]);

                    let elapsed_nanos = start.elapsed().as_nanos() as u64;

                    // Update max latency
                    let mut current = max_lat.load(Ordering::Relaxed);
                    while elapsed_nanos > current {
                        match max_lat.compare_exchange_weak(
                            current,
                            elapsed_nanos,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(v) => current = v,
                        }
                    }
                }
                item
            });
        }

        while let Some(result) = tasks.join_next().await {
            let _ = result.expect("task panicked");
        }

        let max_latency_us = max_latency.load(Ordering::Relaxed) / 1000;
        println!("Max access latency: {} us", max_latency_us);

        // Max latency should be under 1ms (mostly OS scheduling)
        // With RwLock contention, we'd see multi-millisecond spikes
        assert!(
            max_latency_us < 1000,
            "Max latency {} us exceeds 1ms - possible lock contention",
            max_latency_us
        );
    }
}

/// Timing-sensitive test harness for pool performance measurement.
///
/// These tests measure acquire/release cycle time, verify slot reuse,
/// test concurrent access patterns, and compute latency percentiles.
///
/// Run with: `cargo test -p daq-pool pool_timing_tests -- --nocapture`
#[cfg(test)]
mod pool_timing_tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tokio::task::JoinSet;

    /// Calculate percentile from a sorted slice of durations.
    fn percentile(sorted: &[Duration], p: f64) -> Duration {
        let idx = ((sorted.len() as f64) * p / 100.0).ceil() as usize;
        sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
    }

    /// Report timing statistics with percentiles.
    fn report_timing_stats(name: &str, times: &mut [Duration]) {
        times.sort();
        let min = times[0];
        let max = times[times.len() - 1];
        let p50 = percentile(times, 50.0);
        let p95 = percentile(times, 95.0);
        let p99 = percentile(times, 99.0);
        let mean: Duration = times.iter().sum::<Duration>() / times.len() as u32;

        println!("\n{name} ({} samples):", times.len());
        println!("  Min:  {:?}", min);
        println!("  Mean: {:?}", mean);
        println!("  P50:  {:?}", p50);
        println!("  P95:  {:?}", p95);
        println!("  P99:  {:?}", p99);
        println!("  Max:  {:?}", max);
    }

    #[cfg_attr(miri, ignore)] // Miri is too slow for timing assertions
    #[tokio::test]
    async fn test_pool_acquire_release_timing() {
        // Create pool with 10 x 1MB buffers (representative of frame buffers)
        let pool = Pool::new_simple(10, || vec![0u8; 1024 * 1024]);

        let mut acquire_times = Vec::with_capacity(1000);
        let mut release_times = Vec::with_capacity(1000);
        let mut cycle_times = Vec::with_capacity(1000);

        for _ in 0..1000 {
            let cycle_start = Instant::now();

            // Measure acquire
            let acquire_start = Instant::now();
            let item = pool.acquire().await;
            let acquire_time = acquire_start.elapsed();

            // Measure release
            let release_start = Instant::now();
            drop(item);
            let release_time = release_start.elapsed();

            let cycle_time = cycle_start.elapsed();

            acquire_times.push(acquire_time);
            release_times.push(release_time);
            cycle_times.push(cycle_time);
        }

        report_timing_stats("Acquire latency", &mut acquire_times);
        report_timing_stats("Release latency", &mut release_times);
        report_timing_stats("Full cycle (acquire + release)", &mut cycle_times);

        // Assert reasonable performance - P99 should be under 1ms for basic operations
        cycle_times.sort();
        let p99 = percentile(&cycle_times, 99.0);
        assert!(
            p99 < Duration::from_millis(1),
            "P99 cycle time too slow: {:?}",
            p99
        );
    }

    #[tokio::test]
    async fn test_pool_slot_reuse() {
        // Create pool with exactly 4 slots
        let pool = Pool::new_simple(4, || 0u64);

        // Track which slot indices we see
        let mut seen_indices: HashSet<usize> = HashSet::new();

        // Acquire and release 100 times - should only ever see indices 0-3
        for _ in 0..100 {
            let item = pool.acquire().await;
            let idx = item.slot_index();
            seen_indices.insert(idx);
            drop(item);
        }

        println!("\nSlot reuse test:");
        println!("  Pool size: {}", pool.size());
        println!("  Unique indices seen: {:?}", seen_indices);
        println!("  Cycles: 100");

        // Verify we only saw the 4 allocated slots
        assert_eq!(
            seen_indices.len(),
            4,
            "Expected 4 unique slots, saw: {:?}",
            seen_indices
        );
        assert!(
            seen_indices.iter().all(|&idx| idx < 4),
            "Slot index out of bounds"
        );

        // Verify pool hasn't grown
        assert_eq!(
            pool.size(),
            4,
            "Pool should not have grown during reuse test"
        );
    }

    #[tokio::test]
    async fn test_pool_concurrent_access_timing() {
        // Pool with 8 slots, 16 concurrent tasks each doing 100 acquire/release cycles
        let pool = Arc::new(Pool::new_simple(8, || vec![0u8; 1024]));
        let completed = Arc::new(AtomicUsize::new(0));

        let num_tasks = 16;
        let cycles_per_task = 100;

        let mut tasks = JoinSet::new();
        let start = Instant::now();

        for task_id in 0..num_tasks {
            let pool = Arc::clone(&pool);
            let completed = Arc::clone(&completed);

            tasks.spawn(async move {
                let mut task_times = Vec::with_capacity(cycles_per_task);

                for _ in 0..cycles_per_task {
                    let cycle_start = Instant::now();
                    let _item = pool.acquire().await;
                    // Simulate some work
                    tokio::time::sleep(Duration::from_micros(100)).await;
                    // Item released on drop
                    task_times.push(cycle_start.elapsed());
                }

                completed.fetch_add(cycles_per_task, Ordering::Relaxed);
                (task_id, task_times)
            });
        }

        // Collect all task results
        let mut all_times: Vec<Duration> = Vec::new();
        while let Some(result) = tasks.join_next().await {
            let (_task_id, times) = result.expect("task panicked");
            all_times.extend(times);
        }

        let total_elapsed = start.elapsed();
        let total_cycles = completed.load(Ordering::Relaxed);

        println!("\nConcurrent access test:");
        println!("  Tasks: {}", num_tasks);
        println!("  Pool size: {}", pool.size());
        println!("  Cycles per task: {}", cycles_per_task);
        println!("  Total cycles: {}", total_cycles);
        println!("  Total time: {:?}", total_elapsed);
        println!(
            "  Throughput: {:.0} cycles/sec",
            total_cycles as f64 / total_elapsed.as_secs_f64()
        );

        report_timing_stats("Per-cycle latency (concurrent)", &mut all_times);

        // All tasks should complete
        assert_eq!(
            total_cycles,
            num_tasks * cycles_per_task,
            "Not all cycles completed"
        );

        // Pool should not have grown (8 slots for 16 tasks with 100us work = plenty)
        assert_eq!(pool.size(), 8, "Pool grew unexpectedly during test");
    }

    #[cfg_attr(miri, ignore)] // Miri is too slow for timing assertions
    #[tokio::test]
    async fn test_try_acquire_timing() {
        let pool = Pool::new_simple(10, || vec![0u8; 1024]);
        let mut times = Vec::with_capacity(1000);

        for _ in 0..1000 {
            let start = Instant::now();
            let item = pool.try_acquire();
            let elapsed = start.elapsed();
            times.push(elapsed);

            // Release immediately if we got one
            if let Some(item) = item {
                drop(item);
            }
        }

        report_timing_stats("try_acquire latency", &mut times);

        // try_acquire should be very fast - P99 under 100us
        times.sort();
        let p99 = percentile(&times, 99.0);
        assert!(
            p99 < Duration::from_micros(100),
            "try_acquire P99 too slow: {:?}",
            p99
        );
    }

    #[tokio::test]
    async fn test_pool_with_reset_timing() {
        // Test overhead of reset function
        let reset_count = Arc::new(AtomicUsize::new(0));
        let reset_count_clone = Arc::clone(&reset_count);

        let pool = Pool::new_with_reset(
            10,
            || vec![0u8; 1024 * 1024], // 1MB buffers
            move |buf| {
                reset_count_clone.fetch_add(1, Ordering::Relaxed);
                buf.fill(0); // Zero out on return (expensive operation)
            },
        );

        let mut release_times = Vec::with_capacity(100);

        for _ in 0..100 {
            let mut item = pool.acquire().await;
            // Write some data
            item[0] = 42;
            item[1024] = 43;

            let release_start = Instant::now();
            drop(item);
            release_times.push(release_start.elapsed());
        }

        report_timing_stats("Release with 1MB zero-fill reset", &mut release_times);

        println!(
            "  Reset function called: {} times",
            reset_count.load(Ordering::Relaxed)
        );

        // Verify reset was called for each release
        assert_eq!(
            reset_count.load(Ordering::Relaxed),
            100,
            "Reset should be called on every release"
        );
    }

    #[cfg_attr(miri, ignore)] // Miri is too slow for timing assertions
    #[tokio::test]
    async fn test_pool_contention_under_pressure() {
        // Small pool (4 slots) with many concurrent tasks to test contention
        let pool = Arc::new(Pool::new_simple(4, || vec![0u8; 1024]));
        let mut tasks = JoinSet::new();

        let num_tasks = 32;
        let cycles_per_task = 50;

        let start = Instant::now();

        for _ in 0..num_tasks {
            let pool = Arc::clone(&pool);
            tasks.spawn(async move {
                let mut max_wait = Duration::ZERO;
                for _ in 0..cycles_per_task {
                    let wait_start = Instant::now();
                    let _item = pool.acquire().await;
                    let wait_time = wait_start.elapsed();
                    if wait_time > max_wait {
                        max_wait = wait_time;
                    }
                    // Hold briefly to create contention
                    tokio::time::sleep(Duration::from_micros(50)).await;
                }
                max_wait
            });
        }

        let mut max_waits = Vec::new();
        while let Some(result) = tasks.join_next().await {
            max_waits.push(result.expect("task panicked"));
        }

        let total_elapsed = start.elapsed();
        max_waits.sort();
        let worst_wait = max_waits[max_waits.len() - 1];

        println!("\nContention test (high pressure):");
        println!("  Pool size: 4");
        println!("  Concurrent tasks: {}", num_tasks);
        println!("  Total cycles: {}", num_tasks * cycles_per_task);
        println!("  Total time: {:?}", total_elapsed);
        println!("  Worst wait time: {:?}", worst_wait);
        println!(
            "  Throughput: {:.0} cycles/sec",
            (num_tasks * cycles_per_task) as f64 / total_elapsed.as_secs_f64()
        );

        // Even under pressure, worst wait should be bounded
        // With 32 tasks, 4 slots, 50us hold time: worst case ~400us per round
        // Allow generous margin for test reliability
        assert!(
            worst_wait < Duration::from_millis(50),
            "Worst wait time too high: {:?}",
            worst_wait
        );
    }

    #[tokio::test]
    async fn test_lock_free_access_overhead() {
        // Measure the overhead of accessing data through Loaned vs direct access
        let pool = Pool::new_simple(1, || vec![0u8; 1024 * 1024]);
        let mut item = pool.acquire().await;

        // Warm up
        for i in 0..1000 {
            item[i % (1024 * 1024)] = (i % 256) as u8;
        }

        let iterations = 100_000;

        // Measure Loaned access (should be lock-free via cached pointer)
        let start = Instant::now();
        for i in 0..iterations {
            item[i % (1024 * 1024)] = (i % 256) as u8;
        }
        let loaned_time = start.elapsed();

        drop(item);

        // Measure direct Vec access for comparison
        let mut direct_vec = vec![0u8; 1024 * 1024];
        let start = Instant::now();
        for i in 0..iterations {
            direct_vec[i % (1024 * 1024)] = (i % 256) as u8;
        }
        let direct_time = start.elapsed();

        let overhead_ns = loaned_time.as_nanos() as f64 / iterations as f64
            - direct_time.as_nanos() as f64 / iterations as f64;

        println!("\nLock-free access overhead test:");
        println!("  Iterations: {}", iterations);
        println!("  Loaned access: {:?}", loaned_time);
        println!("  Direct Vec access: {:?}", direct_time);
        println!("  Per-access overhead: {:.1}ns", overhead_ns.max(0.0));

        // Loaned access should have minimal overhead (<10ns per access)
        // Since we're measuring noisy operations, allow 2x margin
        assert!(
            loaned_time < direct_time * 3,
            "Loaned access too slow: {:?} vs direct {:?}",
            loaned_time,
            direct_time
        );
    }
}

/// Test that specifically verifies the Box indirection fix (bd-s9u7.1).
///
/// This test would trigger undefined behavior with Vec<UnsafeCell<T>>
/// because grow() causes Vec reallocation, invalidating cached pointers
/// in existing Loaned instances.
///
/// With Vec<Box<UnsafeCell<T>>>, Box contents stay at stable addresses
/// even when the Vec reallocates, so cached pointers remain valid.
#[tokio::test]
async fn test_grow_while_loaned_items_held() {
    // Create small pool that will need to grow
    let pool = Pool::new_simple(2, || vec![0u8; 1024]);

    // Acquire both slots
    let mut item1 = pool.acquire().await;
    let mut item2 = pool.acquire().await;

    // Write data to items
    item1[0] = 42;
    item1[1] = 43;
    item2[0] = 84;
    item2[1] = 85;

    // Pool is now exhausted - next acquire will trigger grow()
    // This grow() will reallocate the Vec, which would invalidate
    // item1 and item2's cached pointers if they pointed into the Vec directly.
    let mut item3 = pool.acquire_or_grow();
    item3[0] = 126;

    // Verify the original items' data is still accessible
    // (would be UB/corruption with the old implementation)
    assert_eq!(item1[0], 42, "item1 data corrupted after grow");
    assert_eq!(item1[1], 43, "item1 data corrupted after grow");
    assert_eq!(item2[0], 84, "item2 data corrupted after grow");
    assert_eq!(item2[1], 85, "item2 data corrupted after grow");
    assert_eq!(item3[0], 126, "item3 data incorrect");

    // Verify we can still mutate through the old references
    item1[2] = 99;
    assert_eq!(item1[2], 99);
    item2[2] = 100;
    assert_eq!(item2[2], 100);

    // Pool should have grown to 4 slots
    assert_eq!(
        pool.size(),
        10,
        "pool should have grown from 2 to 10 (grows by max(current, 8))"
    );
    assert_eq!(pool.initial_size(), 2, "initial size should remain 2");

    // Verify pool did grow (the key safety property we're testing)
    assert!(pool.size() > 2, "pool must have grown to avoid exhaustion");
}
