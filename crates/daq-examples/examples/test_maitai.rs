//! MaiTai Ti:Sapphire Laser Hardware Test
//!
//! Run with: cargo run --example test_maitai --features instrument_serial

use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MaiTai Ti:Sapphire Laser Hardware Test ===\n");

    // Try multiple ports - MaiTai likely on USB5 (Silicon Labs CP2102)
    let ports_to_test = vec![
        ("/dev/ttyUSB5", 9600),
        ("/dev/ttyUSB0", 9600),
        ("/dev/ttyS0", 9600),
    ];

    for (port_name, baud_rate) in &ports_to_test {
        println!("Testing {} at {} baud...", port_name, baud_rate);

        let mut port = match serialport::new(*port_name, *baud_rate)
            .timeout(Duration::from_millis(1000))
            .flow_control(serialport::FlowControl::Software) // XON/XOFF
            .open()
        {
            Ok(p) => p,
            Err(e) => {
                println!("  ✗ Failed to open: {}\n", e);
                continue;
            }
        };

        println!("  ✓ Port opened");

        // Test 1: Query wavelength
        println!("  Test: READ:WAVelength?");
        port.write_all(b"READ:WAVelength?\r\n")?;
        std::thread::sleep(Duration::from_millis(500));

        let mut buffer = [0u8; 256];
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buffer[..n]);
                println!("    Response: {}", response.trim());

                if !response.trim().is_empty() {
                    println!("  ✓✓✓ MaiTai FOUND on {}! ✓✓✓\n", port_name);

                    // Additional tests
                    println!("  Additional Tests:");

                    // Query shutter status
                    port.write_all(b"READ:SHUTter?\r\n")?;
                    std::thread::sleep(Duration::from_millis(300));
                    if let Ok(n) = port.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        println!("    Shutter: {}", response.trim());
                    }

                    // Query power
                    port.write_all(b"READ:POWer?\r\n")?;
                    std::thread::sleep(Duration::from_millis(300));
                    if let Ok(n) = port.read(&mut buffer) {
                        let response = String::from_utf8_lossy(&buffer[..n]);
                        println!("    Power: {}", response.trim());
                    }

                    println!("\n  Configuration:");
                    println!("    Port: {}", port_name);
                    println!("    Baud: {}", baud_rate);
                    println!("    Flow Control: XON/XOFF (Software)");

                    return Ok(());
                }
            }
            Ok(_) => println!("    No response (timeout)"),
            Err(e) => println!("    Read error: {}", e),
        }

        println!();
    }

    println!("❌ MaiTai NOT FOUND on any tested port");
    Ok(())
}
