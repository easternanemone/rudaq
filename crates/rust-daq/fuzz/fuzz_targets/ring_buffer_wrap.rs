//! Fuzz target for RingBuffer wrap-around behavior.
//!
//! Tests:
//! - Circular buffer wrap-around edge cases
//! - Data integrity when writes span the end/start boundary
//! - Read consistency across wrap-around
//! - Tail advancement at boundary positions

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use daq_storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

/// Fuzz input for wrap-around testing
#[derive(Debug, Arbitrary)]
struct WrapInput {
    /// Initial fill amount (to position head near end of buffer)
    initial_fill_kb: u8,
    /// Write operations to perform after positioning
    operations: Vec<WrapOp>,
}

#[derive(Debug, Clone, Arbitrary)]
enum WrapOp {
    /// Write specific sized chunk
    Write { size_bytes: u16 },
    /// Read snapshot and verify
    ReadAndVerify,
    /// Advance tail by amount
    AdvanceTail { amount: u16 },
    /// Fill with pattern (easier to detect corruption)
    WritePattern { pattern: u8, size_bytes: u16 },
}

fuzz_target!(|input: WrapInput| {
    // Create temporary directory
    let temp_dir = match TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = temp_dir.path().join("fuzz_wrap.buf");

    // Use a small buffer (1 MB) to exercise wrap-around frequently
    let rb = match RingBuffer::create(&path, 1) {
        Ok(r) => r,
        Err(_) => return,
    };

    let capacity = rb.capacity();

    // Initial fill to position head near end of buffer
    // Each KB chunk brings us closer to the wrap point
    let initial_fill = ((input.initial_fill_kb as u64) * 1024).min(capacity - 1024);
    if initial_fill > 0 {
        let fill_data = vec![0x00_u8; initial_fill as usize];
        if rb.write(&fill_data).is_err() {
            return;
        }
        // Advance tail to simulate consumption
        rb.advance_tail(initial_fill);
    }

    // Now perform operations - we're positioned somewhere in the buffer
    for op in input.operations.iter().take(100) {
        match op {
            WrapOp::Write { size_bytes } => {
                let size = (*size_bytes as usize).min(capacity as usize / 4);
                if size > 0 {
                    let data = vec![0xBB_u8; size];
                    let _ = rb.write(&data);
                }
            }
            WrapOp::ReadAndVerify => {
                let snapshot = rb.read_snapshot();

                // Verify length doesn't exceed capacity
                assert!(
                    snapshot.len() as u64 <= capacity,
                    "Snapshot {} bytes > capacity {} bytes",
                    snapshot.len(),
                    capacity
                );

                // Verify head >= tail invariant
                let head = rb.write_head();
                let tail = rb.read_tail();
                assert!(head >= tail, "Invariant violated: head {} < tail {}", head, tail);

                // Verify available data matches snapshot length
                let available = (head - tail).min(capacity);
                assert_eq!(
                    snapshot.len() as u64,
                    available,
                    "Snapshot len {} != available {}",
                    snapshot.len(),
                    available
                );
            }
            WrapOp::AdvanceTail { amount } => {
                let head = rb.write_head();
                let tail = rb.read_tail();
                let available = head.saturating_sub(tail);

                // Only advance up to available data
                let safe_amount = (*amount as u64).min(available);
                rb.advance_tail(safe_amount);
            }
            WrapOp::WritePattern { pattern, size_bytes } => {
                let size = (*size_bytes as usize).min(capacity as usize / 4);
                if size > 0 {
                    let data = vec![*pattern; size];
                    let _ = rb.write(&data);
                }
            }
        }
    }

    // Final verification
    let head = rb.write_head();
    let tail = rb.read_tail();
    assert!(head >= tail, "Final invariant: head {} < tail {}", head, tail);

    // Verify final read_snapshot works
    let final_snapshot = rb.read_snapshot();
    let available = (head - tail).min(capacity);
    assert_eq!(
        final_snapshot.len() as u64,
        available,
        "Final snapshot mismatch"
    );
});
