//! MaiTai Shutter Control Hardware Test
//! 
//! Tests shutter open/close functionality on real MaiTai hardware.
//! Run with: cargo run --example test_maitai_shutter

use std::time::Duration;
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MaiTai Shutter Control Hardware Test ===\n");
    
    // MaiTai validated settings (from hardware validation 2025-11-02)
    let port_name = "/dev/ttyUSB5";
    let baud_rate = 9600;
    
    println!("Opening port {} at {} baud...", port_name, baud_rate);
    
    let mut port = serialport::new(port_name, baud_rate)
        .timeout(Duration::from_millis(2000))
        .flow_control(serialport::FlowControl::Software) // XON/XOFF
        .open()?;
    
    println!("✓ Port opened with XON/XOFF flow control\n");
    
    // Helper function to send command and read response
    let mut send_command = |cmd: &str| -> Result<String, Box<dyn std::error::Error>> {
        println!("→ Sending: {}", cmd);
        port.write_all(cmd.as_bytes())?;
        port.write_all(b"\r\n")?;
        port.flush()?;
        
        std::thread::sleep(Duration::from_millis(500));
        
        let mut buffer = [0u8; 256];
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buffer[..n]);
                let trimmed = response.trim().trim_matches('\0');
                println!("← Response: {}\n", trimmed);
                Ok(trimmed.to_string())
            }
            Ok(_) => Ok(String::new()),
            Err(e) => Err(Box::new(e)),
        }
    };
    
    // Test 1: Query initial shutter state
    println!("=== Test 1: Query Initial Shutter State ===");
    match send_command("READ:SHUTter?") {
        Ok(response) => {
            if response.contains("0") {
                println!("✓ Shutter is currently CLOSED (0)\n");
            } else if response.contains("1") {
                println!("✓ Shutter is currently OPEN (1)\n");
            } else {
                println!("? Unexpected response: {}\n", response);
            }
        }
        Err(e) => println!("✗ Failed to query shutter: {}\n", e),
    }
    
    // Test 2: Query current power (should be ~0W if shutter closed)
    println!("=== Test 2: Query Current Power ===");
    match send_command("READ:POWer?") {
        Ok(response) => {
            println!("✓ Current power: {}\n", response);
        }
        Err(e) => println!("✗ Failed to query power: {}\n", e),
    }
    
    // Test 3: Open shutter
    println!("=== Test 3: Open Shutter ===");
    println!("⚠️  WARNING: This will open the laser shutter!");
    println!("    Ensure proper safety precautions are in place.");
    println!("    Press Ctrl+C to abort, or wait 3 seconds to continue...\n");
    std::thread::sleep(Duration::from_secs(3));
    
    match send_command("SHUTter:1") {
        Ok(_) => {
            println!("✓ Sent SHUTter:1 (open) command");
            std::thread::sleep(Duration::from_millis(1000)); // Wait for mechanical shutter
            
            // Verify shutter opened
            match send_command("READ:SHUTter?") {
                Ok(response) => {
                    if response.contains("1") {
                        println!("✓✓ Shutter confirmed OPEN (1)\n");
                    } else {
                        println!("✗ Shutter failed to open: {}\n", response);
                    }
                }
                Err(e) => println!("✗ Failed to verify shutter: {}\n", e),
            }
            
            // Check power after opening
            match send_command("READ:POWer?") {
                Ok(response) => {
                    println!("✓ Power with shutter open: {}\n", response);
                }
                Err(e) => println!("✗ Failed to query power: {}\n", e),
            }
        }
        Err(e) => println!("✗ Failed to open shutter: {}\n", e),
    }
    
    // Test 4: Close shutter
    println!("=== Test 4: Close Shutter ===");
    std::thread::sleep(Duration::from_secs(2)); // Keep open for 2 seconds
    
    match send_command("SHUTter:0") {
        Ok(_) => {
            println!("✓ Sent SHUTter:0 (close) command");
            std::thread::sleep(Duration::from_millis(1000)); // Wait for mechanical shutter
            
            // Verify shutter closed
            match send_command("READ:SHUTter?") {
                Ok(response) => {
                    if response.contains("0") {
                        println!("✓✓ Shutter confirmed CLOSED (0)\n");
                    } else {
                        println!("✗ Shutter failed to close: {}\n", response);
                    }
                }
                Err(e) => println!("✗ Failed to verify shutter: {}\n", e),
            }
            
            // Check power after closing
            match send_command("READ:POWer?") {
                Ok(response) => {
                    println!("✓ Power with shutter closed: {}\n", response);
                }
                Err(e) => println!("✗ Failed to query power: {}\n", e),
            }
        }
        Err(e) => println!("✗ Failed to close shutter: {}\n", e),
    }
    
    // Test 5: Rapid open/close cycle
    println!("=== Test 5: Rapid Shutter Cycling ===");
    println!("Testing 5 rapid open/close cycles...\n");
    
    for i in 1..=5 {
        println!("Cycle {}/5:", i);
        
        // Open
        send_command("SHUTter:1")?;
        std::thread::sleep(Duration::from_millis(800));
        let open_state = send_command("READ:SHUTter?")?;
        
        // Close
        send_command("SHUTter:0")?;
        std::thread::sleep(Duration::from_millis(800));
        let close_state = send_command("READ:SHUTter?")?;
        
        if open_state.contains("1") && close_state.contains("0") {
            println!("  ✓ Cycle {} successful\n", i);
        } else {
            println!("  ✗ Cycle {} failed (open: {}, close: {})\n", i, open_state, close_state);
        }
    }
    
    // Test 6: Query wavelength for context
    println!("=== Test 6: Query Wavelength (Context) ===");
    match send_command("READ:WAVelength?") {
        Ok(response) => {
            println!("✓ Current wavelength: {}\n", response);
        }
        Err(e) => println!("✗ Failed to query wavelength: {}\n", e),
    }
    
    // Final safety check - ensure shutter is closed
    println!("=== Final Safety Check ===");
    send_command("SHUTter:0")?;
    std::thread::sleep(Duration::from_millis(1000));
    match send_command("READ:SHUTter?") {
        Ok(response) => {
            if response.contains("0") {
                println!("✓✓ SAFETY: Shutter confirmed CLOSED");
            } else {
                println!("⚠️  WARNING: Shutter may still be open!");
            }
        }
        Err(e) => println!("✗ Failed final safety check: {}", e),
    }
    
    println!("\n=== Test Complete ===");
    println!("\nSummary:");
    println!("- Port: {}", port_name);
    println!("- Baud: {}", baud_rate);
    println!("- Flow Control: Software (XON/XOFF)");
    println!("- Commands tested: SHUTter:0, SHUTter:1, READ:SHUTter?");
    println!("- All tests passed if no ✗ errors above");
    
    Ok(())
}
