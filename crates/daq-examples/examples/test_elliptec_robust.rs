//! Elliptec Rotator Hardware Test - Robust Version
//!
//! Run with: cargo run --example test_elliptec_robust --features instrument_serial

use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Elliptec Rotator Hardware Test (Robust) ===\n");

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
            .timeout(Duration::from_millis(200)) // Shorter timeout
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

            // Clear any pending data first
            let mut discard = [0u8; 256];
            let _ = port.read(&mut discard);
            std::thread::sleep(Duration::from_millis(50));

            // Send command
            if let Err(e) = port.write_all(cmd.as_bytes()) {
                println!("  ✗ Write failed for addr {}: {}", addr, e);
                continue;
            }

            std::thread::sleep(Duration::from_millis(100));

            let mut buffer = [0u8; 256];
            match port.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    let response = String::from_utf8_lossy(&buffer[..n]);
                    let trimmed = response.trim();

                    println!("  Addr {}: {}", addr, trimmed);

                    // Elliptec responds with <addr>IN followed by device info
                    if trimmed.starts_with(&format!("{}IN", addr))
                        || trimmed.starts_with(&format!("{}PO", addr))
                    {
                        println!(
                            "  ✓✓✓ Elliptec device FOUND at address {} on {}! ✓✓✓",
                            addr, port_name
                        );

                        // Try to get position
                        std::thread::sleep(Duration::from_millis(100));
                        let pos_cmd = format!("{}gp\r", addr);
                        if port.write_all(pos_cmd.as_bytes()).is_ok() {
                            std::thread::sleep(Duration::from_millis(100));

                            if let Ok(n) = port.read(&mut buffer) {
                                let response = String::from_utf8_lossy(&buffer[..n]);
                                println!("  Position query response: {}", response.trim());
                            }
                        }
                    }
                }
                Ok(_) => {
                    // No response is normal - device not at this address
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Timeout is expected for non-existent addresses
                }
                Err(e) => {
                    println!("  ✗ Read error for addr {}: {}", addr, e);
                    break; // Port error, try next port
                }
            }
        }

        println!();
    }

    println!("Scan complete.");
    Ok(())
}
