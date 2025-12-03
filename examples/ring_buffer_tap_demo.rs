//! Demonstration of ring buffer tap mechanism for live data visualization.
//!
//! This example shows how to use the ring buffer tap feature to stream
//! data to remote clients without blocking the main HDF5 writer.
//!
//! Run with:
//! ```bash
//! cargo run --example ring_buffer_tap_demo
//! ```

use anyhow::Result;
use rust_daq::data::ring_buffer::RingBuffer;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    println!("Ring Buffer Tap Demonstration");
    println!("==============================\n");

    // Create ring buffer in /tmp
    let rb_path = Path::new("/tmp/demo_ring_buffer.buf");
    let rb = Arc::new(RingBuffer::create(rb_path, 10)?);
    println!("Created ring buffer at {:?}", rb_path);
    println!("Capacity: {} MB\n", rb.capacity() / (1024 * 1024));

    // Register multiple taps with different frame rates
    let mut rx_preview = rb.register_tap("preview".to_string(), 10)?;
    let mut rx_thumbnail = rb.register_tap("thumbnail".to_string(), 50)?;
    let mut rx_full = rb.register_tap("full_rate".to_string(), 1)?;

    println!("Registered 3 taps:");
    println!("  - 'preview': every 10th frame");
    println!("  - 'thumbnail': every 50th frame");
    println!("  - 'full_rate': every frame\n");

    // Spawn tasks to consume from each tap
    let preview_task = tokio::spawn(async move {
        let mut count = 0;
        while let Some(frame) = rx_preview.recv().await {
            count += 1;
            println!("  [PREVIEW] Received frame #{} ({} bytes)", count, frame.len());
            if count >= 5 {
                break;
            }
        }
        count
    });

    let thumbnail_task = tokio::spawn(async move {
        let mut count = 0;
        while let Some(frame) = rx_thumbnail.recv().await {
            count += 1;
            println!("  [THUMBNAIL] Received frame #{} ({} bytes)", count, frame.len());
            if count >= 2 {
                break;
            }
        }
        count
    });

    let full_rate_task = tokio::spawn(async move {
        let mut count = 0;
        let mut dropped = 0;

        // Simulate slow consumer by only receiving some frames
        loop {
            match tokio::time::timeout(Duration::from_millis(1), rx_full.recv()).await {
                Ok(Some(_)) => count += 1,
                Ok(None) => break,
                Err(_) => {
                    // Timeout - channel might be empty or we're too slow
                    dropped += 1;
                }
            }

            if count + dropped >= 100 {
                break;
            }
        }
        (count, dropped)
    });

    // Simulate data acquisition - write frames continuously
    println!("Simulating data acquisition (writing 100 frames)...\n");

    let rb_writer = Arc::clone(&rb);
    let writer_task = tokio::spawn(async move {
        for i in 0..100 {
            let frame_data = format!("Frame {:04} - simulated camera data", i);
            rb_writer.write(frame_data.as_bytes()).unwrap();

            // Simulate camera acquisition rate (100 Hz = 10ms per frame)
            sleep(Duration::from_millis(10)).await;
        }
    });

    // Wait for writer to complete
    writer_task.await?;
    println!("\nData acquisition complete!\n");

    // Give taps time to process remaining frames
    sleep(Duration::from_millis(100)).await;

    // Wait for all consumer tasks and print results
    let preview_count = preview_task.await?;
    let thumbnail_count = thumbnail_task.await?;
    let (full_count, full_dropped) = full_rate_task.await?;

    println!("\nResults:");
    println!("========");
    println!("Total frames written: 100");
    println!("Preview tap received: {} frames (expected ~10)", preview_count);
    println!("Thumbnail tap received: {} frames (expected ~2)", thumbnail_count);
    println!(
        "Full-rate tap: {} received, {} dropped (backpressure handling)",
        full_count, full_dropped
    );

    // Demonstrate tap management
    println!("\nTap Management:");
    println!("===============");
    println!("Active taps: {}", rb.tap_count());
    for (id, nth) in rb.list_taps() {
        println!("  - '{}' (every {}th frame)", id, nth);
    }

    // Unregister taps
    rb.unregister_tap("preview")?;
    rb.unregister_tap("thumbnail")?;
    rb.unregister_tap("full_rate")?;
    println!("\nAll taps unregistered. Active taps: {}", rb.tap_count());

    println!("\nDemonstration complete!");
    println!("Ring buffer file remains at: {:?}", rb_path);

    Ok(())
}
