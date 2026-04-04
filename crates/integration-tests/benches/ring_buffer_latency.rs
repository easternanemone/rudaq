//! End-to-end latency benchmark for RingBuffer tap delivery.
//!
//! Measures the time from `RingBuffer::write()` completing through tap channel
//! notification to consumer receipt. Tests with 1, 2, and 4 tap consumers at
//! simulated frame rates, reporting p50, p95, and p99 latency.
//!
//! Run with: `cargo bench -p integration-tests --bench ring_buffer_latency`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use storage::ring_buffer::RingBuffer;

/// Number of frames to write per scenario.
const FRAME_COUNT: usize = 2_000;

/// Frame size in bytes (1 KB — representative of small metadata frames).
const FRAME_SIZE: usize = 1024;

/// Ring buffer capacity in MB (small — we only need a few frames in flight).
const RING_BUFFER_MB: usize = 1;

/// Tap consumer configurations to benchmark.
const TAP_COUNTS: &[usize] = &[1, 2, 4];

// =============================================================================
// Percentile helper
// =============================================================================

struct LatencyStats {
    count: usize,
    p50: Duration,
    p95: Duration,
    p99: Duration,
    min: Duration,
    max: Duration,
    mean: Duration,
}

fn compute_stats(latencies: &mut [Duration]) -> LatencyStats {
    latencies.sort();
    let count = latencies.len();
    assert!(count > 0, "latency vector must not be empty");

    let total: Duration = latencies.iter().sum();
    let mean = total / count as u32;

    let p = |pct: f64| -> Duration {
        let idx = ((pct / 100.0) * (count as f64 - 1.0)).round() as usize;
        latencies[idx.min(count - 1)]
    };

    LatencyStats {
        count,
        p50: p(50.0),
        p95: p(95.0),
        p99: p(99.0),
        min: latencies[0],
        max: latencies[count - 1],
        mean,
    }
}

fn print_stats(label: &str, stats: &LatencyStats) {
    println!("---");
    println!("bench: {label}");
    println!("frames: {}", stats.count);
    println!("min_us: {:.1}", stats.min.as_secs_f64() * 1_000_000.0);
    println!("max_us: {:.1}", stats.max.as_secs_f64() * 1_000_000.0);
    println!("mean_us: {:.1}", stats.mean.as_secs_f64() * 1_000_000.0);
    println!("p50_us: {:.1}", stats.p50.as_secs_f64() * 1_000_000.0);
    println!("p95_us: {:.1}", stats.p95.as_secs_f64() * 1_000_000.0);
    println!("p99_us: {:.1}", stats.p99.as_secs_f64() * 1_000_000.0);
}

// =============================================================================
// Temp file helper
// =============================================================================

fn temp_ring_buffer_path(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rust_daq_bench");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(format!("ring_buffer_{label}_{}.dat", std::process::id()))
}

fn cleanup_ring_buffer(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

// =============================================================================
// Benchmark: write-to-tap latency
// =============================================================================

/// Runs a single scenario: write FRAME_COUNT frames, measure per-frame latency
/// from write start to all tap consumers receiving the data.
///
/// Architecture:
/// - Writer thread (blocking, via `spawn_blocking`) writes frames and records
///   a monotonic send timestamp per frame using an `AtomicU64`.
/// - Consumer tasks receive frames asynchronously and compute latency from the
///   send timestamp.
async fn bench_tap_latency(tap_count: usize) -> Vec<Duration> {
    let label = format!("taps_{tap_count}");
    let path = temp_ring_buffer_path(&label);

    let rb = Arc::new(RingBuffer::create(&path, RING_BUFFER_MB).expect("create ring buffer"));

    // Register tap consumers (nth_frame = 1 means every frame)
    let mut receivers = Vec::with_capacity(tap_count);
    for i in 0..tap_count {
        let rx = rb
            .register_tap(format!("bench_tap_{i}"), 1)
            .expect("register tap");
        receivers.push(rx);
    }

    // Shared atomic timestamp: writer stores Instant-as-nanos-since-epoch,
    // consumers read it to compute latency. We use a sequence approach:
    // the writer bumps a frame counter, consumers know which frame they got.
    let send_timestamps: Arc<Vec<AtomicU64>> =
        Arc::new((0..FRAME_COUNT).map(|_| AtomicU64::new(0)).collect());

    // Epoch for relative timing
    let epoch = Instant::now();

    // Spawn consumer tasks
    let mut consumer_handles = Vec::with_capacity(tap_count);
    for mut rx in receivers {
        let timestamps = Arc::clone(&send_timestamps);
        let consumer_epoch = epoch;
        consumer_handles.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(FRAME_COUNT);
            let mut frame_idx = 0usize;

            while let Some(_data) = rx.recv().await {
                let recv_time = consumer_epoch.elapsed().as_nanos() as u64;

                // Read the send timestamp for this frame
                if frame_idx < FRAME_COUNT {
                    let send_time = timestamps[frame_idx].load(Ordering::Acquire);
                    if send_time > 0 {
                        let latency_nanos = recv_time.saturating_sub(send_time);
                        latencies.push(Duration::from_nanos(latency_nanos));
                    }
                }
                frame_idx += 1;
            }
            latencies
        }));
    }

    // Generate frame data
    let frame_data: Vec<u8> = (0..FRAME_SIZE).map(|i| (i % 256) as u8).collect();

    // Writer: use spawn_blocking because RingBuffer::write() warns in async context
    let rb_writer = Arc::clone(&rb);
    let timestamps_writer = Arc::clone(&send_timestamps);
    let writer_epoch = epoch;
    let writer_handle = tokio::task::spawn_blocking(move || {
        for i in 0..FRAME_COUNT {
            // Record send timestamp before write
            let now = writer_epoch.elapsed().as_nanos() as u64;
            timestamps_writer[i].store(now, Ordering::Release);

            rb_writer
                .write(&frame_data)
                .expect("ring buffer write failed");
        }
    });

    // Wait for writer to finish
    writer_handle.await.expect("writer task panicked");

    // Drop the ring buffer to close tap senders, signaling consumers to exit
    drop(rb);

    // Collect consumer latencies
    let mut all_latencies = Vec::new();
    for handle in consumer_handles {
        let mut lats = handle.await.expect("consumer task panicked");
        all_latencies.append(&mut lats);
    }

    // Cleanup
    cleanup_ring_buffer(&path);

    all_latencies
}

// =============================================================================
// Benchmark: write throughput without taps (baseline)
// =============================================================================

async fn bench_write_throughput_baseline() -> Duration {
    let path = temp_ring_buffer_path("baseline");

    let rb = Arc::new(RingBuffer::create(&path, RING_BUFFER_MB).expect("create ring buffer"));

    let frame_data: Vec<u8> = (0..FRAME_SIZE).map(|i| (i % 256) as u8).collect();

    let rb_writer = Arc::clone(&rb);
    let elapsed = tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        for _ in 0..FRAME_COUNT {
            rb_writer
                .write(&frame_data)
                .expect("ring buffer write failed");
        }
        start.elapsed()
    })
    .await
    .expect("writer task panicked");

    drop(rb);
    cleanup_ring_buffer(&path);

    elapsed
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        println!("=== RingBuffer Tap Latency Benchmarks ===");
        println!();

        // Baseline: write throughput without any taps
        {
            let elapsed = bench_write_throughput_baseline().await;
            let fps = FRAME_COUNT as f64 / elapsed.as_secs_f64();
            let throughput_mb =
                (FRAME_COUNT * FRAME_SIZE) as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64();
            println!("---");
            println!("bench: ring_buffer_write_baseline");
            println!("frames: {FRAME_COUNT}");
            println!("frame_size_bytes: {FRAME_SIZE}");
            println!("elapsed_ms: {:.2}", elapsed.as_secs_f64() * 1000.0);
            println!("fps: {fps:.0}");
            println!("throughput_mb_sec: {throughput_mb:.1}");
            println!();
        }

        // Tap latency at various consumer counts
        for &tap_count in TAP_COUNTS {
            let mut latencies = bench_tap_latency(tap_count).await;

            if latencies.is_empty() {
                println!("---");
                println!("bench: ring_buffer_tap_latency_{tap_count}_taps");
                println!("WARNING: no latency samples collected (all frames dropped?)");
                println!();
                continue;
            }

            let stats = compute_stats(&mut latencies);
            print_stats(&format!("ring_buffer_tap_latency_{tap_count}_taps"), &stats);

            // Delivery ratio
            let expected = FRAME_COUNT * tap_count;
            let ratio = stats.count as f64 / expected as f64;
            println!("delivery_ratio: {ratio:.2}");
            println!();

            // Sanity: at least some frames should be delivered
            assert!(
                stats.count > 0,
                "with {tap_count} taps: received zero frames"
            );
        }

        println!("=== Benchmarks complete ===");
    });
}
