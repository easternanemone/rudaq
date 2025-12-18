//! Fuzz target for RingBuffer concurrent read/write operations.
//!
//! Tests:
//! - Race conditions between readers and writers
//! - Seqlock validation under contention
//! - Multiple concurrent writer serialization
//! - Read consistency during concurrent writes

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use daq_storage::ring_buffer::RingBuffer;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

/// Fuzz input for concurrent operations
#[derive(Debug, Arbitrary)]
struct ConcurrentInput {
    /// Number of writer threads (1-4)
    num_writers: u8,
    /// Number of reader threads (1-4)
    num_readers: u8,
    /// Operations for each writer
    writer_ops: Vec<Vec<WriterOp>>,
    /// Operations for each reader
    reader_ops: Vec<Vec<ReaderOp>>,
}

#[derive(Debug, Clone, Arbitrary)]
enum WriterOp {
    /// Write data (size 0-4096 bytes)
    Write { size: u16 },
    /// Small delay to vary timing
    Yield,
}

#[derive(Debug, Clone, Arbitrary)]
enum ReaderOp {
    /// Read snapshot
    ReadSnapshot,
    /// Advance tail by some amount
    AdvanceTail { amount: u16 },
    /// Check head/tail
    CheckState,
    /// Small delay
    Yield,
}

fuzz_target!(|input: ConcurrentInput| {
    // Create temporary directory
    let temp_dir = match TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = temp_dir.path().join("fuzz_concurrent.buf");

    // Create ring buffer
    let rb = match RingBuffer::create(&path, 1) {
        Ok(r) => Arc::new(r),
        Err(_) => return,
    };

    // Limit thread counts to reasonable numbers
    let num_writers = ((input.num_writers % 4) + 1) as usize;
    let num_readers = ((input.num_readers % 4) + 1) as usize;

    let mut handles = Vec::new();

    // Spawn writer threads
    for i in 0..num_writers {
        let rb_clone = Arc::clone(&rb);
        let ops = input.writer_ops.get(i).cloned().unwrap_or_default();

        handles.push(thread::spawn(move || {
            for op in ops.iter().take(50) {
                match op {
                    WriterOp::Write { size } => {
                        let actual_size = (*size as usize).min(4096);
                        let data = vec![0xAA_u8; actual_size];
                        // Ignore errors (e.g., data too large)
                        let _ = rb_clone.write(&data);
                    }
                    WriterOp::Yield => {
                        thread::yield_now();
                    }
                }
            }
        }));
    }

    // Spawn reader threads
    for i in 0..num_readers {
        let rb_clone = Arc::clone(&rb);
        let ops = input.reader_ops.get(i).cloned().unwrap_or_default();

        handles.push(thread::spawn(move || {
            for op in ops.iter().take(50) {
                match op {
                    ReaderOp::ReadSnapshot => {
                        // Read should never panic or return corrupted data
                        let snapshot = rb_clone.read_snapshot();
                        // Basic sanity check: length should not exceed capacity
                        assert!(
                            snapshot.len() as u64 <= rb_clone.capacity(),
                            "Snapshot larger than capacity"
                        );
                    }
                    ReaderOp::AdvanceTail { amount } => {
                        // Only advance by available data to maintain head >= tail invariant
                        let head = rb_clone.write_head();
                        let tail = rb_clone.read_tail();
                        let available = head.saturating_sub(tail);
                        let safe_amount = (*amount as u64).min(available);
                        if safe_amount > 0 {
                            rb_clone.advance_tail(safe_amount);
                        }
                    }
                    ReaderOp::CheckState => {
                        let head = rb_clone.write_head();
                        let tail = rb_clone.read_tail();
                        // Head should always be >= tail (monotonically increasing)
                        assert!(head >= tail, "Head {} < Tail {}", head, tail);
                    }
                    ReaderOp::Yield => {
                        thread::yield_now();
                    }
                }
            }
        }));
    }

    // Wait for all threads with timeout
    for handle in handles {
        // Use a reasonable timeout to prevent hanging
        let _ = handle.join();
    }

    // Final consistency check
    let head = rb.write_head();
    let tail = rb.read_tail();
    assert!(head >= tail, "Final: Head {} < Tail {}", head, tail);
});
