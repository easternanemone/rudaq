//! Digital I/O example.
//!
//! Demonstrates digital input/output operations including pin
//! configuration, individual pin read/write, and port operations.
//!
//! # Usage
//!
//! ```bash
//! cargo build -p daq-driver-comedi --features hardware --example digital_io
//! ./target/debug/examples/digital_io
//! ```

use daq_driver_comedi::{ComediDevice, DioDirection};
use std::env;
use std::thread;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let device_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/comedi0".to_string());

    println!("=== Comedi Digital I/O Example ===\n");
    println!("Device: {}", device_path);

    let device = ComediDevice::open(&device_path)?;
    println!("Board: {}", device.board_name());

    // Get digital I/O subsystem
    let dio = device.digital_io()?;

    println!("\nDigital I/O Subsystem:");
    println!("  Channels: {}", dio.n_channels());

    // Read current state of all pins
    println!("\nCurrent pin states:");
    let states = dio.read_all()?;
    for (i, state) in states.iter().enumerate() {
        println!("  DIO{}: {}", i, if *state { "HIGH" } else { "LOW" });
    }

    // Configure pins: 0-3 as outputs, 4-7 as inputs
    println!("\nConfiguring pins 0-3 as outputs, 4-7 as inputs...");
    for pin in 0..4.min(dio.n_channels()) {
        dio.configure(pin, DioDirection::Output)?;
    }
    for pin in 4..8.min(dio.n_channels()) {
        dio.configure(pin, DioDirection::Input)?;
    }

    // Blink pattern on output pins
    println!("\nBlinking output pins (3 cycles)...");
    for cycle in 0..3 {
        // Set outputs high
        for pin in 0..4.min(dio.n_channels()) {
            dio.set_high(pin)?;
        }
        println!("  Cycle {}: HIGH", cycle + 1);
        thread::sleep(Duration::from_millis(200));

        // Set outputs low
        for pin in 0..4.min(dio.n_channels()) {
            dio.set_low(pin)?;
        }
        println!("  Cycle {}: LOW", cycle + 1);
        thread::sleep(Duration::from_millis(200));
    }

    // Read port as bitmask
    let port_value = dio.read_port(0)?;
    println!("\nPort value: 0x{:02X} (binary: {:08b})", port_value, port_value);

    // Write pattern to outputs
    println!("\nWriting alternating pattern (0xAA) to output pins...");
    dio.write_port(0, 0x0F, 0x0A)?; // Mask 0x0F, value 0x0A
    thread::sleep(Duration::from_millis(100));

    let new_port = dio.read_port(0)?;
    println!("Port value after write: 0x{:02X}", new_port);

    // Reset to all low
    println!("\nResetting outputs to LOW...");
    for pin in 0..4.min(dio.n_channels()) {
        dio.set_low(pin)?;
    }

    println!("\nDone!");
    Ok(())
}
