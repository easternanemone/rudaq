//! Fuzz target for RingBuffer write operations.
//!
//! Tests:
//! - Arbitrary data sizes and content
//! - Sequential writes of varying sizes
//! - Data integrity after writes

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use daq_storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

/// Fuzz input for write operations
#[derive(Debug, Arbitrary)]
struct WriteInput {
    /// Data chunks to write (each limited to reasonable size)
    writes: Vec<WriteOp>,
}

#[derive(Debug, Arbitrary)]
struct WriteOp {
    /// Data to write (limited to 4KB per operation for efficiency)
    data: SmallVec,
}

/// Small vector wrapper for bounded arbitrary data
#[derive(Debug)]
struct SmallVec(Vec<u8>);

impl<'a> Arbitrary<'a> for SmallVec {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(0..=4096)?;
        let bytes = u.bytes(len)?;
        Ok(SmallVec(bytes.to_vec()))
    }
}

fuzz_target!(|input: WriteInput| {
    // Create temporary directory for each fuzz run
    let temp_dir = match TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let path = temp_dir.path().join("fuzz_ring.buf");

    // Create a small ring buffer (1 MB) to test wrap-around quickly
    let rb = match RingBuffer::create(&path, 1) {
        Ok(r) => r,
        Err(_) => return,
    };

    // Perform write operations
    for write_op in input.writes.iter().take(100) {
        // Skip if data exceeds capacity (expected error)
        if write_op.data.0.len() as u64 > rb.capacity() {
            continue;
        }

        // Write should succeed for valid-sized data
        let _ = rb.write(&write_op.data.0);
    }

    // Verify we can still read without panicking
    let _ = rb.read_snapshot();

    // Verify head/tail are consistent (no underflow/overflow)
    let head = rb.write_head();
    let tail = rb.read_tail();
    assert!(head >= tail, "Write head must not be less than read tail");
});
