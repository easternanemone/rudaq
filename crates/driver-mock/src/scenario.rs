//! Multi-device scenario configuration for coordinated mock testing.
//!
//! Scenarios group multiple mock devices under a named configuration with a
//! shared RNG seed, enabling deterministic and reproducible multi-device test
//! environments.

use serde::{Deserialize, Serialize};

/// A named test scenario grouping multiple mock devices with shared configuration.
///
/// Scenarios ensure deterministic, reproducible test environments by sharing
/// an RNG seed across all devices.
///
/// # TOML Example
/// ```toml
/// [scenario]
/// name = "reliability_test"
/// seed = 42
///
/// [[scenario.devices]]
/// id = "camera_1"
/// driver = "mock_camera"
/// profile = "noisy"
///
/// [[scenario.devices]]
/// id = "stage_x"
/// driver = "mock_stage"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioConfig {
    /// Human-readable scenario name.
    pub name: String,

    /// Shared RNG seed for deterministic behavior across all devices.
    /// When `None`, each device uses a random seed from the OS.
    #[serde(default)]
    pub seed: Option<u64>,

    /// Device configurations within this scenario.
    pub devices: Vec<ScenarioDeviceConfig>,
}

/// A single device within a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDeviceConfig {
    /// Unique device ID within the scenario.
    pub id: String,

    /// Driver type (e.g., `"mock_camera"`, `"mock_stage"`).
    pub driver: String,

    /// Optional profile name (for drivers that support profiles, e.g. `MockCameraProfile`).
    #[serde(default)]
    pub profile: Option<String>,

    /// Additional driver-specific configuration passed through to the factory.
    #[serde(default = "default_config")]
    pub config: toml::Value,
}

/// Returns an empty TOML table as the default for driver-specific config.
fn default_config() -> toml::Value {
    toml::Value::Table(toml::Table::new())
}

impl ScenarioConfig {
    /// Returns the number of devices in this scenario.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Finds a device configuration by its ID.
    pub fn get_device(&self, id: &str) -> Option<&ScenarioDeviceConfig> {
        self.devices.iter().find(|d| d.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_toml() -> &'static str {
        r#"
            name = "reliability_test"
            seed = 42

            [[devices]]
            id = "camera_1"
            driver = "mock_camera"
            profile = "noisy"

            [[devices]]
            id = "stage_x"
            driver = "mock_stage"

            [[devices]]
            id = "power_meter"
            driver = "mock_power_meter"
            config = { noise_level = 0.05 }
        "#
    }

    #[test]
    fn test_toml_deserialization_three_devices() {
        let scenario: ScenarioConfig =
            toml::from_str(sample_toml()).expect("should deserialize scenario TOML");

        assert_eq!(scenario.name, "reliability_test");
        assert_eq!(scenario.seed, Some(42));
        assert_eq!(scenario.device_count(), 3);

        // First device: camera with profile
        let cam = &scenario.devices[0];
        assert_eq!(cam.id, "camera_1");
        assert_eq!(cam.driver, "mock_camera");
        assert_eq!(cam.profile.as_deref(), Some("noisy"));

        // Second device: stage without profile
        let stage = &scenario.devices[1];
        assert_eq!(stage.id, "stage_x");
        assert_eq!(stage.driver, "mock_stage");
        assert!(stage.profile.is_none());

        // Third device: power meter with extra config
        let pm = &scenario.devices[2];
        assert_eq!(pm.id, "power_meter");
        assert_eq!(pm.driver, "mock_power_meter");
        assert!(pm.config.get("noise_level").is_some());
    }

    #[test]
    fn test_get_device_found() {
        let scenario: ScenarioConfig = toml::from_str(sample_toml()).expect("should deserialize");

        let device = scenario.get_device("stage_x");
        assert!(device.is_some());
        assert_eq!(device.unwrap().driver, "mock_stage");
    }

    #[test]
    fn test_get_device_not_found() {
        let scenario: ScenarioConfig = toml::from_str(sample_toml()).expect("should deserialize");

        assert!(scenario.get_device("nonexistent").is_none());
    }

    #[test]
    fn test_seed_propagation() {
        let scenario: ScenarioConfig = toml::from_str(sample_toml()).expect("should deserialize");

        // Seed is available at scenario level for all devices to share
        let seed = scenario.seed.expect("seed should be present");
        assert_eq!(seed, 42);

        // Verify all devices could receive this seed
        assert!(
            scenario.device_count() > 0,
            "scenario should have devices to propagate seed to"
        );
    }

    #[test]
    fn test_seed_defaults_to_none() {
        let toml_str = r#"
            name = "unseeded"
            devices = []
        "#;
        let scenario: ScenarioConfig =
            toml::from_str(toml_str).expect("should deserialize without seed");

        assert!(scenario.seed.is_none());
    }

    #[test]
    fn test_serde_round_trip() {
        let original: ScenarioConfig = toml::from_str(sample_toml()).expect("should deserialize");

        let serialized = toml::to_string(&original).expect("should serialize to TOML");
        let round_tripped: ScenarioConfig =
            toml::from_str(&serialized).expect("should deserialize round-tripped TOML");

        assert_eq!(original.name, round_tripped.name);
        assert_eq!(original.seed, round_tripped.seed);
        assert_eq!(original.device_count(), round_tripped.device_count());

        for (orig, rt) in original.devices.iter().zip(&round_tripped.devices) {
            assert_eq!(orig.id, rt.id);
            assert_eq!(orig.driver, rt.driver);
            assert_eq!(orig.profile, rt.profile);
        }
    }

    #[test]
    fn test_empty_devices_list() {
        let toml_str = r#"
            name = "empty_scenario"
            seed = 99
            devices = []
        "#;
        let scenario: ScenarioConfig =
            toml::from_str(toml_str).expect("should deserialize empty scenario");

        assert_eq!(scenario.device_count(), 0);
        assert!(scenario.get_device("anything").is_none());
    }
}
