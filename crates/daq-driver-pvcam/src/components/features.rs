//! PVCAM Feature Control
//!
//! Handles getting/setting camera parameters (Gain, Speed, Cooling, etc).

use anyhow::{anyhow, Result};
use crate::components::connection::PvcamConnection;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use crate::components::connection::get_pvcam_error;

pub struct PvcamFeatures;

impl PvcamFeatures {
    /// Get current sensor temperature in Celsius
    pub fn get_temperature(conn: &PvcamConnection) -> Result<f64> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = conn.handle() {
            let mut temp_raw: i16 = 0;
            unsafe {
                if pl_get_param(h, PARAM_TEMP, ATTR_CURRENT, &mut temp_raw as *mut _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to get temperature: {}", get_pvcam_error()));
                }
            }
            return Ok(temp_raw as f64 / 100.0);
        }
        Ok(-40.0)
    }

    /// Set temperature setpoint in Celsius
    pub fn set_temperature_setpoint(conn: &PvcamConnection, celsius: f64) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = conn.handle() {
            let temp_raw = (celsius * 100.0) as i16;
            unsafe {
                if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set temperature: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    // Add other feature methods as needed...
}
