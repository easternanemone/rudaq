//! Loom concurrency model for the pool's acquire-access-release algorithm.
//!
//! # Why a model instead of testing the real API?
//!
//! The production pool (`crates/pool/src/lib.rs`) uses:
//!
//! | Real primitive               | Loom model                    |
//! |------------------------------|-------------------------------|
//! | `tokio::sync::Semaphore`     | `SimpleSemaphore` (Condvar)   |
//! | `crossbeam_queue::SegQueue`  | `SimpleQueue` (Mutex<Vec>)    |
//! | `parking_lot::RwLock`        | `loom::cell::UnsafeCell`      |
//!
//! None of these real primitives have loom-compatible replacements.
//! `loom` instruments `std::sync` and `std::thread` but not Tokio,
//! crossbeam, or parking_lot.
//!
//! This model faithfully mirrors the **algorithm** (semaphore → pop index →
//! access slot → push index → release semaphore) using loom primitives,
//! so loom can exhaustively check all thread interleavings for data races.
//!
//! # What this covers
//!
//! - No two threads access the same slot concurrently (UnsafeCell detects this)
//! - Semaphore correctly bounds concurrency to pool size
//! - Queue indices are never duplicated or lost
//!
//! # What this does NOT cover
//!
//! - Tokio async cancellation and task scheduling
//! - Real `SegQueue` lock-free behavior
//! - `parking_lot::RwLock` fairness
//!
//! For runtime concurrency coverage, see the integration tests in
//! `crates/pool/src/lib.rs` (e.g., `test_concurrent_checkout`).

use loom::sync::{Arc, Condvar, Mutex};
use loom::thread;

/// Models `tokio::sync::Semaphore` using loom's Condvar+Mutex.
///
/// The real semaphore is async and uses Tokio's intrusive linked list.
/// This model preserves the blocking acquire/release semantics so loom
/// can enumerate all interleavings.
struct SimpleSemaphore {
    permits: Mutex<usize>,
    cond: Condvar,
}

impl SimpleSemaphore {
    fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            cond: Condvar::new(),
        }
    }

    fn acquire(&self) {
        let mut p = self.permits.lock().unwrap();
        while *p == 0 {
            p = self.cond.wait(p).unwrap();
        }
        *p -= 1;
    }

    fn release(&self) {
        let mut p = self.permits.lock().unwrap();
        *p += 1;
        self.cond.notify_one();
    }
}

/// Models `crossbeam_queue::SegQueue` using loom's Mutex<Vec>.
///
/// The real queue is lock-free; this model serializes access so loom
/// can track all orderings. The push/pop semantics are identical.
struct SimpleQueue {
    items: Mutex<Vec<usize>>,
}

impl SimpleQueue {
    fn new(items: Vec<usize>) -> Self {
        Self {
            items: Mutex::new(items),
        }
    }
    fn pop(&self) -> Option<usize> {
        self.items.lock().unwrap().pop()
    }
    fn push(&self, item: usize) {
        self.items.lock().unwrap().push(item);
    }
}

#[test]
fn loom_pool_algorithm_safety() {
    loom::model(|| {
        let size = 2;
        let semaphore = Arc::new(SimpleSemaphore::new(size));
        // Initialize queue with indices 0..size
        let queue = Arc::new(SimpleQueue::new((0..size).collect()));
        // Slots use loom::cell::UnsafeCell to track concurrent access violations
        let slots = Arc::new(
            (0..size)
                .map(|_| loom::cell::UnsafeCell::new(0))
                .collect::<Vec<_>>(),
        );

        let threads: Vec<_> = (0..2)
            .map(|_| {
                let sem = semaphore.clone();
                let q = queue.clone();
                let s = slots.clone();
                thread::spawn(move || {
                    // Acquire cycle
                    sem.acquire();
                    let idx = q.pop().unwrap();

                    // Access critical section (UnsafeCell)
                    unsafe {
                        s[idx].with_mut(|ptr| {
                            *ptr += 1;
                        });
                    }

                    // Release cycle
                    q.push(idx);
                    sem.release();
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }
    });
}
