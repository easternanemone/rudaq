//! Hardware-timed streaming acquisition example.
//!
//! Demonstrates high-speed multi-channel data acquisition using
//! the Comedi command interface for hardware-timed sampling.
//!
//! # Usage
//!
//! ```bash
//! cargo build -p daq-driver-comedi --features hardware --example streaming
//! ./target/debug/examples/streaming
//! ```

use daq_driver_comedi::{ComediDevice, StreamAcquisition, StreamConfig};
use std::env;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let device_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/comedi0".to_string());

    println!("=== Comedi Streaming Acquisition Example ===\n");
    println!("Device: {}", device_path);

    let device = ComediDevice::open(&device_path)?;
    println!("Board: {}", device.board_name());

    // Configure streaming: 4 channels at 10 kS/s per channel
    let channels = vec![0, 1, 2, 3];
    let sample_rate = 10000.0; // Per channel

    println!("\nConfiguration:");
    println!("  Channels: {:?}", channels);
    println!("  Sample rate: {} S/s per channel", sample_rate);
    println!("  Aggregate rate: {} S/s", sample_rate * channels.len() as f64);

    let config = StreamConfig::builder()
        .channels(&channels)
        .sample_rate(sample_rate)
        .buffer_size(8192)
        .build()?;

    let stream = StreamAcquisition::new(&device, config)?;

    // Start acquisition
    println!("\nStarting acquisition...");
    stream.start()?;

    let mut total_samples = 0u64;
    let mut total_scans = 0u64;
    let duration = Duration::from_secs(2);
    let start = Instant::now();

    // Collect data for specified duration
    while start.elapsed() < duration {
        if let Some(samples) = stream.read_available()? {
            let n_samples = samples.len();
            let n_scans = n_samples / channels.len();

            total_samples += n_samples as u64;
            total_scans += n_scans as u64;

            // Print first few values periodically
            if total_scans % 5000 == 0 && !samples.is_empty() {
                print!("\r  Scans: {:8} | Latest:", total_scans);
                for (i, ch) in channels.iter().enumerate() {
                    if let Some(&v) = samples.get(i) {
                        print!(" CH{}={:+.3}V", ch, v);
                    }
                }
                print!("        ");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Stop acquisition
    stream.stop()?;

    let elapsed = start.elapsed();
    let stats = stream.stats();

    println!("\n\nResults:");
    println!("  Duration: {:?}", elapsed);
    println!("  Total samples: {}", total_samples);
    println!("  Total scans: {}", total_scans);
    println!("  Effective rate: {:.1} S/s", total_samples as f64 / elapsed.as_secs_f64());
    println!("  Overflows: {}", stats.overflows);

    Ok(())
}
