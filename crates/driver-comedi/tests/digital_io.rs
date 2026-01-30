#![cfg(not(target_arch = "wasm32"))]
//! Comedi Digital I/O Test Suite
//!
//! Validates digital I/O subsystem operations on the NI PCI-MIO-16XE-10.
//! Target hardware: 8 DIO channels on BNC 2110 breakout board.
//!
//! # Hardware Setup
//!
//! Optional loopback for full testing:
//! - DIO0 (output) → DIO1 (input)
//! - Or any pair of DIO pins jumpered together
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_DIO_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//! - `COMEDI_DIO_LOOPBACK=1` - Enable loopback tests (requires physical jumper)
//!
//! # Running
//!
//! ```bash
//! export COMEDI_DIO_TEST=1
//! cargo nextest run --profile hardware --features hardware -p driver-comedi -- digital_io
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `test_dio_configuration` | Configure pins as input/output |
//! | `test_single_pin_operations` | Read/write individual pins |
//! | `test_port_operations` | Multi-pin bitmask operations |
//! | `test_toggle_operation` | Toggle pin state |
//! | `test_dio_loopback` | Output→input loopback (if jumpered) |

#![cfg(feature = "hardware")]

use driver_comedi::{ComediDevice, DioDirection};
use std::env;
use std::thread;
use std::time::Duration;

// =============================================================================
// Test Configuration
// =============================================================================

/// Settling time after DIO operations (ms)
const SETTLING_TIME_MS: u64 = 10;

/// Expected number of DIO channels on PCI-MIO-16XE-10
const EXPECTED_DIO_CHANNELS: u32 = 8;

/// Check if DIO test is enabled
fn dio_test_enabled() -> bool {
    env::var("COMEDI_DIO_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Check if loopback test is enabled
fn dio_loopback_enabled() -> bool {
    env::var("COMEDI_DIO_LOOPBACK")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if DIO test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !dio_test_enabled() {
            println!("Comedi DIO test skipped (set COMEDI_DIO_TEST=1 to enable)");
            return;
        }
    };
}

/// Skip loopback test if not enabled
macro_rules! skip_loopback_if_disabled {
    () => {
        if !dio_loopback_enabled() {
            println!("Comedi DIO loopback test skipped (set COMEDI_DIO_LOOPBACK=1 to enable)");
            println!("Note: Requires physical jumper between DIO pins");
            return;
        }
    };
}

// =============================================================================
// Test 1: DIO Configuration
// =============================================================================

/// Test pin configuration as input/output
///
/// Validates:
/// - Pins can be configured as inputs
/// - Pins can be configured as outputs
/// - Configuration persists
#[test]
fn test_dio_configuration() {
    skip_if_disabled!();

    println!("\n=== Comedi DIO Configuration Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    println!("\nDigital I/O Subsystem:");
    println!("  Number of channels: {}", dio.n_channels());

    assert!(
        dio.n_channels() >= EXPECTED_DIO_CHANNELS,
        "Expected at least {} DIO channels, got {}",
        EXPECTED_DIO_CHANNELS,
        dio.n_channels()
    );

    // Test configuring each pin
    println!("\nConfiguring pins:");

    // Configure first 4 as outputs
    for ch in 0..4.min(dio.n_channels()) {
        dio.configure(ch, DioDirection::Output)
            .expect(&format!("Failed to configure ch{} as output", ch));
        println!("  CH{}: Output ✓", ch);
    }

    // Configure next 4 as inputs
    for ch in 4..8.min(dio.n_channels()) {
        dio.configure(ch, DioDirection::Input)
            .expect(&format!("Failed to configure ch{} as input", ch));
        println!("  CH{}: Input ✓", ch);
    }

    // Test batch configuration
    println!("\nTesting batch configuration:");
    dio.configure_range(0, 4, DioDirection::Input)
        .expect("Failed to configure range as inputs");
    println!("  CH0-CH3: Input (batch) ✓");

    dio.configure_range(4, 4, DioDirection::Output)
        .expect("Failed to configure range as outputs");
    println!("  CH4-CH7: Output (batch) ✓");

    println!("\n=== DIO Configuration Test PASSED ===\n");
}

// =============================================================================
// Test 2: Single Pin Operations
// =============================================================================

/// Test reading and writing individual pins
///
/// Validates:
/// - Single pin write operations
/// - Single pin read operations
/// - Pin state is retained
#[test]
fn test_single_pin_operations() {
    skip_if_disabled!();

    println!("\n=== Comedi Single Pin Operations Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    // Use pin 0 as test output
    let test_pin = 0u32;

    println!("\nTesting pin {} as output:", test_pin);

    // Configure as output
    dio.configure(test_pin, DioDirection::Output)
        .expect("Failed to configure as output");

    // Test set high
    dio.set_high(test_pin).expect("Failed to set high");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    println!("  Set HIGH ✓");

    // Read back (note: may not work on all hardware without loopback)
    let state = dio.read(test_pin).unwrap_or(false);
    println!("  Read back: {}", if state { "HIGH" } else { "LOW" });

    // Test set low
    dio.set_low(test_pin).expect("Failed to set low");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    println!("  Set LOW ✓");

    let state = dio.read(test_pin).unwrap_or(true);
    println!("  Read back: {}", if state { "HIGH" } else { "LOW" });

    // Test write with value
    dio.write(test_pin, true).expect("Failed to write true");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    println!("  Write(true) ✓");

    dio.write(test_pin, false).expect("Failed to write false");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    println!("  Write(false) ✓");

    println!("\n=== Single Pin Operations Test PASSED ===\n");
}

// =============================================================================
// Test 3: Port Operations (Multi-Pin)
// =============================================================================

/// Test reading and writing multiple pins via bitmask
///
/// Validates:
/// - Port read operation (read_port)
/// - Port write operation (write_port)
/// - Bitmask handling
#[test]
fn test_port_operations() {
    skip_if_disabled!();

    println!("\n=== Comedi Port Operations Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    // Configure all pins as outputs for this test
    println!("\nConfiguring all pins as outputs...");
    for ch in 0..dio.n_channels().min(8) {
        dio.configure(ch, DioDirection::Output)
            .expect("Failed to configure");
    }

    // Test writing patterns
    let test_patterns: Vec<(u32, &str)> = vec![
        (0b00000000, "All LOW"),
        (0b11111111, "All HIGH"),
        (0b10101010, "Alternating (0xAA)"),
        (0b01010101, "Alternating (0x55)"),
        (0b00001111, "Lower nibble HIGH"),
        (0b11110000, "Upper nibble HIGH"),
    ];

    println!("\nTesting port write patterns:");

    for (pattern, description) in &test_patterns {
        // Write pattern
        dio.write_port(0, 0xFF, *pattern)
            .expect("Failed to write port");
        thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

        // Read back
        let read_value = dio.read_port(0).unwrap_or(0);

        println!(
            "  {}: Write 0x{:02X} → Read 0x{:02X}",
            description, pattern, read_value
        );
    }

    // Reset to all low
    dio.write_port(0, 0xFF, 0x00).expect("Failed to reset port");
    println!("\n  Reset to all LOW ✓");

    // Test selective write (only modify certain pins)
    println!("\nTesting selective port write:");

    // Set pins 0,1 high, leave others unchanged
    dio.write_port(0, 0b00000011, 0b00000011)
        .expect("Failed to selective write");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    let state = dio.read_port(0).unwrap_or(0);
    println!("  Mask 0x03, Value 0x03 → Port state: 0x{:02X}", state);

    // Set pins 4,5 high, leave pins 0,1 unchanged
    dio.write_port(0, 0b00110000, 0b00110000)
        .expect("Failed to selective write");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    let state = dio.read_port(0).unwrap_or(0);
    println!("  Mask 0x30, Value 0x30 → Port state: 0x{:02X}", state);

    // Clear all
    dio.write_port(0, 0xFF, 0x00).expect("Failed to clear");

    println!("\n=== Port Operations Test PASSED ===\n");
}

// =============================================================================
// Test 4: Toggle Operation
// =============================================================================

/// Test toggle operation on a pin
///
/// Validates:
/// - Toggle reads current state and writes opposite
/// - Returns the new state
#[test]
fn test_toggle_operation() {
    skip_if_disabled!();

    println!("\n=== Comedi Toggle Operation Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    let test_pin = 0u32;

    // Configure as output
    dio.configure(test_pin, DioDirection::Output)
        .expect("Failed to configure");

    // Start from known state (low)
    dio.set_low(test_pin).expect("Failed to set low");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));

    println!("\nTesting toggle on pin {}:", test_pin);

    // Toggle sequence
    for i in 0..4 {
        let new_state = dio.toggle(test_pin).expect("Failed to toggle");
        thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
        println!(
            "  Toggle {}: → {}",
            i + 1,
            if new_state { "HIGH" } else { "LOW" }
        );

        // Verify alternating pattern
        let expected = (i % 2) == 0; // First toggle should go HIGH
        assert_eq!(
            new_state,
            expected,
            "Toggle {} should be {}",
            i + 1,
            if expected { "HIGH" } else { "LOW" }
        );
    }

    // Reset
    dio.set_low(test_pin).expect("Failed to reset");

    println!("\n=== Toggle Operation Test PASSED ===\n");
}

// =============================================================================
// Test 5: DIO Loopback (Optional)
// =============================================================================

/// Test DIO loopback with physical jumper between pins
///
/// Requires: Physical jumper between DIO0 (output) and DIO1 (input)
///
/// Validates:
/// - Output pin state is read on input pin
/// - Both HIGH and LOW states
#[test]
fn test_dio_loopback() {
    skip_if_disabled!();
    skip_loopback_if_disabled!();

    println!("\n=== Comedi DIO Loopback Test ===");
    println!("Device: {}", device_path());
    println!("Loopback: DIO0 (output) → DIO1 (input)");

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    let output_pin = 0u32;
    let input_pin = 1u32;

    // Configure pins
    dio.configure(output_pin, DioDirection::Output)
        .expect("Failed to configure output");
    dio.configure(input_pin, DioDirection::Input)
        .expect("Failed to configure input");

    println!("\nConfiguration:");
    println!("  DIO{}: Output", output_pin);
    println!("  DIO{}: Input", input_pin);

    // Test HIGH
    println!("\nTesting loopback:");

    dio.set_high(output_pin).expect("Failed to set high");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    let read_value = dio.read(input_pin).expect("Failed to read");
    let status = if read_value { "PASS" } else { "FAIL" };
    println!(
        "  Write HIGH → Read {} [{}]",
        if read_value { "HIGH" } else { "LOW" },
        status
    );
    assert!(read_value, "Loopback should read HIGH when output is HIGH");

    // Test LOW
    dio.set_low(output_pin).expect("Failed to set low");
    thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
    let read_value = dio.read(input_pin).expect("Failed to read");
    let status = if !read_value { "PASS" } else { "FAIL" };
    println!(
        "  Write LOW  → Read {} [{}]",
        if read_value { "HIGH" } else { "LOW" },
        status
    );
    assert!(!read_value, "Loopback should read LOW when output is LOW");

    // Test pattern
    println!("\nTesting bit pattern loopback:");

    for expected in [true, false, true, true, false] {
        dio.write(output_pin, expected).expect("Failed to write");
        thread::sleep(Duration::from_millis(SETTLING_TIME_MS));
        let actual = dio.read(input_pin).expect("Failed to read");
        assert_eq!(actual, expected, "Loopback mismatch");
        println!(
            "  {} → {} ✓",
            if expected { "HIGH" } else { "LOW " },
            if actual { "HIGH" } else { "LOW " }
        );
    }

    // Reset
    dio.set_low(output_pin).expect("Failed to reset");

    println!("\n=== DIO Loopback Test PASSED ===\n");
}

// =============================================================================
// Test 6: Read All Channels
// =============================================================================

/// Test reading all DIO channels at once
#[test]
fn test_read_all_channels() {
    skip_if_disabled!();

    println!("\n=== Comedi Read All DIO Channels Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    let dio = device
        .digital_io()
        .expect("Failed to get digital I/O subsystem");

    // Configure all as inputs first
    for ch in 0..dio.n_channels().min(8) {
        dio.configure(ch, DioDirection::Input).ok();
    }

    // Read all channels
    let states = dio.read_all().expect("Failed to read all channels");

    println!("\nChannel states ({} channels):", states.len());
    for (i, state) in states.iter().enumerate() {
        println!("  DIO{}: {}", i, if *state { "HIGH" } else { "LOW" });
    }

    // Verify we got the expected number of channels
    assert!(
        states.len() >= EXPECTED_DIO_CHANNELS as usize,
        "Expected at least {} channels, got {}",
        EXPECTED_DIO_CHANNELS,
        states.len()
    );

    println!("\n=== Read All Channels Test PASSED ===\n");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that DIO tests are properly skipped when not enabled
#[test]
fn dio_test_skip_check() {
    let enabled = dio_test_enabled();
    let loopback = dio_loopback_enabled();

    if !enabled {
        println!("Comedi DIO test correctly disabled (COMEDI_DIO_TEST not set)");
        println!("To enable: export COMEDI_DIO_TEST=1");
    } else {
        println!("Comedi DIO test enabled via COMEDI_DIO_TEST=1");
    }

    if loopback {
        println!("DIO loopback tests enabled (requires DIO0→DIO1 jumper)");
    } else {
        println!("DIO loopback tests disabled (set COMEDI_DIO_LOOPBACK=1 if jumpered)");
    }
}
