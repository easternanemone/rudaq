#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::manual_div_ceil,
    missing_docs
)]
//! Criterion benchmarks for ring buffer hot paths.
//!
//! These benchmarks establish performance baselines for the memory-mapped ring buffer,
//! which is critical for achieving 10k+ writes/sec throughput in the data acquisition system.
//!
//! Key metrics:
//! - Write throughput (ops/sec) for various data sizes
//! - Read snapshot latency
//! - Concurrent write/read performance
//!
//! Run with: cargo bench --bench ring_buffer

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use daq_storage::ring_buffer::RingBuffer;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Benchmark writing different sizes of data to the ring buffer.
///
/// This measures the core write path throughput, which is critical for
/// high-speed data acquisition. Tests multiple data sizes to understand
/// scaling characteristics.
fn ring_buffer_write_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_buffer_write");

    // Test various data sizes common in scientific data acquisition
    let sizes = vec![
        ("1KB", 1024),
        ("4KB", 4096),
        ("16KB", 16 * 1024),
        ("64KB", 64 * 1024),
        ("256KB", 256 * 1024),
        ("1MB", 1024 * 1024),
    ];

    for (name, size) in sizes {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = RingBuffer::create(&path, 100).unwrap(); // 100 MB buffer

        let data = vec![0u8; size];

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("write", name), &size, |b, _| {
            b.iter(|| {
                rb.write(black_box(&data)).unwrap();
            });
        });
    }

    group.finish();
}

/// Benchmark read snapshot performance.
///
/// Measures the latency of reading the current buffer contents,
/// which is used by consumers to retrieve data for processing.
fn ring_buffer_read_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_buffer_read");

    // Pre-populate buffer with different amounts of data
    let data_amounts = vec![
        ("empty", 0),
        ("1KB", 1024),
        ("16KB", 16 * 1024),
        ("256KB", 256 * 1024),
        ("1MB", 1024 * 1024),
    ];

    for (name, amount) in data_amounts {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = RingBuffer::create(&path, 100).unwrap();

        // Pre-populate buffer
        if amount > 0 {
            let data = vec![0xAA; amount];
            rb.write(&data).unwrap();
        }

        group.bench_with_input(BenchmarkId::new("read_snapshot", name), &amount, |b, _| {
            b.iter(|| {
                let snapshot = rb.read_snapshot();
                black_box(snapshot);
            });
        });
    }

    group.finish();
}

/// Benchmark concurrent write operations.
///
/// Tests write performance with multiple concurrent writers,
/// which is important for multi-threaded acquisition systems.
fn ring_buffer_concurrent_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_buffer_concurrent");

    // Test with different numbers of concurrent writers
    let thread_counts = vec![1, 2, 4, 8];

    for thread_count in thread_counts {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = Arc::new(RingBuffer::create(&path, 100).unwrap());

        let data = vec![0u8; 1024]; // 1KB per write

        group.bench_with_input(
            BenchmarkId::new("concurrent_writes", thread_count),
            &thread_count,
            |b, &thread_count| {
                b.iter(|| {
                    let mut handles = vec![];

                    for _ in 0..thread_count {
                        let rb_clone = Arc::clone(&rb);
                        let data_clone = data.clone();

                        let handle = thread::spawn(move || {
                            for _ in 0..10 {
                                rb_clone.write(&data_clone).unwrap();
                            }
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark atomic position queries.
///
/// Measures the overhead of querying write_head and read_tail positions,
/// which are frequently accessed by monitoring and control code.
fn ring_buffer_position_queries(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("bench_ring.buf");
    let rb = RingBuffer::create(&path, 100).unwrap();

    // Pre-populate with some data
    let data = vec![0xBB; 1024];
    rb.write(&data).unwrap();

    c.bench_function("ring_buffer_write_head", |b| {
        b.iter(|| {
            let head = rb.write_head();
            black_box(head);
        });
    });

    c.bench_function("ring_buffer_read_tail", |b| {
        b.iter(|| {
            let tail = rb.read_tail();
            black_box(tail);
        });
    });

    c.bench_function("ring_buffer_advance_tail", |b| {
        b.iter(|| {
            rb.advance_tail(black_box(64));
        });
    });
}

/// Benchmark wrap-around performance.
///
/// Tests the overhead of circular buffer wrap-around logic,
/// which becomes important as the buffer fills and reuses space.
fn ring_buffer_wrap_around(c: &mut Criterion) {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("bench_ring.buf");

    // Create a small buffer (1MB) to force frequent wrap-around
    let rb = RingBuffer::create(&path, 1).unwrap();

    // Write data that will wrap (512KB chunks in 1MB buffer)
    let data = vec![0xCC; 512 * 1024];

    c.bench_function("ring_buffer_wrap_write", |b| {
        b.iter(|| {
            rb.write(black_box(&data)).unwrap();
        });
    });
}

/// Benchmark tap reader latency.
///
/// Measures end-to-end latency from write() to tap receiver getting data.
/// This is critical for live preview and real-time monitoring use cases.
fn ring_buffer_tap_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("ring_buffer_tap_latency");

    // Test different data sizes
    let sizes = vec![("1KB", 1024), ("16KB", 16 * 1024), ("256KB", 256 * 1024)];

    for (name, size) in sizes {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = RingBuffer::create(&path, 100).unwrap();

        // Register tap to receive every frame
        let mut rx = rb.register_tap("bench_tap".to_string(), 1).unwrap();

        let data = vec![0xDD; size];

        group.bench_with_input(BenchmarkId::new("latency", name), &size, |b, _| {
            b.iter(|| {
                // Write data
                rb.write(black_box(&data)).unwrap();

                // Receive through tap (blocking wait with timeout)
                let received = rt.block_on(async {
                    tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
                });

                let _ = black_box(received);
            });
        });

        // Cleanup
        let _ = rb.unregister_tap("bench_tap");
    }

    group.finish();
}

/// Benchmark tap reader throughput.
///
/// Measures sustained throughput when reading frames through a tap,
/// simulating a live preview consumer.
fn ring_buffer_tap_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("ring_buffer_tap_throughput");

    // Test with 4KB frames (typical for measurements)
    let frame_size = 4096;
    let frames_per_iter = 100;

    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("bench_ring.buf");
    let rb = Arc::new(RingBuffer::create(&path, 100).unwrap());

    // Register tap
    let mut rx = rb.register_tap("bench_tap".to_string(), 1).unwrap();

    let data = vec![0xEE; frame_size];

    group.throughput(Throughput::Bytes((frame_size * frames_per_iter) as u64));
    group.bench_function("tap_throughput_4KB_frames", |b| {
        b.iter(|| {
            // Write frames
            for _ in 0..frames_per_iter {
                rb.write(black_box(&data)).unwrap();
            }

            // Drain tap receiver
            rt.block_on(async {
                for _ in 0..frames_per_iter {
                    if let Ok(Some(frame)) =
                        tokio::time::timeout(Duration::from_millis(10), rx.recv()).await
                    {
                        black_box(frame);
                    }
                }
            });
        });
    });

    group.finish();
}

/// Benchmark multiple taps overhead.
///
/// Tests how performance scales with multiple concurrent tap consumers,
/// which is important for systems with multiple live preview windows
/// or monitoring clients.
fn ring_buffer_multiple_taps(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("ring_buffer_multiple_taps");

    let frame_size = 4096;
    let tap_counts = vec![1, 2, 4, 8];

    for tap_count in tap_counts {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = RingBuffer::create(&path, 100).unwrap();

        // Register multiple taps
        let mut receivers: Vec<_> = (0..tap_count)
            .map(|i| rb.register_tap(format!("tap_{}", i), 1).unwrap())
            .collect();

        let data = vec![0xFF; frame_size];

        group.bench_with_input(
            BenchmarkId::new("write_with_taps", tap_count),
            &tap_count,
            |b, _| {
                b.iter(|| {
                    // Write one frame (sent to all taps)
                    rb.write(black_box(&data)).unwrap();

                    // Drain all taps
                    rt.block_on(async {
                        for rx in &mut receivers {
                            if let Ok(Some(frame)) =
                                tokio::time::timeout(Duration::from_millis(10), rx.recv()).await
                            {
                                black_box(frame);
                            }
                        }
                    });
                });
            },
        );

        // Cleanup
        for i in 0..tap_count {
            let _ = rb.unregister_tap(&format!("tap_{}", i));
        }
    }

    group.finish();
}

/// Benchmark tap with decimation (nth_frame > 1).
///
/// Tests tap performance when receiving only every Nth frame,
/// which is common for bandwidth-limited preview scenarios.
fn ring_buffer_tap_decimation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("ring_buffer_tap_decimation");

    let frame_size = 4096;
    let total_frames = 100;
    let decimation_factors = vec![1, 5, 10, 20];

    for nth_frame in decimation_factors {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("bench_ring.buf");
        let rb = RingBuffer::create(&path, 100).unwrap();

        // Register tap with decimation
        let mut rx = rb.register_tap("bench_tap".to_string(), nth_frame).unwrap();

        let data = vec![0xAA; frame_size];
        let expected_frames = (total_frames + nth_frame - 1) / nth_frame;

        group.bench_with_input(
            BenchmarkId::new("decimation", format!("1:{}", nth_frame)),
            &nth_frame,
            |b, _| {
                b.iter(|| {
                    // Write all frames
                    for _ in 0..total_frames {
                        rb.write(black_box(&data)).unwrap();
                    }

                    // Receive decimated frames
                    rt.block_on(async {
                        for _ in 0..expected_frames {
                            if let Ok(Some(frame)) =
                                tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
                            {
                                black_box(frame);
                            }
                        }
                    });
                });
            },
        );

        let _ = rb.unregister_tap("bench_tap");
    }

    group.finish();
}

criterion_group!(
    benches,
    ring_buffer_write_throughput,
    ring_buffer_read_snapshot,
    ring_buffer_concurrent_writes,
    ring_buffer_position_queries,
    ring_buffer_wrap_around,
    ring_buffer_tap_latency,
    ring_buffer_tap_throughput,
    ring_buffer_multiple_taps,
    ring_buffer_tap_decimation
);
criterion_main!(benches);
