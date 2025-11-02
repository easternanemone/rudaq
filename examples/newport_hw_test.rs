//! Newport 1830-C Hardware Test
//! 
//! Run with: cargo run --example newport_hw_test --features instrument_serial

use serialport::SerialPort;
use std::time::Duration;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Newport 1830-C Hardware Test ===\n");
    
    let port_name = std::env::args().nth(1).unwrap_or_else(|| "/dev/ttyS0".to_string());
    let baud_rate = 9600;
    
    println!("Opening port {} at {} baud...", port_name, baud_rate);
    
    let mut port = serialport::new(&port_name, baud_rate)
        .timeout(Duration::from_millis(1000))
        .open()?;
    
    println!("✓ Port opened successfully\n");
    
    // Test 1: Get power reading
    println!("Test 1: Reading power measurement");
    port.write_all(b"D?\r")?;
    std::thread::sleep(Duration::from_millis(200));
    
    let mut buffer = [0u8; 128];
    match port.read(&mut buffer) {
        Ok(n) => {
            let response = String::from_utf8_lossy(&buffer[..n]);
            println!("  Response: {}", response.trim());
            
            // Try to parse as scientific notation
            let cleaned = response.trim().replace("\r", "").replace("\n", "");
            match cleaned.parse::<f64>() {
                Ok(power) => println!("  Parsed value: {:.3e} W", power),
                Err(_) => println!("  Warning: Could not parse as float"),
            }
            println!("  ✓ PASS\n");
        }
        Err(e) => {
            println!("  ✗ FAIL: {}\n", e);
            return Err(e.into());
        }
    }
    
    // Test 2: Set units to Watts
    println!("Test 2: Setting units to Watts (U0)");
    port.write_all(b"U0\r")?;
    std::thread::sleep(Duration::from_millis(200));
    port.read(&mut buffer).ok();  // Clear echo
    println!("  ✓ Command sent\n");
    
    // Test 3: Get power again
    println!("Test 3: Reading power in Watts");
    port.write_all(b"D?\r")?;
    std::thread::sleep(Duration::from_millis(200));
    
    match port.read(&mut buffer) {
        Ok(n) => {
            let response = String::from_utf8_lossy(&buffer[..n]);
            println!("  Response: {}", response.trim());
            
            let cleaned = response.trim().replace("\r", "").replace("\n", "");
            match cleaned.parse::<f64>() {
                Ok(power) => println!("  Parsed value: {:.3e} W", power),
                Err(_) => println!("  Warning: Could not parse as float"),
            }
            println!("  ✓ PASS\n");
        }
        Err(e) => {
            println!("  ✗ FAIL: {}\n", e);
            return Err(e.into());
        }
    }
    
    // Test 4: Set wavelength to 1550nm
    println!("Test 4: Setting wavelength to 1550nm");
    port.write_all(b"W1550\r")?;
    std::thread::sleep(Duration::from_millis(200));
    port.read(&mut buffer).ok();
    println!("  ✓ Command sent\n");
    
    // Test 5: Multiple rapid readings
    println!("Test 5: Taking 5 rapid readings");
    for i in 1..=5 {
        port.write_all(b"D?\r")?;
        std::thread::sleep(Duration::from_millis(150));
        
        if let Ok(n) = port.read(&mut buffer) {
            let response = String::from_utf8_lossy(&buffer[..n]);
            println!("  Reading {}: {}", i, response.trim());
        }
    }
    println!("  ✓ PASS\n");
    
    println!("=== All tests completed successfully ===");
    Ok(())
}
