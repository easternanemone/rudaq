//! Panel dispatch logic for device-to-control-panel mapping.
//!
//! This module provides a centralized function to determine which control panel
//! type should be used for a given device based on its capabilities and TOML config.
//!
//! The dispatch priority is:
//! 0. **gRPC-driven** — If `device.metadata.ui_schema_json` contains a valid `UiConfig`
//! 1. **Config-driven** — If a TOML `[ui.control_panel]` exists for the device's driver type
//! 2. **Capability-based** — Hardcoded panels matched by driver name and capabilities

#![allow(dead_code)]

use crate::device_ext::DeviceInfoExt;
use hardware::config::schema::{ControlPanelConfig, UiConfig};
use protocol::daq::DeviceInfo;

use super::config_loader::DeviceConfigCache;

/// The type of control panel to use for a device.
#[derive(Debug, Clone)]
pub enum PanelType {
    /// Config-driven panel from TOML `[ui.control_panel]`
    ConfigDriven(ControlPanelConfig),
    /// MaiTai Ti:Sapphire laser control panel (wavelength, emission, shutter)
    MaiTai,
    /// Power meter control panel (readable sensors)
    PowerMeter,
    /// Rotator control panel (ELL14-style rotation mounts)
    Rotator,
    /// Stage control panel (linear/XY stages)
    Stage,
    /// Comedi DAQ unified control panel (AI, AO, DIO, counters)
    Comedi,
}

/// Extract a `ControlPanelConfig` from the device's gRPC metadata `ui_schema_json`.
///
/// Returns `Some(config)` if the device has valid `ui_schema_json` metadata that
/// deserializes to a `UiConfig` with a `control_panel` section. Returns `None` if
/// metadata is absent, the JSON is missing or malformed, or `control_panel` is unset.
pub fn try_grpc_ui_config(device: &DeviceInfo) -> Option<ControlPanelConfig> {
    let json_str = device
        .metadata
        .as_ref()
        .and_then(|m| m.ui_schema_json.as_ref())?;
    let ui_config: UiConfig = serde_json::from_str(json_str)
        .map_err(|e| {
            tracing::warn!(
                device_id = %device.id,
                error = %e,
                "Failed to deserialize ui_schema_json from gRPC metadata"
            );
            e
        })
        .ok()?;
    ui_config.control_panel
}

/// Determine the appropriate control panel type for a device.
///
/// Checks gRPC metadata first, then TOML config cache, then falls back to
/// capability-based dispatch.
///
/// # Priority order
/// 0. gRPC metadata (`ui_schema_json`) → ConfigDriven
/// 1. Config-driven (TOML `[ui.control_panel]` exists) → ConfigDriven
/// 2. Comedi DAQ devices → Comedi
/// 3. Laser capabilities (emission/shutter/wavelength) → MaiTai
/// 4. Readable without motion (sensors, meters) → PowerMeter
/// 5. Movable with "ell14" in driver name → Rotator
/// 6. Movable → Stage (default for motion devices)
pub fn determine_panel_type_with_config(
    device: &DeviceInfo,
    config_cache: Option<&DeviceConfigCache>,
) -> PanelType {
    // Priority 0: gRPC-driven panel from device metadata
    if let Some(config) = try_grpc_ui_config(device) {
        return PanelType::ConfigDriven(config);
    }

    // Priority 1: Config-driven panel from local TOML
    if let Some(cache) = config_cache {
        if let Some(config) = cache.get_ui_config_for_driver(&device.driver_type) {
            return PanelType::ConfigDriven(config.clone());
        }
    }

    // Fall back to capability-based dispatch
    determine_panel_type(device)
}

/// Determine the appropriate control panel type for a device (capability-based only).
///
/// Priority order:
/// 1. Comedi DAQ devices (comedi_analog_input, comedi_analog_output, ni_daq) → Comedi
/// 2. Laser capabilities (emission/shutter/wavelength) → MaiTai
/// 3. Readable without motion (sensors, meters) → PowerMeter
/// 4. Movable with "ell14" in driver name → Rotator
/// 5. Movable → Stage (default for motion devices)
///
/// # Arguments
/// * `device` - Device info with capability flags
///
/// # Returns
/// The `PanelType` to use for this device's control panel
pub fn determine_panel_type(device: &DeviceInfo) -> PanelType {
    let driver_lower = device.driver_type.to_lowercase();

    // Priority 1: Comedi DAQ devices
    if driver_lower.contains("comedi")
        || driver_lower.contains("ni_daq")
        || driver_lower.contains("nidaq")
        || driver_lower.contains("pci-mio")
        || driver_lower.contains("pcimio")
    {
        return PanelType::Comedi;
    }

    // Priority 2: Laser controls (MaiTai-style devices)
    if device.is_emission_controllable()
        || device.is_shutter_controllable()
        || device.is_wavelength_tunable()
    {
        return PanelType::MaiTai;
    }

    // Priority 3: Pure readable devices (power meters, sensors)
    if device.is_readable() && !device.is_movable() {
        return PanelType::PowerMeter;
    }

    // Priority 4: Movable devices - distinguish rotator vs stage
    if device.is_movable() {
        if driver_lower.contains("ell14") || driver_lower.contains("rotator") {
            return PanelType::Rotator;
        }
        return PanelType::Stage;
    }

    // Default fallback: Stage panel (most generic)
    PanelType::Stage
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a DeviceInfo with specified capabilities
    fn make_device(
        driver: &str,
        movable: bool,
        readable: bool,
        emission: bool,
        shutter: bool,
        wavelength: bool,
    ) -> DeviceInfo {
        let mut capabilities = Vec::new();
        if movable {
            capabilities.push("movable".to_string());
        }
        if readable {
            capabilities.push("readable".to_string());
        }
        if emission {
            capabilities.push("emission_controllable".to_string());
        }
        if shutter {
            capabilities.push("shutter_controllable".to_string());
        }
        if wavelength {
            capabilities.push("wavelength_tunable".to_string());
        }
        #[allow(deprecated)]
        DeviceInfo {
            id: "test-device".to_string(),
            name: "Test Device".to_string(),
            driver_type: driver.to_string(),
            capabilities,
            ..Default::default()
        }
    }

    #[test]
    fn test_dispatch_maitai_by_emission() {
        let dev = make_device("MaiTai DeepSee", false, true, true, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::MaiTai));
    }

    #[test]
    fn test_dispatch_maitai_by_shutter() {
        let dev = make_device("SomeLaser", false, true, false, true, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::MaiTai));
    }

    #[test]
    fn test_dispatch_maitai_by_wavelength() {
        let dev = make_device("TunableLaser", false, true, false, false, true);
        assert!(matches!(determine_panel_type(&dev), PanelType::MaiTai));
    }

    #[test]
    fn test_dispatch_maitai_priority_over_readable() {
        // MaiTai priority even if device is also readable
        let dev = make_device("MaiTai", false, true, true, true, true);
        assert!(matches!(determine_panel_type(&dev), PanelType::MaiTai));
    }

    #[test]
    fn test_dispatch_power_meter() {
        let dev = make_device("Newport 1830-C", false, true, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::PowerMeter));
    }

    #[test]
    fn test_dispatch_rotator_ell14() {
        let dev = make_device("Thorlabs ELL14", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Rotator));
    }

    #[test]
    fn test_dispatch_rotator_by_keyword() {
        let dev = make_device("Custom Rotator Mount", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Rotator));
    }

    #[test]
    fn test_dispatch_stage_esp300() {
        let dev = make_device("Newport ESP300", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Stage));
    }

    #[test]
    fn test_dispatch_stage_fallback() {
        // Generic movable device defaults to Stage
        let dev = make_device("Unknown Motor", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Stage));
    }

    #[test]
    fn test_dispatch_no_capabilities_fallback() {
        // Device with no known capabilities falls back to Stage
        let dev = make_device("Unknown Device", false, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Stage));
    }

    #[test]
    fn test_dispatch_readable_movable_is_stage() {
        // Readable + movable should be Stage (not PowerMeter)
        let dev = make_device("Encoder Stage", true, true, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Stage));
    }

    #[test]
    fn test_dispatch_config_driven_takes_priority() {
        // When a config cache has a matching config, ConfigDriven takes priority
        let dev = make_device("Thorlabs ELL14", true, false, false, false, false);

        // Without config: capability-based dispatch
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::Rotator
        ));

        // With config cache but no matching config: falls back
        let cache = DeviceConfigCache::new();
        assert!(matches!(
            determine_panel_type_with_config(&dev, Some(&cache)),
            PanelType::Rotator
        ));
    }

    /// Helper to create a DeviceInfo with `ui_schema_json` in metadata
    fn make_device_with_schema(
        driver: &str,
        movable: bool,
        readable: bool,
        emission: bool,
        shutter: bool,
        wavelength: bool,
        ui_schema_json: Option<String>,
    ) -> DeviceInfo {
        let mut dev = make_device(driver, movable, readable, emission, shutter, wavelength);
        dev.metadata = Some(protocol::daq::DeviceMetadata {
            ui_schema_json,
            ..Default::default()
        });
        dev
    }

    #[test]
    fn test_grpc_schema_takes_priority() {
        // Device with valid ui_schema_json → try_grpc_ui_config returns Some
        let json = r#"{"control_panel":{"layout":"vertical","sections":[{"type":"sensor","label":"Power"}]}}"#;
        let dev = make_device_with_schema(
            "universal_ipg_ylpp",
            false,
            true,
            false,
            false,
            false,
            Some(json.to_string()),
        );
        let config = try_grpc_ui_config(&dev);
        assert!(
            config.is_some(),
            "gRPC ui_schema_json should produce a ControlPanelConfig"
        );

        // Full dispatch also returns ConfigDriven
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::ConfigDriven(_)
        ));
    }

    #[test]
    fn test_grpc_schema_precedence_over_capability_dispatch() {
        // IPG-like device: readable (would normally → PowerMeter) but has ui_schema_json
        let json = r#"{"control_panel":{"layout":"vertical","sections":[{"type":"custom_action","label":"Emission","command":"emission_on"},{"type":"sensor","label":"Output Power"}]}}"#;
        let dev = make_device_with_schema(
            "universal_ipg_ylpp-200-1-50-r",
            false,
            true,
            false,
            false,
            false,
            Some(json.to_string()),
        );

        // Without gRPC metadata, this would be PowerMeter
        let dev_no_meta = make_device(
            "universal_ipg_ylpp-200-1-50-r",
            false,
            true,
            false,
            false,
            false,
        );
        assert!(matches!(
            determine_panel_type_with_config(&dev_no_meta, None),
            PanelType::PowerMeter
        ));

        // With gRPC metadata, ConfigDriven takes precedence
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::ConfigDriven(_)
        ));
    }

    #[test]
    fn test_native_driver_no_schema_falls_through() {
        // Device without ui_schema_json → try_grpc_ui_config returns None
        let dev = make_device_with_schema("MaiTai DeepSee", false, true, true, false, false, None);
        assert!(try_grpc_ui_config(&dev).is_none());

        // Falls through to capability dispatch → MaiTai
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::MaiTai
        ));
    }

    #[test]
    fn test_malformed_json_falls_through() {
        // Invalid JSON → try_grpc_ui_config returns None, falls to capability dispatch
        let dev = make_device_with_schema(
            "Newport 1830-C",
            false,
            true,
            false,
            false,
            false,
            Some("{not valid json!!!}".to_string()),
        );
        assert!(try_grpc_ui_config(&dev).is_none());

        // Falls through to capability dispatch → PowerMeter
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::PowerMeter
        ));
    }
}
