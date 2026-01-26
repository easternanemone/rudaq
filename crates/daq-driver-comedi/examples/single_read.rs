//! Single-sample analog input example.
//!
//! Demonstrates basic analog input reading from a Comedi device.
//!
//! # Usage
//!
//! ```bash
//! # Build with hardware feature
//! cargo build -p daq-driver-comedi --features hardware --example single_read
//!
//! # Run (requires /dev/comedi0)
//! ./target/debug/examples/single_read
//! ```

use daq_driver_comedi::{ComediDevice, Range};
use std::env;

fn main() -> anyhow::Result<()> {
    // Get device path from args or use default
    let device_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/comedi0".to_string());

    println!("Opening device: {}", device_path);

    // Open the Comedi device
    let device = ComediDevice::open(&device_path)?;

    // Print device info
    println!("\nDevice Information:");
    println!("  Board:  {}", device.board_name());
    println!("  Driver: {}", device.driver_name());
    println!("  Subdevices: {}", device.n_subdevices());

    // Get analog input subsystem
    let ai = device.analog_input()?;

    println!("\nAnalog Input Subsystem:");
    println!("  Channels: {}", ai.n_channels());
    println!("  Resolution: {} bits", ai.resolution_bits());

    // Get default voltage range
    let range = ai.range_info(0, 0)?;
    println!("  Range 0: {} to {} V", range.min, range.max);

    // Read from first 4 channels
    println!("\nReading voltages:");
    for ch in 0..4.min(ai.n_channels()) {
        let voltage = ai.read_voltage(ch, range)?;
        println!("  CH{}: {:+.6} V", ch, voltage);
    }

    // Also show raw ADC values
    println!("\nRaw ADC values:");
    for ch in 0..4.min(ai.n_channels()) {
        let raw = ai.read_raw(ch, range.index, daq_driver_comedi::subsystem::AnalogReference::Ground)?;
        println!("  CH{}: {} (0x{:04X})", ch, raw, raw);
    }

    Ok(())
}
