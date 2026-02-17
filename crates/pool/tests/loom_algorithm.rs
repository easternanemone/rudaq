use loom::sync::{Arc, Condvar, Mutex};
use loom::thread;

// Simplified Semaphore for Loom model
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

// Queue using Mutex for simplicity in model
// (Real implementation uses crossbeam_queue::SegQueue which is also correct)
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
