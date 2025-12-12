//! Micro-benchmark: encode Measurement -> bytes and write into RingBuffer.
//!
//! Env vars:
//! - RING_BENCH_MESSAGES (default: 100_000)
//! - RING_BENCH_BUFFER_MB (default: 64) backing file size
//! - RING_BENCH_PAYLOAD (default: 0) extra bytes in measurement name
//! - RING_BENCH_PATH (default: /dev/shm/ring_writer_bench.buf)
//!
//! Example:
//! ```bash
//! cargo run -p daq-storage --example ring_writer_bench --release
//! ```

use daq_core::core::{Measurement, PixelBuffer};
use daq_storage::ring_buffer::RingBuffer;
use std::path::Path;
use std::time::Instant;

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn encode_measurement_frame(measurement: &Measurement) -> Vec<u8> {
    // Simplified: serialize measurement via bincode (placeholder for actual frame encoding).
    bincode::serialize(measurement).expect("serialize")
}

fn main() {
    let total: usize = env_or("RING_BENCH_MESSAGES", 100_000);
    let payload: usize = env_or("RING_BENCH_PAYLOAD", 0);
    let buffer_mb: usize = env_or("RING_BENCH_BUFFER_MB", 64);
    let path = env_or("RING_BENCH_PATH", "/dev/shm/ring_writer_bench.buf".to_string());

    let rb = RingBuffer::create(Path::new(&path), buffer_mb).expect("create ring buffer");

    let name = "ring-bench".to_string() + &"x".repeat(payload);
    let meas = Measurement::Image {
        name,
        width: 64,
        height: 64,
        buffer: PixelBuffer::U8(vec![0; 64 * 64]),
        unit: "arb".to_string(),
        metadata: daq_core::core::ImageMetadata::default(),
        timestamp: chrono::Utc::now(),
    };

    let frame = encode_measurement_frame(&meas);

    let start = Instant::now();
    for _ in 0..total {
        rb.write(&frame).expect("write frame");
    }
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let rate = total as f64 / secs;

    println!(
        "Ring write: {} frames in {:.3}s -> {:.0} writes/s (buffer {} MB, payload {} bytes, path {})",
        total, secs, rate, buffer_mb, payload, path
    );
}
