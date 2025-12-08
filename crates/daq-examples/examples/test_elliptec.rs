//! Elliptec Rotator Hardware Test
//!
//! Run with: cargo run --example test_elliptec --features instrument_serial

use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Elliptec Rotator Hardware Test ===\n");

    // Elliptec uses RS-485 bus, likely on one of the FTDI cables
    let ports_to_test = vec![
        "/dev/ttyUSB0",
        "/dev/ttyUSB1",
        "/dev/ttyUSB2",
        "/dev/ttyUSB3",
        "/dev/ttyUSB4",
    ];

    for port_name in &ports_to_test {
        println!("Testing {}...", port_name);

        let mut port = match serialport::new(*port_name, 9600)
            .timeout(Duration::from_millis(500))
            .open()
        {
            Ok(p) => p,
            Err(e) => {
                println!("  ✗ Failed to open: {}\n", e);
                continue;
            }
        };

        println!("  ✓ Port opened");

        // Test addresses 0-3 (Elliptec devices on RS-485 bus)
        for addr in 0..4 {
            // Send "get info" command: <address>in
            let cmd = format!("{}in\r", addr);
            port.write_all(cmd.as_bytes())?;
            std::thread::sleep(Duration::from_millis(200));

            let mut buffer = [0u8; 256];
            match port.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    let response = String::from_utf8_lossy(&buffer[..n]);
                    let trimmed = response.trim();

                    // Elliptec responds with <addr>IN followed by device info
                    if trimmed.starts_with(&format!("{}IN", addr))
                        || trimmed.starts_with(&format!("{}PO", addr))
                    {
                        println!(
                            "  ✓✓✓ Elliptec device FOUND at address {} on {}! ✓✓✓",
                            addr, port_name
                        );
                        println!("    Response: {}", trimmed);

                        // Try to get position
                        let pos_cmd = format!("{}gp\r", addr);
                        port.write_all(pos_cmd.as_bytes())?;
                        std::thread::sleep(Duration::from_millis(200));

                        if let Ok(n) = port.read(&mut buffer) {
                            let response = String::from_utf8_lossy(&buffer[..n]);
                            println!("    Position: {}", response.trim());
                        }
                    }
                }
                _ => {}
            }
        }

        println!();
    }

    println!("Scan complete.");
    Ok(())
}
