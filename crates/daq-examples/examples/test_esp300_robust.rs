//! ESP300 Motion Controller Hardware Test - Robust Version
//!
//! Run with: cargo run --example test_esp300_robust --features instrument_serial

use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== ESP300 Motion Controller Hardware Test (Robust) ===\n");

    // Try different combinations
    let test_configs = vec![
        // Port, Baud, Flow Control, Description
        (
            "/dev/ttyUSB1",
            19200,
            serialport::FlowControl::Hardware,
            "USB1 @ 19200 RTS/CTS",
        ),
        (
            "/dev/ttyUSB1",
            19200,
            serialport::FlowControl::None,
            "USB1 @ 19200 No FC",
        ),
        (
            "/dev/ttyUSB2",
            19200,
            serialport::FlowControl::Hardware,
            "USB2 @ 19200 RTS/CTS",
        ),
        (
            "/dev/ttyUSB3",
            19200,
            serialport::FlowControl::Hardware,
            "USB3 @ 19200 RTS/CTS",
        ),
        (
            "/dev/ttyUSB1",
            9600,
            serialport::FlowControl::Hardware,
            "USB1 @ 9600 RTS/CTS",
        ),
        (
            "/dev/ttyUSB1",
            9600,
            serialport::FlowControl::None,
            "USB1 @ 9600 No FC",
        ),
    ];

    for (port_name, baud_rate, flow_control, desc) in &test_configs {
        println!("Testing {}...", desc);

        let mut port = match serialport::new(*port_name, *baud_rate)
            .timeout(Duration::from_millis(500))
            .flow_control(*flow_control)
            .open()
        {
            Ok(p) => p,
            Err(e) => {
                println!("  ✗ Failed to open: {}\n", e);
                continue;
            }
        };

        println!("  ✓ Port opened");

        // Clear any pending data
        let mut discard = [0u8; 256];
        let _ = port.read(&mut discard);
        std::thread::sleep(Duration::from_millis(100));

        // Test 1: *IDN? (SCPI identification)
        println!("  Test: *IDN?");
        if let Err(e) = port.write_all(b"*IDN?\r\n") {
            println!("  ✗ Write failed: {}\n", e);
            continue;
        }

        std::thread::sleep(Duration::from_millis(300));

        let mut buffer = [0u8; 256];
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buffer[..n]);
                let trimmed = response.trim();
                println!("  Response: {}", trimmed);

                if trimmed.contains("ESP300")
                    || trimmed.contains("ESP")
                    || trimmed.contains("Newport")
                {
                    println!("  ✓✓✓ ESP300 FOUND! ✓✓✓\n");

                    // Additional tests
                    println!("  Additional Tests:");

                    // Query version
                    port.write_all(b"VE?\r\n").ok();
                    std::thread::sleep(Duration::from_millis(200));
                    if let Ok(n) = port.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        println!("    Version: {}", response.trim());
                    }

                    println!("\n  Configuration:");
                    println!("    Port: {}", port_name);
                    println!("    Baud: {}", baud_rate);
                    println!("    Flow Control: {:?}", flow_control);

                    return Ok(());
                }
            }
            Ok(_) => println!("  No response (timeout)"),
            Err(e) => println!("  Read error: {}", e),
        }

        println!();
    }

    println!("❌ ESP300 NOT FOUND on any tested configuration");
    Ok(())
}
