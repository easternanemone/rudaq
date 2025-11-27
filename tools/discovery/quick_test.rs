// Quick targeted test for specific known ports
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

fn test_port(
    port_name: &str,
    baud: u32,
    flow: serialport::FlowControl,
    command: &[u8],
    expected: &str,
    device_name: &str,
) -> bool {
    match serialport::new(port_name, baud)
        .timeout(Duration::from_millis(3000)) // MaiTai needs 2+ seconds
        .flow_control(flow)
        .open()
    {
        Ok(mut port) => {
            let _ = port.clear(serialport::ClearBuffer::All);

            if port.write_all(command).is_err() {
                return false;
            }

            if port.flush().is_err() {
                return false;
            }

            thread::sleep(Duration::from_millis(2000)); // MaiTai is slow

            let mut buf = vec![0u8; 1024];
            match port.read(&mut buf) {
                Ok(n) => {
                    let response = String::from_utf8_lossy(&buf[..n]);
                    println!("  Response from {}: {:?}", port_name, response);
                    if response.contains(expected) {
                        println!("✅ FOUND: {} on {}", device_name, port_name);
                        return true;
                    }
                }
                Err(e) => println!("  Read error: {}", e),
            }
        }
        Err(e) => println!("  Open error: {}", e),
    }
    false
}

fn main() {
    println!("🔍 Quick Hardware Test (Known Ports Only)...\n");

    let mut found = 0;

    // Test Newport 1830C on /dev/ttyS0
    println!("Testing Newport 1830C on /dev/ttyS0...");
    if test_port(
        "/dev/ttyS0",
        9600,
        serialport::FlowControl::None,
        b"D?\n",
        "E",
        "Newport 1830-C",
    ) {
        found += 1;
    }

    // Test MaiTai on /dev/ttyUSB5
    println!("\nTesting MaiTai on /dev/ttyUSB5...");
    if test_port(
        "/dev/ttyUSB5",
        9600,
        serialport::FlowControl::Software,
        b"*IDN?\r",
        "Spectra Physics",
        "Spectra Physics MaiTai",
    ) {
        found += 1;
    }

    // Test ESP300 on /dev/ttyUSB1
    println!("\nTesting ESP300 on /dev/ttyUSB1...");
    if test_port(
        "/dev/ttyUSB1",
        19200,
        serialport::FlowControl::Hardware,
        b"ID?\r",
        "ESP300",
        "Newport ESP300",
    ) {
        found += 1;
    }

    // Test ELL14 on /dev/ttyUSB0 (scan addresses 2, 3, 8)
    println!("\nTesting ELL14 bus on /dev/ttyUSB0...");
    let ell14_addresses = [2, 3, 8];
    let mut ell14_found = 0;
    for addr in ell14_addresses {
        let cmd = format!("{}in", addr);
        let expected = format!("{}IN", addr);
        if test_port(
            "/dev/ttyUSB0",
            9600,
            serialport::FlowControl::None,
            cmd.as_bytes(),
            &expected,
            &format!("Elliptec ELL14 (addr {})", addr),
        ) {
            ell14_found += 1;
        }
    }
    if ell14_found > 0 {
        found += 1;
        println!("  Found {} ELL14 devices on bus", ell14_found);
    }

    println!("\n===================");
    println!("Total devices found: {}/4", found);
    println!(
        "(ELL14 bus counts as 1 device, contains {} units)",
        ell14_found
    );
}
