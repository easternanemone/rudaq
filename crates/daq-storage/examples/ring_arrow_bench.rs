//! Micro-benchmark: write Arrow RecordBatch to RingBuffer and report throughput.
//!
//! Env vars:
//! - RING_ARROW_MESSAGES (default: 10_000)   : number of RecordBatches
//! - RING_ARROW_ROWS     (default: 1_000)    : rows per batch
//! - RING_ARROW_BUFFER_MB(default: 64)       : RingBuffer backing size
//! - RING_ARROW_PATH     (default: /tmp/ring_arrow_bench.buf)
//!
//! Requires `storage_arrow` feature.

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use daq_storage::ring_buffer::RingBuffer;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn make_batch(rows: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let names = StringArray::from_iter_values((0..rows).map(|i| format!("sig_{}", i % 16)));
    let values = Float64Array::from_iter_values((0..rows).map(|i| i as f64));

    RecordBatch::try_new(schema, vec![Arc::new(names), Arc::new(values)]).unwrap()
}

fn main() {
    let batches: usize = env_or("RING_ARROW_MESSAGES", 10_000);
    let rows: usize = env_or("RING_ARROW_ROWS", 1_000);
    let buffer_mb: usize = env_or("RING_ARROW_BUFFER_MB", 64);
    let path = env_or("RING_ARROW_PATH", "/tmp/ring_arrow_bench.buf".to_string());

    let rb = RingBuffer::create(Path::new(&path), buffer_mb).expect("create ring");
    let batch = make_batch(rows);

    let start = Instant::now();
    for _ in 0..batches {
        rb.write_arrow_batch(&batch).expect("write arrow");
    }
    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let rate = batches as f64 / secs;

    println!(
        "Arrow write: {} batches ({} rows) in {:.3}s -> {:.0} batches/s (buffer {} MB, path {})",
        batches,
        rows,
        secs,
        rate,
        buffer_mb,
        path
    );
}
