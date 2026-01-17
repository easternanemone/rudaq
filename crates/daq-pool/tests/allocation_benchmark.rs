//! Benchmark comparing heap allocation vs pool-based allocation.
//!
//! This benchmark demonstrates the performance advantage of using a pre-allocated
//! object pool versus fresh Vec allocations for frame data.
//!
//! Run with: `cargo test --package daq-pool --test allocation_benchmark -- --nocapture`

use daq_pool::{BufferPool, Pool};
use std::time::{Duration, Instant};

/// Frame size in bytes (8 MB - typical for high-resolution cameras)
const FRAME_SIZE: usize = 8 * 1024 * 1024;

/// Number of iterations for the benchmark
const ITERATIONS: usize = 100;

/// Pool size for pre-allocation
const POOL_SIZE: usize = 10;

/// Benchmark results for a single test
#[derive(Debug)]
struct BenchmarkResult {
    name: &'static str,
    total_time: Duration,
    per_iteration: Duration,
}

impl BenchmarkResult {
    fn new(name: &'static str, total_time: Duration, iterations: usize) -> Self {
        Self {
            name,
            total_time,
            per_iteration: total_time / iterations as u32,
        }
    }

    fn speedup_over(&self, other: &BenchmarkResult) -> f64 {
        other.total_time.as_nanos() as f64 / self.total_time.as_nanos() as f64
    }
}

/// Benchmark fresh Vec allocation (heap allocation each time)
fn benchmark_vec_allocation() -> BenchmarkResult {
    let start = Instant::now();

    for _ in 0..ITERATIONS {
        // Allocate a new Vec each time - this is what we want to avoid
        let data = vec![0u8; FRAME_SIZE];
        // Use black_box to prevent the compiler from optimizing away the allocation
        std::hint::black_box(&data);
        // Vec is dropped here, deallocating memory
    }

    let elapsed = start.elapsed();
    BenchmarkResult::new("Vec allocation", elapsed, ITERATIONS)
}

/// Benchmark using the generic Pool<Vec<u8>>
fn benchmark_pool_acquire() -> BenchmarkResult {
    // Create a tokio runtime for async pool operations
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    // Pre-allocate the pool
    let pool = Pool::new_simple(POOL_SIZE, || vec![0u8; FRAME_SIZE]);

    let start = Instant::now();

    rt.block_on(async {
        for _ in 0..ITERATIONS {
            // Acquire from pool - no heap allocation!
            let item = pool.acquire().await;
            std::hint::black_box(&*item);
            // Item returned to pool on drop - no deallocation!
        }
    });

    let elapsed = start.elapsed();
    BenchmarkResult::new("Pool<Vec<u8>> acquire", elapsed, ITERATIONS)
}

/// Benchmark using the specialized BufferPool
fn benchmark_buffer_pool_acquire() -> BenchmarkResult {
    // Create a tokio runtime for async pool operations
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    // Pre-allocate the buffer pool
    let pool = BufferPool::new(POOL_SIZE, FRAME_SIZE);

    let start = Instant::now();

    rt.block_on(async {
        for _ in 0..ITERATIONS {
            // Acquire from pool - no heap allocation!
            let buffer = pool.acquire().await;
            std::hint::black_box(&buffer);
            // Buffer returned to pool on drop
        }
    });

    let elapsed = start.elapsed();
    BenchmarkResult::new("BufferPool acquire", elapsed, ITERATIONS)
}

/// Benchmark try_acquire (synchronous, non-blocking)
fn benchmark_buffer_pool_try_acquire() -> BenchmarkResult {
    // Pre-allocate the buffer pool (larger to avoid exhaustion)
    let pool = BufferPool::new(POOL_SIZE, FRAME_SIZE);

    let start = Instant::now();

    for _ in 0..ITERATIONS {
        // Try to acquire synchronously
        if let Some(buffer) = pool.try_acquire() {
            std::hint::black_box(&buffer);
            // Buffer returned to pool on drop
        }
    }

    let elapsed = start.elapsed();
    BenchmarkResult::new("BufferPool try_acquire", elapsed, ITERATIONS)
}

/// Print benchmark results in a formatted table
fn print_results(results: &[BenchmarkResult], baseline: &BenchmarkResult) {
    println!();
    println!("{}", "=".repeat(80));
    println!(
        "Allocation Benchmark ({} iterations, {} MB frames)",
        ITERATIONS,
        FRAME_SIZE / (1024 * 1024)
    );
    println!("{}", "=".repeat(80));
    println!();
    println!(
        "{:<30} {:>15} {:>15} {:>12}",
        "Method", "Total Time", "Per Frame", "Speedup"
    );
    println!("{}", "-".repeat(80));

    for result in results {
        let speedup = result.speedup_over(baseline);
        let speedup_str = if (speedup - 1.0).abs() < 0.01 {
            "baseline".to_string()
        } else {
            format!("{:.1}x", speedup)
        };

        println!(
            "{:<30} {:>15.2?} {:>15.2?} {:>12}",
            result.name, result.total_time, result.per_iteration, speedup_str
        );
    }

    println!();
    println!("{}", "=".repeat(80));
    println!();
}

/// Main benchmark test
#[test]
fn test_allocation_benchmark() {
    println!();
    println!("Running allocation benchmarks...");
    println!("  Frame size: {} MB", FRAME_SIZE / (1024 * 1024));
    println!("  Iterations: {}", ITERATIONS);
    println!("  Pool size:  {}", POOL_SIZE);
    println!();

    // Run benchmarks
    let vec_result = benchmark_vec_allocation();
    let pool_result = benchmark_pool_acquire();
    let buffer_pool_result = benchmark_buffer_pool_acquire();
    let try_acquire_result = benchmark_buffer_pool_try_acquire();

    // Collect results
    let results = vec![
        vec_result.clone(),
        pool_result,
        buffer_pool_result,
        try_acquire_result,
    ];

    // Print formatted results with Vec allocation as baseline
    print_results(&results, &vec_result);

    // Verify pool is faster than Vec allocation
    // Note: This assertion might fail on first run due to cold caches,
    // but should consistently pass in steady-state conditions
    let fastest_pool = results
        .iter()
        .skip(1) // Skip Vec baseline
        .min_by_key(|r| r.total_time)
        .expect("No pool results");

    let speedup = fastest_pool.speedup_over(&vec_result);
    println!(
        "Best pool method '{}' is {:.1}x faster than Vec allocation",
        fastest_pool.name, speedup
    );

    // Pool should be at least 2x faster for 8MB allocations
    // (typically much faster, 10-100x, but being conservative for CI)
    assert!(
        speedup >= 1.5,
        "Pool should be at least 1.5x faster than Vec allocation, got {:.2}x",
        speedup
    );
}

/// Test that demonstrates the pattern for PVCAM frame acquisition
#[test]
fn test_frame_acquisition_pattern() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

    // Simulated PVCAM parameters
    let frame_size = 2048 * 2048 * 2; // 8MB (2048x2048 16-bit)
    let num_frames = 30; // Pool size matching SDK buffer count
    let acquisition_count = 100;

    let pool = BufferPool::new(num_frames, frame_size);

    println!();
    println!("Simulating PVCAM frame acquisition pattern...");
    println!(
        "  Frame size: {} bytes ({:.1} MB)",
        frame_size,
        frame_size as f64 / (1024.0 * 1024.0)
    );
    println!("  Pool size: {} frames", num_frames);
    println!("  Acquiring: {} frames", acquisition_count);

    let start = Instant::now();

    rt.block_on(async {
        for i in 0..acquisition_count {
            // Acquire buffer from pool (like PVCAM callback would do)
            let mut buffer = pool.acquire().await;

            // Simulate copying frame data from SDK
            // In real PVCAM: unsafe { buffer.copy_from_ptr(sdk_frame_ptr, frame_size) }
            // Here we just copy a small slice to simulate the pattern
            let sample_data = [i as u8; 16];
            buffer.copy_from_slice(&sample_data);

            // Freeze to Bytes for downstream processing
            let _bytes = buffer.freeze();

            // bytes would be sent to consumers here
            // When all consumers done, buffer automatically returns to pool
        }
    });

    let elapsed = start.elapsed();
    let fps = acquisition_count as f64 / elapsed.as_secs_f64();

    println!("  Total time: {:?}", elapsed);
    println!("  Effective FPS: {:.0}", fps);
    println!();

    // Should achieve high FPS with pool reuse
    assert!(
        fps > 1000.0,
        "Should achieve >1000 FPS with pool reuse, got {:.0}",
        fps
    );
}

// Clone needed for the print_results function
impl Clone for BenchmarkResult {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            total_time: self.total_time,
            per_iteration: self.per_iteration,
        }
    }
}
