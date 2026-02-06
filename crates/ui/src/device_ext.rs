//! Extension trait for DeviceInfo capability checking.
//!
//! Replaces deprecated boolean fields (is_movable, is_readable, etc.)
//! with lookups into the canonical `capabilities` repeated string field.

use protocol::daq::DeviceInfo;

pub(crate) trait DeviceInfoExt {
    fn has_capability(&self, name: &str) -> bool;
    fn is_movable(&self) -> bool {
        self.has_capability("movable")
    }
    fn is_readable(&self) -> bool {
        self.has_capability("readable")
    }
    fn is_frame_producer(&self) -> bool {
        self.has_capability("frame_producer")
    }
    fn is_triggerable(&self) -> bool {
        self.has_capability("triggerable")
    }
    fn is_exposure_controllable(&self) -> bool {
        self.has_capability("exposure_controllable")
    }
    fn is_shutter_controllable(&self) -> bool {
        self.has_capability("shutter_controllable")
    }
    fn is_wavelength_tunable(&self) -> bool {
        self.has_capability("wavelength_tunable")
    }
    fn is_emission_controllable(&self) -> bool {
        self.has_capability("emission_controllable")
    }
    fn is_parameterized(&self) -> bool {
        self.has_capability("parameterized")
    }
}

impl DeviceInfoExt for DeviceInfo {
    fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }
}
