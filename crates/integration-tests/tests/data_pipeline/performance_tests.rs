// =============================================================================
// Performance Benchmarks
// =============================================================================

use super::helpers::*;
use common::core::Measurement;
use storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

#[tokio::test]
async fn test_arrow_batch_creation_performance() {
    let start = std::time::Instant::now();
    let iterations = 1000;

    for _ in 0..iterations {
        let measurements = vec![create_test_scalar("test", 42.0)];
        let _batches = Measurement::into_arrow_batches(&measurements).unwrap();
    }

    let elapsed = start.elapsed();
    let ops_per_sec = f64::from(iterations) / elapsed.as_secs_f64();

    println!("Arrow batch creation: {:.0} ops/sec", ops_per_sec);
    assert!(ops_per_sec > 1000.0, "Should create batches quickly");
}

#[tokio::test]
async fn test_ringbuffer_write_performance() {
    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("test_ring_perf.buf");

    let ring = RingBuffer::create(&ring_path, 100).unwrap();
    let test_data = vec![0u8; 1024]; // 1 KB per write

    let start = std::time::Instant::now();
    let iterations = 10_000;

    for _ in 0..iterations {
        ring.write(&test_data).unwrap();
    }

    let elapsed = start.elapsed();
    let ops_per_sec = f64::from(iterations) / elapsed.as_secs_f64();

    println!("Ring buffer write: {:.0} ops/sec", ops_per_sec);
    assert!(ops_per_sec > 5_000.0, "Should write quickly");
}
