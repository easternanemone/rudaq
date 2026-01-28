//! Type definitions for the Instrument Manager Panel.
//!
//! Note: DeviceCategory here is a GUI-specific type with `from_device_info()` inference
//! and icon methods. It mirrors `common::capabilities::DeviceCategory` but adds
//! GUI presentation logic that depends on `protocol::daq::DeviceInfo`.

use protocol::daq::DeviceInfo;

/// Device category for grouping in the tree view.
///
/// This is a GUI-specific type that provides inference from `DeviceInfo` proto
/// and presentation methods (icons, labels). Mirrors `common::capabilities::DeviceCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceCategory {
    Camera,
    Stage,
    Detector,
    Laser,
    PowerMeter,
    Other,
}

impl DeviceCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Camera => "Cameras",
            Self::Stage => "Stages",
            Self::Detector => "Detectors",
            Self::Laser => "Lasers",
            Self::PowerMeter => "Power Meters",
            Self::Other => "Other",
        }
    }

    /// Icon for UI display (aligned with common::capabilities::DeviceCategory)
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Camera => "📷",
            Self::Stage => "🔄",
            Self::Detector => "📊",
            Self::Laser => "💡",
            Self::PowerMeter => "⚡",
            Self::Other => "🔧",
        }
    }

    /// Infer category from device capabilities
    pub fn from_device_info(info: &DeviceInfo) -> Self {
        // Priority: Laser > Camera > Stage > PowerMeter > Detector > Other
        if info.is_emission_controllable
            || info.is_shutter_controllable
            || info.is_wavelength_tunable
        {
            Self::Laser
        } else if info.is_frame_producer {
            Self::Camera
        } else if info.is_movable {
            Self::Stage
        } else if info.is_readable {
            // Could be detector or power meter - check driver name
            if info.driver_type.to_lowercase().contains("power") {
                Self::PowerMeter
            } else {
                Self::Detector
            }
        } else {
            Self::Other
        }
    }
}

/// Grouped devices for tree display
#[derive(Clone)]
pub struct DeviceGroup {
    pub category: DeviceCategory,
    pub devices: Vec<DeviceInfo>,
    pub expanded: bool,
}

/// Parameter with current value for display
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
    pub dtype: String,
    pub units: String,
    #[allow(dead_code)]
    pub readable: bool,
    pub writable: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub enum_values: Vec<String>,
    pub current_value: Option<String>,
}

/// Request to pop out a device control panel into a dockable window
#[derive(Debug, Clone)]
pub struct PopOutRequest {
    /// Full device info with capability flags
    pub device_info: DeviceInfo,
}
