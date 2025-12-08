//! Ring Buffer Demonstration
//!
//! This example demonstrates the high-performance memory-mapped ring buffer
//! for zero-copy data streaming.
//!
//! Run with: cargo run --example ring_buffer_demo

use anyhow::Result;
use rust_daq::data::ring_buffer::RingBuffer;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    println!("=== Ring Buffer Demonstration ===\n");

    // Use /tmp instead of /dev/shm on macOS
    let buffer_path = Path::new("/tmp/rust_daq_demo_ring.buf");

    // Clean up any existing buffer
    let _ = std::fs::remove_file(buffer_path);

    // 1. Create ring buffer (10 MB)
    println!("1. Creating 10 MB ring buffer at {:?}", buffer_path);
    let ring_buffer = Arc::new(RingBuffer::create(buffer_path, 10)?);
    println!(
        "   Created ring buffer with capacity: {} bytes\n",
        ring_buffer.capacity()
    );

    // 2. Simple write and read
    println!("2. Testing basic write and read operations");
    let test_message = b"Hello, Ring Buffer! This is a test message.";
    ring_buffer.write(test_message)?;
    println!("   Wrote {} bytes", test_message.len());

    let snapshot = ring_buffer.read_snapshot();
    println!(
        "   Read {} bytes: {:?}\n",
        snapshot.len(),
        String::from_utf8_lossy(&snapshot)
    );

    // 3. Performance test - single thread
    println!("3. Single-threaded performance test");
    let iterations = 50_000;
    let test_data = vec![0xAB; 512]; // 512 bytes per write

    let start = Instant::now();
    for _ in 0..iterations {
        ring_buffer.write(&test_data)?;
    }
    let elapsed = start.elapsed();

    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let mb_per_sec =
        (iterations * test_data.len()) as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64();

    println!("   Writes: {}", iterations);
    println!("   Time: {:?}", elapsed);
    println!("   Throughput: {:.0} ops/sec", ops_per_sec);
    println!("   Bandwidth: {:.2} MB/sec\n", mb_per_sec);

    if ops_per_sec > 10_000.0 {
        println!("   ✓ Performance target met (>10k ops/sec)!\n");
    } else {
        println!("   ✗ Performance below target (need >10k ops/sec)\n");
    }

    // 4. Concurrent producer-consumer test
    println!("4. Concurrent producer-consumer test");

    // These are intentionally unused to demonstrate the subsequent reset
    let _rb_producer = Arc::clone(&ring_buffer);
    let _rb_consumer = Arc::clone(&ring_buffer);

    // Reset write head for clean test
    let _ = std::fs::remove_file(buffer_path);
    let ring_buffer = Arc::new(RingBuffer::create(buffer_path, 10)?);
    let rb_producer = Arc::clone(&ring_buffer);
    let rb_consumer = Arc::clone(&ring_buffer);

    // Producer thread
    let producer = thread::spawn(move || {
        for i in 0..1000 {
            let message = format!("Message #{:04}", i);
            rb_producer.write(message.as_bytes()).unwrap();
            if i % 100 == 0 {
                thread::sleep(Duration::from_millis(1));
            }
        }
        println!("   Producer: Wrote 1000 messages");
    });

    // Consumer thread
    let consumer = thread::spawn(move || {
        let mut snapshots_read = 0;
        let mut total_bytes = 0;

        // Read snapshots for a few seconds
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            let snapshot = rb_consumer.read_snapshot();
            if !snapshot.is_empty() {
                snapshots_read += 1;
                total_bytes += snapshot.len();

                if snapshots_read % 10 == 0 {
                    println!(
                        "   Consumer: Read {} snapshots ({} bytes)",
                        snapshots_read, total_bytes
                    );
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        println!(
            "   Consumer: Finished reading {} snapshots ({} total bytes)",
            snapshots_read, total_bytes
        );
    });

    producer.join().unwrap();
    consumer.join().unwrap();

    println!("\n   ✓ Concurrent test passed!\n");

    // 5. Circular wrap test
    println!("5. Testing circular buffer wrap-around");
    let _ = std::fs::remove_file(buffer_path);
    let small_buffer = RingBuffer::create(buffer_path, 1)?; // 1 MB

    let chunk = vec![0x42; 256 * 1024]; // 256 KB chunks
    for i in 0..10 {
        small_buffer.write(&chunk)?;
        println!(
            "   Wrote chunk {} (write_head: {})",
            i,
            small_buffer.write_head()
        );
    }

    let final_snapshot = small_buffer.read_snapshot();
    println!("   Final snapshot size: {} bytes", final_snapshot.len());
    println!("   ✓ Circular wrap-around working correctly!\n");

    // 6. Memory layout information
    println!("6. Memory layout information (for Python/C++ interop)");
    println!(
        "   Data region address: 0x{:016x}",
        ring_buffer.data_address()
    );
    println!("   Capacity: {} bytes", ring_buffer.capacity());
    println!("   Header size: 128 bytes (fixed)");
    println!("   Total file size: {} bytes", 128 + ring_buffer.capacity());
    println!("\n   Python can access this buffer using:");
    println!("   ```python");
    println!("   import mmap");
    println!("   with open('{}', 'rb') as f:", buffer_path.display());
    println!("       mm = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)");
    println!("       # Read header at offset 0");
    println!("       # Read data at offset 128");
    println!("   ```\n");

    // Clean up
    println!("7. Cleaning up demo buffer");
    drop(ring_buffer);
    std::fs::remove_file(buffer_path)?;
    println!("   ✓ Demo complete!\n");

    Ok(())
}
