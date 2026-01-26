//! Counter/timer example.
//!
//! Demonstrates counter operations including reading counts,
//! writing/preloading values, and resetting counters.
//!
//! # Usage
//!
//! ```bash
//! cargo build -p daq-driver-comedi --features hardware --example counter
//! ./target/debug/examples/counter
//! ```

use daq_driver_comedi::ComediDevice;
use std::env;
use std::thread;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let device_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/comedi0".to_string());

    println!("=== Comedi Counter Example ===\n");
    println!("Device: {}", device_path);

    let device = ComediDevice::open(&device_path)?;
    println!("Board: {}", device.board_name());

    // Get counter subsystem
    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("\nCounter subsystem not available: {}", e);
            println!("(This is normal for some device configurations)");
            return Ok(());
        }
    };

    println!("\nCounter Subsystem:");
    println!("  Channels: {}", counter.n_channels());
    println!("  Bit width: {} bits", counter.bit_width());
    println!("  Max value: {} (0x{:X})", counter.maxdata(), counter.maxdata());

    // Read all counter values
    println!("\nInitial counter values:");
    for ch in 0..counter.n_channels().min(3) {
        let value = counter.read(ch)?;
        println!("  Counter {}: {}", ch, value);
    }

    // Reset counter 0
    println!("\nResetting counter 0...");
    counter.reset(0)?;
    let value = counter.read(0)?;
    println!("  Counter 0 after reset: {}", value);

    // Preload a value
    println!("\nPreloading counter 0 with 10000...");
    counter.write(0, 10000)?;
    let value = counter.read(0)?;
    println!("  Counter 0 after preload: {}", value);

    // Monitor counter 0 for a bit
    println!("\nMonitoring counter 0 for 1 second...");
    let initial = counter.read(0)?;
    thread::sleep(Duration::from_secs(1));
    let final_val = counter.read(0)?;

    let delta = if final_val >= initial {
        final_val - initial
    } else {
        // Handle wraparound
        counter.maxdata() - initial + final_val
    };

    println!("  Initial: {}", initial);
    println!("  Final: {}", final_val);
    println!("  Delta: {} counts/second", delta);

    if delta > 0 {
        println!("  (Counter is incrementing - signal detected or noise)");
    } else {
        println!("  (Counter stable - no signal)");
    }

    // Reset all counters
    println!("\nResetting all counters...");
    counter.reset_all()?;

    println!("\nFinal counter values:");
    for ch in 0..counter.n_channels().min(3) {
        let value = counter.read(ch)?;
        println!("  Counter {}: {}", ch, value);
    }

    println!("\nDone!");
    Ok(())
}
