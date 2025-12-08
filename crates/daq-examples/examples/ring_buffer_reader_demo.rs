//! Demonstration of RingBufferReader for decoding frames from ring buffer taps.
//!
//! This example shows how to:
//! 1. Create a ring buffer
//! 2. Register a tap to receive frames
//! 3. Use RingBufferReader to decode and process frames
//! 4. Track statistics about frame reception
//!
//! Run with: cargo run --example ring_buffer_reader_demo

use anyhow::Result;
use rust_daq::data::{RingBuffer, RingBufferReader};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

/// Example measurement frame sent through the ring buffer
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Measurement {
    /// Timestamp in seconds
    timestamp: f64,
    /// Position in mm
    position: f64,
    /// Power in watts
    power: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("RingBufferReader Demo");
    println!("=====================\n");

    // Create a temporary ring buffer
    let temp_dir = tempfile::tempdir()?;
    let rb_path = temp_dir.path().join("demo_ring_buffer.buf");

    println!("Creating ring buffer at: {:?}", rb_path);
    let rb = RingBuffer::create(&rb_path, 10)?; // 10 MB buffer

    // Register a tap to receive every 3rd frame
    println!("Registering tap 'client_1' to receive every 3rd frame");
    let rx = rb.register_tap("client_1".to_string(), 3)?;

    // Create reader from tap receiver
    let mut reader = RingBufferReader::new(rx);

    println!("Reader initialized: {:?}\n", reader);

    // Spawn a writer task that generates data
    let rb_writer = rb;
    let writer_handle = tokio::spawn(async move {
        println!("Writer: Starting to generate frames...");

        for i in 0..30 {
            let measurement = Measurement {
                timestamp: i as f64 * 0.1,
                position: i as f64 * 0.5,
                power: (i as f64 * 0.1).sin() * 10.0 + 50.0,
            };

            // Serialize to JSON
            let json_data = serde_json::to_vec(&measurement).unwrap();

            // Write to ring buffer
            rb_writer.write(&json_data).unwrap();

            println!("Writer: Sent frame {} (t={:.1}s)", i, measurement.timestamp);

            // Simulate data rate (10 frames/sec)
            sleep(Duration::from_millis(100)).await;
        }

        println!("Writer: Finished sending 30 frames");
    });

    // Read frames in main task
    println!("\nReader: Starting to receive frames...\n");

    let mut frame_count = 0;
    while let Some(measurement) = reader.read_typed::<Measurement>().await? {
        frame_count += 1;
        println!(
            "Reader: Frame {} - t={:.1}s, pos={:.1}mm, power={:.1}W",
            frame_count, measurement.timestamp, measurement.position, measurement.power
        );

        // Check if we've received all expected frames
        // We registered for every 3rd frame, so expect 10 frames (0, 3, 6, ..., 27)
        if frame_count >= 10 {
            println!("\nReader: Received all expected frames");
            break;
        }
    }

    // Wait for writer to finish
    writer_handle.await?;

    // Display final statistics
    let stats = reader.stats();
    println!("\n=== Final Statistics ===");
    println!("Frames received:  {}", stats.frames_received);
    println!("Frames dropped:   {}", stats.frames_dropped);
    println!("Loss rate:        {:.2}%", stats.loss_rate());
    println!("Queued frames:    {}", reader.queued_frames());

    println!("\nDemo completed successfully!");

    Ok(())
}
