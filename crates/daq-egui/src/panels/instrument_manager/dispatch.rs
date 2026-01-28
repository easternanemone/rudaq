//! Panel dispatch logic for device-to-control-panel mapping.
//!
//! This module provides a centralized function to determine which control panel
//! type should be used for a given device based on its capabilities.
//!
//! Note: Currently unused - panel selection is inline in `render_device_control_panel`.
//! This module is retained for future refactoring to centralize panel dispatch logic.

#![allow(dead_code)]

use protocol::daq::DeviceInfo;

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
    /// Comedi DAQ unified control panel (AI, AO, DIO, counters)
    Comedi,
}

/// Determine the appropriate control panel type for a device.
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
    if device.is_emission_controllable
        || device.is_shutter_controllable
        || device.is_wavelength_tunable
    {
        return PanelType::MaiTai;
    }

    // Priority 3: Pure readable devices (power meters, sensors)
    if device.is_readable && !device.is_movable {
        return PanelType::PowerMeter;
    }

    // Priority 4: Movable devices - distinguish rotator vs stage
    if device.is_movable {
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
