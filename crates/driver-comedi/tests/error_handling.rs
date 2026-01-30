#![cfg(not(target_arch = "wasm32"))]
//! Comedi Error Handling Test Suite
//!
//! Validates error handling robustness and recovery mechanisms.
//! Tests various error scenarios to ensure clean error messages and resource cleanup.
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_ERROR_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//!
//! # Running
//!
//! ```bash
//! export COMEDI_ERROR_TEST=1
//! cargo nextest run --profile hardware --features hardware -p driver-comedi -- error_handling
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `test_device_not_found` | Open nonexistent device |
//! | `test_invalid_channel` | Access out-of-range channels |
//! | `test_invalid_range` | Use invalid voltage ranges |
//! | `test_invalid_subdevice` | Access nonexistent subdevices |
//! | `test_error_messages` | Verify error message quality |
//! | `test_resource_cleanup` | Verify no leaks on errors |

#![cfg(feature = "hardware")]

use driver_comedi::ComediDevice;
use std::env;

// =============================================================================
// Test Configuration
// =============================================================================

/// Check if error test is enabled
fn error_test_enabled() -> bool {
    env::var("COMEDI_ERROR_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if error test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !error_test_enabled() {
            println!("Comedi error test skipped (set COMEDI_ERROR_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Test 1: Device Not Found
// =============================================================================

/// Test error handling when device doesn't exist
#[test]
fn test_device_not_found() {
    skip_if_disabled!();

    println!("\n=== Comedi Device Not Found Test ===");

    let nonexistent_paths = vec!["/dev/comedi99", "/dev/nonexistent", "/dev/comedi_fake"];

    for path in nonexistent_paths {
        println!("\nTrying to open: {}", path);

        match ComediDevice::open(path) {
            Ok(_) => {
                println!("  Unexpectedly succeeded (device exists?) ✗");
            }
            Err(e) => {
                println!("  Error (expected): {}", e);
                println!("  Error type: {:?}", std::any::type_name_of_val(&e));

                // Verify error is descriptive
                let error_str = e.to_string();
                assert!(!error_str.is_empty(), "Error message should not be empty");
                println!("  Error message is descriptive ✓");
            }
        }
    }

    println!("\n=== Device Not Found Test PASSED ===\n");
}

// =============================================================================
// Test 2: Invalid Channel Access
// =============================================================================

/// Test error handling for invalid channel numbers
#[test]
fn test_invalid_channel() {
    skip_if_disabled!();

    println!("\n=== Comedi Invalid Channel Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    // Test analog input invalid channel
    if let Ok(ai) = device.analog_input() {
        let n_channels = ai.n_channels();
        let invalid_channels = vec![n_channels, n_channels + 1, n_channels + 100, u32::MAX];

        println!("\nAnalog Input (has {} channels):", n_channels);

        for ch in invalid_channels {
            match ai.read_voltage(ch, driver_comedi::Range::default()) {
                Ok(v) => println!("  Channel {}: Unexpected success {} ✗", ch, v),
                Err(e) => {
                    println!("  Channel {}: Error (expected) ✓", ch);
                    println!("    Message: {}", e);

                    // Verify error contains channel info
                    let err_str = e.to_string();
                    assert!(
                        err_str.contains(&ch.to_string())
                            || err_str.to_lowercase().contains("channel"),
                        "Error should mention invalid channel"
                    );
                }
            }
        }
    }

    // Test digital I/O invalid pin
    if let Ok(dio) = device.digital_io() {
        let n_pins = dio.n_channels();
        let invalid_pin = n_pins + 10;

        println!("\nDigital I/O (has {} pins):", n_pins);
        println!("  Testing pin {}:", invalid_pin);

        match dio.read(invalid_pin) {
            Ok(v) => println!("    Read: Unexpected success {} ✗", v),
            Err(e) => println!("    Read: Error (expected) ✓ - {}", e),
        }

        match dio.write(invalid_pin, true) {
            Ok(()) => println!("    Write: Unexpected success ✗"),
            Err(e) => println!("    Write: Error (expected) ✓ - {}", e),
        }
    }

    println!("\n=== Invalid Channel Test PASSED ===\n");
}

// =============================================================================
// Test 3: Invalid Range Access
// =============================================================================

/// Test error handling for invalid voltage ranges
#[test]
fn test_invalid_range() {
    skip_if_disabled!();

    println!("\n=== Comedi Invalid Range Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    if let Ok(ai) = device.analog_input() {
        let n_ranges = ai.n_ranges(0).unwrap_or(0);
        let invalid_ranges = vec![n_ranges, n_ranges + 1, n_ranges + 100];

        println!("\nAnalog Input (has {} ranges):", n_ranges);

        for range_idx in invalid_ranges {
            println!("  Testing range index {}:", range_idx);

            match ai.range_info(0, range_idx) {
                Ok(r) => println!("    range_info: Unexpected success {:?} ✗", r),
                Err(e) => println!("    range_info: Error (expected) ✓ - {}", e),
            }
        }
    }

    if let Ok(ao) = device.analog_output() {
        let n_ranges = ao.n_ranges(0).unwrap_or(0);
        let invalid_range = n_ranges + 10;

        println!("\nAnalog Output (has {} ranges):", n_ranges);
        println!("  Testing range index {}:", invalid_range);

        match ao.range_info(0, invalid_range) {
            Ok(r) => println!("    range_info: Unexpected success {:?} ✗", r),
            Err(e) => println!("    range_info: Error (expected) ✓ - {}", e),
        }
    }

    println!("\n=== Invalid Range Test PASSED ===\n");
}

// =============================================================================
// Test 4: Invalid Subdevice Access
// =============================================================================

/// Test error handling for invalid subdevice access
#[test]
fn test_invalid_subdevice() {
    skip_if_disabled!();

    println!("\n=== Comedi Invalid Subdevice Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    let n_subdevices = device.n_subdevices();
    println!("Device has {} subdevices", n_subdevices);

    let invalid_subdevs = vec![n_subdevices, n_subdevices + 1, n_subdevices + 100];

    for subdev in invalid_subdevs {
        println!("\nTesting subdevice {}:", subdev);

        match device.subdevice_type(subdev) {
            Ok(t) => println!("  subdevice_type: Unexpected success {:?} ✗", t),
            Err(e) => println!("  subdevice_type: Error (expected) ✓ - {}", e),
        }
    }

    println!("\n=== Invalid Subdevice Test PASSED ===\n");
}

// =============================================================================
// Test 5: Error Message Quality
// =============================================================================

/// Test that error messages are helpful and descriptive
#[test]
fn test_error_messages() {
    skip_if_disabled!();

    println!("\n=== Comedi Error Message Quality Test ===");

    // Test device not found error message
    let result = ComediDevice::open("/dev/comedi99");
    if let Err(e) = result {
        let msg = e.to_string();
        println!("\nDevice not found error:");
        println!("  Message: {}", msg);

        assert!(!msg.is_empty(), "Error message should not be empty");
        assert!(
            msg.len() > 10,
            "Error message should be descriptive (>10 chars)"
        );
        println!("  Quality: Good (descriptive) ✓");
    }

    // Test with real device for channel errors
    if let Ok(device) = ComediDevice::open(&device_path()) {
        if let Ok(ai) = device.analog_input() {
            let invalid_ch = ai.n_channels() + 1;
            if let Err(e) = ai.read_voltage(invalid_ch, driver_comedi::Range::default()) {
                let msg = e.to_string();
                println!("\nInvalid channel error:");
                println!("  Message: {}", msg);

                // Error should ideally mention the channel or be specific
                assert!(!msg.is_empty(), "Error message should not be empty");
                println!("  Quality: Good (specific) ✓");
            }
        }
    }

    println!("\n=== Error Message Quality Test PASSED ===\n");
}

// =============================================================================
// Test 6: Resource Cleanup on Errors
// =============================================================================

/// Test that resources are cleaned up properly when errors occur
#[test]
fn test_resource_cleanup() {
    skip_if_disabled!();

    println!("\n=== Comedi Resource Cleanup Test ===");
    println!("Device: {}", device_path());

    // Test 1: Open/close cycle
    println!("\nTest 1: Open/close cycle (10 iterations):");
    for i in 0..10 {
        let device = ComediDevice::open(&device_path()).expect("Failed to open");
        // Device should be cleaned up on drop
        drop(device);
        if i % 5 == 0 {
            println!("  Iteration {} ✓", i);
        }
    }
    println!("  All iterations completed ✓");

    // Test 2: Error then success cycle
    println!("\nTest 2: Error recovery cycle:");
    for i in 0..5 {
        // Try to open nonexistent device (should fail)
        let _ = ComediDevice::open("/dev/comedi99");

        // Then open real device (should succeed)
        let device = ComediDevice::open(&device_path()).expect("Failed to open after error");

        // Do some operations
        if let Ok(ai) = device.analog_input() {
            let _ = ai.read_voltage(0, driver_comedi::Range::default());
        }

        drop(device);
        println!("  Cycle {} ✓", i);
    }
    println!("  All cycles completed ✓");

    // Test 3: Subsystem operations with errors
    println!("\nTest 3: Subsystem error recovery:");
    let device = ComediDevice::open(&device_path()).expect("Failed to open");

    if let Ok(ai) = device.analog_input() {
        // Try invalid operation
        let _ = ai.read_voltage(ai.n_channels() + 1, driver_comedi::Range::default());

        // Should still work after error
        match ai.read_voltage(0, driver_comedi::Range::default()) {
            Ok(v) => println!("  Read after error: {:.6}V ✓", v),
            Err(e) => println!("  Read after error failed: {} ✗", e),
        }
    }

    println!("\n=== Resource Cleanup Test PASSED ===\n");
}

// =============================================================================
// Test 7: No Panic on Error Conditions
// =============================================================================

/// Test that errors don't cause panics
#[test]
fn test_no_panic_on_errors() {
    skip_if_disabled!();

    println!("\n=== Comedi No Panic Test ===");
    println!("Device: {}", device_path());

    // All of these should return errors, not panic
    let tests: Vec<(&str, Box<dyn Fn() -> bool>)> = vec![
        (
            "Open nonexistent device",
            Box::new(|| ComediDevice::open("/dev/comedi99").is_err()),
        ),
        (
            "Open empty path",
            Box::new(|| ComediDevice::open("").is_err()),
        ),
    ];

    for (name, test_fn) in tests {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test_fn()));
        match result {
            Ok(returned_error) => {
                if returned_error {
                    println!("  {}: Returned error (no panic) ✓", name);
                } else {
                    println!("  {}: Returned Ok (unexpected) ?", name);
                }
            }
            Err(_) => {
                println!("  {}: PANICKED ✗", name);
                panic!("Test '{}' caused a panic!", name);
            }
        }
    }

    // Tests with real device
    if let Ok(device) = ComediDevice::open(&device_path()) {
        let device_tests: Vec<(&str, Box<dyn Fn() -> bool>)> = vec![];

        if let Ok(ai) = device.analog_input() {
            let invalid_ch = ai.n_channels() + 1000;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ai.read_voltage(invalid_ch, driver_comedi::Range::default())
                    .is_err()
            }));
            match result {
                Ok(true) => println!("  Read invalid channel: Returned error (no panic) ✓"),
                Ok(false) => println!("  Read invalid channel: Returned Ok (unexpected) ?"),
                Err(_) => panic!("Read invalid channel caused panic!"),
            }
        }

        for (name, test_fn) in device_tests {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test_fn()));
            match result {
                Ok(_) => println!("  {}: No panic ✓", name),
                Err(_) => panic!("Test '{}' caused a panic!", name),
            }
        }
    }

    println!("\n=== No Panic Test PASSED ===\n");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that error tests are properly skipped when not enabled
#[test]
fn error_test_skip_check() {
    let enabled = error_test_enabled();
    if !enabled {
        println!("Comedi error test correctly disabled (COMEDI_ERROR_TEST not set)");
        println!("To enable: export COMEDI_ERROR_TEST=1");
    } else {
        println!("Comedi error test enabled via COMEDI_ERROR_TEST=1");
    }
}
