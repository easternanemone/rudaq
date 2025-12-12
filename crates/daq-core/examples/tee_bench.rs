//! Benchmark the Tee pipeline (reliable mpsc + lossy broadcast).
//! 
//! Runs a synthetic source sending `Measurement::Scalar` into a Tee that
//! forwards to both reliable (mpsc) and lossy (broadcast) consumers.
//! 
//! Environment variables:
//! - `TEE_BENCH_MESSAGES` (default: 100_000)
//! - `TEE_BENCH_PAYLOAD`  (default: 0) additional bytes in the scalar name to
//!   simulate metadata size.
//! - `TEE_BENCH_BUFFER`   (default: 1024) size of reliable mpsc channel
//! - `TEE_BENCH_LATENCY`  (default: 1) set to 0 to skip latency stats
//! 
//! Example:
//! ```bash
//! cargo run -p daq-core --example tee_bench
//! TEE_BENCH_MESSAGES=500000 cargo run -p daq-core --example tee_bench --release
//! ```

use daq_core::core::Measurement;
use daq_core::pipeline::{MeasurementSink, Tee};
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, oneshot};

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let total: usize = env_or("TEE_BENCH_MESSAGES", 100_000);
    let payload: usize = env_or("TEE_BENCH_PAYLOAD", 0);
    let collect_latency: bool = env_or("TEE_BENCH_LATENCY", 1) != 0;
    let buffer: usize = env_or("TEE_BENCH_BUFFER", 1024);

    let (lossy_tx, mut lossy_rx) = broadcast::channel::<Measurement>(buffer);
    let (rel_tx, mut rel_rx) = mpsc::channel::<Measurement>(buffer);
    let (src_tx, src_rx) = mpsc::channel::<Measurement>(buffer);

    // Drain lossy path
    tokio::spawn(async move {
        let mut count = 0usize;
        while let Ok(_m) = lossy_rx.recv().await {
            count += 1;
        }
        println!("Lossy received: {}", count);
    });

    // Drain reliable path
    let (lat_tx, lat_rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut count = 0usize;
        let mut lats: Vec<u128> = Vec::with_capacity(total);
        while let Some(m) = rel_rx.recv().await {
            if collect_latency {
                let end = chrono::Utc::now();
                let start = m.timestamp();
                if let Ok(dt) = (end - start).to_std() {
                    lats.push(dt.as_nanos());
                }
            }
            count += 1;
        }
        let _ = lat_tx.send((count, lats));
    });

    // Tee wiring
    let mut tee = Tee::new(lossy_tx);
    tee.connect_reliable(rel_tx);
    let tee_handle = tee.register_input(src_rx).expect("tee register");

    // Produce measurements
    let name = "bench".to_string() + &"x".repeat(payload);
    let start = Instant::now();
    for _ in 0..total {
        let meas = Measurement::Scalar {
            name: name.clone(),
            value: 1.0,
            unit: "arb".into(),
            timestamp: chrono::Utc::now(),
        };
        if src_tx.send(meas).await.is_err() {
            break;
        }
    }
    drop(src_tx); // close channel to stop tee

    tee_handle.await.unwrap();
    drop(tee); // Drop senders to close receivers
    let (reliable_count, mut lats) = lat_rx.await.unwrap_or((0, Vec::new()));
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let rate = total as f64 / secs;

    println!(
        "Sent {} messages in {:.3}s -> {:.0} msgs/s (payload {} bytes)",
        total, secs, rate, payload
    );
    println!("Reliable received: {}", reliable_count);

    if collect_latency && !lats.is_empty() {
        lats.sort_unstable();
        let pct = |p: f64| {
            let idx = ((p / 100.0) * ((lats.len() - 1) as f64)).round() as usize;
            lats[idx]
        };
        let to_us = |ns: u128| ns as f64 / 1_000.0;
        println!(
            "Latency p50/p90/p99/max: {:.2}/{:.2}/{:.2}/{:.2} µs",
            to_us(pct(50.0)),
            to_us(pct(90.0)),
            to_us(pct(99.0)),
            to_us(*lats.last().unwrap())
        );
    }
}
