#![cfg(not(target_arch = "wasm32"))]
//! Comedi Hardware Smoke Test Suite
//!
//! Comprehensive smoke tests for verifying Comedi DAQ device connectivity and operation.
//! Target hardware: National Instruments PCI-MIO-16XE-10 with BNC 2110 breakout board.
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_SMOKE_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//!
//! # Quick Setup
//!
//! ```bash
//! # Verify device exists
//! ls -la /dev/comedi0
//!
//! # Run tests
//! export COMEDI_SMOKE_TEST=1
//! cargo nextest run --profile hardware --features hardware -p daq-driver-comedi
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `device_discovery_test` | Open device, verify board name |
//! | `subdevice_enumeration_test` | Verify AI/AO/DIO/Counter subdevices |
//! | `board_info_test` | Read driver name, board name, n_subdevices |
//! | `analog_input_ranges_test` | Query voltage ranges for AI channel 0 |
//! | `analog_input_single_read_test` | Read single sample from AI channel 0 |

#![cfg(feature = "hardware")]

use daq_driver_comedi::{ComediDevice, DeviceInfo, Range, SubdeviceType};
use std::env;

// =============================================================================
// Test Configuration
// =============================================================================

/// Check if smoke test is enabled via environment variable
fn smoke_test_enabled() -> bool {
    env::var("COMEDI_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default to /dev/comedi0
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if smoke test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Comedi smoke test skipped (set COMEDI_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// NI PCI-MIO-16XE-10 Expected Configuration
// =============================================================================

/// Expected board name for NI PCI-MIO-16XE-10
const EXPECTED_BOARD_NAME: &str = "pci-mio-16xe-10";

/// Expected driver name
const EXPECTED_DRIVER_NAME: &str = "ni_pcimio";

/// Expected number of analog input channels
const EXPECTED_AI_CHANNELS: u32 = 16;

/// Expected number of analog output channels
const EXPECTED_AO_CHANNELS: u32 = 2;

/// Expected number of digital I/O channels
const EXPECTED_DIO_CHANNELS: u32 = 8;

// =============================================================================
// Test 1: Device Discovery
// =============================================================================

/// Test that we can open the Comedi device and get basic info
///
/// This test verifies:
/// 1. Device path is accessible
/// 2. Board name matches expected (ni_pcimio/pci-mio-16xe-10)
/// 3. Device can be opened without error
#[test]
fn device_discovery_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Device Discovery Test ===");
    println!("Device path: {}", path);

    // Step 1: Open device
    println!("\n[1/3] Opening device...");
    let device = ComediDevice::open(&path)
        .expect("Failed to open Comedi device - check that /dev/comedi0 exists and has correct permissions");

    // Step 2: Get board name
    println!("[2/3] Reading board name...");
    let board_name = device.board_name();
    println!("  Board name: {}", board_name);

    // Verify board name (case-insensitive comparison)
    assert!(
        board_name.to_lowercase().contains("mio-16xe-10")
            || board_name.to_lowercase().contains("pci-mio-16xe"),
        "Expected NI PCI-MIO-16XE-10 board, got: {}",
        board_name
    );

    // Step 3: Get driver name
    println!("[3/3] Reading driver name...");
    let driver_name = device.driver_name();
    println!("  Driver name: {}", driver_name);

    assert!(
        driver_name.contains("ni_pcimio") || driver_name.contains("ni_mio"),
        "Expected ni_pcimio driver, got: {}",
        driver_name
    );

    println!("\n=== Device Discovery Test PASSED ===");
}

// =============================================================================
// Test 2: Subdevice Enumeration
// =============================================================================

/// Test that all expected subdevices are present and have correct channel counts
///
/// NI PCI-MIO-16XE-10 should have:
/// - Analog Input: 16 channels (100 kS/s aggregate)
/// - Analog Output: 2 channels
/// - Digital I/O: 8 channels
/// - Counter/Timer: 2 counters (via 8254 chip)
#[test]
fn subdevice_enumeration_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Subdevice Enumeration Test ===");
    println!("Device path: {}", path);

    let device = ComediDevice::open(&path).expect("Failed to open Comedi device");

    // Get device info
    let info: DeviceInfo = device.info().expect("Failed to get device info");

    println!("\nDevice: {} ({})", info.board_name, info.driver_name);
    println!("Number of subdevices: {}", info.n_subdevices);
    println!("\nSubdevice details:");

    let mut found_ai = false;
    let mut found_ao = false;
    let mut found_dio = false;
    let mut found_counter = false;

    for (i, subdev) in info.subdevices.iter().enumerate() {
        let type_name = match subdev.subdev_type {
            SubdeviceType::AnalogInput => {
                found_ai = true;
                println!(
                    "  [{}] Analog Input: {} channels, {} ranges, {}-bit resolution",
                    i,
                    subdev.n_channels,
                    subdev.n_ranges,
                    subdev.resolution_bits()
                );
                assert!(
                    subdev.n_channels >= EXPECTED_AI_CHANNELS,
                    "Expected at least {} AI channels, got {}",
                    EXPECTED_AI_CHANNELS,
                    subdev.n_channels
                );
                "AI"
            }
            SubdeviceType::AnalogOutput => {
                found_ao = true;
                println!(
                    "  [{}] Analog Output: {} channels, {} ranges, {}-bit resolution",
                    i,
                    subdev.n_channels,
                    subdev.n_ranges,
                    subdev.resolution_bits()
                );
                assert!(
                    subdev.n_channels >= EXPECTED_AO_CHANNELS,
                    "Expected at least {} AO channels, got {}",
                    EXPECTED_AO_CHANNELS,
                    subdev.n_channels
                );
                "AO"
            }
            SubdeviceType::DigitalIO => {
                found_dio = true;
                println!("  [{}] Digital I/O: {} channels", i, subdev.n_channels);
                assert!(
                    subdev.n_channels >= EXPECTED_DIO_CHANNELS,
                    "Expected at least {} DIO channels, got {}",
                    EXPECTED_DIO_CHANNELS,
                    subdev.n_channels
                );
                "DIO"
            }
            SubdeviceType::Counter => {
                found_counter = true;
                println!("  [{}] Counter: {} channels", i, subdev.n_channels);
                "Counter"
            }
            SubdeviceType::Timer => {
                println!("  [{}] Timer: {} channels", i, subdev.n_channels);
                "Timer"
            }
            SubdeviceType::Calibration => {
                println!("  [{}] Calibration", i);
                "Calib"
            }
            SubdeviceType::Memory => {
                println!("  [{}] EEPROM/Memory", i);
                "Memory"
            }
            other => {
                println!("  [{}] {:?}: {} channels", i, other, subdev.n_channels);
                "Other"
            }
        };

        // Print flags
        let mut flags = Vec::new();
        if subdev.is_readable() {
            flags.push("readable");
        }
        if subdev.is_writable() {
            flags.push("writable");
        }
        if subdev.supports_commands() {
            flags.push("async");
        }
        if !flags.is_empty() {
            println!("       Flags: {}", flags.join(", "));
        }
        let _ = type_name; // Silence unused warning
    }

    // Verify all expected subdevices were found
    assert!(
        found_ai,
        "Analog Input subdevice not found - is this the right board?"
    );
    assert!(
        found_ao,
        "Analog Output subdevice not found - is this the right board?"
    );
    assert!(
        found_dio,
        "Digital I/O subdevice not found - is this the right board?"
    );
    // Counter is optional, some configs may not expose it
    if !found_counter {
        println!("\n  Note: Counter subdevice not found (may be expected for some configs)");
    }

    println!("\n=== Subdevice Enumeration Test PASSED ===");
}

// =============================================================================
// Test 3: Board Info
// =============================================================================

/// Test comprehensive board info retrieval
#[test]
fn board_info_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Board Info Test ===");
    println!("Device path: {}", path);

    let device = ComediDevice::open(&path).expect("Failed to open Comedi device");

    // Get basic info
    let board_name = device.board_name();
    let driver_name = device.driver_name();
    let n_subdevices = device.n_subdevices();

    println!("\nBoard Information:");
    println!("  Board name: {}", board_name);
    println!("  Driver name: {}", driver_name);
    println!("  Number of subdevices: {}", n_subdevices);
    println!("  Device path: {}", device.path());

    // Verify info is reasonable
    assert!(!board_name.is_empty(), "Board name should not be empty");
    assert!(
        board_name != "unknown",
        "Board name should be recognized, got 'unknown'"
    );
    assert!(!driver_name.is_empty(), "Driver name should not be empty");
    assert!(
        driver_name != "unknown",
        "Driver name should be recognized, got 'unknown'"
    );
    assert!(
        n_subdevices > 0,
        "Device should have at least one subdevice"
    );
    assert!(
        n_subdevices < 100,
        "Subdevice count seems unreasonably high: {}",
        n_subdevices
    );

    // Get full device info
    let info = device.info().expect("Failed to get device info");
    assert_eq!(
        info.subdevices.len(),
        n_subdevices as usize,
        "Subdevice count should match"
    );

    println!("\n=== Board Info Test PASSED ===");
}

// =============================================================================
// Test 4: Analog Input Ranges
// =============================================================================

/// Test querying voltage ranges for analog input
#[test]
fn analog_input_ranges_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Analog Input Ranges Test ===");
    println!("Device path: {}", path);

    let device = ComediDevice::open(&path).expect("Failed to open Comedi device");

    // Get analog input subsystem
    let ai = device
        .analog_input()
        .expect("Failed to get analog input subsystem");

    println!("\nAnalog Input Subsystem:");
    println!("  Number of channels: {}", ai.n_channels());
    println!("  Max data value: {}", ai.maxdata());
    println!("  Resolution: {} bits", ai.resolution_bits());

    // Query ranges for channel 0
    let n_ranges = ai.n_ranges(0).expect("Failed to get number of ranges");
    println!("\nVoltage ranges for channel 0 ({} total):", n_ranges);

    for i in 0..n_ranges.min(10) {
        // Limit to first 10 ranges
        let range = ai.get_range(0, i).expect("Failed to get range");
        println!(
            "  [{}] {} to {} {} ({})",
            i,
            range.min,
            range.max,
            range.unit_description(),
            if range.is_bipolar() {
                "bipolar"
            } else {
                "unipolar"
            }
        );
    }

    // Verify we have at least one range
    assert!(n_ranges > 0, "Should have at least one voltage range");

    // Check for common NI ranges (typically ±10V, ±5V, ±1V, etc.)
    let range0 = ai.get_range(0, 0).expect("Failed to get range 0");
    println!(
        "\nDefault range span: {} {}",
        range0.span(),
        range0.unit_description()
    );

    println!("\n=== Analog Input Ranges Test PASSED ===");
}

// =============================================================================
// Test 5: Single Sample Read
// =============================================================================

/// Test reading a single sample from analog input
#[test]
fn analog_input_single_read_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Analog Input Single Read Test ===");
    println!("Device path: {}", path);

    let device = ComediDevice::open(&path).expect("Failed to open Comedi device");

    let ai = device
        .analog_input()
        .expect("Failed to get analog input subsystem");

    println!("\nReading from channel 0...");

    // Read raw value
    let raw = ai.read_raw(0, Range::default()).expect("Failed to read raw value");
    println!("  Raw value: {}", raw);

    // Read voltage
    let voltage = ai
        .read_voltage(0, Range::default())
        .expect("Failed to read voltage");
    println!("  Voltage: {:.6} V", voltage);

    // Verify values are reasonable
    assert!(
        raw <= ai.maxdata(),
        "Raw value {} should be <= maxdata {}",
        raw,
        ai.maxdata()
    );

    // Voltage should be within the range (typically ±10V for NI cards)
    assert!(
        voltage >= -15.0 && voltage <= 15.0,
        "Voltage {:.3}V seems out of reasonable range",
        voltage
    );

    // Test reading from multiple channels
    println!("\nReading from multiple channels:");
    for ch in 0..4.min(ai.n_channels()) {
        let v = ai.read_voltage(ch, Range::default()).unwrap_or(f64::NAN);
        println!("  Channel {}: {:.6} V", ch, v);
    }

    println!("\n=== Analog Input Single Read Test PASSED ===");
}

// =============================================================================
// Test 6: Analog Input with ACH1→ACH2 Loopback (if configured)
// =============================================================================

/// Test analog loopback if ACH1→ACH2 is physically connected
/// This test is informational - it won't fail if loopback isn't connected
#[test]
fn analog_loopback_info_test() {
    skip_if_disabled!();

    let path = device_path();
    println!("=== Comedi Analog Loopback Info Test ===");
    println!("Device path: {}", path);
    println!("Note: This test requires ACH1 to be physically connected to ACH2");

    let device = ComediDevice::open(&path).expect("Failed to open Comedi device");

    let ai = device
        .analog_input()
        .expect("Failed to get analog input subsystem");

    // Read both channels
    let ch1_voltage = ai
        .read_voltage(1, Range::default())
        .expect("Failed to read channel 1");
    let ch2_voltage = ai
        .read_voltage(2, Range::default())
        .expect("Failed to read channel 2");

    println!("\nChannel readings:");
    println!("  ACH1: {:.6} V", ch1_voltage);
    println!("  ACH2: {:.6} V", ch2_voltage);

    let difference = (ch1_voltage - ch2_voltage).abs();
    println!("  Difference: {:.6} V", difference);

    // If loopback is connected, they should be very close
    if difference < 0.05 {
        println!("\n  Loopback appears to be connected (difference < 50mV)");
    } else {
        println!("\n  Loopback may not be connected or signals differ");
        println!("  (This is informational - test still passes)");
    }

    println!("\n=== Analog Loopback Info Test PASSED ===");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that smoke test is properly skipped when not enabled
#[test]
fn smoke_test_skip_check() {
    let enabled = smoke_test_enabled();
    if !enabled {
        println!("Comedi smoke test correctly disabled (COMEDI_SMOKE_TEST not set)");
    } else {
        println!("Comedi smoke test enabled via COMEDI_SMOKE_TEST=1");
    }
}
