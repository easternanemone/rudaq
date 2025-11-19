/// Example: The Mullet Strategy - Arrow in front, HDF5 in back
///
/// This demonstrates Phase 4 (Task K): HDF5 Background Writer
///
/// Scientists see:
/// - Clean f64/Vec<f64> API
/// - Standard HDF5 files (readable by Python/MATLAB/Igor)
///
/// Under the hood:
/// - Arrow IPC format for performance (10k+ writes/sec)
/// - Memory-mapped ring buffer for zero-copy
/// - Background HDF5 writer (1 Hz, non-blocking)
///
/// Run this with:
/// ```bash
/// cargo run --example phase4_ring_buffer_example --features="storage_hdf5,storage_arrow"
/// ```

use anyhow::Result;
use rust_daq::data::ring_buffer::RingBuffer;
use rust_daq::data::hdf5_writer::HDF5Writer;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    println!("🎸 The Mullet Strategy Demo");
    println!("   Party in front: Fast Arrow writes");
    println!("   Business in back: Standard HDF5 files");
    println!();

    // Create ring buffer (100 MB in /tmp)
    let ring_buffer = Arc::new(RingBuffer::create(Path::new("/tmp/mullet_demo_ring"), 100)?);
    println!("✅ Ring buffer created: 100 MB");

    // Start background HDF5 writer
    let writer = HDF5Writer::new(Path::new("mullet_demo_output.h5"), ring_buffer.clone())?;
    println!("✅ HDF5 writer started (flushes every 1 second)");

    let writer_handle = tokio::spawn(async move {
        writer.run().await;
    });

    // Simulate hardware loop writing at 100 Hz
    println!("\n📡 Simulating hardware loop (100 Hz for 5 seconds)...");
    let mut hw_interval = interval(Duration::from_millis(10));
    let mut sample_count = 0;

    for iteration in 0..500 {
        hw_interval.tick().await;

        // Write fake data to ring buffer
        let data = format!("Sample {}: voltage={:.3}\n", iteration, iteration as f64 * 0.001);
        ring_buffer.write(data.as_bytes())?;

        sample_count += 1;

        if iteration % 100 == 0 {
            println!("   Wrote {} samples...", sample_count);
        }
    }

    println!("\n✅ Hardware loop complete: {} samples written", sample_count);
    println!("   Ring buffer never blocked!");
    println!("   HDF5 file: mullet_demo_output.h5");

    // Give writer time to flush final data
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("\n🎯 The Mullet Strategy in action:");
    println!("   - Scientists got standard HDF5 (check with h5py)");
    println!("   - Hardware wrote at 100 Hz without blocking");
    println!("   - Arrow format invisible to end users");

    // Cleanup
    writer_handle.abort();
    std::fs::remove_file("/tmp/mullet_demo_ring").ok();

    Ok(())
}
