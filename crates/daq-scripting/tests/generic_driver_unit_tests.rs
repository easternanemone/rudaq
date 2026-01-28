#![cfg(feature = "generic_driver")]
//! Unit Tests for GenericDriverHandle
//!
//! Tests for GenericDriverHandle that don't require hardware or serial ports.
//! Focuses on type behavior, soft limits, and config loading.
//!
//! Run with: `cargo nextest run -p daq-scripting --test generic_driver_unit_tests`

use daq_hardware::config::load_device_config;
use daq_scripting::SoftLimits;
use std::path::PathBuf;

/// Get path to ELL14 config file
fn ell14_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml")
}

// =============================================================================
// SoftLimits Tests
// =============================================================================

#[test]
fn test_soft_limits_validation_within_range() {
    let limits = SoftLimits::new(0.0, 360.0);
    assert!(limits.validate(0.0).is_ok(), "Min boundary should be valid");
    assert!(
        limits.validate(180.0).is_ok(),
        "Middle value should be valid"
    );
    assert!(
        limits.validate(360.0).is_ok(),
        "Max boundary should be valid"
    );
}

#[test]
fn test_soft_limits_validation_outside_range() {
    let limits = SoftLimits::new(0.0, 360.0);
    assert!(
        limits.validate(-0.1).is_err(),
        "Below min should be invalid"
    );
    assert!(
        limits.validate(-10.0).is_err(),
        "Well below min should be invalid"
    );
    assert!(
        limits.validate(360.1).is_err(),
        "Above max should be invalid"
    );
    assert!(
        limits.validate(400.0).is_err(),
        "Well above max should be invalid"
    );
}

#[test]
fn test_soft_limits_unlimited() {
    let limits = SoftLimits::unlimited();
    // Unlimited should accept any value
    assert!(
        limits.validate(f64::MIN).is_ok(),
        "Min f64 should be valid for unlimited"
    );
    assert!(
        limits.validate(f64::MAX).is_ok(),
        "Max f64 should be valid for unlimited"
    );
    assert!(
        limits.validate(0.0).is_ok(),
        "Zero should be valid for unlimited"
    );
    assert!(
        limits.validate(-1000.0).is_ok(),
        "Negative should be valid for unlimited"
    );
    assert!(
        limits.validate(1000.0).is_ok(),
        "Positive should be valid for unlimited"
    );
}

#[test]
fn test_soft_limits_partial_min_only() {
    let limits = SoftLimits {
        min: Some(0.0),
        max: None,
    };
    assert!(limits.validate(0.0).is_ok(), "At min should be valid");
    assert!(limits.validate(1000.0).is_ok(), "Above min should be valid");
    assert!(
        limits.validate(-1.0).is_err(),
        "Below min should be invalid"
    );
}

#[test]
fn test_soft_limits_partial_max_only() {
    let limits = SoftLimits {
        min: None,
        max: Some(360.0),
    };
    assert!(limits.validate(360.0).is_ok(), "At max should be valid");
    assert!(
        limits.validate(-1000.0).is_ok(),
        "Below max should be valid"
    );
    assert!(
        limits.validate(361.0).is_err(),
        "Above max should be invalid"
    );
}

// =============================================================================
// Config Loading Tests
// =============================================================================

#[test]
fn test_config_loading_ell14() {
    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load ELL14 config");

    // Verify basic config fields
    assert_eq!(
        config.device.name, "Thorlabs ELL14",
        "Device name should match"
    );
    assert_eq!(
        config.connection.baud_rate, 9600,
        "Baud rate should be 9600"
    );

    // Verify commands are loaded
    assert!(
        config.commands.contains_key("move_absolute"),
        "Should have move_absolute command"
    );
    assert!(
        config.commands.contains_key("get_position"),
        "Should have get_position command"
    );
}

#[test]
fn test_config_loading_nonexistent_file() {
    let result = load_device_config(std::path::Path::new("/nonexistent/config.toml"));
    assert!(result.is_err(), "Loading nonexistent config should fail");
}

#[test]
fn test_config_has_trait_mappings() {
    let config_path = ell14_config_path();
    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found at {:?}", config_path);
        return;
    }

    let config = load_device_config(&config_path).expect("Failed to load ELL14 config");

    // ELL14 should have Movable trait mapping
    assert!(
        config.trait_mapping.contains_key("Movable"),
        "ELL14 should have Movable trait mapping"
    );

    let movable = config.trait_mapping.get("Movable").unwrap();
    assert!(
        movable.methods.contains_key("move_abs"),
        "Movable should map move_abs"
    );
    assert!(
        movable.methods.contains_key("position"),
        "Movable should map position"
    );
}

// =============================================================================
// GenericDriverHandle Type Tests
// =============================================================================

#[test]
fn test_soft_limits_new_constructor() {
    let limits = SoftLimits::new(10.0, 20.0);
    assert_eq!(limits.min, Some(10.0));
    assert_eq!(limits.max, Some(20.0));
}

#[test]
fn test_soft_limits_clone() {
    let limits = SoftLimits::new(0.0, 100.0);
    let cloned = limits.clone();
    assert_eq!(limits.min, cloned.min);
    assert_eq!(limits.max, cloned.max);
}

#[test]
fn test_soft_limits_error_message_below_min() {
    let limits = SoftLimits::new(0.0, 360.0);
    let result = limits.validate(-10.0);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("-10") || err.contains("below"),
        "Error should mention the value or 'below': {}",
        err
    );
}

#[test]
fn test_soft_limits_error_message_above_max() {
    let limits = SoftLimits::new(0.0, 360.0);
    let result = limits.validate(400.0);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("400") || err.contains("above"),
        "Error should mention the value or 'above': {}",
        err
    );
}
