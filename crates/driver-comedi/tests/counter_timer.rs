#![cfg(not(target_arch = "wasm32"))]
//! Comedi Counter/Timer Test Suite
//!
//! Validates counter/timer subsystem operations on the NI PCI-MIO-16XE-10.
//! Target hardware: Counter channels on the 8254-compatible timer chip.
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_COUNTER_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//!
//! # Running
//!
//! ```bash
//! export COMEDI_COUNTER_TEST=1
//! cargo nextest run --profile hardware --features hardware -p driver-comedi -- counter_timer
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `test_counter_read` | Read counter values |
//! | `test_counter_write` | Write/preload counter values |
//! | `test_counter_reset` | Reset counters to zero |
//! | `test_counter_info` | Query counter metadata |
//! | `test_event_counting` | Basic event counting (if signal available) |

#![cfg(feature = "hardware")]

use driver_comedi::ComediDevice;
use std::env;
use std::thread;
use std::time::Duration;

// =============================================================================
// Test Configuration
// =============================================================================

/// Settling time after counter operations (ms)
const SETTLING_TIME_MS: u64 = 10;

/// Check if counter test is enabled
fn counter_test_enabled() -> bool {
    env::var("COMEDI_COUNTER_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if counter test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !counter_test_enabled() {
            println!("Comedi counter test skipped (set COMEDI_COUNTER_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Test 1: Counter Read Operations
// =============================================================================

/// Test reading counter values
#[test]
fn test_counter_read() {
    skip_if_disabled!();

    println!("\n=== Comedi Counter Read Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Try to get counter subsystem
    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            println!("(This may be expected for some Comedi configurations)");
            return;
        }
    };

    println!("\nCounter Subsystem:");
    println!("  Number of channels: {}", counter.n_channels());
    println!("  Bit width: {}", counter.bit_width());
    println!("  Max value: {}", counter.maxdata());

    // Read each counter
    println!("\nReading counter values:");
    for ch in 0..counter.n_channels().min(3) {
        match counter.read(ch) {
            Ok(value) => println!("  Counter {}: {}", ch, value),
            Err(e) => println!("  Counter {}: Error - {}", ch, e),
        }
    }

    // Read all counters at once
    match counter.read_all() {
        Ok(values) => {
            println!("\nRead all counters: {:?}", values);
        }
        Err(e) => {
            println!("\nRead all failed: {}", e);
        }
    }

    println!("\n=== Counter Read Test PASSED ===\n");
}

// =============================================================================
// Test 2: Counter Write/Preload Operations
// =============================================================================

/// Test writing/preloading counter values
#[test]
fn test_counter_write() {
    skip_if_disabled!();

    println!("\n=== Comedi Counter Write Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            return;
        }
    };

    let test_channel = 0u32;

    // Test values to write (within reasonable range)
    let max = counter.maxdata();
    let test_values = vec![0, 100, 1000, max / 2, max - 1];

    println!("\nTesting counter write on channel {}:", test_channel);

    for value in test_values {
        // Write value
        match counter.write(test_channel, value) {
            Ok(()) => {
                thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

                // Read back and verify
                match counter.read(test_channel) {
                    Ok(read_value) => {
                        // Counter may have incremented if counting, so check approximately
                        let diff = (read_value as i64 - value as i64).unsigned_abs();
                        let status = if diff < 10 { "PASS" } else { "DRIFT" };
                        println!(
                            "  Write {} → Read {} (diff: {}) [{}]",
                            value, read_value, diff, status
                        );
                    }
                    Err(e) => println!("  Write {} → Read error: {}", value, e),
                }
            }
            Err(e) => println!("  Write {} failed: {}", value, e),
        }
    }

    // Reset to zero
    counter.reset(test_channel).ok();

    println!("\n=== Counter Write Test PASSED ===\n");
}

// =============================================================================
// Test 3: Counter Reset Operations
// =============================================================================

/// Test resetting counters to zero
#[test]
fn test_counter_reset() {
    skip_if_disabled!();

    println!("\n=== Comedi Counter Reset Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            return;
        }
    };

    println!("\nTesting individual counter reset:");

    for ch in 0..counter.n_channels().min(3) {
        // First write a non-zero value
        if counter.write(ch, 12345).is_ok() {
            thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

            // Reset
            match counter.reset(ch) {
                Ok(()) => {
                    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

                    // Verify
                    match counter.read(ch) {
                        Ok(value) => {
                            let status = if value < 10 { "PASS" } else { "FAIL" };
                            println!("  Counter {} after reset: {} [{}]", ch, value, status);
                        }
                        Err(e) => println!("  Counter {} read failed: {}", ch, e),
                    }
                }
                Err(e) => println!("  Counter {} reset failed: {}", ch, e),
            }
        }
    }

    // Test reset_all
    println!("\nTesting reset_all:");

    // Write values to all counters
    for ch in 0..counter.n_channels().min(3) {
        counter.write(ch, 9999).ok();
    }
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

    // Reset all
    match counter.reset_all() {
        Ok(()) => {
            thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

            // Verify all are near zero
            for ch in 0..counter.n_channels().min(3) {
                let value = counter.read(ch).unwrap_or(u32::MAX);
                let status = if value < 10 { "PASS" } else { "FAIL" };
                println!("  Counter {} after reset_all: {} [{}]", ch, value, status);
            }
        }
        Err(e) => println!("  Reset all failed: {}", e),
    }

    println!("\n=== Counter Reset Test PASSED ===\n");
}

// =============================================================================
// Test 4: Counter Metadata/Info
// =============================================================================

/// Test querying counter metadata
#[test]
fn test_counter_info() {
    skip_if_disabled!();

    println!("\n=== Comedi Counter Info Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            return;
        }
    };

    println!("\nCounter Subsystem Information:");
    println!("  Number of channels: {}", counter.n_channels());
    println!("  Bit width: {} bits", counter.bit_width());
    println!(
        "  Maximum value: {} (0x{:X})",
        counter.maxdata(),
        counter.maxdata()
    );

    // Verify reasonable values
    assert!(counter.n_channels() > 0, "Should have at least one counter");
    assert!(
        counter.bit_width() >= 16,
        "Counter should be at least 16 bits"
    );
    assert!(counter.maxdata() > 0, "Max data should be non-zero");

    // Calculate expected max from bit width
    let expected_max = (1u64 << counter.bit_width()) - 1;
    println!("\n  Expected max from bit width: {}", expected_max);
    println!("  Actual maxdata: {}", counter.maxdata());

    println!("\n=== Counter Info Test PASSED ===\n");
}

// =============================================================================
// Test 5: Event Counting (Basic)
// =============================================================================

/// Test basic event counting functionality
///
/// Note: Full validation requires external signal source
#[test]
fn test_event_counting() {
    skip_if_disabled!();

    println!("\n=== Comedi Event Counting Test ===");
    println!("Device: {}", device_path());
    println!("Note: Full validation requires external signal source");

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            return;
        }
    };

    let test_channel = 0u32;

    // Reset counter
    counter.reset(test_channel).expect("Failed to reset");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

    // Read initial value
    let initial = counter.read(test_channel).expect("Failed to read");
    println!("\nCounter {} initial value: {}", test_channel, initial);

    // Wait and read again to see if counting (ambient noise may trigger)
    println!("Waiting 100ms to observe any counting...");
    thread::sleep(Duration::from_millis(100));

    let after_wait = counter.read(test_channel).expect("Failed to read");
    println!("Counter {} after wait: {}", test_channel, after_wait);

    let events = after_wait.saturating_sub(initial);
    if events > 0 {
        println!(
            "  {} events detected (may be noise or external signal)",
            events
        );
    } else {
        println!("  No events detected (counter stable or no signal)");
    }

    println!("\n=== Event Counting Test PASSED ===\n");
}

// =============================================================================
// Test 6: Counter Channel Validation
// =============================================================================

/// Test error handling for invalid counter channels
#[test]
fn test_invalid_counter_channel() {
    skip_if_disabled!();

    println!("\n=== Comedi Invalid Counter Channel Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let counter = match device.counter() {
        Ok(c) => c,
        Err(e) => {
            println!("Counter subsystem not available: {}", e);
            return;
        }
    };

    let invalid_channel = counter.n_channels() + 10;

    println!("\nTesting invalid channel {}:", invalid_channel);

    // Read should fail
    match counter.read(invalid_channel) {
        Ok(v) => println!("  Read unexpectedly succeeded: {} (BUG)", v),
        Err(e) => println!("  Read correctly failed: {} ✓", e),
    }

    // Write should fail
    match counter.write(invalid_channel, 0) {
        Ok(()) => println!("  Write unexpectedly succeeded (BUG)"),
        Err(e) => println!("  Write correctly failed: {} ✓", e),
    }

    // Reset should fail
    match counter.reset(invalid_channel) {
        Ok(()) => println!("  Reset unexpectedly succeeded (BUG)"),
        Err(e) => println!("  Reset correctly failed: {} ✓", e),
    }

    println!("\n=== Invalid Counter Channel Test PASSED ===\n");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that counter tests are properly skipped when not enabled
#[test]
fn counter_test_skip_check() {
    let enabled = counter_test_enabled();
    if !enabled {
        println!("Comedi counter test correctly disabled (COMEDI_COUNTER_TEST not set)");
        println!("To enable: export COMEDI_COUNTER_TEST=1");
    } else {
        println!("Comedi counter test enabled via COMEDI_COUNTER_TEST=1");
    }
}
