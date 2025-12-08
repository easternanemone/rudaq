//! Hardware Scanner for DAQ Instruments
//!
//! Automatically detects and identifies connected instruments by:
//! - Scanning USB device metadata
//! - Testing serial communication
//! - Matching response patterns
//!
//! Run with: cargo run --example scan_hardware --features instrument_serial

use serialport::{available_ports, SerialPortType};
use std::io::{Read, Write};
use std::time::Duration;

#[derive(Debug, Clone)]
struct InstrumentMatch {
    port: String,
    baud_rate: u32,
    instrument_type: String,
    confidence: u8, // 0-100
    details: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== DAQ Hardware Scanner ===\n");

    // Step 1: Enumerate all serial ports
    println!("Scanning for serial ports...");
    let ports = available_ports()?;

    if ports.is_empty() {
        println!("No serial ports found!");
        return Ok(());
    }

    println!("Found {} serial ports:\n", ports.len());

    for port in &ports {
        print!("  {} - ", port.port_name);
        match &port.port_type {
            SerialPortType::UsbPort(info) => {
                println!("USB Device");
                println!("    Vendor ID: {:04x}", info.vid);
                println!("    Product ID: {:04x}", info.pid);
                if let Some(ref manufacturer) = info.manufacturer {
                    println!("    Manufacturer: {}", manufacturer);
                }
                if let Some(ref product) = info.product {
                    println!("    Product: {}", product);
                }
                if let Some(ref serial) = info.serial_number {
                    println!("    Serial: {}", serial);
                }
            }
            SerialPortType::PciPort => println!("PCI Port"),
            SerialPortType::BluetoothPort => println!("Bluetooth Port"),
            SerialPortType::Unknown => println!("Unknown Type"),
        }
        println!();
    }

    // Step 2: Test each port with various protocols
    println!("\nProbing instruments...\n");
    let mut matches: Vec<InstrumentMatch> = Vec::new();

    for port_info in &ports {
        let port_name = &port_info.port_name;
        println!("Testing {}...", port_name);

        // Try different baud rates
        let baud_rates = vec![9600, 19200, 115200];

        for &baud_rate in &baud_rates {
            // Test Newport 1830-C protocol
            if let Some(m) = test_newport_1830c(port_name, baud_rate) {
                println!("  ✓ Newport 1830-C detected!");
                matches.push(m);
                break; // Found match, no need to try other baud rates
            }

            // Test SCPI protocol (ESP300, MaiTai)
            if let Some(m) = test_scpi_idn(port_name, baud_rate) {
                println!("  ✓ SCPI instrument detected!");
                matches.push(m);
                break;
            }

            // Test Elliptec protocol
            if let Some(m) = test_elliptec(port_name, baud_rate) {
                println!("  ✓ Elliptec device detected!");
                matches.push(m);
                break;
            }
        }

        println!();
    }

    // Step 3: Display results
    println!("\n=== Detection Results ===\n");

    if matches.is_empty() {
        println!("No instruments detected.");
        println!("\nTroubleshooting:");
        println!("  - Check that instruments are powered on");
        println!("  - Verify cable connections");
        println!("  - Check user permissions (may need to be in 'dialout' group)");
    } else {
        println!("Found {} instrument(s):\n", matches.len());

        for (i, m) in matches.iter().enumerate() {
            println!(
                "{}. {} ({}% confidence)",
                i + 1,
                m.instrument_type,
                m.confidence
            );
            println!("   Port: {}", m.port);
            println!("   Baud: {}", m.baud_rate);
            println!("   Info: {}", m.details);
            println!();
        }

        // Generate config snippet
        println!("\n=== Suggested Configuration ===\n");
        println!("Add to config/default.toml:\n");

        for m in &matches {
            match m.instrument_type.as_str() {
                "Newport 1830-C" => {
                    println!("[instruments.newport_1830c]");
                    println!("type = \"newport_1830c\"");
                    println!("name = \"Newport 1830-C Power Meter\"");
                    println!("port = \"{}\"", m.port);
                    println!("baud_rate = {}", m.baud_rate);
                    println!("polling_rate_hz = 2.0");
                    println!();
                }
                "ESP300" => {
                    println!("[instruments.esp300]");
                    println!("type = \"esp300\"");
                    println!("name = \"Newport ESP300 Motion Controller\"");
                    println!("port = \"{}\"", m.port);
                    println!("baud_rate = {}", m.baud_rate);
                    println!("polling_rate_hz = 5.0");
                    println!();
                }
                "MaiTai" => {
                    println!("[instruments.maitai]");
                    println!("type = \"maitai\"");
                    println!("name = \"MaiTai Ti:Sapphire Laser\"");
                    println!("port = \"{}\"", m.port);
                    println!("baud_rate = {}", m.baud_rate);
                    println!("polling_rate_hz = 1.0");
                    println!();
                }
                "Elliptec ELL14" => {
                    println!("[instruments.elliptec]");
                    println!("type = \"elliptec\"");
                    println!("name = \"Elliptec ELL14 Rotation Mounts\"");
                    println!("port = \"{}\"", m.port);
                    println!("baud_rate = {}", m.baud_rate);
                    println!("device_addresses = [0, 1]  # Verify actual addresses");
                    println!("polling_rate_hz = 2.0");
                    println!();
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn test_newport_1830c(port_name: &str, baud_rate: u32) -> Option<InstrumentMatch> {
    let mut port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(500))
        .open()
        .ok()?;

    // Newport 1830-C uses simple ASCII protocol
    // Send "D?" to get power reading
    port.write_all(b"D?\r").ok()?;
    std::thread::sleep(Duration::from_millis(200));

    let mut buffer = [0u8; 128];
    let n = port.read(&mut buffer).ok()?;
    let response = String::from_utf8_lossy(&buffer[..n]);

    // Newport responds with scientific notation like "+1.23E-9"
    let cleaned = response.trim();
    if cleaned.contains('E') || cleaned.contains('e') {
        // Try to parse as float to confirm
        let parse_str = cleaned.replace("+", "").replace("\r", "").replace("\n", "");
        if parse_str.parse::<f64>().is_ok() {
            return Some(InstrumentMatch {
                port: port_name.to_string(),
                baud_rate,
                instrument_type: "Newport 1830-C".to_string(),
                confidence: 95,
                details: format!("Power reading: {}", cleaned),
            });
        }
    }

    None
}

fn test_scpi_idn(port_name: &str, baud_rate: u32) -> Option<InstrumentMatch> {
    // Try with hardware flow control (ESP300 needs this)
    let mut port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(500))
        .flow_control(serialport::FlowControl::Hardware)
        .open()
        .ok()?;

    // Send SCPI identification query
    port.write_all(b"*IDN?\r\n").ok()?;
    std::thread::sleep(Duration::from_millis(300));

    let mut buffer = [0u8; 256];
    let n = port.read(&mut buffer).ok()?;
    let response = String::from_utf8_lossy(&buffer[..n]);
    let cleaned = response.trim();

    if cleaned.is_empty() || n == 0 {
        return None;
    }

    // Identify specific instruments by response patterns
    if cleaned.contains("ESP300") || cleaned.contains("Newport") && cleaned.contains("300") {
        return Some(InstrumentMatch {
            port: port_name.to_string(),
            baud_rate,
            instrument_type: "ESP300".to_string(),
            confidence: 90,
            details: format!("IDN: {}", cleaned),
        });
    }

    if cleaned.contains("MaiTai") || cleaned.contains("Spectra-Physics") {
        return Some(InstrumentMatch {
            port: port_name.to_string(),
            baud_rate,
            instrument_type: "MaiTai".to_string(),
            confidence: 90,
            details: format!("IDN: {}", cleaned),
        });
    }

    // Generic SCPI device
    if !cleaned.is_empty() && cleaned.len() > 5 {
        return Some(InstrumentMatch {
            port: port_name.to_string(),
            baud_rate,
            instrument_type: "SCPI Device".to_string(),
            confidence: 70,
            details: format!("IDN: {}", cleaned),
        });
    }

    None
}

fn test_elliptec(port_name: &str, baud_rate: u32) -> Option<InstrumentMatch> {
    let mut port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(300))
        .open()
        .ok()?;

    // Elliptec protocol: address + command
    // Try address 0: "0in" to get info
    port.write_all(b"0in\r").ok()?;
    std::thread::sleep(Duration::from_millis(200));

    let mut buffer = [0u8; 128];
    let n = port.read(&mut buffer).ok()?;
    let response = String::from_utf8_lossy(&buffer[..n]);

    // Elliptec responds with "0IN" followed by device info
    if response.starts_with("0IN") || response.starts_with("0PO") {
        return Some(InstrumentMatch {
            port: port_name.to_string(),
            baud_rate,
            instrument_type: "Elliptec ELL14".to_string(),
            confidence: 85,
            details: format!("Response: {}", response.trim()),
        });
    }

    None
}
