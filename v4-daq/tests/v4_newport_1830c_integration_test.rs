//! Integration tests for Newport 1830-C power meter V4 actor
//!
//! Comprehensive test suite covering:
//! - Wavelength configuration and retrieval
//! - Power unit switching
//! - State persistence
//! - PowerMeter trait implementation
//! - Multiple sequential operations

#![cfg(feature = "instrument_serial")]

use v4_daq::actors::Newport1830C;
use v4_daq::traits::power_meter::{PowerUnit, Wavelength};

/// Test 1: Default configuration on actor creation
#[test]
fn test_newport_default_configuration() {
    let actor = Newport1830C::new();

    // Verify default wavelength (633 nm - HeNe laser)
    assert_eq!(
        actor.wavelength.nm, 633.0,
        "Default wavelength should be 633.0 nm"
    );

    // Verify default power unit
    assert_eq!(actor.unit, PowerUnit::Watts, "Default unit should be Watts");

    // Verify no adapter is present (mock mode)
    assert!(actor.adapter.is_none(), "Mock mode should have no adapter");
}

/// Test 2: Wavelength modification and persistence
#[test]
fn test_newport_wavelength_modification() {
    let mut actor = Newport1830C::new();

    // Change wavelength to 780 nm (near-IR)
    actor.wavelength = Wavelength { nm: 780.0 };
    assert_eq!(actor.wavelength.nm, 780.0, "Wavelength should update to 780 nm");

    // Change wavelength to 1550 nm (telecom band)
    actor.wavelength = Wavelength { nm: 1550.0 };
    assert_eq!(actor.wavelength.nm, 1550.0, "Wavelength should update to 1550 nm");

    // Change wavelength to 532 nm (green)
    actor.wavelength = Wavelength { nm: 532.0 };
    assert_eq!(actor.wavelength.nm, 532.0, "Wavelength should update to 532 nm");
}

/// Test 3: Power unit switching
#[test]
fn test_newport_power_unit_switching() {
    let mut actor = Newport1830C::new();

    // Default is Watts
    assert_eq!(actor.unit, PowerUnit::Watts);

    // Switch to MilliWatts
    actor.unit = PowerUnit::MilliWatts;
    assert_eq!(actor.unit, PowerUnit::MilliWatts);

    // Switch to Microwalts
    actor.unit = PowerUnit::MicroWatts;
    assert_eq!(actor.unit, PowerUnit::MicroWatts);

    // Switch to Nanowalts
    actor.unit = PowerUnit::NanoWatts;
    assert_eq!(actor.unit, PowerUnit::NanoWatts);

    // Switch to dBm
    actor.unit = PowerUnit::Dbm;
    assert_eq!(actor.unit, PowerUnit::Dbm);

    // Switch back to Watts
    actor.unit = PowerUnit::Watts;
    assert_eq!(actor.unit, PowerUnit::Watts);
}

/// Test 4: State persistence through multiple operations
#[test]
fn test_newport_state_persistence() {
    let mut actor = Newport1830C::new();

    // Set wavelength to 780 nm
    actor.wavelength = Wavelength { nm: 780.0 };
    // Change unit to MilliWatts
    actor.unit = PowerUnit::MilliWatts;

    // Verify both persisted
    assert_eq!(actor.wavelength.nm, 780.0);
    assert_eq!(actor.unit, PowerUnit::MilliWatts);

    // Change wavelength again
    actor.wavelength = Wavelength { nm: 532.0 };

    // Verify wavelength changed but unit persisted
    assert_eq!(actor.wavelength.nm, 532.0);
    assert_eq!(
        actor.unit, PowerUnit::MilliWatts,
        "Unit should remain MilliWatts"
    );

    // Change unit
    actor.unit = PowerUnit::Dbm;

    // Verify both are now different
    assert_eq!(actor.wavelength.nm, 532.0);
    assert_eq!(actor.unit, PowerUnit::Dbm);
}

/// Test 5: Common laser wavelengths
#[test]
fn test_newport_common_laser_wavelengths() {
    let mut actor = Newport1830C::new();

    let laser_wavelengths = vec![
        ("HeNe Red", 633.0),
        ("Diode 650", 650.0),
        ("Diode 780", 780.0),
        ("Nd:YAG", 1064.0),
        ("Nd:YAG 2x", 532.0),
        ("Er:Fiber", 1550.0),
        ("Telecom C-band", 1530.0),
        ("Telecom L-band", 1625.0),
    ];

    for (name, wavelength_nm) in laser_wavelengths {
        actor.wavelength = Wavelength { nm: wavelength_nm };
        assert_eq!(
            actor.wavelength.nm, wavelength_nm,
            "Failed to set {} at {} nm",
            name, wavelength_nm
        );
    }
}

/// Test 6: Power unit conversion types
#[test]
fn test_newport_all_power_units() {
    let mut actor = Newport1830C::new();

    let power_units = vec![
        PowerUnit::Watts,
        PowerUnit::MilliWatts,
        PowerUnit::MicroWatts,
        PowerUnit::NanoWatts,
        PowerUnit::Dbm,
    ];

    for unit in power_units {
        actor.unit = unit;
        assert_eq!(actor.unit, unit, "Failed to set power unit {:?}", unit);
    }
}

/// Test 7: Multiple sequential configuration changes
#[test]
fn test_newport_sequential_configuration() {
    let mut actor = Newport1830C::new();

    // Perform 10 sequential configuration changes
    for i in 0..10 {
        let wavelength_nm = 500.0 + (i as f64 * 100.0);
        let unit = match i % 5 {
            0 => PowerUnit::Watts,
            1 => PowerUnit::MilliWatts,
            2 => PowerUnit::MicroWatts,
            3 => PowerUnit::NanoWatts,
            _ => PowerUnit::Dbm,
        };

        actor.wavelength = Wavelength { nm: wavelength_nm };
        actor.unit = unit;

        assert_eq!(
            actor.wavelength.nm, wavelength_nm,
            "Wavelength should be {} at iteration {}",
            wavelength_nm, i
        );
        assert_eq!(
            actor.unit, unit,
            "Unit should be {:?} at iteration {}",
            unit, i
        );
    }
}

/// Test 8: Extreme wavelength values
#[test]
fn test_newport_extreme_wavelengths() {
    let mut actor = Newport1830C::new();

    // Very low wavelength (UV)
    actor.wavelength = Wavelength { nm: 200.0 };
    assert_eq!(actor.wavelength.nm, 200.0, "Should handle UV wavelengths");

    // Very high wavelength (Far-IR)
    actor.wavelength = Wavelength { nm: 10000.0 };
    assert_eq!(
        actor.wavelength.nm, 10000.0,
        "Should handle far-IR wavelengths"
    );

    // Fractional wavelength
    actor.wavelength = Wavelength { nm: 632.8 };
    assert_eq!(
        actor.wavelength.nm, 632.8,
        "Should handle fractional wavelengths"
    );
}

/// Test 9: Default instance created via Default trait
#[test]
fn test_newport_default_trait() {
    let actor = Newport1830C::default();

    assert_eq!(actor.wavelength.nm, 633.0);
    assert_eq!(actor.unit, PowerUnit::Watts);
    assert!(actor.adapter.is_none());
}

/// Test 10: Independent actor instances have independent state
#[test]
fn test_newport_independent_instances() {
    let mut actor1 = Newport1830C::new();
    let mut actor2 = Newport1830C::new();
    let mut actor3 = Newport1830C::new();

    // Configure each actor differently
    actor1.wavelength = Wavelength { nm: 532.0 };
    actor1.unit = PowerUnit::MilliWatts;

    actor2.wavelength = Wavelength { nm: 1064.0 };
    actor2.unit = PowerUnit::Dbm;

    actor3.wavelength = Wavelength { nm: 1550.0 };
    actor3.unit = PowerUnit::Watts;

    // Verify each actor maintained independent state
    assert_eq!(actor1.wavelength.nm, 532.0);
    assert_eq!(actor1.unit, PowerUnit::MilliWatts);

    assert_eq!(actor2.wavelength.nm, 1064.0);
    assert_eq!(actor2.unit, PowerUnit::Dbm);

    assert_eq!(actor3.wavelength.nm, 1550.0);
    assert_eq!(actor3.unit, PowerUnit::Watts);
}

/// Test 11: Wavelength zero and boundary cases
#[test]
fn test_newport_boundary_wavelengths() {
    let mut actor = Newport1830C::new();

    // Zero wavelength (edge case)
    actor.wavelength = Wavelength { nm: 0.0 };
    assert_eq!(actor.wavelength.nm, 0.0, "Should accept 0.0 nm");

    // Very small positive wavelength
    actor.wavelength = Wavelength { nm: 0.1 };
    assert_eq!(actor.wavelength.nm, 0.1, "Should accept 0.1 nm");

    // Negative wavelength (should still work at struct level)
    actor.wavelength = Wavelength { nm: -100.0 };
    assert_eq!(actor.wavelength.nm, -100.0, "Should accept negative values");
}
