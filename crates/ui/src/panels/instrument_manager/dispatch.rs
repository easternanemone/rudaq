//! Panel dispatch logic for device-to-control-panel mapping.
//!
//! This module provides a centralized function to determine which control panel
//! type should be used for a given device based on its capabilities and TOML config.
//!
//! The dispatch priority is:
//! 0. **gRPC-driven** — If `device.metadata.ui_schema_json` contains a valid `[ui.control_panel]`
//! 1. **Explicit panel_kind** — If `device.metadata.panel_kind` is set, routes directly (no heuristics)
//! 2. **Config-driven** — If a local TOML `[ui.control_panel]` exists for the device's driver type
//! 3. **Capability-based** — Hardcoded panels matched by driver name and capabilities (legacy fallback)

use crate::device_ext::DeviceInfoExt;
use hardware::config::schema::{ControlPanelConfig, UiConfig};
use protocol::daq::DeviceInfo;

use super::config_loader::DeviceConfigCache;

/// The type of control panel to use for a device.
#[derive(Debug, Clone)]
pub enum PanelType {
    /// Config-driven panel from gRPC `ui_schema_json` or TOML `[ui.control_panel]`
    ConfigDriven(ControlPanelConfig),
    /// PVCAM camera panel (image viewer + camera controls)
    Camera,
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
    /// Andor iStar gated camera (DDG, MCP gain, trigger mode)
    AndorCamera,
    /// Andor Shamrock spectrograph (grating, wavelength, slit, shutter)
    Spectrograph,
    /// Dover SmartStage with Trigger-On-Position support
    DoverStage,
    /// Generic fallback panel for unrecognized or simple devices
    Generic,
}

/// Extract a `ControlPanelConfig` from the device's gRPC metadata `ui_schema_json` field.
///
/// Returns `None` if the device has no metadata, no `ui_schema_json`, the JSON is
/// malformed, or the deserialized `UiConfig` has no `control_panel` section.
pub fn try_grpc_ui_config(device: &DeviceInfo) -> Option<ControlPanelConfig> {
    let has_metadata = device.metadata.is_some();
    let has_schema = device
        .metadata
        .as_ref()
        .and_then(|m| m.ui_schema_json.as_ref())
        .is_some();
    tracing::trace!(
        device_id = %device.id,
        driver_type = %device.driver_type,
        has_metadata,
        has_schema,
        "try_grpc_ui_config called"
    );
    let json_str = device
        .metadata
        .as_ref()
        .and_then(|m| m.ui_schema_json.as_ref())?;
    tracing::trace!(
        device_id = %device.id,
        json_len = json_str.len(),
        "Found ui_schema_json"
    );
    let ui_config: UiConfig = serde_json::from_str(json_str)
        .map_err(|e| {
            tracing::warn!(
                device_id = %device.id,
                error = %e,
                "Failed to deserialize ui_schema_json"
            );
            e
        })
        .ok()?;
    let has_panel = ui_config.control_panel.is_some();
    tracing::debug!(
        device_id = %device.id,
        has_panel,
        "Deserialized UiConfig"
    );
    ui_config.control_panel
}

/// Determine the appropriate control panel type for a device.
///
/// Checks gRPC metadata first, then explicit `panel_kind`, then local TOML config,
/// then falls back to capability-based dispatch.
///
/// # Priority order
/// 0. gRPC-driven (`device.metadata.ui_schema_json`) → ConfigDriven
/// 1. Explicit `device.metadata.panel_kind` → matching PanelType (no heuristics)
/// 2. Config-driven (local TOML `[ui.control_panel]` exists) → ConfigDriven
/// 3. Andor iStar gated camera → AndorCamera
/// 4. Andor Shamrock spectrograph → Spectrograph
/// 5. Dover SmartStage → DoverStage
/// 6. Comedi DAQ devices → Comedi
/// 7. Laser capabilities (emission/shutter/wavelength) → MaiTai
/// 8. Readable without motion (sensors, meters) → PowerMeter
/// 9. Movable with "ell14" in driver name → Rotator
/// 10. Movable → Stage (default for motion devices)
pub fn determine_panel_type_with_config(
    device: &DeviceInfo,
    config_cache: Option<&DeviceConfigCache>,
) -> PanelType {
    // Priority 0: gRPC-driven panel from device metadata
    if let Some(config) = try_grpc_ui_config(device) {
        return PanelType::ConfigDriven(config);
    }

    // Priority 1: Explicit panel_kind field — deterministic routing, no heuristics
    if let Some(pk) = device
        .metadata
        .as_ref()
        .and_then(|m| m.panel_kind.as_deref())
    {
        return panel_kind_to_panel_type(pk);
    }

    // Priority 2: Config-driven panel from local TOML
    if let Some(cache) = config_cache
        && let Some(config) = cache.get_ui_config_for_driver(&device.driver_type)
    {
        return PanelType::ConfigDriven(config.clone());
    }

    // Fall back to capability-based dispatch
    determine_panel_type(device)
}

/// Map a `panel_kind` string to the corresponding [`PanelType`].
///
/// Unrecognized strings fall through to [`PanelType::Generic`] for forward
/// compatibility — new panel kinds added by future drivers won't break old UI builds.
pub fn panel_kind_to_panel_type(pk: &str) -> PanelType {
    match pk {
        common::panel_kind::PVCAM => PanelType::Camera,
        common::panel_kind::ANDOR_CAMERA => PanelType::AndorCamera,
        common::panel_kind::ANDOR_SHAMROCK => PanelType::Spectrograph,
        common::panel_kind::COMEDI => PanelType::Comedi,
        common::panel_kind::DOVER_STAGE => PanelType::DoverStage,
        common::panel_kind::MAITAI => PanelType::MaiTai,
        common::panel_kind::POWER_METER => PanelType::PowerMeter,
        common::panel_kind::ROTATOR => PanelType::Rotator,
        common::panel_kind::STAGE => PanelType::Stage,
        _ => PanelType::Generic,
    }
}

/// Determine the appropriate control panel type for a device (capability-based only).
///
/// Priority order:
/// 1. Andor iStar gated camera → AndorCamera
/// 2. Andor Shamrock spectrograph → Spectrograph
/// 3. Dover SmartStage → DoverStage
/// 4. Comedi DAQ devices (comedi_analog_input, comedi_analog_output, ni_daq) → Comedi
/// 5. Laser capabilities (emission/shutter/wavelength) → MaiTai
/// 6. Readable without motion (sensors, meters) → PowerMeter
/// 7. Movable with "ell14" in driver name → Rotator
/// 8. Movable → Stage (default for motion devices)
///
/// # Arguments
/// * `device` - Device info with capability flags
///
/// # Returns
/// The `PanelType` to use for this device's control panel
pub fn determine_panel_type(device: &DeviceInfo) -> PanelType {
    let driver_lower = device.driver_type.to_lowercase();

    // Priority 1: Andor iStar gated camera
    if driver_lower.contains("andor_istar") || driver_lower.contains("andor_camera") {
        return PanelType::AndorCamera;
    }

    // Priority 2: Andor Shamrock spectrograph
    if driver_lower.contains("andor_shamrock") || driver_lower.contains("shamrock") {
        return PanelType::Spectrograph;
    }

    // Priority 3: Dover SmartStage
    if driver_lower.contains("dover") {
        return PanelType::DoverStage;
    }

    // Priority 4: Comedi DAQ devices
    if driver_lower.contains("comedi")
        || driver_lower.contains("ni_daq")
        || driver_lower.contains("nidaq")
        || driver_lower.contains("pci-mio")
        || driver_lower.contains("pcimio")
    {
        return PanelType::Comedi;
    }

    // Priority 5: Laser controls (MaiTai-style devices)
    if device.is_emission_controllable()
        || device.is_shutter_controllable()
        || device.is_wavelength_tunable()
    {
        return PanelType::MaiTai;
    }

    // Priority 6: Pure readable devices (power meters, sensors)
    if device.is_readable() && !device.is_movable() {
        return PanelType::PowerMeter;
    }

    // Priority 7: Movable devices - distinguish rotator vs stage
    if device.is_movable() {
        if driver_lower.contains("ell14") || driver_lower.contains("rotator") {
            return PanelType::Rotator;
        }
        return PanelType::Stage;
    }

    // Default fallback: Generic panel for devices with no recognized capabilities.
    // Non-movable devices must not be placed in a motion UI by default.
    PanelType::Generic
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
        // Device with no known capabilities falls back to Generic (not Stage, to avoid
        // placing non-movable devices in a motion UI).
        let dev = make_device("Unknown Device", false, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::Generic));
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

    /// Helper: create a DeviceInfo with ui_schema_json in metadata
    fn make_device_with_schema(
        driver: &str,
        capabilities: &[&str],
        ui_schema_json: Option<&str>,
    ) -> DeviceInfo {
        #[allow(deprecated)]
        DeviceInfo {
            id: "test-device".to_string(),
            name: "Test Device".to_string(),
            driver_type: driver.to_string(),
            capabilities: capabilities.iter().map(|s| (*s).to_string()).collect(),
            metadata: Some(protocol::daq::DeviceMetadata {
                ui_schema_json: ui_schema_json.map(ToString::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_grpc_schema_takes_priority() {
        // Device with valid ui_schema_json → try_grpc_ui_config returns Some
        let schema = r#"{"control_panel":{"layout":"vertical","sections":[{"type":"sensor","label":"Power"}]}}"#;
        let dev = make_device_with_schema("ipg_laser", &["readable"], Some(schema));
        let config = try_grpc_ui_config(&dev);
        assert!(
            config.is_some(),
            "Expected gRPC schema to produce a ControlPanelConfig"
        );
        assert!(!config.unwrap().sections.is_empty());
    }

    #[test]
    fn test_grpc_schema_precedence_over_capability_dispatch() {
        // IPG-like device: readable (no movable, no frame_producer) + ui_schema_json
        // Without schema → PowerMeter; with schema → ConfigDriven
        let dev_no_schema = make_device(
            "universal_ipg_ylpp-200-1-50-r",
            false,
            true,
            false,
            false,
            false,
        );
        assert!(
            matches!(determine_panel_type(&dev_no_schema), PanelType::PowerMeter),
            "Without schema, IPG should fall to PowerMeter"
        );

        let schema = r#"{"control_panel":{"layout":"vertical","sections":[{"type":"sensor","label":"Output Power"}]}}"#;
        let dev_with_schema =
            make_device_with_schema("universal_ipg_ylpp-200-1-50-r", &["readable"], Some(schema));
        assert!(
            matches!(
                determine_panel_type_with_config(&dev_with_schema, None),
                PanelType::ConfigDriven(_)
            ),
            "With schema, IPG should get ConfigDriven panel"
        );
    }

    #[test]
    fn test_native_driver_no_schema_falls_through() {
        // Device with metadata but no ui_schema_json → None
        let dev = make_device_with_schema("pvcam_prime95b", &["frame_producer"], None);
        assert!(
            try_grpc_ui_config(&dev).is_none(),
            "No ui_schema_json should return None"
        );
    }

    #[test]
    fn test_malformed_json_falls_through() {
        // Invalid JSON in ui_schema_json → None (logs warning, doesn't panic)
        let dev = make_device_with_schema("ipg_laser", &["readable"], Some("{not valid json!!!}"));
        assert!(
            try_grpc_ui_config(&dev).is_none(),
            "Malformed JSON should return None"
        );
    }

    #[test]
    fn test_dispatch_andor_istar() {
        let dev = make_device("andor_istar", false, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::AndorCamera));
    }

    #[test]
    fn test_dispatch_andor_shamrock() {
        let dev = make_device("andor_shamrock", false, false, false, false, false);
        assert!(matches!(
            determine_panel_type(&dev),
            PanelType::Spectrograph
        ));
    }

    #[test]
    fn test_dispatch_dover_axis() {
        let dev = make_device("dover_axis", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::DoverStage));
    }

    #[test]
    fn test_dispatch_dover_over_generic_stage() {
        // Dover devices should get DoverStage, not generic Stage, even though movable
        let dev = make_device("dover_smartstage", true, false, false, false, false);
        assert!(matches!(determine_panel_type(&dev), PanelType::DoverStage));
    }

    #[test]
    fn test_pm400_not_misrouted_to_rotator() {
        // Regression: PM400 driver type contains "thorlabs" but must NOT route to Rotator.
        // Only "ell14" or "rotator" keywords should trigger Rotator dispatch.
        let dev = make_device("universal_thorlabs_pm400", false, true, false, false, false);
        assert!(
            matches!(determine_panel_type(&dev), PanelType::PowerMeter),
            "PM400 should be PowerMeter, not Rotator"
        );
    }

    #[test]
    fn test_thorlabs_ell14_still_routes_to_rotator() {
        // Ensure ELL14 rotators still correctly route after the thorlabs fix
        let dev = make_device("universal_thorlabs_ell14", true, false, false, false, false);
        assert!(
            matches!(determine_panel_type(&dev), PanelType::Rotator),
            "ELL14 should be Rotator"
        );
    }

    #[test]
    fn test_grpc_schema_with_command_driven_parameter() {
        // Full IPG-like schema with command-driven parameter section
        let schema = r#"{
            "control_panel": {
                "layout": "vertical",
                "sections": [
                    {"type": "sensor", "label": "Output Power", "precision": 2, "unit": "W"},
                    {"type": "custom_action", "label": "Emission ON", "command": "emission_on", "style": "success"},
                    {"type": "parameter", "label": "Rep Rate", "parameter": "rep_rate", "widget": "spinner", "read_command": "read_rep_rate", "write_command": "set_rep_rate"}
                ]
            }
        }"#;
        let dev = make_device_with_schema(
            "universal_ipg_ylpp-200-1-50-r",
            &["readable", "emission_controllable", "commandable"],
            Some(schema),
        );
        let config = try_grpc_ui_config(&dev);
        assert!(
            config.is_some(),
            "Schema with command-driven params should parse"
        );
        let panel = config.unwrap();
        assert_eq!(panel.sections.len(), 3);
        // Verify the parameter section preserved command fields
        match &panel.sections[2] {
            hardware::config::schema::ControlSection::Parameter(cfg) => {
                assert_eq!(cfg.read_command.as_deref(), Some("read_rep_rate"));
                assert_eq!(cfg.write_command.as_deref(), Some("set_rep_rate"));
            }
            _ => panic!("Expected Parameter section at index 2"),
        }
    }

    // =========================================================================
    // Phase 4: panel_kind regression tests
    // =========================================================================

    /// Helper: create a DeviceInfo with explicit panel_kind in metadata
    fn make_device_with_panel_kind(driver: &str, capabilities: &[&str], pk: &str) -> DeviceInfo {
        #[allow(deprecated)]
        DeviceInfo {
            id: "test-device".to_string(),
            name: "Test Device".to_string(),
            driver_type: driver.to_string(),
            capabilities: capabilities.iter().map(|s| (*s).to_string()).collect(),
            metadata: Some(protocol::daq::DeviceMetadata {
                panel_kind: Some(pk.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_panel_kind_power_meter_overrides_wavelength_tunable() {
        // Newport 1830-C: readable + wavelength_tunable → heuristic would route to MaiTai.
        // With panel_kind = "power_meter", must route to PowerMeter.
        let dev = make_device_with_panel_kind(
            "universal_newport_1830c",
            &["readable", "wavelength_tunable"],
            common::panel_kind::POWER_METER,
        );
        assert!(
            matches!(
                determine_panel_type_with_config(&dev, None),
                PanelType::PowerMeter
            ),
            "panel_kind=power_meter must override WavelengthTunable heuristic"
        );
    }

    #[test]
    fn test_panel_kind_pvcam_routes_to_camera() {
        let dev = make_device_with_panel_kind(
            "pvcam_prime95b",
            &["frame_producer"],
            common::panel_kind::PVCAM,
        );
        assert!(
            matches!(
                determine_panel_type_with_config(&dev, None),
                PanelType::Camera
            ),
            "panel_kind=pvcam must route to Camera"
        );
    }

    #[test]
    fn test_panel_kind_unknown_falls_through_to_generic() {
        // Forward-compat: unknown panel_kind should route to Generic, not panic.
        let dev = make_device_with_panel_kind("future_device", &[], "quantum_sensor_v2");
        assert!(
            matches!(
                determine_panel_type_with_config(&dev, None),
                PanelType::Generic
            ),
            "Unrecognized panel_kind must fall through to Generic"
        );
    }

    #[test]
    fn test_panel_kind_maitai_explicit() {
        let dev = make_device_with_panel_kind(
            "universal_maitai",
            &[
                "emission_controllable",
                "wavelength_tunable",
                "shutter_controllable",
            ],
            common::panel_kind::MAITAI,
        );
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::MaiTai
        ));
    }

    #[test]
    fn test_panel_kind_rotator_explicit() {
        let dev = make_device_with_panel_kind(
            "universal_thorlabs_ell14",
            &["movable"],
            common::panel_kind::ROTATOR,
        );
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::Rotator
        ));
    }

    #[test]
    fn test_panel_kind_comedi_explicit() {
        let dev = make_device_with_panel_kind(
            "comedi_analog_input",
            &["readable"],
            common::panel_kind::COMEDI,
        );
        assert!(matches!(
            determine_panel_type_with_config(&dev, None),
            PanelType::Comedi
        ));
    }
}
