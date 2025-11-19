/*
 * Hardware Discovery Tool
 *
 * This utility scans all available serial ports to identify connected instruments
 * by sending safe "identification" commands.
 *
 * ARCHITECTURAL WARNING:
 * Do not run this loop during an active experiment!
 * 1. Latency: Scanning ports blocks the thread and causes jitters.
 * 2. Safety: Sending probe bytes (even safe ones) to high-power devices
 * like lasers can be dangerous if baud rates are mismatched (interpreted as junk commands).
 * Run this ONCE at startup or via a manual configuration step.
 *
 * NOTE ON CAMERAS:
 * The Photometrics Prime BSI sCMOS is NOT a serial device (USB 3.0/PCIe).
 * It cannot be detected by this script. Use the `pvcam-sys` library bindings
 * to detect cameras via the PVCAM C-Library.
 */

use serialport::{SerialPortType, SerialPort};
use std::time::Duration;
use std::io::{Read, Write};
use std::thread;

/// Configuration for a hardware probe
struct Probe {
    name: &'static str,
    // Primary baud rate to try first
    default_baud_rate: u32,
    // Fallback baud rates if the default fails (common alternatives)
    fallback_baud_rates: &'static [u32],
    command: &'static [u8],
    expected_response: &'static str,
    // Flow control setting
    flow_control: serialport::FlowControl,
}

const PROBES: &[Probe] = &[
    // Newport 1830-C Power Meter
    // Protocol: Simple ASCII commands (NOT SCPI)
    // Command: D? (Query power reading)
    // Expected: Scientific notation like "9E-9"
    // NOTE: Uses LF terminator only, no flow control
    Probe {
        name: "Newport 1830-C",
        default_baud_rate: 9600,
        fallback_baud_rates: &[19200, 38400, 115200],
        command: b"D?\n",
        expected_response: "E",  // Scientific notation contains "E"
        flow_control: serialport::FlowControl::None,
    },
    // Spectra Physics MaiTai Laser
    // Protocol: SCPI-like with CR terminator
    // Command: *IDN?
    // Expected: "Spectra Physics" (note: no dash in actual response!)
    // Note: Requires XON/XOFF flow control, CR terminator
    Probe {
        name: "Spectra Physics MaiTai",
        default_baud_rate: 9600,
        fallback_baud_rates: &[19200, 38400, 57600, 115200],
        command: b"*IDN?\r",
        expected_response: "Spectra Physics",  // No dash!
        flow_control: serialport::FlowControl::Software,  // XON/XOFF
    },
    // Thorlabs Elliptec Rotation Mounts (ELL14)
    // Protocol: Binary/ASCII hybrid (Manual Issue 7, Page 11)
    // Command: "in" (Information Request) prefixed with address
    // Format: [Addr][Cmd] -> "0in"
    // Response: Device echoes command type in caps -> "0IN..."
    // Note: This probe checks for Address 0 (the default bus master).
    Probe {
        name: "Elliptec Bus (Address 0)",
        default_baud_rate: 9600,
        fallback_baud_rates: &[], // Elliptec is strictly 9600
        command: b"0in",
        expected_response: "0IN",
        flow_control: serialport::FlowControl::None,
    },
    // Newport ESP300 Stage Controller
    // Protocol: ASCII
    // Command: ID?
    // Expected: "ESP300"
    // Note: Unlike most devices, ESP300 defaults to 19200 baud with hardware flow control.
    Probe {
        name: "Newport ESP300",
        default_baud_rate: 19200,
        fallback_baud_rates: &[9600, 38400, 115200],
        command: b"ID?\r",
        expected_response: "ESP300",
        flow_control: serialport::FlowControl::Hardware,  // RTS/CTS
    },
];

fn main() {
    println!("🔍 Starting Hardware Discovery Scan...");
    println!("⚠️  WARNING: Ensure high-power devices (Lasers) are in a safe state.");
    
    let ports = serialport::available_ports().expect("No ports found!");
    
    if ports.is_empty() {
        println!("❌ No serial ports detected on this system.");
        return;
    }

    for p in ports {
        println!("Checking port: {}", p.port_name);
        
        // Optimization: Filter out obvious non-instrument ports on Linux/Mac
        // if p.port_name.contains("Bluetooth") { continue; }

        let mut identified = false;

        for probe in PROBES {
            // Build list of rates: default + fallbacks
            let mut rates_to_try = vec![probe.default_baud_rate];
            rates_to_try.extend_from_slice(probe.fallback_baud_rates);

            for &baud in &rates_to_try {
                if try_probe(&p.port_name, probe, baud) {
                    println!("✅ FOUND: {} on {} (Baud: {})", probe.name, p.port_name, baud);
                    
                    // Special handling for Elliptec Bus Enumeration
                    // The Elliptec protocol allows multiple devices on one bus.
                    // Once the port is identified, we scan specifically for addresses 0-F.
                    if probe.name.contains("Elliptec") {
                        scan_elliptec_bus(&p.port_name);
                    }
                    
                    identified = true;
                    break; 
                }
            }
            if identified { break; } // Stop probing this port if found
        }

        if !identified {
            println!("   (Unknown Device or No Response)");
        }
    }
    
    // Reminder for non-serial hardware
    println!("\nNOTE: Photometrics Prime BSI is NOT a serial device.");
    println!("      It must be detected via the PVCAM C-Library driver initialization.");
}

fn try_probe(port_name: &str, probe: &Probe, baud_rate: u32) -> bool {
    // "Gentle Handshake" Strategy with Fallback:
    // 1. Open port with specified baud rate and flow control
    // 2. Set short timeout (250ms) to fail fast
    // 3. Clear buffers to remove stale data
    // 4. Send challenge command
    // 5. Check response substring
    let port_result = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(3000))  // MaiTai needs 2+ seconds to respond
        .flow_control(probe.flow_control)
        .open();

    match port_result {
        Ok(mut port) => {
            // Clear buffer to ensure we aren't reading old garbage
            let _ = port.clear(serialport::ClearBuffer::All);
            
            // Send Command
            if let Err(_) = port.write_all(probe.command) {
                return false;
            }

            // Flush to ensure command is actually sent
            if let Err(_) = port.flush() {
                return false;
            }

            // Wait for hardware processing time (MaiTai needs 2+ seconds)
            thread::sleep(Duration::from_millis(2000));  // MaiTai is very slow to respond

            // Read Response
            let mut serial_buf: Vec<u8> = vec![0; 1024];
            match port.read(serial_buf.as_mut_slice()) {
                Ok(t) => {
                    let response = String::from_utf8_lossy(&serial_buf[..t]);
                    if response.contains(probe.expected_response) {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
        Err(_) => return false, // Port busy or unavailable
    }
    false
}

/// Special routine to find all 3 active rotators on the bus
/// Iterates addresses 0-9 and A-F to find attached ELL14 units.
fn scan_elliptec_bus(port_name: &str) {
    println!("   Create Elliptec Bus Map for {}:", port_name);
    
    // Try addresses 0-9 and A-F
    let addresses = "0123456789ABCDEF";
    
    if let Ok(mut port) = serialport::new(port_name, 9600).timeout(Duration::from_millis(100)).open() {
        for char_addr in addresses.chars() {
            let cmd = format!("{}in", char_addr); // e.g., "0in", "1in"
            let _ = port.write_all(cmd.as_bytes());
            thread::sleep(Duration::from_millis(50));
            
            let mut buf = [0u8; 32];
            if let Ok(n) = port.read(&mut buf) {
                let resp = String::from_utf8_lossy(&buf[..n]);
                // Valid response format: {Addr}IN{ModelInfo} e.g. "0INELL14"
                if resp.len() >= 3 && resp.contains("IN") {
                     // Check if response starts with our address
                     if resp.starts_with(char_addr) {
                         // Parse Model (e.g., "0INELL14")
                         let model = resp.replace(&format!("{}IN", char_addr), "").trim().to_string();
                         println!("   -> Address [{}]: Active (Model: {})", char_addr, model);
                     }
                }
            }
        }
    }
}
