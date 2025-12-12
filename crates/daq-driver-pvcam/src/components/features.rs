//! PVCAM Feature Control
//!
//! Handles getting/setting camera parameters (Gain, Speed, Cooling, etc).

use anyhow::Result;
#[cfg(feature = "pvcam_hardware")]
use anyhow::anyhow;
use crate::components::connection::PvcamConnection;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use crate::components::connection::get_pvcam_error;

// =============================================================================
// Data Structures
// =============================================================================

/// Comprehensive camera information
#[derive(Debug, Clone)]
pub struct CameraInfo {
    pub chip_name: String,
    pub temperature_c: f64,
    pub bit_depth: u16,
    pub readout_time_us: f64,
    pub pixel_size_nm: (u32, u32),
    pub sensor_size: (u32, u32),
    pub gain_name: String,
    pub speed_name: String,
}

#[derive(Debug, Clone)]
pub struct GainMode {
    pub index: u16,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SpeedMode {
    pub index: u16,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanSpeed {
    High,
    Medium,
    Low,
    Off,
}

impl FanSpeed {
    #[cfg(feature = "pvcam_hardware")]
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FanSpeed::High,
            1 => FanSpeed::Medium,
            2 => FanSpeed::Low,
            3 => FanSpeed::Off,
            _ => FanSpeed::High,
        }
    }

    #[cfg(feature = "pvcam_hardware")]
    pub fn to_pvcam(self) -> i32 {
        match self {
            FanSpeed::High => 0,
            FanSpeed::Medium => 1,
            FanSpeed::Low => 2,
            FanSpeed::Off => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PPFeature {
    pub index: u16,
    pub id: u16,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PPParam {
    pub index: u16,
    pub id: u16,
    pub name: String,
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CentroidsMode {
    Locate,
    Track,
    Blob,
}

impl CentroidsMode {
    #[cfg(feature = "pvcam_hardware")]
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => CentroidsMode::Locate,
            1 => CentroidsMode::Track,
            2 => CentroidsMode::Blob,
            _ => CentroidsMode::Locate,
        }
    }

    #[cfg(feature = "pvcam_hardware")]
    pub fn to_pvcam(self) -> i32 {
        match self {
            CentroidsMode::Locate => 0,
            CentroidsMode::Track => 1,
            CentroidsMode::Blob => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CentroidsConfig {
    pub mode: CentroidsMode,
    pub radius: u16,
    pub max_count: u16,
    pub threshold: u32,
}

// =============================================================================
// Feature Logic
// =============================================================================

pub struct PvcamFeatures;

impl PvcamFeatures {
    /// Get current sensor temperature in Celsius
    pub fn get_temperature(_conn: &PvcamConnection) -> Result<f64> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut temp_raw: i16 = 0;
            unsafe {
                // SAFETY: h is a valid open handle; temp_raw is a writable i16 on the stack.
                if pl_get_param(h, PARAM_TEMP, ATTR_CURRENT, &mut temp_raw as *mut _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to get temperature: {}", get_pvcam_error()));
                }
            }
            return Ok(temp_raw as f64 / 100.0);
        }
        Ok(-40.0)
    }

    /// Set temperature setpoint in Celsius
    pub fn set_temperature_setpoint(_conn: &PvcamConnection, _celsius: f64) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let temp_raw = (_celsius * 100.0) as i16;
            unsafe {
                // SAFETY: h is a valid open handle; temp_raw pointer valid for duration of call.
                if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set temperature: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    // Add other feature methods as needed...
}
