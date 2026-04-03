//! Lock contention benchmark for DeviceRegistry concurrent access patterns.
//!
//! Measures throughput and p99 latency of DashMap-backed `DeviceRegistry` under
//! concurrent read and write pressure from multiple tokio tasks. This establishes
//! a baseline for detecting lock contention regressions.
//!
//! Run with: `cargo bench -p integration-tests --bench lock_contention`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use hardware::registry::{DeviceRegistry, register_mock_factories};

/// Number of iterations per task for read-heavy benchmarks.
const READ_ITERATIONS: usize = 10_000;

/// Number of iterations per task for write benchmarks.
const WRITE_ITERATIONS: usize = 1_000;

/// Number of concurrent reader tasks.
const READER_TASKS: usize = 8;

/// Number of concurrent writer tasks.
const WRITER_TASKS: usize = 4;

/// Device IDs used across benchmarks (registered via mock factories).
const DEVICE_IDS: &[&str] = &["mock_stage", "mock_power_meter", "mock_camera"];

// =============================================================================
// Percentile helper
// =============================================================================

struct LatencyStats {
    count: usize,
    min: Duration,
    max: Duration,
    mean: Duration,
    p50: Duration,
    p95: Duration,
    p99: Duration,
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
        min: latencies[0],
        max: latencies[count - 1],
        mean,
        p50: p(50.0),
        p95: p(95.0),
        p99: p(99.0),
    }
}

fn print_stats(label: &str, stats: &LatencyStats) {
    let throughput = stats.count as f64 / (stats.mean.as_secs_f64() * stats.count as f64);
    println!("---");
    println!("bench: {label}");
    println!("operations: {}", stats.count);
    println!("min_ns: {}", stats.min.as_nanos());
    println!("max_ns: {}", stats.max.as_nanos());
    println!("mean_ns: {}", stats.mean.as_nanos());
    println!("p50_ns: {}", stats.p50.as_nanos());
    println!("p95_ns: {}", stats.p95.as_nanos());
    println!("p99_ns: {}", stats.p99.as_nanos());
    println!("throughput_ops_sec: {throughput:.0}");
}

// =============================================================================
// Registry setup
// =============================================================================

async fn create_populated_registry() -> Arc<DeviceRegistry> {
    let registry = Arc::new(DeviceRegistry::new());
    register_mock_factories(&registry);

    // Register three mock devices via TOML config
    registry
        .register_from_toml(
            "mock_stage",
            "Bench Mock Stage",
            "mock_stage",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock_stage");

    registry
        .register_from_toml(
            "mock_power_meter",
            "Bench Mock Power Meter",
            "mock_power_meter",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock_power_meter");

    registry
        .register_from_toml(
            "mock_camera",
            "Bench Mock Camera",
            "mock_camera",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock_camera");

    registry
}

// =============================================================================
// Benchmark: Read-only contention (get_device_info, get_movable, list_devices)
// =============================================================================

async fn bench_read_only(registry: &Arc<DeviceRegistry>) -> Vec<Duration> {
    let mut handles = Vec::with_capacity(READER_TASKS);
    let barrier = Arc::new(tokio::sync::Barrier::new(READER_TASKS));

    for task_id in 0..READER_TASKS {
        let reg = Arc::clone(registry);
        let bar = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(READ_ITERATIONS);
            bar.wait().await;

            for i in 0..READ_ITERATIONS {
                let device_id = DEVICE_IDS[i % DEVICE_IDS.len()];
                let start = Instant::now();

                // Alternate between different read operations
                match (task_id + i) % 3 {
                    0 => {
                        let _ = reg.get_device_info(device_id);
                    }
                    1 => {
                        let _ = reg.get_movable(device_id);
                    }
                    _ => {
                        let _ = reg.list_devices();
                    }
                }

                latencies.push(start.elapsed());
            }
            latencies
        }));
    }

    let mut all_latencies = Vec::new();
    for handle in handles {
        let mut lats = handle.await.expect("reader task panicked");
        all_latencies.append(&mut lats);
    }
    all_latencies
}

// =============================================================================
// Benchmark: Mixed read/write contention
// =============================================================================

async fn bench_mixed_rw(registry: &Arc<DeviceRegistry>) -> (Vec<Duration>, Vec<Duration>) {
    let reader_barrier = Arc::new(tokio::sync::Barrier::new(READER_TASKS + WRITER_TASKS));
    let mut reader_handles = Vec::with_capacity(READER_TASKS);
    let mut writer_handles = Vec::with_capacity(WRITER_TASKS);

    // Spawn reader tasks
    for task_id in 0..READER_TASKS {
        let reg = Arc::clone(registry);
        let bar = Arc::clone(&reader_barrier);
        reader_handles.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(READ_ITERATIONS);
            bar.wait().await;

            for i in 0..READ_ITERATIONS {
                let device_id = DEVICE_IDS[i % DEVICE_IDS.len()];
                let start = Instant::now();

                match (task_id + i) % 4 {
                    0 => {
                        let _ = reg.get_device_info(device_id);
                    }
                    1 => {
                        let _ = reg.get_movable(device_id);
                    }
                    2 => {
                        let _ = reg.get_readable(device_id);
                    }
                    _ => {
                        let _ = reg.contains(device_id);
                    }
                }

                latencies.push(start.elapsed());
            }
            latencies
        }));
    }

    // Spawn writer tasks that register/unregister ephemeral devices
    for task_id in 0..WRITER_TASKS {
        let reg = Arc::clone(registry);
        let bar = Arc::clone(&reader_barrier);
        writer_handles.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(WRITE_ITERATIONS * 2);
            bar.wait().await;

            for i in 0..WRITE_ITERATIONS {
                let ephemeral_id = format!("ephemeral_{task_id}_{i}");

                // Register
                let start = Instant::now();
                let result = reg
                    .register_from_toml(
                        &ephemeral_id,
                        "Ephemeral Device",
                        "mock_stage",
                        toml::Value::Table(Default::default()),
                    )
                    .await;
                latencies.push(start.elapsed());

                // Unregister if successful
                if result.is_ok() {
                    let start = Instant::now();
                    let _ = reg.unregister(&ephemeral_id).await;
                    latencies.push(start.elapsed());
                }
            }
            latencies
        }));
    }

    let mut read_latencies = Vec::new();
    for handle in reader_handles {
        let mut lats = handle.await.expect("reader task panicked");
        read_latencies.append(&mut lats);
    }

    let mut write_latencies = Vec::new();
    for handle in writer_handles {
        let mut lats = handle.await.expect("writer task panicked");
        write_latencies.append(&mut lats);
    }

    (read_latencies, write_latencies)
}

// =============================================================================
// Benchmark: Capability lookup throughput
// =============================================================================

async fn bench_capability_lookups(registry: &Arc<DeviceRegistry>) -> Vec<Duration> {
    let mut handles = Vec::with_capacity(READER_TASKS);
    let barrier = Arc::new(tokio::sync::Barrier::new(READER_TASKS));

    for _task_id in 0..READER_TASKS {
        let reg = Arc::clone(registry);
        let bar = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(READ_ITERATIONS);
            bar.wait().await;

            for i in 0..READ_ITERATIONS {
                let start = Instant::now();

                // Cycle through capability lookups on different device types
                match i % 5 {
                    0 => {
                        let _ = reg.get_movable("mock_stage");
                    }
                    1 => {
                        let _ = reg.get_readable("mock_power_meter");
                    }
                    2 => {
                        let _ = reg.get_triggerable("mock_camera");
                    }
                    3 => {
                        let _ = reg.get_frame_producer("mock_camera");
                    }
                    _ => {
                        let _ = reg.get_exposure_control("mock_camera");
                    }
                }

                latencies.push(start.elapsed());
            }
            latencies
        }));
    }

    let mut all_latencies = Vec::new();
    for handle in handles {
        let mut lats = handle.await.expect("reader task panicked");
        all_latencies.append(&mut lats);
    }
    all_latencies
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
        let registry = create_populated_registry().await;

        println!("=== DeviceRegistry Lock Contention Benchmarks ===");
        println!();

        // 1. Read-only contention
        {
            let mut latencies = bench_read_only(&registry).await;
            let stats = compute_stats(&mut latencies);
            print_stats("registry_read_only_contention", &stats);
            println!();
        }

        // 2. Mixed read/write contention
        {
            let (mut read_lats, mut write_lats) = bench_mixed_rw(&registry).await;
            let read_stats = compute_stats(&mut read_lats);
            let write_stats = compute_stats(&mut write_lats);
            print_stats("registry_mixed_rw_reads", &read_stats);
            println!();
            print_stats("registry_mixed_rw_writes", &write_stats);
            println!();
        }

        // 3. Capability lookup throughput
        {
            let mut latencies = bench_capability_lookups(&registry).await;
            let stats = compute_stats(&mut latencies);
            print_stats("registry_capability_lookups", &stats);
            println!();
        }

        println!("=== Benchmarks complete ===");
    });
}
