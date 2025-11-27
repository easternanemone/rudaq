use serialport;
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    println!("Scanning for Elliptec rotators...\n");

    // Ports to scan (unknown devices from hardware report)
    let ports_to_scan = vec![
        "/dev/ttyUSB0",
        "/dev/ttyUSB2",
        "/dev/ttyUSB3",
        "/dev/ttyUSB4",
    ];

    // Elliptec device addresses to try (0-9, focusing on 2&3 from previous validation)
    let addresses_to_try = vec![2, 3, 0, 1, 4, 5, 6, 7, 8, 9];

    for port_name in &ports_to_scan {
        println!("Scanning port: {}", port_name);

        // Try to open the port with Elliptec settings
        match serialport::new(*port_name, 9600)
            .timeout(Duration::from_millis(500))
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .open()
        {
            Ok(mut port) => {
                println!("  ✓ Port opened successfully");

                // Clear any buffered data
                port.clear(serialport::ClearBuffer::All).ok();
                std::thread::sleep(Duration::from_millis(100));

                // Try each address
                for addr in &addresses_to_try {
                    let command = format!("{}in\r\n", addr);

                    // Send info command
                    if let Err(e) = port.write_all(command.as_bytes()) {
                        println!("  ✗ Address {}: Write error - {}", addr, e);
                        continue;
                    }

                    // Wait for response
                    std::thread::sleep(Duration::from_millis(200));

                    // Try to read response
                    let mut buffer = [0u8; 256];
                    match port.read(&mut buffer) {
                        Ok(n) if n > 0 => {
                            let response = String::from_utf8_lossy(&buffer[..n]);
                            if response.len() > 2 {
                                // Valid response
                                println!(
                                    "  ✓✓✓ FOUND DEVICE at address {}: {}",
                                    addr,
                                    response.trim()
                                );
                            }
                        }
                        _ => {
                            // No response, continue silently
                        }
                    }

                    // Small delay between attempts
                    std::thread::sleep(Duration::from_millis(100));
                }

                println!();
            }
            Err(e) => {
                println!("  ✗ Could not open port: {}\n", e);
            }
        }
    }

    println!("\n=== Scan Complete ===");
    println!("If no devices found, check:");
    println!("  1. Physical connections (USB, power)");
    println!("  2. Device addresses (may need full 0-F scan)");
    println!("  3. Port permissions (should be in 'uucp' group)");
}
