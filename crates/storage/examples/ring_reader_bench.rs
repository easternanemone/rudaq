//! Micro-benchmark: tap RingBuffer and measure read throughput/latency.
//!
//! Env vars:
//! - RING_READ_MESSAGES (default: 100_000)
//! - RING_READ_BUFFER_MB (default: 64)
//! - RING_READ_PATH (default: /tmp/ring_reader_bench.buf)
//!
//! Writes synthetic frames into the ring, registers a tap, and drains it.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use storage::ring_buffer::RingBuffer;

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let total: usize = env_or("RING_READ_MESSAGES", 100_000);
    let buffer_mb: usize = env_or("RING_READ_BUFFER_MB", 64);
    let path = env_or("RING_READ_PATH", "/tmp/ring_reader_bench.buf".to_string());

    let rb = Arc::new(RingBuffer::create(Path::new(&path), buffer_mb).expect("create ring"));
    let mut tap = rb.register_tap("bench".to_string(), 1).expect("tap");

    let frame = vec![0u8; 1024]; // 1 KB frame

    // Writer
    {
        let rb = rb.clone();
        for _ in 0..total {
            rb.write(&frame).expect("write");
        }
    }

    // Reader
    let start = Instant::now();
    let mut received = 0usize;
    while let Ok(_bytes) = tap.try_recv() {
        received += 1;
        if received >= total {
            break;
        }
    }
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: test/benchmark values are bounded
    let rate = received as f64 / secs;

    println!(
        "Ring read: {received} frames in {secs:.3}s -> {rate:.0} reads/s (buffer {buffer_mb} MB, path {path})"
    );
}
