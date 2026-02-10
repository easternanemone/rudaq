#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "universal")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]
//! Universal Driver (TOML-based) Hardware Validation Test Suite
//!
//! Tests the declarative driver-universal system against real hardware,
//! validating that TOML device configs produce drivers functionally
//! equivalent to the legacy hand-coded Rust drivers.
//!
//! # Test Categories
//!
//! ## Mock Tests (run in CI, no hardware needed)
//! - Factory creation from real TOML config files
//! - Capability wiring (correct DeviceComponents fields populated)
//! - Command generation and response parsing via MockTransport
//!
//! ## Hardware Tests (require physical hardware on maitai)
//! - Newport 1830-C: Readable + WavelengthTunable via real serial
//! - ESP300: Movable via real serial
//!
//! # Running Tests
//!
//! ```bash
//! # Mock tests only (CI-safe)
//! cargo nextest run -p integration-tests --features universal \
//!   -- hardware_universal_driver
//!
//! # Hardware tests (on maitai with real instruments)
//! cargo nextest run -p integration-tests \
//!   --features "universal,hardware_tests" \
//!   --run-ignored all -- hardware_universal
//! ```

use std::path::PathBuf;

use driver_universal::factory::UniversalDriverFactory;
use driver_universal::transport::MockTransport;

/// Resolve the path to a device config TOML file.
///
/// Looks relative to the workspace root (two levels up from crates/integration-tests).
fn config_path(filename: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // integration-tests is at crates/integration-tests, go up to workspace root
    path.pop(); // crates/
    path.pop(); // workspace root
    path.push("config");
    path.push("devices");
    path.push(filename);
    path
}

// ============================================================================
// MOCK TESTS: Factory Creation & Capability Wiring
// ============================================================================
// These tests load real TOML configs and verify correct factory/driver behavior
// using MockTransport. They run in CI without any hardware.

mod mock_tests {
    use super::*;
    use common::capabilities::{Movable, Readable, WavelengthTunable};
    use common::driver::DriverFactory;

    // --- Newport 1830-C ---

    #[test]
    fn test_newport_1830c_factory_loads() {
        let path = config_path("newport_1830c.toml");
        let factory =
            UniversalDriverFactory::from_file(&path).expect("Failed to load newport_1830c.toml");

        assert!(
            factory.driver_type().contains("newport"),
            "driver_type should contain 'newport', got: {}",
            factory.driver_type()
        );
        assert_eq!(factory.name(), "Newport 1830-C");

        let caps = factory.capabilities();
        assert!(
            caps.contains(&common::driver::Capability::Readable),
            "Newport should have Readable capability"
        );
        assert!(
            caps.contains(&common::driver::Capability::WavelengthTunable),
            "Newport should have WavelengthTunable capability"
        );
    }

    #[tokio::test]
    async fn test_newport_1830c_mock_readable() {
        let path = config_path("newport_1830c.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        let mock = MockTransport::new(vec!["1.234E-6".to_string()]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock),
            "0",
        );

        let power = Readable::read(&driver).await.unwrap();
        assert!(
            (power - 1.234e-6).abs() < 1e-12,
            "Expected ~1.234e-6, got {power}"
        );
    }

    #[tokio::test]
    async fn test_newport_1830c_mock_get_wavelength() {
        let path = config_path("newport_1830c.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        let mock = MockTransport::new(vec!["0800".to_string()]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock),
            "0",
        );

        let wavelength = WavelengthTunable::get_wavelength(&driver).await.unwrap();
        assert_eq!(wavelength, 800.0, "Expected 800nm, got {wavelength}");
    }

    #[tokio::test]
    async fn test_newport_1830c_mock_set_wavelength_command() {
        let path = config_path("newport_1830c.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        // set_wavelength doesn't expect a response
        let mock = MockTransport::new(vec![]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock.clone()),
            "0",
        );

        WavelengthTunable::set_wavelength(&driver, 780.0)
            .await
            .unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "W0780", "Expected W0780, got {}", sent[0]);
    }

    #[tokio::test]
    async fn test_newport_1830c_mock_components_wiring() {
        // Newport has an init_sequence that drains the E0 echo, so we can't
        // use factory.build() with an empty MockTransport. Verify capabilities
        // at the manifest level instead.
        let path = config_path("newport_1830c.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        assert!(
            manifest.capabilities.readable.is_some(),
            "Newport should have Readable configured"
        );
        assert!(
            manifest.capabilities.wavelength_tunable.is_some(),
            "Newport should have WavelengthTunable configured"
        );
        assert!(
            manifest.capabilities.movable.is_none(),
            "Newport should NOT have Movable configured"
        );
        assert!(
            manifest.capabilities.shutter_control.is_none(),
            "Newport should NOT have ShutterControl configured"
        );
    }

    // --- ESP300 ---

    #[test]
    fn test_esp300_factory_loads() {
        let path = config_path("esp300.toml");
        let factory = UniversalDriverFactory::from_file(&path).expect("Failed to load esp300.toml");

        assert!(
            factory.driver_type().contains("esp300"),
            "driver_type should contain 'esp300', got: {}",
            factory.driver_type()
        );
        assert_eq!(factory.name(), "Newport ESP300");

        let caps = factory.capabilities();
        assert!(
            caps.contains(&common::driver::Capability::Movable),
            "ESP300 should have Movable capability"
        );
    }

    #[tokio::test]
    async fn test_esp300_mock_position() {
        let path = config_path("esp300.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        let mock = MockTransport::new(vec!["12.345".to_string()]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock),
            "1",
        );

        let position = Movable::position(&driver).await.unwrap();
        assert!(
            (position - 12.345).abs() < 0.001,
            "Expected ~12.345, got {position}"
        );
    }

    #[tokio::test]
    async fn test_esp300_mock_move_abs_command() {
        let path = config_path("esp300.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        // move_abs doesn't expect a response
        let mock = MockTransport::new(vec![]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock.clone()),
            "1",
        );

        Movable::move_abs(&driver, 25.0).await.unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        // Float values render with decimal point in MiniJinja
        assert!(
            sent[0] == "1PA25" || sent[0] == "1PA25.0",
            "Expected 1PA25 or 1PA25.0, got {}",
            sent[0]
        );
    }

    #[tokio::test]
    async fn test_esp300_mock_stop_command() {
        let path = config_path("esp300.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        let mock = MockTransport::new(vec![]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock.clone()),
            "1",
        );

        Movable::stop(&driver).await.unwrap();

        let sent = mock.sent_strings();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "1ST", "Expected 1ST, got {}", sent[0]);
    }

    #[tokio::test]
    async fn test_esp300_mock_components_wiring() {
        let path = config_path("esp300.toml");
        let factory = UniversalDriverFactory::from_file(&path).unwrap();

        let config = toml::toml! {
            port = "/dev/ttyUSB0"
            address = "1"
            mock = true
        };

        let components = factory.build(config.into()).await.unwrap();
        assert!(components.movable.is_some(), "ESP300 should wire Movable");
        assert!(
            components.readable.is_none(),
            "ESP300 should NOT wire Readable"
        );
    }

    // --- MaiTai ---

    #[test]
    fn test_maitai_factory_loads() {
        let path = config_path("maitai.toml");
        let factory = UniversalDriverFactory::from_file(&path).expect("Failed to load maitai.toml");

        assert!(
            factory.driver_type().contains("maitai"),
            "driver_type should contain 'maitai', got: {}",
            factory.driver_type()
        );

        let caps = factory.capabilities();
        assert!(
            caps.contains(&common::driver::Capability::Readable),
            "MaiTai should have Readable"
        );
        assert!(
            caps.contains(&common::driver::Capability::WavelengthTunable),
            "MaiTai should have WavelengthTunable"
        );
        assert!(
            caps.contains(&common::driver::Capability::ShutterControl),
            "MaiTai should have ShutterControl"
        );
    }

    #[tokio::test]
    async fn test_maitai_mock_components_wiring() {
        // MaiTai has an init_sequence that queries *IDN?, so we can't use
        // factory.build() with an empty MockTransport. Instead, construct
        // the driver directly to verify capability wiring.
        let path = config_path("maitai.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        // Verify the manifest has the expected capabilities configured
        assert!(
            manifest.capabilities.readable.is_some(),
            "MaiTai should have Readable configured"
        );
        assert!(
            manifest.capabilities.wavelength_tunable.is_some(),
            "MaiTai should have WavelengthTunable configured"
        );
        assert!(
            manifest.capabilities.shutter_control.is_some(),
            "MaiTai should have ShutterControl configured"
        );
    }

    // --- ELL14 ---

    #[test]
    fn test_ell14_factory_loads() {
        let path = config_path("ell14.toml");
        let factory = UniversalDriverFactory::from_file(&path).expect("Failed to load ell14.toml");

        assert!(
            factory.driver_type().contains("ell14"),
            "driver_type should contain 'ell14', got: {}",
            factory.driver_type()
        );

        let caps = factory.capabilities();
        assert!(
            caps.contains(&common::driver::Capability::Movable),
            "ELL14 should have Movable"
        );
    }

    #[tokio::test]
    async fn test_ell14_mock_position() {
        let path = config_path("ell14.toml");
        let toml_content = std::fs::read_to_string(&path).unwrap();
        let raw: driver_universal::config::RawManifest = toml::from_str(&toml_content).unwrap();
        let manifest = driver_universal::config::parse_manifest(raw).unwrap();

        // 0x0000A1B3 = 41395 pulses, 41395 / 398.2222 ≈ 103.95°
        let mock = MockTransport::new(vec!["2PO0000A1B3".to_string()]);
        let driver = driver_universal::driver::UniversalDriver::new(
            std::sync::Arc::new(manifest),
            Box::new(mock),
            "2",
        );

        let position = Movable::position(&driver).await.unwrap();
        assert!(
            (position - 103.95).abs() < 0.1,
            "Expected ~103.95°, got {position}"
        );
    }

    // --- load_all_factories ---

    #[test]
    fn test_load_all_factories_from_config_dir() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.push("config");
        path.push("devices");

        let factories = driver_universal::factory::load_all_factories(&path).unwrap();
        assert!(
            factories.len() >= 4,
            "Should load at least 4 factories (newport, esp300, maitai, ell14), got {}",
            factories.len()
        );
    }
}

// ============================================================================
// HARDWARE TESTS: Real Instrument Validation
// ============================================================================
// These tests are only compiled when hardware_tests feature is enabled.
// They require physical hardware to be connected on the maitai machine.

#[cfg(feature = "hardware_tests")]
mod hardware_tests {
    use super::*;
    use common::capabilities::{Movable, Readable, WavelengthTunable};
    use common::driver::DriverFactory;
    use std::env;
    use std::time::Duration;

    // --- Newport 1830-C Hardware Tests ---

    mod newport_1830c {
        use super::*;

        fn get_port() -> String {
            env::var("NEWPORT_1830C_PORT").unwrap_or_else(|_| "/dev/ttyS0".to_string())
        }

        async fn build_newport_driver() -> (
            std::sync::Arc<dyn Readable>,
            std::sync::Arc<dyn WavelengthTunable>,
        ) {
            let path = config_path("newport_1830c.toml");
            let factory = UniversalDriverFactory::from_file(&path)
                .expect("Failed to load newport_1830c.toml");

            let port = get_port();
            let mut table = toml::map::Map::new();
            table.insert("port".into(), toml::Value::String(port));
            table.insert("address".into(), toml::Value::String("0".into()));
            let config = toml::Value::Table(table);

            let components = factory
                .build(config)
                .await
                .expect("Failed to build Newport universal driver");

            // Allow serial port to settle after init_sequence
            tokio::time::sleep(Duration::from_millis(100)).await;

            let readable = components.readable.expect("Newport should have Readable");
            let wavelength = components
                .wavelength_tunable
                .expect("Newport should have WavelengthTunable");

            (readable, wavelength)
        }

        /// Test: Universal driver can read power from real Newport 1830-C
        #[tokio::test]
        #[ignore]
        async fn test_universal_newport_read_power() {
            let (readable, _) = build_newport_driver().await;

            let power = readable.read().await.expect("Failed to read power");
            println!("Universal Newport 1830-C power reading: {power} W");

            // Power should be a finite number (can be negative noise floor)
            assert!(power.is_finite(), "Power should be finite, got {power}");
        }

        /// Test: Universal driver can query wavelength from real Newport 1830-C
        #[tokio::test]
        #[ignore]
        async fn test_universal_newport_get_wavelength() {
            let (_, wavelength_tunable) = build_newport_driver().await;

            let wl = wavelength_tunable
                .get_wavelength()
                .await
                .expect("Failed to get wavelength");
            println!("Universal Newport 1830-C wavelength: {wl} nm");

            assert!(
                (300.0..=1100.0).contains(&wl),
                "Wavelength should be 300-1100nm, got {wl}"
            );
        }

        /// Test: Universal driver can set and verify wavelength on real Newport 1830-C
        #[tokio::test]
        #[ignore]
        async fn test_universal_newport_set_wavelength() {
            let (_, wavelength_tunable) = build_newport_driver().await;

            // Save initial wavelength
            let initial = wavelength_tunable
                .get_wavelength()
                .await
                .expect("Failed to get initial wavelength");

            // Set to 800nm
            wavelength_tunable
                .set_wavelength(800.0)
                .await
                .expect("Failed to set wavelength");

            // Small delay for device to process
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Verify
            let actual = wavelength_tunable
                .get_wavelength()
                .await
                .expect("Failed to verify wavelength");
            println!("Set 800nm -> Read {actual}nm");
            assert_eq!(actual, 800.0, "Wavelength should be 800nm, got {actual}");

            // Restore
            wavelength_tunable
                .set_wavelength(initial)
                .await
                .expect("Failed to restore wavelength");
        }

        /// Test: Rapid sequential reads (serial reliability)
        #[tokio::test]
        #[ignore]
        async fn test_universal_newport_rapid_reads() {
            let (readable, _) = build_newport_driver().await;

            let mut readings = Vec::new();
            for i in 0..5 {
                let power = readable
                    .read()
                    .await
                    .unwrap_or_else(|e| panic!("Read {i} failed: {e}"));
                assert!(power.is_finite(), "Read {i} returned non-finite: {power}");
                readings.push(power);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            println!(
                "Universal Newport 5 rapid reads: {:?}",
                readings
                    .iter()
                    .map(|p| format!("{p:.3e}"))
                    .collect::<Vec<_>>()
            );
        }

        /// Test: Multiple wavelength set/get cycles
        #[tokio::test]
        #[ignore]
        async fn test_universal_newport_wavelength_cycle() {
            let (_, wavelength_tunable) = build_newport_driver().await;

            let initial = wavelength_tunable.get_wavelength().await.unwrap();
            let test_wavelengths = [500.0, 780.0, 1064.0];

            for target in test_wavelengths {
                wavelength_tunable.set_wavelength(target).await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;

                let actual = wavelength_tunable.get_wavelength().await.unwrap();
                println!("Set {target}nm -> Read {actual}nm");
                assert_eq!(
                    actual, target,
                    "Wavelength mismatch: set {target}, got {actual}"
                );
            }

            // Restore
            wavelength_tunable.set_wavelength(initial).await.unwrap();
        }
    }

    // --- ESP300 Hardware Tests ---

    mod esp300 {
        use super::*;

        fn get_port() -> String {
            env::var("ESP300_PORT").unwrap_or_else(|_| "/dev/ttyUSB0".to_string())
        }

        async fn build_esp300_driver() -> std::sync::Arc<dyn Movable> {
            let path = config_path("esp300.toml");
            let factory =
                UniversalDriverFactory::from_file(&path).expect("Failed to load esp300.toml");

            let port = get_port();
            let mut table = toml::map::Map::new();
            table.insert("port".into(), toml::Value::String(port));
            table.insert("address".into(), toml::Value::String("1".into()));
            let config = toml::Value::Table(table);

            let components = factory
                .build(config)
                .await
                .expect("Failed to build ESP300 universal driver");

            components.movable.expect("ESP300 should have Movable")
        }

        /// Test: Universal driver can query position from real ESP300
        #[tokio::test]
        #[ignore]
        async fn test_universal_esp300_position() {
            let movable = build_esp300_driver().await;

            let position = movable.position().await.expect("Failed to query position");
            println!("Universal ESP300 position: {position} mm");

            assert!(
                position.is_finite(),
                "Position should be finite, got {position}"
            );
        }

        /// Test: Universal driver can stop motion on real ESP300
        #[tokio::test]
        #[ignore]
        async fn test_universal_esp300_stop() {
            let movable = build_esp300_driver().await;

            movable.stop().await.expect("Failed to stop");
            println!("Universal ESP300 stop: OK");
        }

        /// Test: Universal driver can move and verify position on real ESP300
        #[tokio::test]
        #[ignore]
        async fn test_universal_esp300_move_abs() {
            let movable = build_esp300_driver().await;

            // Read current position
            let initial = movable
                .position()
                .await
                .expect("Failed to get initial position");
            println!("Initial position: {initial} mm");

            // Move a small distance (0.1mm from current position)
            let target = initial + 0.1;
            movable.move_abs(target).await.expect("Failed to move_abs");

            // Wait for motion to complete
            movable
                .wait_settled()
                .await
                .expect("Failed to wait_settled");

            // Verify position
            let actual = movable
                .position()
                .await
                .expect("Failed to get final position");
            println!("Moved to {target}mm -> Read {actual}mm");
            assert!(
                (actual - target).abs() < 0.01,
                "Position mismatch: expected {target}, got {actual}"
            );

            // Return to original position
            movable.move_abs(initial).await.unwrap();
            movable.wait_settled().await.unwrap();
        }
    }
}
