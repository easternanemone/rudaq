#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Latency benchmark for gRPC server under async/blocking load (bd-z5s9).
//!
//! This test demonstrates the catastrophic effect of blocking code (std::thread::sleep)
//! in an async runtime compared to proper async code (tokio::time::sleep).
//!
//! TDD: This test is designed to FAIL when blocking patterns exist in the codebase.

// Guard: Requires server+scripting, but NOT storage_hdf5 (different DaqServer constructor)
#![cfg(all(
    feature = "server",
    feature = "scripting",
    not(feature = "storage_hdf5")
))]

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;
use tonic::transport::Server;
use tonic::Request;

use daq_experiment::RunEngine;
use daq_hardware::registry::DeviceRegistry;
use daq_proto::daq::control_service_client::ControlServiceClient;
use daq_proto::daq::control_service_server::ControlServiceServer;
use daq_proto::daq::DaemonInfoRequest;
use daq_server::grpc::server::DaqServer;

/// Timeout for individual gRPC requests to prevent test hangs.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Setup a test server on a random local port and return the address
async fn setup_server() -> std::net::SocketAddr {
    let registry = Arc::new(DeviceRegistry::new());
    let run_engine = Arc::new(RunEngine::new(registry));
    let service = DaqServer::new(run_engine).expect("Failed to create DaqServer");

    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let serve_future = Server::builder()
        .add_service(ControlServiceServer::new(service))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));

    tokio::spawn(serve_future);
    local_addr
}

async fn run_benchmark_case(
    client: &mut ControlServiceClient<tonic::transport::Channel>,
    requests: usize,
) -> Vec<Duration> {
    let mut latencies = Vec::with_capacity(requests);
    let mut errors = 0;

    for _ in 0..requests {
        let start = Instant::now();
        // Light-weight call to measure responsiveness, with timeout
        let result = tokio::time::timeout(
            REQUEST_TIMEOUT,
            client.get_daemon_info(Request::new(DaemonInfoRequest {})),
        )
        .await;

        match result {
            Ok(Ok(_)) => latencies.push(start.elapsed()),
            Ok(Err(e)) => {
                errors += 1;
                eprintln!("gRPC error: {}", e);
            }
            Err(_) => {
                errors += 1;
                eprintln!("Request timeout after {:?}", REQUEST_TIMEOUT);
            }
        }
        // Small delay to prevent tight-loop client-side bottleneck
        tokio::time::sleep(Duration::from_micros(100)).await;
    }

    if errors > requests / 10 {
        panic!("Too many errors: {}/{} requests failed", errors, requests);
    }

    latencies.sort();
    latencies
}

fn calc_percentiles(latencies: &[Duration]) -> (Duration, Duration, Duration) {
    if latencies.is_empty() {
        return (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    }
    let len = latencies.len();
    let p50 = latencies[len * 50 / 100];
    let p90 = latencies[len * 90 / 100];
    let p99 = latencies[len * 99 / 100];
    (p50, p90, p99)
}

/// This test benchmarks the gRPC server latency under different load conditions.
/// It is designed to demonstrate the catastrophic effect of blocking code (std::thread::sleep)
/// in an async runtime compared to proper async code (tokio::time::sleep).
///
/// It will FAIL initially if the blocking code issues are present/simulated,
/// or properly asserted to show degradation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn benchmark_grpc_latency_under_load() {
    let addr = setup_server().await;
    let mut client = ControlServiceClient::connect(format!("http://{}", addr))
        .await
        .expect("Failed to connect client");

    let num_requests = 200;

    // === Warmup ===
    println!("Warming up...");
    let _ = run_benchmark_case(&mut client, 50).await;

    // === Case A: Baseline ===
    // No extra load, just the requests
    println!("Starting Case A: Baseline...");
    let latencies_a = run_benchmark_case(&mut client, num_requests).await;
    let (p50_a, p90_a, p99_a) = calc_percentiles(&latencies_a);
    println!(
        "Case A (Baseline): P50={:?}, P90={:?}, P99={:?}",
        p50_a, p90_a, p99_a
    );

    // === Case B: Degraded (Blocking Load) ===
    // Simulate background tasks incorrectly using blocking I/O or sleep.
    // We launch tasks equal to worker_threads to saturate the runtime.
    println!("Starting Case B: Blocking Load...");

    let barrier = Arc::new(Barrier::new(5)); // 4 workers + main thread

    // Spawn tasks that periodically block
    let keep_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let keep_running_clone = keep_running.clone();

    // Spawn 4 tasks (equal to worker count) to maximize chance of starvation
    for _ in 0..4 {
        let kr = keep_running_clone.clone();
        let b = barrier.clone();
        tokio::spawn(async move {
            b.wait().await;
            while kr.load(std::sync::atomic::Ordering::Relaxed) {
                // THE BAD PATTERN: Blocking sleep in async task
                // 50ms block means we are unresponsive for 50ms at a time per thread
                std::thread::sleep(Duration::from_millis(50));
                tokio::time::sleep(Duration::from_millis(1)).await; // Yield briefly
            }
        });
    }

    // Wait for all blocking tasks to be ready
    barrier.wait().await;

    // Give tasks a moment to start clogging the runtime
    tokio::time::sleep(Duration::from_millis(100)).await;

    let latencies_b = run_benchmark_case(&mut client, num_requests).await;
    let (p50_b, p90_b, p99_b) = calc_percentiles(&latencies_b);
    println!(
        "Case B (Blocking): P50={:?}, P90={:?}, P99={:?}",
        p50_b, p90_b, p99_b
    );

    // Stop blocking tasks
    keep_running.store(false, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(100)).await; // Cooldown

    // === Case C: Target (Async Load) ===
    // Simulate heavy background activity using proper async await
    println!("Starting Case C: Async Load...");

    let keep_running_c = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let keep_running_c_clone = keep_running_c.clone();

    for _ in 0..20 {
        // More tasks, well-behaved
        let kr = keep_running_c_clone.clone();
        tokio::spawn(async move {
            while kr.load(std::sync::atomic::Ordering::Relaxed) {
                // THE GOOD PATTERN: Async sleep yields the thread
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    // Give tasks a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    let latencies_c = run_benchmark_case(&mut client, num_requests).await;
    let (p50_c, p90_c, p99_c) = calc_percentiles(&latencies_c);
    println!(
        "Case C (Async):    P50={:?}, P90={:?}, P99={:?}",
        p50_c, p90_c, p99_c
    );

    keep_running_c.store(false, std::sync::atomic::Ordering::Relaxed);

    // === Assertions ===
    // Use both ratio AND absolute threshold to reduce flakiness

    // Minimum absolute degradation expected (20ms) when blocking
    let min_degradation = Duration::from_millis(20);

    // Check if Case B is slower than Baseline
    // With 50ms blocks, we expect severe degradation
    let degradation_vs_baseline = p99_b.saturating_sub(p99_a);
    assert!(
        p99_b > p99_a * 2 || degradation_vs_baseline > min_degradation,
        "Case B (Blocking) P99 {:?} should be worse than Baseline P99 {:?} (degradation: {:?})",
        p99_b,
        p99_a,
        degradation_vs_baseline
    );

    // Check if Case B is slower than Case C
    // This is the key assertion: blocking code causes worse latency than async
    let degradation_vs_async = p99_b.saturating_sub(p99_c);
    assert!(
        p99_b > p99_c * 2 || degradation_vs_async > min_degradation,
        "Case B (Blocking) P99 {:?} should be worse than Case C (Async) P99 {:?} (degradation: {:?})",
        p99_b, p99_c, degradation_vs_async
    );

    println!("\n=== Summary ===");
    println!(
        "Blocking degradation vs Baseline: {:?}",
        degradation_vs_baseline
    );
    println!(
        "Blocking degradation vs Async:    {:?}",
        degradation_vs_async
    );
}
