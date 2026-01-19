//! Panel dispatch logic for device-to-control-panel mapping.
//!
//! This module provides a centralized function to determine which control panel
//! type should be used for a given device based on its capabilities.

use daq_proto::daq::DeviceInfo;

/// The type of control panel to use for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelType {
    /// MaiTai Ti:Sapphire laser control panel (wavelength, emission, shutter)
    MaiTai,
    /// Power meter control panel (readable sensors)
    PowerMeter,
    /// Rotator control panel (ELL14-style rotation mounts)
    Rotator,
    /// Stage control panel (linear/XY stages)
    Stage,
}

/// Determine the appropriate control panel type for a device.
///
/// Priority order:
/// 1. Laser capabilities (emission/shutter/wavelength) → MaiTai
/// 2. Readable without motion (sensors, meters) → PowerMeter
/// 3. Movable with "ell14" in driver name → Rotator
/// 4. Movable → Stage (default for motion devices)
///
/// # Arguments
/// * `device` - Device info with capability flags
///
/// # Returns
/// The `PanelType` to use for this device's control panel
pub fn determine_panel_type(device: &DeviceInfo) -> PanelType {
    // Priority 1: Laser controls (MaiTai-style devices)
    if device.is_emission_controllable
        || device.is_shutter_controllable
        || device.is_wavelength_tunable
    {
        return PanelType::MaiTai;
    }

    // Priority 2: Pure readable devices (power meters, sensors)
    if device.is_readable && !device.is_movable {
        return PanelType::PowerMeter;
    }

    // Priority 3: Movable devices - distinguish rotator vs stage
    if device.is_movable {
        let driver_lower = device.driver_type.to_lowercase();
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
        DeviceInfo {
            id: "test-device".to_string(),
            name: "Test Device".to_string(),
            driver_type: driver.to_string(),
            is_movable: movable,
            is_readable: readable,
            is_emission_controllable: emission,
            is_shutter_controllable: shutter,
            is_wavelength_tunable: wavelength,
            ..Default::default()
        }
    }

    #[test]
    fn test_dispatch_maitai_by_emission() {
        let dev = make_device("MaiTai DeepSee", false, true, true, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::MaiTai);
    }

    #[test]
    fn test_dispatch_maitai_by_shutter() {
        let dev = make_device("SomeLaser", false, true, false, true, false);
        assert_eq!(determine_panel_type(&dev), PanelType::MaiTai);
    }

    #[test]
    fn test_dispatch_maitai_by_wavelength() {
        let dev = make_device("TunableLaser", false, true, false, false, true);
        assert_eq!(determine_panel_type(&dev), PanelType::MaiTai);
    }

    #[test]
    fn test_dispatch_maitai_priority_over_readable() {
        // MaiTai priority even if device is also readable
        let dev = make_device("MaiTai", false, true, true, true, true);
        assert_eq!(determine_panel_type(&dev), PanelType::MaiTai);
    }

    #[test]
    fn test_dispatch_power_meter() {
        let dev = make_device("Newport 1830-C", false, true, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::PowerMeter);
    }

    #[test]
    fn test_dispatch_rotator_ell14() {
        let dev = make_device("Thorlabs ELL14", true, false, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Rotator);
    }

    #[test]
    fn test_dispatch_rotator_by_keyword() {
        let dev = make_device("Custom Rotator Mount", true, false, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Rotator);
    }

    #[test]
    fn test_dispatch_stage_esp300() {
        let dev = make_device("Newport ESP300", true, false, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Stage);
    }

    #[test]
    fn test_dispatch_stage_fallback() {
        // Generic movable device defaults to Stage
        let dev = make_device("Unknown Motor", true, false, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Stage);
    }

    #[test]
    fn test_dispatch_no_capabilities_fallback() {
        // Device with no known capabilities falls back to Stage
        let dev = make_device("Unknown Device", false, false, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Stage);
    }

    #[test]
    fn test_dispatch_readable_movable_is_stage() {
        // Readable + movable should be Stage (not PowerMeter)
        let dev = make_device("Encoder Stage", true, true, false, false, false);
        assert_eq!(determine_panel_type(&dev), PanelType::Stage);
    }
}
