//! ESP300 Motion Controller Hardware Test
//!
//! Run with: cargo run --example test_esp300 --features instrument_serial

use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== ESP300 Motion Controller Hardware Test ===\n");

    // ESP300 uses SCPI protocol with hardware flow control at 19200 baud
    let ports_to_test = vec![
        ("/dev/ttyUSB1", 19200),
        ("/dev/ttyUSB2", 19200),
        ("/dev/ttyUSB3", 19200),
        ("/dev/ttyUSB4", 19200),
        ("/dev/ttyUSB0", 19200),
    ];

    for (port_name, baud_rate) in &ports_to_test {
        println!("Testing {} at {} baud...", port_name, baud_rate);

        let mut port = match serialport::new(*port_name, *baud_rate)
            .timeout(Duration::from_millis(1000))
            .flow_control(serialport::FlowControl::Hardware) // RTS/CTS
            .open()
        {
            Ok(p) => p,
            Err(e) => {
                println!("  ✗ Failed to open: {}\n", e);
                continue;
            }
        };

        println!("  ✓ Port opened");

        // Test 1: *IDN? (SCPI identification)
        println!("  Test: *IDN?");
        port.write_all(b"*IDN?\r\n")?;
        std::thread::sleep(Duration::from_millis(500));

        let mut buffer = [0u8; 256];
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buffer[..n]);
                let trimmed = response.trim();
                println!("    Response: {}", trimmed);

                if trimmed.contains("ESP300") || trimmed.contains("Newport") {
                    println!("  ✓✓✓ ESP300 FOUND on {}! ✓✓✓\n", port_name);

                    // Additional tests
                    println!("  Additional Tests:");

                    // Query version
                    port.write_all(b"VE?\r\n")?;
                    std::thread::sleep(Duration::from_millis(300));
                    if let Ok(n) = port.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        println!("    Version: {}", response.trim());
                    }

                    // Query axis 1 position
                    port.write_all(b"1TP?\r\n")?;
                    std::thread::sleep(Duration::from_millis(300));
                    if let Ok(n) = port.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        println!("    Axis 1 Position: {}", response.trim());
                    }

                    println!("\n  Configuration:");
                    println!("    Port: {}", port_name);
                    println!("    Baud: {}", baud_rate);
                    println!("    Flow Control: RTS/CTS (Hardware)");

                    return Ok(());
                }
            }
            Ok(_) => println!("    No response (timeout)"),
            Err(e) => println!("    Read error: {}", e),
        }

        println!();
    }

    println!("❌ ESP300 NOT FOUND on any tested port");
    Ok(())
}
