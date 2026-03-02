#![cfg(not(target_arch = "wasm32"))]
//! Dynamic Parameter Enumeration Tests (bd-c4hf.6)
//!
//! Verifies that PVCAM driver parameters expose dynamic choices (dropdowns)
//! rather than free-form text fields. Tests cover:
//!
//! 1. **Initial enumeration**: readout.port, readout.speed_mode, readout.gain_mode
//!    have `dtype="enum"` and non-empty `enum_values` after driver creation
//! 2. **Dependency refresh**: changing readout.port cascades to speed_mode and gain_mode
//! 3. **Speed table cache**: the `SpeedTable` hierarchy drives all choices
//! 4. **GUI metadata contract**: parameters match what the GUI expects for ComboBox rendering
//!
//! # Running
//!
//! ```bash
//! # Mock mode (no hardware required)
//! cargo nextest run -p driver-pvcam --test dynamic_param_test
//!
//! # Hardware mode (on maitai)
//! source scripts/env-check.sh
//! export PVCAM_SMOKE_TEST=1
//! cargo nextest run -p driver-pvcam --test dynamic_param_test --features "pvcam_sdk,hardware_tests"
//! ```

use common::capabilities::Parameterized;
use driver_pvcam::PvcamDriver;
use serde_json::json;

// =============================================================================
// Helper: assert a parameter has dtype="enum" with non-empty choices
// =============================================================================

fn assert_enum_param(driver: &PvcamDriver, name: &str) {
    let param = driver
        .parameters()
        .get(name)
        .unwrap_or_else(|| panic!("Parameter '{}' not found", name));
    let meta = param.metadata();
    assert_eq!(
        meta.dtype, "enum",
        "Parameter '{}' should have dtype='enum' for ComboBox rendering, got '{}'",
        name, meta.dtype
    );
    assert!(
        !meta.enum_values.is_empty(),
        "Parameter '{}' should have non-empty enum_values for dropdown options",
        name
    );
}

fn get_enum_values(driver: &PvcamDriver, name: &str) -> Vec<String> {
    driver
        .parameters()
        .get(name)
        .unwrap_or_else(|| panic!("Parameter '{}' not found", name))
        .metadata()
        .enum_values
}

fn get_param_value(driver: &PvcamDriver, name: &str) -> serde_json::Value {
    driver
        .parameters()
        .get(name)
        .unwrap_or_else(|| panic!("Parameter '{}' not found", name))
        .get_json()
        .unwrap_or_else(|e| panic!("Failed to get JSON for '{}': {}", name, e))
}

// =============================================================================
// Mock Mode Tests (always run, no hardware required)
// =============================================================================

#[cfg(not(feature = "pvcam_sdk"))]
mod mock_dynamic_params {
    use super::*;

    // -------------------------------------------------------------------------
    // Test 1: Initial Enumeration — choices populated after creation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn choices_populated_after_creation() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // All three readout parameters should be enum type with choices
        assert_enum_param(&driver, "readout.port");
        assert_enum_param(&driver, "readout.speed_mode");
        assert_enum_param(&driver, "readout.gain_mode");

        // Print for debugging
        let ports = get_enum_values(&driver, "readout.port");
        let speeds = get_enum_values(&driver, "readout.speed_mode");
        let gains = get_enum_values(&driver, "readout.gain_mode");

        println!("Readout ports: {:?}", ports);
        println!("Speed modes: {:?}", speeds);
        println!("Gain modes: {:?}", gains);
    }

    // -------------------------------------------------------------------------
    // Test 2: Mock SpeedTable structure matches expected values
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn mock_speed_table_choices_correct() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // Mock SpeedTable has: 1 port ("Normal Port"), 2 speeds, 2+1 gains
        let ports = get_enum_values(&driver, "readout.port");
        assert_eq!(ports, vec!["Normal Port"], "Mock should have one port");

        let speeds = get_enum_values(&driver, "readout.speed_mode");
        assert_eq!(
            speeds,
            vec!["100 MHz", "50 MHz"],
            "Mock port should have two speeds"
        );

        // Default speed is "100 MHz" which has 2 gains
        let gains = get_enum_values(&driver, "readout.gain_mode");
        assert_eq!(
            gains,
            vec!["High Gain", "Low Gain"],
            "100 MHz speed should have two gains"
        );
    }

    // -------------------------------------------------------------------------
    // Test 3: Current values are valid choices
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn current_values_are_valid_choices() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        for name in &["readout.port", "readout.speed_mode", "readout.gain_mode"] {
            let value = get_param_value(&driver, name);
            let choices = get_enum_values(&driver, name);
            let value_str = value.as_str().unwrap_or("");
            assert!(
                choices.contains(&value_str.to_string()),
                "Current value '{}' for '{}' should be in choices {:?}",
                value_str,
                name,
                choices
            );
        }
    }

    // -------------------------------------------------------------------------
    // Test 4: Dependency refresh — speed change updates gain choices
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn speed_change_updates_gain_choices() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // Initially at "100 MHz" with 2 gains
        let initial_gains = get_enum_values(&driver, "readout.gain_mode");
        assert_eq!(initial_gains.len(), 2, "100 MHz should have 2 gains");

        // Switch to "50 MHz" which has 1 gain ("Medium Gain") in mock SpeedTable
        driver
            .parameters()
            .get("readout.speed_mode")
            .unwrap()
            .set_json(json!("50 MHz"))
            .expect("Should accept valid speed choice");

        // Give the async change listener time to propagate
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // The critical assertion: gain CHOICES are updated from the SpeedTable cache.
        // This is what the GUI reads to render dropdown options.
        let updated_gains = get_enum_values(&driver, "readout.gain_mode");
        assert_eq!(
            updated_gains,
            vec!["Medium Gain"],
            "50 MHz speed should have only 'Medium Gain'"
        );

        // Note: In mock mode, the gain VALUE may not auto-reset because
        // the hardware write callback queries mock list_gain_modes() which
        // returns ["HDR", "CMS"] — a different set than the SpeedTable cache.
        // On real hardware, the SpeedTable and SDK agree, so the value resets.
        // The key behavior is that the *choices* (metadata.enum_values) updated.
    }

    // -------------------------------------------------------------------------
    // Test 5: Invalid choice is rejected by validator
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn invalid_choice_rejected() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // Try to set a non-existent port
        let result = driver
            .parameters()
            .get("readout.port")
            .unwrap()
            .set_json(json!("NonExistentPort"));

        assert!(
            result.is_err(),
            "Setting invalid choice should be rejected by validator"
        );

        // Original value should be unchanged
        let current = get_param_value(&driver, "readout.port");
        assert_ne!(
            current,
            json!("NonExistentPort"),
            "Invalid value should not persist"
        );
    }

    // -------------------------------------------------------------------------
    // Test 6: Other enum parameters also have proper metadata
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn other_enum_params_have_metadata() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // These are statically configured with with_choices_introspectable
        assert_enum_param(&driver, "acquisition.trigger_mode");
        assert_enum_param(&driver, "acquisition.clear_mode");
        assert_enum_param(&driver, "acquisition.expose_out_mode");
        assert_enum_param(&driver, "thermal.fan_speed");
        assert_enum_param(&driver, "shutter.mode");
        assert_enum_param(&driver, "acquisition.buffer_mode");
    }

    // -------------------------------------------------------------------------
    // Test 7: Read-only timing params update with speed selection
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn timing_params_update_with_speed() {
        let driver = PvcamDriver::new_async("MockCamera".to_string())
            .await
            .expect("Failed to create mock driver");

        // Default speed is "100 MHz" with pix_time_ns=10, bit_depth=16
        let pixel_time = get_param_value(&driver, "acquisition.pixel_time_ns");
        let bit_depth = get_param_value(&driver, "info.bit_depth");

        println!(
            "At 100 MHz: pixel_time_ns={}, bit_depth={}",
            pixel_time, bit_depth
        );

        // Switch to "50 MHz" with pix_time_ns=20, bit_depth=12
        driver
            .parameters()
            .get("readout.speed_mode")
            .unwrap()
            .set_json(json!("50 MHz"))
            .expect("Should accept valid speed choice");

        // Give async propagation time
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let pixel_time_after = get_param_value(&driver, "acquisition.pixel_time_ns");
        let bit_depth_after = get_param_value(&driver, "info.bit_depth");

        println!(
            "At 50 MHz: pixel_time_ns={}, bit_depth={}",
            pixel_time_after, bit_depth_after
        );

        assert_eq!(
            pixel_time_after,
            json!(20),
            "Pixel time should update to 20ns for 50 MHz"
        );
        assert_eq!(
            bit_depth_after,
            json!(12),
            "Bit depth should update to 12 for 50 MHz"
        );
    }
}

// =============================================================================
// Hardware Tests (require real PVCAM camera, run with PVCAM_SMOKE_TEST=1)
// =============================================================================

#[cfg(all(feature = "pvcam_sdk", feature = "hardware_tests"))]
mod hardware_dynamic_params {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    lazy_static::lazy_static! {
        static ref CAMERA_LOCK: Mutex<()> = Mutex::new(());
    }

    fn smoke_test_enabled() -> bool {
        env::var("PVCAM_SMOKE_TEST")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    }

    macro_rules! skip_if_disabled {
        () => {
            if !smoke_test_enabled() {
                println!("PVCAM dynamic param test skipped (set PVCAM_SMOKE_TEST=1 to enable)");
                return;
            }
        };
    }

    fn camera_name() -> String {
        env::var("PVCAM_CAMERA_NAME").unwrap_or_else(|_| "pvcamUSB_0".to_string())
    }

    // -------------------------------------------------------------------------
    // Test 1: Readout parameters populated with real hardware choices
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_choices_populated() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Choices Populated ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        // Verify enum metadata
        assert_enum_param(&driver, "readout.port");
        assert_enum_param(&driver, "readout.speed_mode");
        assert_enum_param(&driver, "readout.gain_mode");

        let ports = get_enum_values(&driver, "readout.port");
        let speeds = get_enum_values(&driver, "readout.speed_mode");
        let gains = get_enum_values(&driver, "readout.gain_mode");

        println!("Readout ports ({}): {:?}", ports.len(), ports);
        println!("Speed modes ({}): {:?}", speeds.len(), speeds);
        println!("Gain modes ({}): {:?}", gains.len(), gains);

        // Prime BSI typically has at least 1 port and multiple speeds
        assert!(
            !ports.is_empty(),
            "Camera should expose at least one readout port"
        );
        assert!(
            !speeds.is_empty(),
            "Camera should expose at least one speed mode"
        );
        assert!(
            !gains.is_empty(),
            "Camera should expose at least one gain mode"
        );

        let _ = driver.close().await;
        println!("=== Hardware Choices Populated PASSED ===");
    }

    // -------------------------------------------------------------------------
    // Test 2: Current values match hardware state
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_current_values_valid() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Current Values Valid ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        for name in &["readout.port", "readout.speed_mode", "readout.gain_mode"] {
            let value = get_param_value(&driver, name);
            let choices = get_enum_values(&driver, name);
            let value_str = value.as_str().unwrap_or("");

            println!("  {}: '{}' (choices: {:?})", name, value_str, choices);

            assert!(
                choices.contains(&value_str.to_string()),
                "Current value '{}' for '{}' must be in choices {:?}",
                value_str,
                name,
                choices
            );
        }

        let _ = driver.close().await;
        println!("=== Hardware Current Values Valid PASSED ===");
    }

    // -------------------------------------------------------------------------
    // Test 3: Port change cascades to speed and gain choices
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_port_change_cascades() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Port Change Cascade ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        let ports = get_enum_values(&driver, "readout.port");
        println!("Available ports: {:?}", ports);

        if ports.len() < 2 {
            println!("Only one port available — skipping cascade test");
            let _ = driver.close().await;
            return;
        }

        // Record initial state
        let initial_port = get_param_value(&driver, "readout.port");
        let initial_speeds = get_enum_values(&driver, "readout.speed_mode");
        let initial_gains = get_enum_values(&driver, "readout.gain_mode");

        println!("Initial port: {}", initial_port);
        println!("Initial speeds: {:?}", initial_speeds);
        println!("Initial gains: {:?}", initial_gains);

        // Switch to a different port
        let other_port = ports
            .iter()
            .find(|p| json!(p.as_str()) != initial_port)
            .expect("Should find a different port");

        println!("\nSwitching to port: {}", other_port);
        driver
            .parameters()
            .get("readout.port")
            .unwrap()
            .set_json(json!(other_port))
            .expect("Should accept valid port");

        // Allow async cascade to propagate
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let new_speeds = get_enum_values(&driver, "readout.speed_mode");
        let new_gains = get_enum_values(&driver, "readout.gain_mode");

        println!("After port change:");
        println!("  Speeds: {:?}", new_speeds);
        println!("  Gains: {:?}", new_gains);

        // Speeds should be updated (may or may not differ, but must be non-empty)
        assert!(
            !new_speeds.is_empty(),
            "Speed modes must be non-empty after port change"
        );
        assert!(
            !new_gains.is_empty(),
            "Gain modes must be non-empty after port change"
        );

        // Current values should be valid in the new choice set
        let current_speed = get_param_value(&driver, "readout.speed_mode");
        let current_gain = get_param_value(&driver, "readout.gain_mode");

        assert!(
            new_speeds.contains(&current_speed.as_str().unwrap_or("").to_string()),
            "Current speed '{}' should be valid after port change",
            current_speed
        );
        assert!(
            new_gains.contains(&current_gain.as_str().unwrap_or("").to_string()),
            "Current gain '{}' should be valid after port change",
            current_gain
        );

        // Restore original port
        driver
            .parameters()
            .get("readout.port")
            .unwrap()
            .set_json(initial_port)
            .expect("Should restore original port");

        let _ = driver.close().await;
        println!("=== Hardware Port Change Cascade PASSED ===");
    }

    // -------------------------------------------------------------------------
    // Test 4: Speed change updates gain choices and read-only params
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_speed_change_updates_gains() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Speed Change Updates ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        let speeds = get_enum_values(&driver, "readout.speed_mode");
        println!("Available speeds: {:?}", speeds);

        if speeds.len() < 2 {
            println!("Only one speed available — skipping speed change test");
            let _ = driver.close().await;
            return;
        }

        // Record initial state
        let initial_speed = get_param_value(&driver, "readout.speed_mode");
        let initial_pixel_time = get_param_value(&driver, "acquisition.pixel_time_ns");
        let initial_bit_depth = get_param_value(&driver, "info.bit_depth");

        println!(
            "Initial: speed={}, pixel_time={}, bit_depth={}",
            initial_speed, initial_pixel_time, initial_bit_depth
        );

        // Switch to a different speed
        let other_speed = speeds
            .iter()
            .find(|s| json!(s.as_str()) != initial_speed)
            .expect("Should find a different speed");

        println!("Switching to speed: {}", other_speed);
        driver
            .parameters()
            .get("readout.speed_mode")
            .unwrap()
            .set_json(json!(other_speed))
            .expect("Should accept valid speed");

        // Allow async cascade
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let new_gains = get_enum_values(&driver, "readout.gain_mode");
        let new_pixel_time = get_param_value(&driver, "acquisition.pixel_time_ns");
        let new_bit_depth = get_param_value(&driver, "info.bit_depth");

        println!("After speed change:");
        println!("  Gains: {:?}", new_gains);
        println!("  Pixel time: {}", new_pixel_time);
        println!("  Bit depth: {}", new_bit_depth);

        assert!(
            !new_gains.is_empty(),
            "Gain choices must be non-empty after speed change"
        );

        // Pixel time or bit depth should likely differ for a different speed
        // (not asserted as strictly required — some speeds could share values)
        println!(
            "  Pixel time changed: {}",
            new_pixel_time != initial_pixel_time
        );
        println!(
            "  Bit depth changed: {}",
            new_bit_depth != initial_bit_depth
        );

        // Current gain must be valid
        let current_gain = get_param_value(&driver, "readout.gain_mode");
        assert!(
            new_gains.contains(&current_gain.as_str().unwrap_or("").to_string()),
            "Current gain '{}' should be valid after speed change",
            current_gain
        );

        let _ = driver.close().await;
        println!("=== Hardware Speed Change Updates PASSED ===");
    }

    // -------------------------------------------------------------------------
    // Test 5: Trigger mode enum populated from hardware
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_trigger_mode_choices() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Trigger Mode Choices ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        assert_enum_param(&driver, "acquisition.trigger_mode");

        let modes = get_enum_values(&driver, "acquisition.trigger_mode");
        let current = get_param_value(&driver, "acquisition.trigger_mode");

        println!("Trigger modes ({}): {:?}", modes.len(), modes);
        println!("Current trigger mode: {}", current);

        // Should have at least "Timed" mode
        assert!(
            modes
                .iter()
                .any(|m| m.contains("Timed") || m.contains("timed")),
            "Should have a Timed trigger mode, got {:?}",
            modes
        );

        let _ = driver.close().await;
        println!("=== Hardware Trigger Mode Choices PASSED ===");
    }

    // -------------------------------------------------------------------------
    // Test 6: Full hierarchy dump for diagnostic purposes
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn hardware_dump_full_hierarchy() {
        skip_if_disabled!();
        let _lock = CAMERA_LOCK.lock().unwrap();

        println!("=== Hardware Dynamic Param: Full Hierarchy Dump ===");

        let driver = PvcamDriver::new_async(camera_name())
            .await
            .expect("Failed to create hardware driver");

        let ports = get_enum_values(&driver, "readout.port");

        for port_name in &ports {
            // Set port
            driver
                .parameters()
                .get("readout.port")
                .unwrap()
                .set_json(json!(port_name))
                .expect("Should set port");
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

            let speeds = get_enum_values(&driver, "readout.speed_mode");
            println!("Port: {}", port_name);

            for speed_name in &speeds {
                driver
                    .parameters()
                    .get("readout.speed_mode")
                    .unwrap()
                    .set_json(json!(speed_name))
                    .expect("Should set speed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                let gains = get_enum_values(&driver, "readout.gain_mode");
                let pixel_time = get_param_value(&driver, "acquisition.pixel_time_ns");
                let bit_depth = get_param_value(&driver, "info.bit_depth");

                println!(
                    "  Speed: {} (pixel_time={}ns, bit_depth={})",
                    speed_name, pixel_time, bit_depth
                );
                for gain_name in &gains {
                    println!("    Gain: {}", gain_name);
                }
            }
        }

        let _ = driver.close().await;
        println!("\n=== Hardware Full Hierarchy Dump PASSED ===");
    }
}
