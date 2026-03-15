//! Borrow guard for preventing pool slot reclamation during foreign access.
//!
//! When an external consumer (Python via PyO3, C/C++ via FFI) holds a
//! `BorrowGuard`, the underlying pool slot cannot be recycled until all
//! guards are dropped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::foreign_view::ForeignView;
use crate::FrameData;

/// Tracks the number of active borrows on a pool slot.
///
/// While the borrow count is > 0, the slot must not be recycled.
#[derive(Debug)]
pub struct BorrowCount {
    count: AtomicUsize,
}

impl BorrowCount {
    /// Create a new borrow count initialized to zero.
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    /// Increment the borrow count, returning the new count.
    pub fn increment(&self) -> usize {
        self.count.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Decrement the borrow count, returning the new count.
    pub fn decrement(&self) -> usize {
        self.count.fetch_sub(1, Ordering::AcqRel) - 1
    }

    /// Returns `true` if there are any active borrows.
    pub fn is_borrowed(&self) -> bool {
        self.count.load(Ordering::Acquire) > 0
    }

    /// Returns the current borrow count.
    pub fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}

impl Default for BorrowCount {
    fn default() -> Self {
        Self::new()
    }
}

/// A guard that keeps a pool slot alive while held.
///
/// Implements `ForeignView` by delegating to the underlying `FrameData`.
/// When all guards for a slot are dropped, the slot becomes eligible for recycling.
pub struct BorrowGuard {
    /// The frame data being borrowed.
    frame: Arc<FrameData>,
    /// Shared borrow counter for this slot.
    borrow_count: Arc<BorrowCount>,
}

impl BorrowGuard {
    /// Create a new borrow guard for a frame.
    ///
    /// Increments the borrow count on creation. The count is decremented
    /// when this guard is dropped.
    pub fn new(frame: Arc<FrameData>, borrow_count: Arc<BorrowCount>) -> Self {
        borrow_count.increment();
        Self {
            frame,
            borrow_count,
        }
    }

    /// Access the underlying frame data.
    pub fn frame(&self) -> &FrameData {
        &self.frame
    }
}

impl Drop for BorrowGuard {
    fn drop(&mut self) {
        self.borrow_count.decrement();
    }
}

impl ForeignView for BorrowGuard {
    fn as_ptr(&self) -> *const u8 {
        ForeignView::as_ptr(self.frame.as_ref())
    }

    fn len(&self) -> usize {
        ForeignView::len(self.frame.as_ref())
    }

    fn shape(&self) -> (u32, u32) {
        ForeignView::shape(self.frame.as_ref())
    }

    fn dtype(&self) -> &str {
        ForeignView::dtype(self.frame.as_ref())
    }

    fn bit_depth(&self) -> u32 {
        ForeignView::bit_depth(self.frame.as_ref())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_frame() -> Arc<FrameData> {
        let mut frame = FrameData::with_capacity(1024);
        frame.width = 32;
        frame.height = 32;
        frame.bit_depth = 16;
        frame.actual_len = 512;
        Arc::new(frame)
    }

    #[test]
    fn borrow_count_starts_at_zero() {
        let bc = BorrowCount::new();
        assert_eq!(bc.count(), 0);
        assert!(!bc.is_borrowed());
    }

    #[test]
    fn borrow_count_default_starts_at_zero() {
        let bc = BorrowCount::default();
        assert_eq!(bc.count(), 0);
        assert!(!bc.is_borrowed());
    }

    #[test]
    fn borrow_count_increment_and_decrement() {
        let bc = BorrowCount::new();

        assert_eq!(bc.increment(), 1);
        assert_eq!(bc.count(), 1);
        assert!(bc.is_borrowed());

        assert_eq!(bc.increment(), 2);
        assert_eq!(bc.count(), 2);

        assert_eq!(bc.decrement(), 1);
        assert_eq!(bc.count(), 1);
        assert!(bc.is_borrowed());

        assert_eq!(bc.decrement(), 0);
        assert_eq!(bc.count(), 0);
        assert!(!bc.is_borrowed());
    }

    #[test]
    fn guard_increments_on_creation() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());

        let _guard = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));
        assert_eq!(bc.count(), 1);
        assert!(bc.is_borrowed());
    }

    #[test]
    fn guard_decrements_on_drop() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());

        let guard = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));
        assert_eq!(bc.count(), 1);

        drop(guard);
        assert_eq!(bc.count(), 0);
        assert!(!bc.is_borrowed());
    }

    #[test]
    fn multiple_guards_stack_correctly() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());

        let g1 = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));
        assert_eq!(bc.count(), 1);

        let g2 = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));
        assert_eq!(bc.count(), 2);

        let g3 = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));
        assert_eq!(bc.count(), 3);

        drop(g2);
        assert_eq!(bc.count(), 2);

        drop(g1);
        assert_eq!(bc.count(), 1);
        assert!(bc.is_borrowed());

        drop(g3);
        assert_eq!(bc.count(), 0);
        assert!(!bc.is_borrowed());
    }

    #[test]
    fn foreign_view_delegation() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());
        let guard = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));

        // Pointer should match the underlying frame
        assert_eq!(
            ForeignView::as_ptr(&guard),
            ForeignView::as_ptr(frame.as_ref())
        );
        assert_eq!(ForeignView::len(&guard), 512);
        assert_eq!(guard.shape(), (32, 32));
        assert_eq!(guard.dtype(), "u16");
        assert_eq!(ForeignView::bit_depth(&guard), 16);
        assert!(!guard.is_empty());
    }

    #[test]
    fn foreign_view_empty_frame() {
        let frame = Arc::new(FrameData::with_capacity(1024));
        let bc = Arc::new(BorrowCount::new());
        let guard = BorrowGuard::new(frame, bc);

        assert_eq!(ForeignView::len(&guard), 0);
        assert!(guard.is_empty());
    }

    #[test]
    fn guard_exposes_frame_reference() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());
        let guard = BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc));

        assert_eq!(guard.frame().width, 32);
        assert_eq!(guard.frame().height, 32);
        assert_eq!(guard.frame().actual_len, 512);
    }

    #[test]
    fn concurrent_guard_creation_from_multiple_threads() {
        let frame = make_test_frame();
        let bc = Arc::new(BorrowCount::new());
        let num_threads = 8;
        let guards_per_thread = 100;

        let barrier = Arc::new(std::sync::Barrier::new(num_threads));
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let frame = Arc::clone(&frame);
                let bc = Arc::clone(&bc);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut guards = Vec::new();
                    for _ in 0..guards_per_thread {
                        guards.push(BorrowGuard::new(Arc::clone(&frame), Arc::clone(&bc)));
                    }
                    // All guards alive here
                    assert!(bc.is_borrowed());
                    // Drop all guards
                    drop(guards);
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        // After all threads complete, borrow count must be exactly zero
        assert_eq!(
            bc.count(),
            0,
            "borrow count must be zero after all concurrent guards are dropped"
        );
        assert!(!bc.is_borrowed());
    }
}
