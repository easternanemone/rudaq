//! PVCAM Feature Control
//!
//! Handles getting/setting camera parameters (Gain, Speed, Cooling, etc).
//!
//! ## Prime BSI Features (bd-3apt)
//!
//! This module provides access to Prime BSI camera features:
//! - **Camera Info**: Serial number, firmware version, chip name (bd-565x)
//! - **Fan Control**: Fan speed settings for thermal management (bd-glia)
//! - **Readout/Speed**: Port selection, speed tables, bit depth (bd-v54z)
//! - **Gain Control**: Gain index and multiplication factor (bd-doju)
//! - **Temperature**: Sensor temperature monitoring and setpoint

mod enums;
mod types;

pub use enums::*;
pub use types::*;

use crate::components::connection::PvcamConnection;
#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use anyhow::anyhow;
use anyhow::{Context as _, Result};

#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use crate::components::connection::get_pvcam_error;
#[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
use pvcam_sys::*;
#[cfg(feature = "pvcam_sdk")]
use std::ffi::CStr;

// =============================================================================
// Feature Logic
// =============================================================================

pub struct PvcamFeatures;

/// Normalize a PP feature name for matching: lowercase and strip spaces/underscores.
///
/// The PVCAM SDK may report feature names as "PrimeEnhance", "PRIME ENHANCE",
/// "Prime_Enhance", etc. depending on firmware version and camera model.
/// This normalizes to a canonical form for reliable comparison (bd-ldjy.1).
pub fn normalize_pp_name(name: &str) -> String {
    name.to_lowercase().replace([' ', '_', '-'], "")
}

/// Check if a PP feature name matches a target name, case-insensitive and
/// tolerant of spaces, underscores, and hyphens (bd-ldjy.1).
///
/// # Examples
/// All of these match "primeenhance":
/// - "PrimeEnhance"
/// - "PRIME ENHANCE"
/// - "Prime_Enhance"
/// - "prime-enhance"
pub fn pp_name_matches(feature_name: &str, target: &str) -> bool {
    normalize_pp_name(feature_name) == normalize_pp_name(target)
}

/// Check if a PP feature name contains a target substring, case-insensitive and
/// tolerant of spaces, underscores, and hyphens (bd-ldjy.1).
pub fn pp_name_contains(feature_name: &str, target: &str) -> bool {
    normalize_pp_name(feature_name).contains(&normalize_pp_name(target))
}

/// Check if a PP feature name refers to PrimeEnhance (bd-ldjy.1).
///
/// The PVCAM SDK registers PrimeEnhance as `PP_FEATURE_DENOISING`, so cameras
/// may report the feature name as "Denoising" rather than "PrimeEnhance".
/// This helper checks both canonical names.
pub fn is_prime_enhance_name(feature_name: &str) -> bool {
    pp_name_matches(feature_name, "PrimeEnhance") || pp_name_contains(feature_name, "denoising")
}

/// Check if a PP feature name refers to PrimeLocate (bd-ldjy.1).
///
/// The PVCAM SDK registers PrimeLocate as `PP_FEATURE_LOCATE` / particle
/// tracking, so cameras may report the feature name as "Locate" rather than
/// "PrimeLocate".  This helper checks both canonical names.
pub fn is_prime_locate_name(feature_name: &str) -> bool {
    pp_name_matches(feature_name, "PrimeLocate") || pp_name_contains(feature_name, "locate")
}

impl PvcamFeatures {
    // =========================================================================
    // Parameter Availability Check (SDK Pattern - bd-ng5p)
    // =========================================================================

    /// Check if a parameter is available on the connected camera.
    ///
    /// This implements the SDK's `IsParamAvailable` pattern. The PVCAM SDK
    /// documentation emphasizes checking parameter availability before access
    /// because not all cameras support all parameters.
    ///
    /// # SDK Reference
    /// From PVCAM SDK Common.cpp:
    /// ```cpp
    /// bool IsParamAvailable(int16 hcam, uns32 paramID, const char* paramName)
    /// {
    ///     rs_bool isAvailable;
    ///     if (PV_OK != pl_get_param(hcam, paramID, ATTR_AVAIL, (void*)&isAvailable))
    ///         return false;
    ///     return isAvailable != FALSE;
    /// }
    /// ```
    ///
    /// # Returns
    /// - `true` if the parameter is available on this camera
    /// - `false` if the parameter is unavailable or the check failed
    #[cfg(feature = "pvcam_sdk")]
    pub fn is_param_available(hcam: i16, param_id: u32) -> bool {
        let mut avail: rs_bool = 0;
        // SAFETY: hcam is a valid camera handle. avail is a stack-allocated rs_bool
        // of the correct type for ATTR_AVAIL. This is a read-only query.
        unsafe {
            if pl_get_param(
                hcam,
                param_id,
                ATTR_AVAIL as i16,
                &mut avail as *mut _ as *mut std::ffi::c_void,
            ) != 0
            {
                avail != 0
            } else {
                false
            }
        }
    }

    /// Check if a parameter is available, returning an error with context if not.
    ///
    /// Use this variant when parameter unavailability should produce an error
    /// rather than a silent fallback.
    #[cfg(feature = "pvcam_sdk")]
    pub fn require_param_available(hcam: i16, param_id: u32, param_name: &str) -> Result<()> {
        if Self::is_param_available(hcam, param_id) {
            Ok(())
        } else {
            Err(anyhow!(
                "Parameter {} (0x{:08X}) is not available on this camera",
                param_name,
                param_id
            ))
        }
    }

    // =========================================================================
    // Temperature Control
    // =========================================================================

    /// Get current sensor temperature in Celsius
    ///
    /// # SDK Pattern (bd-ng5p)
    /// Checks PARAM_TEMP availability before access, matching SDK pattern
    /// from FanSpeedAndTemperature.cpp example.
    pub fn get_temperature(_conn: &PvcamConnection) -> Result<f64> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_TEMP) {
                return Err(anyhow!("PARAM_TEMP is not available on this camera"));
            }

            let mut temp_raw: i16 = 0;
            // SAFETY: h is a valid open handle; temp_raw is a writable i16 on the stack.
            unsafe {
                // SAFETY: h is a valid open handle; temp_raw is a writable i16 on the stack.
                if pl_get_param(
                    h,
                    PARAM_TEMP,
                    ATTR_CURRENT,
                    &mut temp_raw as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get temperature: {}", get_pvcam_error()));
                }
            }
            return Ok(temp_raw as f64 / 100.0);
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(_conn.mock_state.lock().unwrap().temperature_c);

        #[cfg(feature = "pvcam_sdk")]
        Ok(-40.0)
    }

    /// Set temperature setpoint in Celsius
    ///
    /// # SDK Pattern (bd-ng5p)
    /// Checks PARAM_TEMP_SETPOINT availability before access.
    pub fn set_temperature_setpoint(_conn: &PvcamConnection, _celsius: f64) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_TEMP_SETPOINT) {
                return Err(anyhow!(
                    "PARAM_TEMP_SETPOINT is not available on this camera"
                ));
            }

            let temp_raw = (_celsius * 100.0) as i16;
            // SAFETY: h is a valid open handle; temp_raw pointer valid for duration of call.
            unsafe {
                // SAFETY: h is a valid open handle; temp_raw pointer valid for duration of call.
                if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set temperature: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.temperature_setpoint_c = _celsius;
        }
        Ok(())
    }

    // =========================================================================
    // Camera Info & Diagnostics (bd-565x)
    // =========================================================================

    /// Get comprehensive camera information
    pub fn get_camera_info(_conn: &PvcamConnection) -> Result<CameraInfo> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Ok(CameraInfo {
                serial_number: Self::get_serial_number_impl(h)?,
                firmware_version: Self::get_firmware_version_impl(h)?,
                chip_name: Self::get_chip_name_impl(h)?,
                temperature_c: Self::get_temperature(_conn)?,
                bit_depth: Self::get_bit_depth_impl(h)?,
                pixel_time_ns: Self::get_pixel_time_impl(h)?,
                pixel_size_nm: Self::get_pixel_size_impl(h)?,
                sensor_size: Self::get_sensor_size_impl(h)?,
                gain_name: Self::get_enum_string_impl(h, PARAM_GAIN_INDEX).unwrap_or_default(),
                speed_name: Self::get_enum_string_impl(h, PARAM_SPDTAB_INDEX).unwrap_or_default(),
                port_name: Self::get_enum_string_impl(h, PARAM_READOUT_PORT).unwrap_or_default(),
                gain_index: Self::get_u16_param_impl(h, PARAM_GAIN_INDEX).unwrap_or(0),
                speed_index: Self::get_u16_param_impl(h, PARAM_SPDTAB_INDEX).unwrap_or(0),
            });
        }
        // Mock mode returns default values
        Ok(CameraInfo {
            serial_number: "MOCK-001".to_string(),
            firmware_version: "1.0.0".to_string(),
            chip_name: "MockSensor".to_string(),
            temperature_c: -40.0,
            bit_depth: 16,
            pixel_time_ns: 10,
            pixel_size_nm: (6500, 6500),
            sensor_size: (2048, 2048),
            gain_name: "HDR".to_string(),
            speed_name: "100 MHz".to_string(),
            port_name: "Sensitivity".to_string(),
            gain_index: 0,
            speed_index: 0,
        })
    }

    /// Get camera serial number (bd-565x)
    pub fn get_serial_number(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_serial_number_impl(h);
        }
        Ok("MOCK-001".to_string())
    }

    /// Get firmware version string (bd-565x)
    pub fn get_firmware_version(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_firmware_version_impl(h);
        }
        Ok("1.0.0".to_string())
    }

    /// Get sensor chip name (bd-565x)
    pub fn get_chip_name(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_chip_name_impl(h);
        }
        Ok("MockSensor".to_string())
    }

    /// Get device driver version string (bd-qijv)
    ///
    /// # SDK Pattern (bd-qijv)
    /// Checks PARAM_DD_VERSION availability before access.
    pub fn get_device_driver_version(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_device_driver_version_impl(h);
        }
        Ok("3.0.0".to_string())
    }

    /// Get current bit depth (bd-565x)
    pub fn get_bit_depth(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_bit_depth_impl(h);
        }
        Ok(16)
    }

    /// Get pixel readout time in nanoseconds (bd-565x)
    pub fn get_pixel_time(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_pixel_time_impl(h);
        }
        Ok(10)
    }

    // =========================================================================
    // Fan Speed Control (bd-glia)
    // =========================================================================

    /// Get current fan speed setting (bd-glia)
    pub fn get_fan_speed(_conn: &PvcamConnection) -> Result<FanSpeed> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_FAN_SPEED_SETPOINT,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get fan speed: {}", get_pvcam_error()));
                }
            }
            return Ok(FanSpeed::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(FanSpeed::from_pvcam(state.fan_speed))
        }

        #[cfg(feature = "pvcam_sdk")]
        Ok(FanSpeed::High)
    }

    /// Set fan speed (bd-glia)
    pub fn set_fan_speed(_conn: &PvcamConnection, _speed: FanSpeed) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _speed.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_FAN_SPEED_SETPOINT, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set fan speed: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.fan_speed = _speed.to_pvcam();
        }
        Ok(())
    }

    // =========================================================================
    // Readout & Speed Control (bd-v54z)
    // =========================================================================

    /// Get current speed table index (bd-v54z)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_SPDTAB_INDEX availability before access.
    pub fn get_speed_index(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SPDTAB_INDEX) {
                return Err(anyhow!(
                    "PARAM_SPDTAB_INDEX is not available on this camera"
                ));
            }
            return Self::get_u16_param_impl(h, PARAM_SPDTAB_INDEX)
                .map_err(|e| anyhow!("Failed to get speed index: {}", e));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(_conn.mock_state.lock().unwrap().speed_index);

        #[cfg(feature = "pvcam_sdk")]
        Ok(0)
    }

    /// Set speed table index (bd-v54z)
    ///
    /// Changes the readout speed. Valid indices can be obtained from `list_speed_modes()`.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_SPDTAB_INDEX availability before access.
    pub fn set_speed_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SPDTAB_INDEX) {
                return Err(anyhow!(
                    "PARAM_SPDTAB_INDEX is not available on this camera"
                ));
            }
            let value = _index as i32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set speed index: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.speed_index = _index;
        }
        Ok(())
    }

    /// Get current readout port index (bd-v54z)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_READOUT_PORT availability before access.
    pub fn get_readout_port(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_READOUT_PORT) {
                return Err(anyhow!(
                    "PARAM_READOUT_PORT is not available on this camera"
                ));
            }
            return Self::get_u16_param_impl(h, PARAM_READOUT_PORT)
                .map_err(|e| anyhow!("Failed to get readout port: {}", e));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(_conn.mock_state.lock().unwrap().readout_port_index);

        #[cfg(feature = "pvcam_sdk")]
        Ok(0)
    }

    /// Set readout port (bd-v54z)
    ///
    /// Valid ports can be obtained from `list_readout_ports()`.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_READOUT_PORT availability before access.
    pub fn set_readout_port(_conn: &PvcamConnection, _port: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_READOUT_PORT) {
                return Err(anyhow!(
                    "PARAM_READOUT_PORT is not available on this camera"
                ));
            }
            let value = _port as i32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_READOUT_PORT, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set readout port: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.readout_port_index = _port;
        }
        Ok(())
    }

    /// List available speed modes (bd-v54z)
    ///
    /// Returns all speed table entries with their properties.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_SPDTAB_INDEX availability before access.
    pub fn list_speed_modes(_conn: &PvcamConnection) -> Result<Vec<SpeedMode>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SPDTAB_INDEX) {
                return Err(anyhow!(
                    "PARAM_SPDTAB_INDEX is not available on this camera"
                ));
            }
            let count = Self::get_enum_count_impl(h, PARAM_SPDTAB_INDEX)?;
            let mut modes = Vec::with_capacity(count as usize);

            // Save current speed index to restore after enumeration
            let current_speed = Self::get_u16_param_impl(h, PARAM_SPDTAB_INDEX).unwrap_or(0);

            for i in 0..count {
                // Set speed index to enumerate its properties
                let idx = i as i32;
                // SAFETY: h is valid; setting speed index to enumerate properties.
                unsafe {
                    // SAFETY: h is valid; setting speed index to enumerate properties.
                    if pl_set_param(h, PARAM_SPDTAB_INDEX, &idx as *const _ as *mut _) != 0 {
                        modes.push(SpeedMode {
                            index: i as u16,
                            name: Self::get_enum_item_by_index_impl(h, PARAM_SPDTAB_INDEX, i)
                                .map(|(_, name)| name)
                                .unwrap_or_else(|_| format!("Speed {}", i)),
                            pixel_time_ns: Self::get_pixel_time_impl(h).unwrap_or(0),
                            bit_depth: Self::get_bit_depth_impl(h).unwrap_or(16),
                            port_index: Self::get_u16_param_impl(h, PARAM_READOUT_PORT)
                                .unwrap_or(0),
                        });
                    }
                }
            }

            // Restore original speed index
            let restore = current_speed as i32;
            // SAFETY: h is valid; restoring original speed index.
            unsafe {
                // SAFETY: h is valid; restoring original speed index.
                let _ = pl_set_param(h, PARAM_SPDTAB_INDEX, &restore as *const _ as *mut _);
            }

            return Ok(modes);
        }
        // Mock mode
        Ok(vec![
            SpeedMode {
                index: 0,
                name: "100 MHz".to_string(),
                pixel_time_ns: 10,
                bit_depth: 16,
                port_index: 0,
            },
            SpeedMode {
                index: 1,
                name: "50 MHz".to_string(),
                pixel_time_ns: 20,
                bit_depth: 16,
                port_index: 0,
            },
        ])
    }

    /// List available readout ports (bd-v54z)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_READOUT_PORT availability before access.
    pub fn list_readout_ports(_conn: &PvcamConnection) -> Result<Vec<ReadoutPort>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_READOUT_PORT) {
                return Err(anyhow!(
                    "PARAM_READOUT_PORT is not available on this camera"
                ));
            }
            let count = Self::get_enum_count_impl(h, PARAM_READOUT_PORT)?;
            let mut ports = Vec::with_capacity(count as usize);

            // Save current port to restore after enumeration
            let current_port = Self::get_u16_param_impl(h, PARAM_READOUT_PORT).unwrap_or(0);

            for i in 0..count {
                let idx = i as i32;
                // SAFETY: h is valid; setting port to enumerate properties.
                unsafe {
                    // SAFETY: h is valid; setting port to enumerate properties.
                    if pl_set_param(h, PARAM_READOUT_PORT, &idx as *const _ as *mut _) != 0 {
                        ports.push(ReadoutPort {
                            index: i as u16,
                            name: Self::get_enum_item_by_index_impl(h, PARAM_READOUT_PORT, i)
                                .map(|(_, name)| name)
                                .unwrap_or_else(|_| format!("Port {}", i)),
                        });
                    }
                }
            }

            // Restore original port
            let restore = current_port as i32;
            // SAFETY: h is valid; restoring original port.
            unsafe {
                // SAFETY: h is valid; restoring original port.
                let _ = pl_set_param(h, PARAM_READOUT_PORT, &restore as *const _ as *mut _);
            }

            return Ok(ports);
        }
        // Mock mode
        Ok(vec![
            ReadoutPort {
                index: 0,
                name: "Sensitivity".to_string(),
            },
            ReadoutPort {
                index: 1,
                name: "Speed".to_string(),
            },
        ])
    }

    // =========================================================================
    // Gain Control (bd-doju)
    // =========================================================================

    /// Get current gain index (bd-doju)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_GAIN_INDEX availability before access.
    pub fn get_gain_index(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_GAIN_INDEX) {
                return Err(anyhow!("PARAM_GAIN_INDEX is not available on this camera"));
            }
            return Self::get_u16_param_impl(h, PARAM_GAIN_INDEX)
                .map_err(|e| anyhow!("Failed to get gain index: {}", e));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(_conn.mock_state.lock().unwrap().gain_index);

        #[cfg(feature = "pvcam_sdk")]
        Ok(0)
    }

    /// Set gain index (bd-doju)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_GAIN_INDEX availability before access.
    pub fn set_gain_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_GAIN_INDEX) {
                return Err(anyhow!("PARAM_GAIN_INDEX is not available on this camera"));
            }
            let value = _index as i32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_GAIN_INDEX, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set gain index: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.gain_index = _index;
        }
        Ok(())
    }

    /// List available gain modes (bd-doju)
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_GAIN_INDEX availability before access.
    pub fn list_gain_modes(_conn: &PvcamConnection) -> Result<Vec<GainMode>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_GAIN_INDEX) {
                return Err(anyhow!("PARAM_GAIN_INDEX is not available on this camera"));
            }
            let count = Self::get_enum_count_impl(h, PARAM_GAIN_INDEX)?;
            let mut modes = Vec::with_capacity(count as usize);

            // Save current gain to restore after enumeration
            let current_gain = Self::get_u16_param_impl(h, PARAM_GAIN_INDEX).unwrap_or(0);

            for i in 0..count {
                let idx = i as i32;
                // SAFETY: h is valid; setting gain index to enumerate properties.
                unsafe {
                    // SAFETY: h is valid; setting gain index to enumerate properties.
                    if pl_set_param(h, PARAM_GAIN_INDEX, &idx as *const _ as *mut _) != 0 {
                        modes.push(GainMode {
                            index: i as u16,
                            name: Self::get_enum_item_by_index_impl(h, PARAM_GAIN_INDEX, i)
                                .map(|(_, name)| name)
                                .unwrap_or_else(|_| format!("Gain {}", i)),
                        });
                    }
                }
            }

            // Restore original gain
            let restore = current_gain as i32;
            // SAFETY: h is valid; restoring original gain index.
            unsafe {
                // SAFETY: h is valid; restoring original gain index.
                let _ = pl_set_param(h, PARAM_GAIN_INDEX, &restore as *const _ as *mut _);
            }

            return Ok(modes);
        }
        // Mock mode
        Ok(vec![
            GainMode {
                index: 0,
                name: "HDR".to_string(),
            },
            GainMode {
                index: 1,
                name: "CMS".to_string(),
            },
        ])
    }

    // =========================================================================
    // Triggering & Exposure Modes (bd-iai9)
    // =========================================================================

    /// Get current exposure mode (bd-iai9)
    ///
    /// # SDK Pattern (bd-smn3)
    /// Checks PARAM_EXPOSURE_MODE availability before access.
    pub fn get_exposure_mode(_conn: &PvcamConnection) -> Result<ExposureMode> {
        #[cfg(feature = "pvcam_sdk")]
        {
            return Ok(ExposureMode::from_pvcam(Self::get_exposure_mode_raw(
                _conn,
            )?));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ExposureMode::from_pvcam(state.exposure_mode))
        }
    }

    /// Get current exposure mode as raw PVCAM integer value.
    ///
    /// Useful when the camera reports extended exposure modes outside the legacy
    /// enum range, e.g. `EXT_TRIG_*` values.
    pub fn get_exposure_mode_raw(_conn: &PvcamConnection) -> Result<i32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSURE_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSURE_MODE is not available on this camera"
                ));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_EXPOSURE_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get exposure mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value);
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(state.exposure_mode)
        }

        #[cfg(feature = "pvcam_sdk")]
        Ok(0)
    }

    /// Set exposure mode (bd-iai9)
    ///
    /// # SDK Pattern (bd-smn3)
    /// Checks PARAM_EXPOSURE_MODE availability before access.
    pub fn set_exposure_mode(_conn: &PvcamConnection, _mode: ExposureMode) -> Result<()> {
        Self::set_exposure_mode_raw(_conn, _mode.to_pvcam())
    }

    /// Set exposure mode using raw PVCAM integer value.
    pub fn set_exposure_mode_raw(_conn: &PvcamConnection, _mode_raw: i32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSURE_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSURE_MODE is not available on this camera"
                ));
            }
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_EXPOSURE_MODE, &_mode_raw as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set exposure mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.exposure_mode = _mode_raw;
        }
        Ok(())
    }

    /// List available exposure modes from hardware (bd-q4wz)
    ///
    /// Dynamically queries the camera to discover which exposure modes are supported.
    /// Returns a list of (value, name) pairs for all available modes.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_EXPOSURE_MODE availability before access.
    pub fn list_exposure_modes(_conn: &PvcamConnection) -> Result<Vec<(i32, String)>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSURE_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSURE_MODE is not available on this camera"
                ));
            }
            let count = Self::get_enum_count_impl(h, PARAM_EXPOSURE_MODE)?;
            let mut modes = Vec::with_capacity(count as usize);

            for idx in 0..count {
                if let Ok((value, name)) =
                    Self::get_enum_item_by_index_impl(h, PARAM_EXPOSURE_MODE, idx)
                {
                    modes.push((value, name));
                }
            }
            return Ok(modes);
        }
        // Mock mode: return standard exposure modes
        Ok(vec![
            (0, "Timed".to_string()),
            (1, "Strobe".to_string()),
            (2, "Bulb".to_string()),
            (3, "TriggerFirst".to_string()),
            (4, "EdgeTrigger".to_string()),
        ])
    }

    /// Get current clear mode (bd-iai9)
    pub fn get_clear_mode(_conn: &PvcamConnection) -> Result<ClearMode> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_CLEAR_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get clear mode: {}", get_pvcam_error()));
                }
            }
            return Ok(ClearMode::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ClearMode::from_pvcam(state.clear_mode))
        }

        #[cfg(feature = "pvcam_sdk")]
        Ok(ClearMode::PreExposure)
    }

    /// Set clear mode (bd-iai9)
    pub fn set_clear_mode(_conn: &PvcamConnection, _mode: ClearMode) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_CLEAR_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set clear mode: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.clear_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// List available clear modes from hardware (bd-q4wz)
    ///
    /// Dynamically queries the camera to discover which clear modes are supported.
    /// Returns a list of (value, name) pairs for all available modes.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_CLEAR_MODE availability before access.
    pub fn list_clear_modes(_conn: &PvcamConnection) -> Result<Vec<(i32, String)>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CLEAR_MODE) {
                return Err(anyhow!("PARAM_CLEAR_MODE is not available on this camera"));
            }
            let count = Self::get_enum_count_impl(h, PARAM_CLEAR_MODE)?;
            let mut modes = Vec::with_capacity(count as usize);

            for idx in 0..count {
                // SAFETY: h is a valid camera handle. idx is within 0..count from prior
                // enumeration. name buffer is stack-allocated [0i8; 256] with sufficient size.
                unsafe {
                    let mut value: i32 = 0;
                    let mut name = [0i8; 256];
                    let mut name_len: uns32 = 256;

                    // Get string length first
                    if pl_enum_str_length(h, PARAM_CLEAR_MODE, idx, &mut name_len) != 0 {
                        // Get the enum entry with value and name
                        if pl_get_enum_param(
                            h,
                            PARAM_CLEAR_MODE,
                            idx,
                            &mut value,
                            name.as_mut_ptr(),
                            name_len.min(256),
                        ) != 0
                        {
                            let name_str =
                                CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
                            modes.push((value, name_str));
                        }
                    }
                }
            }
            return Ok(modes);
        }
        // Mock mode: return standard clear modes
        Ok(vec![
            (0, "Never".to_string()),
            (1, "PreExposure".to_string()),
            (2, "PreSequence".to_string()),
            (3, "PostSequence".to_string()),
            (4, "PrePostSequence".to_string()),
            (5, "PreExposurePostSequence".to_string()),
        ])
    }

    /// Get the number of sensor clearing cycles (bd-0yho)
    ///
    /// # SDK Pattern (bd-0yho)
    /// Checks PARAM_CLEAR_CYCLES availability before access.
    pub fn get_clear_cycles(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CLEAR_CYCLES) {
                return Err(anyhow!(
                    "PARAM_CLEAR_CYCLES is not available on this camera"
                ));
            }
            let mut value: uns16 = 0;
            // SAFETY: h is valid handle; value is writable uns16 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable uns16 on stack.
                if pl_get_param(
                    h,
                    PARAM_CLEAR_CYCLES,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get clear cycles: {}", get_pvcam_error()));
                }
            }
            return Ok(value);
        }
        // Mock mode default: 2 clear cycles
        Ok(2)
    }

    /// Set the number of sensor clearing cycles (bd-0yho)
    ///
    /// # SDK Pattern (bd-0yho)
    /// Checks PARAM_CLEAR_CYCLES availability before access.
    pub fn set_clear_cycles(_conn: &PvcamConnection, _cycles: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CLEAR_CYCLES) {
                return Err(anyhow!(
                    "PARAM_CLEAR_CYCLES is not available on this camera"
                ));
            }
            let value: uns16 = _cycles;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_CLEAR_CYCLES, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set clear cycles: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get parallel clocking mode (bd-0yho)
    ///
    /// Returns the raw PVCAM PMODE value. Common values:
    /// - 0: PMODE_NORMAL - Normal parallel shift
    /// - 1: PMODE_FT - Frame transfer mode
    /// - 2: PMODE_MPP - Multi-pinned phase mode
    /// - 3: PMODE_FT_MPP - Frame transfer with MPP
    ///
    /// # SDK Pattern (bd-0yho)
    /// Checks PARAM_PMODE availability before access.
    pub fn get_pmode(_conn: &PvcamConnection) -> Result<i32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PMODE) {
                return Err(anyhow!("PARAM_PMODE is not available on this camera"));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(h, PARAM_PMODE, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to get pmode: {}", get_pvcam_error()));
                }
            }
            return Ok(value);
        }
        // Mock mode default: PMODE_NORMAL (0)
        Ok(0)
    }

    /// Set parallel clocking mode (bd-0yho)
    ///
    /// Accepts raw PVCAM PMODE value. Common values:
    /// - 0: PMODE_NORMAL - Normal parallel shift
    /// - 1: PMODE_FT - Frame transfer mode
    /// - 2: PMODE_MPP - Multi-pinned phase mode
    /// - 3: PMODE_FT_MPP - Frame transfer with MPP
    ///
    /// # SDK Pattern (bd-0yho)
    /// Checks PARAM_PMODE availability before access.
    pub fn set_pmode(_conn: &PvcamConnection, _mode: i32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PMODE) {
                return Err(anyhow!("PARAM_PMODE is not available on this camera"));
            }
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_PMODE, &_mode as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set pmode: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get expose out mode (bd-iai9)
    ///
    /// # SDK Pattern (bd-smn3)
    /// Checks PARAM_EXPOSE_OUT_MODE availability before access.
    pub fn get_expose_out_mode(_conn: &PvcamConnection) -> Result<ExposeOutMode> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSE_OUT_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSE_OUT_MODE is not available on this camera"
                ));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_EXPOSE_OUT_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get expose out mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(ExposeOutMode::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ExposeOutMode::from_pvcam(state.expose_out_mode))
        }

        #[cfg(feature = "pvcam_sdk")]
        Ok(ExposeOutMode::FirstRow)
    }

    /// Set expose out mode (bd-iai9)
    ///
    /// # SDK Pattern (bd-smn3)
    /// Checks PARAM_EXPOSE_OUT_MODE availability before access.
    pub fn set_expose_out_mode(_conn: &PvcamConnection, _mode: ExposeOutMode) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSE_OUT_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSE_OUT_MODE is not available on this camera"
                ));
            }
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_EXPOSE_OUT_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set expose out mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.expose_out_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// List available expose out modes from hardware (bd-q4wz)
    ///
    /// Dynamically queries the camera to discover which expose out modes are supported.
    /// Returns a list of (value, name) pairs for all available modes.
    ///
    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_EXPOSE_OUT_MODE availability before access.
    pub fn list_expose_out_modes(_conn: &PvcamConnection) -> Result<Vec<(i32, String)>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_EXPOSE_OUT_MODE) {
                return Err(anyhow!(
                    "PARAM_EXPOSE_OUT_MODE is not available on this camera"
                ));
            }
            let count = Self::get_enum_count_impl(h, PARAM_EXPOSE_OUT_MODE)?;
            let mut modes = Vec::with_capacity(count as usize);

            for idx in 0..count {
                // SAFETY: h is a valid camera handle. idx is within 0..count from prior
                // enumeration. name buffer is stack-allocated [0i8; 256] with sufficient size.
                unsafe {
                    let mut value: i32 = 0;
                    let mut name = [0i8; 256];
                    let mut name_len: uns32 = 256;

                    // Get string length first
                    if pl_enum_str_length(h, PARAM_EXPOSE_OUT_MODE, idx, &mut name_len) != 0 {
                        // Get the enum entry with value and name
                        if pl_get_enum_param(
                            h,
                            PARAM_EXPOSE_OUT_MODE,
                            idx,
                            &mut value,
                            name.as_mut_ptr(),
                            name_len.min(256),
                        ) != 0
                        {
                            let name_str =
                                CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
                            modes.push((value, name_str));
                        }
                    }
                }
            }
            return Ok(modes);
        }
        // Mock mode: return standard expose out modes
        Ok(vec![
            (0, "First Row".to_string()),
            (1, "All Rows".to_string()),
            (2, "Any Row".to_string()),
            (3, "Rolling Shutter".to_string()),
            (4, "Line Output".to_string()),
        ])
    }

    // =========================================================================
    // Programmable Scan Mode (bd-ldjy.4)
    // =========================================================================

    /// Get programmable scan mode.
    pub fn get_scan_mode(_conn: &PvcamConnection) -> Result<ScanMode> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_MODE) {
                return Err(anyhow!("PARAM_SCAN_MODE is not available on this camera"));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_SCAN_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get scan mode: {}", get_pvcam_error()));
                }
            }
            return ScanMode::from_pvcam(value)
                .ok_or_else(|| anyhow!("Unknown scan mode value {}", value));
        }
        Ok(ScanMode::Auto)
    }

    /// Set programmable scan mode.
    pub fn set_scan_mode(_conn: &PvcamConnection, _mode: ScanMode) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_MODE) {
                return Err(anyhow!("PARAM_SCAN_MODE is not available on this camera"));
            }
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_SCAN_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set scan mode: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get programmable scan direction.
    pub fn get_scan_direction(_conn: &PvcamConnection) -> Result<ScanDirection> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_DIRECTION) {
                return Err(anyhow!(
                    "PARAM_SCAN_DIRECTION is not available on this camera"
                ));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_SCAN_DIRECTION,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get scan direction: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return ScanDirection::from_pvcam(value)
                .ok_or_else(|| anyhow!("Unknown scan direction value {}", value));
        }
        Ok(ScanDirection::Down)
    }

    /// Set programmable scan direction.
    pub fn set_scan_direction(_conn: &PvcamConnection, _direction: ScanDirection) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_DIRECTION) {
                return Err(anyhow!(
                    "PARAM_SCAN_DIRECTION is not available on this camera"
                ));
            }
            let value = _direction.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_SCAN_DIRECTION, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set scan direction: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get programmable scan line delay.
    pub fn get_scan_line_delay(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_LINE_DELAY) {
                return Err(anyhow!(
                    "PARAM_SCAN_LINE_DELAY is not available on this camera"
                ));
            }
            return Self::get_u16_param_impl(h, PARAM_SCAN_LINE_DELAY)
                .map_err(|e| anyhow!("Failed to get scan line delay: {}", e));
        }
        Ok(0)
    }

    /// Set programmable scan line delay.
    pub fn set_scan_line_delay(_conn: &PvcamConnection, _line_delay: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_LINE_DELAY) {
                return Err(anyhow!(
                    "PARAM_SCAN_LINE_DELAY is not available on this camera"
                ));
            }
            let value: uns16 = _line_delay;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_SCAN_LINE_DELAY, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set scan line delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get programmable scan line time in nanoseconds.
    pub fn get_scan_line_time_ns(_conn: &PvcamConnection) -> Result<i64> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_LINE_TIME) {
                return Err(anyhow!(
                    "PARAM_SCAN_LINE_TIME is not available on this camera"
                ));
            }
            let mut value: i64 = 0;
            // SAFETY: h is valid handle; value is writable i64 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_SCAN_LINE_TIME,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get scan line time: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value);
        }
        Ok(0)
    }

    /// Get programmable scan width.
    pub fn get_scan_width(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_WIDTH) {
                return Err(anyhow!("PARAM_SCAN_WIDTH is not available on this camera"));
            }
            return Self::get_u16_param_impl(h, PARAM_SCAN_WIDTH)
                .map_err(|e| anyhow!("Failed to get scan width: {}", e));
        }
        Ok(0)
    }

    /// Set programmable scan width.
    pub fn set_scan_width(_conn: &PvcamConnection, _width: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_SCAN_WIDTH) {
                return Err(anyhow!("PARAM_SCAN_WIDTH is not available on this camera"));
            }
            let value: uns16 = _width;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_SCAN_WIDTH, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set scan width: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Edge Trigger (bd-oqo7.5)
    // =========================================================================

    /// Get edge trigger mode (bd-oqo7.5)
    ///
    /// Returns the current edge trigger selection (First or All).
    /// Used for external laser synchronization in LIBS experiments.
    pub fn get_edge_trigger(_conn: &PvcamConnection) -> Result<EdgeTrigger> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_EDGE_TRIGGER) {
                return Err(anyhow!(
                    "PARAM_EDGE_TRIGGER is not available on this camera"
                ));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_EDGE_TRIGGER,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get edge trigger: {}", get_pvcam_error()));
                }
            }
            return Ok(EdgeTrigger::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(EdgeTrigger::from_pvcam(state.edge_trigger))
        }

        #[cfg(feature = "pvcam_sdk")]
        Ok(EdgeTrigger::First)
    }

    /// Set edge trigger mode (bd-oqo7.5)
    ///
    /// Selects rising/falling edge for external trigger synchronization.
    pub fn set_edge_trigger(_conn: &PvcamConnection, _mode: EdgeTrigger) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_EDGE_TRIGGER) {
                return Err(anyhow!(
                    "PARAM_EDGE_TRIGGER is not available on this camera"
                ));
            }
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_EDGE_TRIGGER, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set edge trigger: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.edge_trigger = _mode.to_pvcam();
        }
        Ok(())
    }

    // =========================================================================
    // Pre/Post Trigger Delay Setters (bd-oqo7.5)
    // =========================================================================

    /// Set pre-trigger delay in microseconds (bd-oqo7.5)
    ///
    /// Configures the delay between trigger signal and start of exposure.
    /// The SDK stores the value in nanoseconds; this function converts from microseconds.
    pub fn set_pre_trigger_delay_us(_conn: &PvcamConnection, _delay_us: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value_ns: i64 = i64::from(_delay_us) * 1000;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_PRE_TRIGGER_DELAY, &value_ns as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set pre-trigger delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.pre_trigger_delay_us = _delay_us;
        }
        Ok(())
    }

    /// Set post-trigger delay in microseconds (bd-oqo7.5)
    ///
    /// Configures the delay after exposure ends before the next trigger is accepted.
    /// The SDK stores the value in nanoseconds; this function converts from microseconds.
    pub fn set_post_trigger_delay_us(_conn: &PvcamConnection, _delay_us: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value_ns: i64 = i64::from(_delay_us) * 1000;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_POST_TRIGGER_DELAY, &value_ns as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set post-trigger delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.post_trigger_delay_us = _delay_us;
        }
        Ok(())
    }

    // =========================================================================
    // Exposure Resolution (bd-i2k7.1)
    // =========================================================================

    /// Get current exposure resolution
    pub fn get_exposure_resolution(_conn: &PvcamConnection) -> Result<ExposureResolution> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_EXP_RES,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get exposure resolution: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(ExposureResolution::from_pvcam(value));
        }
        Ok(ExposureResolution::Milliseconds)
    }

    /// Set exposure resolution
    pub fn set_exposure_resolution(
        _conn: &PvcamConnection,
        _res: ExposureResolution,
    ) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _res.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_EXP_RES, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set exposure resolution: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get current exposure resolution index
    pub fn get_exposure_resolution_index(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_EXP_RES_INDEX)
                .map_err(|e| anyhow!("Failed to get exposure resolution index: {}", e));
        }
        Ok(0)
    }

    /// Set exposure resolution index
    pub fn set_exposure_resolution_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _index as i32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_EXP_RES_INDEX, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set exposure resolution index: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Exposure Time (bd-aruo.1)
    // =========================================================================

    /// Set exposure time in milliseconds (bd-aruo.1)
    ///
    /// Writes PARAM_EXP_TIME to the PVCAM SDK. The value is converted to the
    /// camera's current exposure resolution units before writing.
    ///
    /// Note: This parameter is used at acquisition setup time. Changes take
    /// effect on the next `start_stream()` or `acquire_single_frame()` call.
    /// Some cameras also support live updates during streaming.
    ///
    /// # SDK Pattern (bd-smn3)
    /// Checks PARAM_EXP_TIME availability before access.
    pub fn set_exposure_time_ms(_conn: &PvcamConnection, _ms: f64) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // Check if PARAM_EXP_TIME is available and writable
            if !Self::is_param_available(h, PARAM_EXP_TIME) {
                // Not all cameras expose PARAM_EXP_TIME as writable.
                // The exposure value is still used at acquisition setup.
                tracing::debug!("PARAM_EXP_TIME not available — value stored for next acquisition");
                return Ok(());
            }

            // Get current exposure resolution to convert ms to correct units
            let exp_res = Self::get_exposure_resolution(_conn)?;
            let value: uns32 = match exp_res {
                ExposureResolution::Milliseconds => _ms as uns32,
                ExposureResolution::Microseconds => (_ms * 1000.0) as uns32,
                ExposureResolution::Seconds => {
                    let secs = _ms / 1000.0;
                    if secs < 1.0 {
                        tracing::warn!(
                            requested_ms = _ms,
                            "Exposure {_ms}ms cannot be represented in seconds resolution (min 1s)"
                        );
                        return Err(anyhow!(
                            "Exposure {_ms}ms below minimum 1s for camera seconds-resolution mode"
                        ));
                    }
                    secs as uns32
                }
            };

            // SAFETY: h is valid handle from successful pl_cam_open();
            unsafe {
                // SAFETY: h is valid handle from successful pl_cam_open();
                // value pointer is valid stack-allocated uns32 for duration of call.
                if pl_set_param(h, PARAM_EXP_TIME, &value as *const _ as *mut _) == 0 {
                    let err = get_pvcam_error();
                    return Err(anyhow!("Failed to set PARAM_EXP_TIME: {}", err));
                }
            }
            tracing::debug!(
                exposure_ms = _ms,
                sdk_value = value,
                ?exp_res,
                "PARAM_EXP_TIME set"
            );
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.exposure_time_ms = _ms;
        }
        Ok(())
    }

    // =========================================================================
    // ADC & Sensor Parameters (bd-i2k7.2, bd-i2k7.3)
    // =========================================================================

    /// Get current ADC offset (bd-i2k7.2)
    pub fn get_adc_offset(_conn: &PvcamConnection) -> Result<i16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i16 = 0;
            // SAFETY: h is valid handle; value is writable i16 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i16 on stack.
                if pl_get_param(
                    h,
                    PARAM_ADC_OFFSET,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get ADC offset: {}", get_pvcam_error()));
                }
            }
            return Ok(value);
        }
        Ok(0)
    }

    /// Set ADC offset (bd-i2k7.2)
    pub fn set_adc_offset(_conn: &PvcamConnection, _offset: i16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SAFETY: h is valid handle; offset pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; offset pointer valid for duration of call.
                if pl_set_param(h, PARAM_ADC_OFFSET, &_offset as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set ADC offset: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get full well capacity (bd-i2k7.3)
    pub fn get_full_well_capacity(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u32_param_impl(h, PARAM_FWELL_CAPACITY)
                .map_err(|e| anyhow!("Failed to get full well capacity: {}", e));
        }
        Ok(60000)
    }

    // =========================================================================
    // Optical Black / Scan Parameters (bd-03ny)
    // =========================================================================

    pub fn get_pre_mask(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_PREMASK)
                .map_err(|e| anyhow!("Failed to get pre-mask: {}", e));
        }
        Ok(0)
    }

    pub fn get_post_mask(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_POSTMASK)
                .map_err(|e| anyhow!("Failed to get post-mask: {}", e));
        }
        Ok(0)
    }

    pub fn get_pre_scan(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_PRESCAN)
                .map_err(|e| anyhow!("Failed to get pre-scan: {}", e));
        }
        Ok(0)
    }

    pub fn get_post_scan(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_POSTSCAN)
                .map_err(|e| anyhow!("Failed to get post-scan: {}", e));
        }
        Ok(0)
    }

    // =========================================================================
    // Readout Timing (bd-ejx3)
    // =========================================================================

    /// Get full frame readout time in microseconds (bd-ejx3)
    ///
    /// Returns the time required to read out a full frame from the sensor.
    /// Essential for calculating maximum frame rate and dead time.
    /// Total Frame Time = Exposure + Readout (in overlapped mode).
    pub fn get_readout_time_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: f64 = 0.0;
            // SAFETY: h is valid handle; value is writable f64 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable f64 on stack.
                if pl_get_param(
                    h,
                    PARAM_READOUT_TIME,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get readout time: {}", get_pvcam_error()));
                }
            }
            // PARAM_READOUT_TIME is in nanoseconds (f64)
            return Ok((value / 1000.0) as u32);
        }
        // Mock mode - typical Prime BSI readout time for full frame (15ms)
        Ok(15000)
    }

    /// Get clearing time in microseconds (bd-ejx3)
    ///
    /// Time required to clear the sensor before an exposure.
    pub fn get_clearing_time_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
            // SAFETY: h is valid handle; value is writable i64 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i64 on stack.
                if pl_get_param(
                    h,
                    PARAM_CLEARING_TIME,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get clearing time: {}",
                        get_pvcam_error()
                    ));
                }
            }
            // PARAM_CLEARING_TIME is in nanoseconds (i64)
            return Ok((value / 1000) as u32);
        }
        Ok(1000)
    }

    /// Get pre-trigger delay in microseconds (bd-ejx3)
    pub fn get_pre_trigger_delay_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
            // SAFETY: h is valid handle; value is writable i64 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i64 on stack.
                if pl_get_param(
                    h,
                    PARAM_PRE_TRIGGER_DELAY,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get pre-trigger delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
            // PARAM_PRE_TRIGGER_DELAY is in nanoseconds (i64)
            return Ok((value / 1000) as u32);
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            return Ok(state.pre_trigger_delay_us);
        }
        #[expect(unreachable_code, reason = "fallback after cfg-conditional early returns; unreachable but required by compiler")]
        Ok(0)
    }

    /// Get post-trigger delay in microseconds (bd-ejx3)
    pub fn get_post_trigger_delay_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
            // SAFETY: h is valid handle; value is writable i64 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i64 on stack.
                if pl_get_param(
                    h,
                    PARAM_POST_TRIGGER_DELAY,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get post-trigger delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
            // PARAM_POST_TRIGGER_DELAY is in nanoseconds (i64)
            return Ok((value / 1000) as u32);
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            return Ok(state.post_trigger_delay_us);
        }
        #[expect(unreachable_code, reason = "fallback after cfg-conditional early returns; unreachable but required by compiler")]
        Ok(0)
    }

    // =========================================================================
    // Shutter Control (bd-e8ah)
    // =========================================================================

    /// Get current shutter status (bd-e8ah)
    pub fn get_shutter_status(_conn: &PvcamConnection) -> Result<ShutterStatus> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_SHTR_STATUS,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get shutter status: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(ShutterStatus::from_pvcam(value));
        }
        Ok(ShutterStatus::Closed)
    }

    /// Get current shutter open mode (bd-e8ah)
    pub fn get_shutter_mode(_conn: &PvcamConnection) -> Result<ShutterMode> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_SHTR_OPEN_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get shutter mode: {}", get_pvcam_error()));
                }
            }
            return Ok(ShutterMode::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(ShutterMode::from_pvcam(
            _conn.mock_state.lock().unwrap().shutter_mode,
        ));

        #[cfg(feature = "pvcam_sdk")]
        Ok(ShutterMode::Normal)
    }

    /// Set shutter open mode (bd-e8ah)
    ///
    /// Controls the physical shutter behavior or TTL output signal.
    pub fn set_shutter_mode(_conn: &PvcamConnection, _mode: ShutterMode) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SHTR_OPEN_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set shutter mode: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get shutter open delay in microseconds (bd-e8ah)
    pub fn get_shutter_open_delay_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: uns32 = 0;
            // SAFETY: h is valid handle; value is writable uns32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable uns32 on stack.
                if pl_get_param(
                    h,
                    PARAM_SHTR_OPEN_DELAY,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get shutter open delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value);
        }
        Ok(0)
    }

    /// Get shutter close delay in microseconds (bd-e8ah)
    pub fn get_shutter_close_delay_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: uns32 = 0;
            // SAFETY: h is valid handle; value is writable uns32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable uns32 on stack.
                if pl_get_param(
                    h,
                    PARAM_SHTR_CLOSE_DELAY,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get shutter close delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value);
        }
        Ok(0)
    }

    /// Set shutter open delay in microseconds (bd-e8ah)
    pub fn set_shutter_open_delay_us(_conn: &PvcamConnection, _delay_us: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _delay_us as uns32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SHTR_OPEN_DELAY, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set shutter open delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Set shutter close delay in microseconds (bd-e8ah)
    pub fn set_shutter_close_delay_us(_conn: &PvcamConnection, _delay_us: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _delay_us as uns32;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SHTR_CLOSE_DELAY, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set shutter close delay: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Post-Processing Infrastructure (bd-we5p)
    // =========================================================================

    /// List all available post-processing features (bd-we5p)
    ///
    /// Returns features like PrimeEnhance, PrimeLocate, etc.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_PP_INDEX availability before access.
    pub fn list_pp_features(_conn: &PvcamConnection) -> Result<Vec<PPFeature>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PP_INDEX) {
                return Err(anyhow!("PARAM_PP_INDEX is not available on this camera"));
            }
            let count = Self::get_enum_count_impl(h, PARAM_PP_INDEX)?;
            let mut features = Vec::with_capacity(count as usize);

            for i in 0..count {
                // Select the PP feature
                let idx = i as i32;
                // SAFETY: h is valid; setting PP index to enumerate features.
                unsafe {
                    // SAFETY: h is valid; setting PP index to enumerate features.
                    if pl_set_param(h, PARAM_PP_INDEX, &(idx as i16) as *const i16 as *mut _) == 0 {
                        continue;
                    }
                }

                // Get feature info
                let feature = PPFeature {
                    index: i as u16,
                    id: Self::get_u16_param_impl(h, PARAM_PP_FEAT_ID).unwrap_or(0),
                    name: Self::get_pp_feature_name_impl(h)
                        .unwrap_or_else(|_| format!("Feature {}", i)),
                    params: Vec::new(),
                };
                features.push(feature);
            }

            return Ok(features);
        }
        // Mock mode
        Ok(vec![
            PPFeature {
                index: 0,
                id: 1,
                name: "PrimeEnhance".to_string(),
                params: vec![PPParam {
                    index: 0,
                    id: 0,
                    name: "Enable".to_string(),
                    value: 0,
                    min: 0,
                    max: 1,
                }],
            },
            PPFeature {
                index: 1,
                id: 2,
                name: "PrimeLocate".to_string(),
                params: vec![PPParam {
                    index: 0,
                    id: 0,
                    name: "Enable".to_string(),
                    value: 0,
                    min: 0,
                    max: 1,
                }],
            },
        ])
    }

    /// Get parameters for a specific PP feature (bd-we5p)
    ///
    /// First call `select_pp_feature()` to select the feature, then call this.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_PP_INDEX availability before access.
    pub fn list_pp_params(_conn: &PvcamConnection, _feature_index: u16) -> Result<Vec<PPParam>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PP_INDEX) {
                return Err(anyhow!("PARAM_PP_INDEX is not available on this camera"));
            }
            // Select the feature first
            let feat_idx = _feature_index as i32;
            // SAFETY: h is valid; selecting PP feature.
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(
                    h,
                    PARAM_PP_INDEX,
                    &(feat_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Get parameter count for this feature
            let count = Self::get_enum_count_impl(h, PARAM_PP_PARAM_INDEX)?;
            let mut params = Vec::with_capacity(count as usize);

            for i in 0..count {
                // Select the parameter
                let param_idx = i as i32;
                // SAFETY: h is valid; selecting PP parameter.
                unsafe {
                    // SAFETY: h is valid; selecting PP parameter.
                    if pl_set_param(
                        h,
                        PARAM_PP_PARAM_INDEX,
                        &(param_idx as i16) as *const i16 as *mut _,
                    ) == 0
                    {
                        continue;
                    }
                }

                // Get parameter info
                let param = PPParam {
                    index: i as u16,
                    id: Self::get_u16_param_impl(h, PARAM_PP_PARAM_ID).unwrap_or(i as u16),
                    name: Self::get_pp_param_name_impl(h)
                        .unwrap_or_else(|_| format!("Param {}", i)),
                    value: Self::get_u32_param_impl(h, PARAM_PP_PARAM).unwrap_or(0),
                    min: Self::get_u32_param_attr_impl(h, PARAM_PP_PARAM, ATTR_MIN).unwrap_or(0),
                    max: Self::get_u32_param_attr_impl(h, PARAM_PP_PARAM, ATTR_MAX)
                        .unwrap_or(u32::MAX),
                };
                params.push(param);
            }

            return Ok(params);
        }
        // Mock mode
        Ok(vec![
            PPParam {
                index: 0,
                id: 1,
                name: "Enabled".to_string(),
                value: 1,
                min: 0,
                max: 1,
            },
            PPParam {
                index: 1,
                id: 2,
                name: "Threshold".to_string(),
                value: 100,
                min: 0,
                max: 255,
            },
        ])
    }

    /// Set a PP parameter value (bd-we5p)
    ///
    /// Select feature and parameter first using their indices.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_PP_INDEX availability before access.
    pub fn set_pp_param(
        _conn: &PvcamConnection,
        _feature_index: u16,
        _param_index: u16,
        _value: u32,
    ) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PP_INDEX) {
                return Err(anyhow!("PARAM_PP_INDEX is not available on this camera"));
            }
            // Select feature
            let feat_idx = _feature_index as i32;
            // SAFETY: h is valid; selecting PP feature.
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(
                    h,
                    PARAM_PP_INDEX,
                    &(feat_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Select parameter
            let param_idx = _param_index as i32;
            // SAFETY: h is valid; selecting PP parameter.
            unsafe {
                // SAFETY: h is valid; selecting PP parameter.
                if pl_set_param(
                    h,
                    PARAM_PP_PARAM_INDEX,
                    &(param_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP parameter: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Set value
            // SAFETY: h is valid; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_PP_PARAM, &_value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set PP parameter value: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get a PP parameter value (bd-we5p)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_PP_INDEX availability before access.
    pub fn get_pp_param(
        _conn: &PvcamConnection,
        _feature_index: u16,
        _param_index: u16,
    ) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_PP_INDEX) {
                return Err(anyhow!("PARAM_PP_INDEX is not available on this camera"));
            }
            // Select feature
            let feat_idx = _feature_index as i32;
            // SAFETY: h is valid; selecting PP feature.
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(
                    h,
                    PARAM_PP_INDEX,
                    &(feat_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Select parameter
            let param_idx = _param_index as i32;
            // SAFETY: h is valid; selecting PP parameter.
            unsafe {
                // SAFETY: h is valid; selecting PP parameter.
                if pl_set_param(
                    h,
                    PARAM_PP_PARAM_INDEX,
                    &(param_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP parameter: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Get value
            return Self::get_u32_param_impl(h, PARAM_PP_PARAM)
                .map_err(|e| anyhow!("Failed to get PP parameter value: {}", e));
        }
        Ok(0)
    }

    /// Enable or disable a PP feature (bd-we5p)
    ///
    /// Convenience method to enable/disable features like PrimeEnhance.
    pub fn set_pp_feature_enabled(
        _conn: &PvcamConnection,
        _feature_index: u16,
        _enabled: bool,
    ) -> Result<()> {
        // PP features typically have an "Enabled" parameter at index 0
        Self::set_pp_param(_conn, _feature_index, 0, u32::from(_enabled))
    }

    /// Reset all post-processing features to defaults
    pub fn reset_pp_features(_conn: &PvcamConnection) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SAFETY: h is a valid camera handle. pl_pp_reset resets all PP features.
            unsafe {
                if pl_pp_reset(h) == 0 {
                    return Err(anyhow!(
                        "Failed to reset PP features: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Hardware Binning Support (bd-fqi8)
    // =========================================================================

    /// List available serial (horizontal) binning factors (bd-fqi8)
    pub fn list_serial_binning(_conn: &PvcamConnection) -> Result<Vec<u16>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_BINNING_SER)?;
            let mut factors = Vec::with_capacity(count as usize);

            for i in 0..count {
                let idx = i as i32;
                let mut value: i32 = 0;
                // SAFETY: h is valid; setting index and getting value.
                unsafe {
                    // SAFETY: h is valid; setting index and getting value.
                    if pl_set_param(h, PARAM_BINNING_SER, &idx as *const _ as *mut _) != 0 {
                        if pl_get_param(
                            h,
                            PARAM_BINNING_SER,
                            ATTR_CURRENT,
                            &mut value as *mut _ as *mut _,
                        ) != 0
                        {
                            factors.push(value as u16);
                        }
                    }
                }
            }
            return Ok(factors);
        }
        // Mock mode - common binning factors
        Ok(vec![1, 2, 4, 8])
    }

    /// List available parallel (vertical) binning factors (bd-fqi8)
    pub fn list_parallel_binning(_conn: &PvcamConnection) -> Result<Vec<u16>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_BINNING_PAR)?;
            let mut factors = Vec::with_capacity(count as usize);

            for i in 0..count {
                let idx = i as i32;
                let mut value: i32 = 0;
                // SAFETY: h is valid; setting index and getting value.
                unsafe {
                    // SAFETY: h is valid; setting index and getting value.
                    if pl_set_param(h, PARAM_BINNING_PAR, &idx as *const _ as *mut _) != 0 {
                        if pl_get_param(
                            h,
                            PARAM_BINNING_PAR,
                            ATTR_CURRENT,
                            &mut value as *mut _ as *mut _,
                        ) != 0
                        {
                            factors.push(value as u16);
                        }
                    }
                }
            }
            return Ok(factors);
        }
        // Mock mode - common binning factors
        Ok(vec![1, 2, 4, 8])
    }

    /// Set hardware binning (serial, parallel) (bd-aruo.5)
    ///
    /// Writes PARAM_BINNING_SER and PARAM_BINNING_PAR. The new binning takes
    /// effect at the next acquisition setup; cannot be changed while streaming.
    pub fn set_binning(_conn: &PvcamConnection, _serial: u16, _parallel: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let ser_val = _serial as i32;
            let par_val = _parallel as i32;
            // SAFETY: h is valid handle; value pointers are valid stack-allocated i32.
            unsafe {
                // SAFETY: h is valid handle; value pointers are valid stack-allocated i32.
                if pl_set_param(h, PARAM_BINNING_SER, &ser_val as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set serial binning: {}",
                        get_pvcam_error()
                    ));
                }
                if pl_set_param(h, PARAM_BINNING_PAR, &par_val as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set parallel binning: {}",
                        get_pvcam_error()
                    ));
                }
            }
            tracing::debug!(serial = _serial, parallel = _parallel, "PARAM_BINNING set");
        }
        Ok(())
    }

    /// Get current binning as (serial, parallel) (bd-fqi8)
    pub fn get_binning(_conn: &PvcamConnection) -> Result<(u16, u16)> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let serial = Self::get_u16_param_impl(h, PARAM_BINNING_SER).unwrap_or(1);
            let parallel = Self::get_u16_param_impl(h, PARAM_BINNING_PAR).unwrap_or(1);
            return Ok((serial, parallel));
        }
        Ok((1, 1))
    }

    // =========================================================================
    // Frame Metadata Support (bd-ne6a)
    // =========================================================================

    /// Check if frame metadata is enabled (bd-ne6a)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_METADATA_ENABLED availability before access.
    pub fn is_metadata_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_METADATA_ENABLED) {
                return Err(anyhow!(
                    "PARAM_METADATA_ENABLED is not available on this camera"
                ));
            }
            let mut value: rs_bool = 0;
            // SAFETY: h is valid; value is writable rs_bool on stack.
            unsafe {
                // SAFETY: h is valid; value is writable rs_bool on stack.
                if pl_get_param(
                    h,
                    PARAM_METADATA_ENABLED,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get metadata enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value != 0);
        }
        Ok(false)
    }

    /// Enable or disable frame metadata (bd-ne6a, bd-32f4)
    ///
    /// When enabled, PVCAM embeds per-frame metadata headers in frame buffers.
    /// The frame loop decodes these via `pl_md_frame_decode` to extract:
    /// - Hardware timestamps (BOF/EOF with nanosecond resolution)
    /// - Exposure time (FPGA-measured)
    /// - Bit depth and ROI count
    ///
    /// The acquisition's `metadata_enabled` flag is synced via the driver's
    /// `processing.metadata_enabled` parameter write callback (bd-32f4),
    /// ensuring the frame loop always decodes when metadata is embedded.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_METADATA_ENABLED availability before access.
    pub fn set_metadata_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_METADATA_ENABLED) {
                return Err(anyhow!(
                    "PARAM_METADATA_ENABLED is not available on this camera"
                ));
            }
            let value: rs_bool = if _enabled { 1 } else { 0 };
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_METADATA_ENABLED, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set metadata enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get centroids configuration (bd-ne6a)
    ///
    /// Centroids are used with PrimeLocate for particle tracking.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_CENTROIDS_MODE availability before access.
    pub fn get_centroids_config(_conn: &PvcamConnection) -> Result<CentroidsConfig> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_MODE) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_MODE is not available on this camera"
                ));
            }
            let mode = {
                let mut value: i32 = 0;
                // SAFETY: h is valid; value is writable i32 on stack.
                unsafe {
                    // SAFETY: h is valid; value is writable i32 on stack.
                    let _ = pl_get_param(
                        h,
                        PARAM_CENTROIDS_MODE,
                        ATTR_CURRENT,
                        &mut value as *mut _ as *mut _,
                    );
                }
                CentroidsMode::from_pvcam(value)
            };

            let radius = Self::get_u16_param_impl(h, PARAM_CENTROIDS_RADIUS).unwrap_or(3);
            let max_count = Self::get_u16_param_impl(h, PARAM_CENTROIDS_COUNT).unwrap_or(1000);
            let threshold = Self::get_u32_param_impl(h, PARAM_CENTROIDS_THRESHOLD).unwrap_or(100);

            return Ok(CentroidsConfig {
                mode,
                radius,
                max_count,
                threshold,
            });
        }
        Ok(CentroidsConfig {
            mode: CentroidsMode::Locate,
            radius: 3,
            max_count: 1000,
            threshold: 100,
        })
    }

    /// Set centroids configuration (bd-ne6a)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_CENTROIDS_MODE availability before access.
    pub fn set_centroids_config(_conn: &PvcamConnection, _config: &CentroidsConfig) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_MODE) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_MODE is not available on this camera"
                ));
            }
            // SAFETY: h is a valid camera handle; value pointers are valid stack-allocated
            // variables of the correct type for each PARAM_CENTROIDS_* constant.
            unsafe {
                // Set mode
                let mode = _config.mode.to_pvcam();
                if pl_set_param(h, PARAM_CENTROIDS_MODE, &mode as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set centroids mode: {}",
                        get_pvcam_error()
                    ));
                }

                // Set radius
                let radius = _config.radius as i32;
                if pl_set_param(h, PARAM_CENTROIDS_RADIUS, &radius as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set centroids radius: {}",
                        get_pvcam_error()
                    ));
                }

                // Set max count
                let count = _config.max_count as i32;
                if pl_set_param(h, PARAM_CENTROIDS_COUNT, &count as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set centroids count: {}",
                        get_pvcam_error()
                    ));
                }

                // Set threshold
                if pl_set_param(
                    h,
                    PARAM_CENTROIDS_THRESHOLD,
                    &_config.threshold as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to set centroids threshold: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Check if centroids detection is enabled (bd-cq4y)
    ///
    /// # SDK Pattern (bd-cq4y)
    /// Checks PARAM_CENTROIDS_ENABLED availability before access.
    pub fn get_centroids_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_ENABLED) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_ENABLED is not available on this camera"
                ));
            }
            let mut value: rs_bool = 0;
            // SAFETY: h is valid handle; value is writable rs_bool on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable rs_bool on stack.
                if pl_get_param(
                    h,
                    PARAM_CENTROIDS_ENABLED,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get centroids enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value != 0);
        }
        // Mock mode default: disabled
        Ok(false)
    }

    /// Enable or disable centroids detection (bd-cq4y)
    ///
    /// # SDK Pattern (bd-cq4y)
    /// Checks PARAM_CENTROIDS_ENABLED availability before access.
    pub fn set_centroids_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_ENABLED) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_ENABLED is not available on this camera"
                ));
            }
            let value: rs_bool = if _enabled { 1 } else { 0 };
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_CENTROIDS_ENABLED, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set centroids enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get centroids detection threshold (bd-cq4y)
    ///
    /// The threshold is the minimum pixel intensity for centroid detection.
    ///
    /// # SDK Pattern (bd-cq4y)
    /// Checks PARAM_CENTROIDS_THRESHOLD availability before access.
    pub fn get_centroids_threshold(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_THRESHOLD) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_THRESHOLD is not available on this camera"
                ));
            }
            let mut value: uns32 = 0;
            // SAFETY: h is valid handle; value is writable uns32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable uns32 on stack.
                if pl_get_param(
                    h,
                    PARAM_CENTROIDS_THRESHOLD,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get centroids threshold: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value);
        }
        // Mock mode default: 100
        Ok(100)
    }

    /// Set centroids detection threshold (bd-cq4y)
    ///
    /// The threshold is the minimum pixel intensity for centroid detection.
    ///
    /// # SDK Pattern (bd-cq4y)
    /// Checks PARAM_CENTROIDS_THRESHOLD availability before access.
    pub fn set_centroids_threshold(_conn: &PvcamConnection, _threshold: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_CENTROIDS_THRESHOLD) {
                return Err(anyhow!(
                    "PARAM_CENTROIDS_THRESHOLD is not available on this camera"
                ));
            }
            let value: uns32 = _threshold;
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_CENTROIDS_THRESHOLD, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set centroids threshold: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Smart Streaming (bd-0zge)
    // =========================================================================

    /// Check if Smart Streaming is enabled (bd-0zge)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_SMART_STREAM_MODE_ENABLED availability before access.
    pub fn is_smart_stream_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SMART_STREAM_MODE_ENABLED) {
                return Err(anyhow!(
                    "PARAM_SMART_STREAM_MODE_ENABLED is not available on this camera"
                ));
            }
            let mut value: rs_bool = 0;
            // SAFETY: h is valid; value is writable rs_bool on stack.
            unsafe {
                // SAFETY: h is valid; value is writable rs_bool on stack.
                if pl_get_param(
                    h,
                    PARAM_SMART_STREAM_MODE_ENABLED,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get smart stream enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(value != 0);
        }
        Ok(false)
    }

    /// Enable or disable Smart Streaming (bd-0zge)
    ///
    /// When enabled, the camera will cycle through a pre-configured sequence
    /// of exposure times without software intervention.
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_SMART_STREAM_MODE_ENABLED availability before access.
    pub fn set_smart_stream_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SMART_STREAM_MODE_ENABLED) {
                return Err(anyhow!(
                    "PARAM_SMART_STREAM_MODE_ENABLED is not available on this camera"
                ));
            }
            let value: rs_bool = if _enabled { 1 } else { 0 };
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(
                    h,
                    PARAM_SMART_STREAM_MODE_ENABLED,
                    &value as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to set smart stream enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.smart_stream_enabled = _enabled;
        }
        Ok(())
    }

    /// Get current Smart Streaming mode (bd-0zge)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_SMART_STREAM_MODE availability before access.
    pub fn get_smart_stream_mode(_conn: &PvcamConnection) -> Result<SmartStreamMode> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SMART_STREAM_MODE) {
                return Err(anyhow!(
                    "PARAM_SMART_STREAM_MODE is not available on this camera"
                ));
            }
            let mut value: i32 = 0;
            // SAFETY: h is valid handle; value is writable i32 on stack.
            unsafe {
                // SAFETY: h is valid handle; value is writable i32 on stack.
                if pl_get_param(
                    h,
                    PARAM_SMART_STREAM_MODE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get smart stream mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(SmartStreamMode::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        return Ok(SmartStreamMode::from_pvcam(
            _conn.mock_state.lock().unwrap().smart_stream_mode,
        ));

        #[cfg(feature = "pvcam_sdk")]
        Ok(SmartStreamMode::Exposures)
    }

    /// Set Smart Streaming mode (bd-0zge)
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_SMART_STREAM_MODE availability before access.
    pub fn set_smart_stream_mode(_conn: &PvcamConnection, _mode: SmartStreamMode) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SMART_STREAM_MODE) {
                return Err(anyhow!(
                    "PARAM_SMART_STREAM_MODE is not available on this camera"
                ));
            }
            let value = _mode.to_pvcam();
            // SAFETY: h is valid handle; value pointer valid for duration of call.
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SMART_STREAM_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set smart stream mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.smart_stream_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// Upload smart streaming exposure sequence to camera hardware
    ///
    /// # SDK Pattern (bd-l35g)
    /// Checks PARAM_SMART_STREAM_EXP_PARAMS availability before access.
    pub fn upload_smart_stream(_conn: &PvcamConnection, _exposures_ms: &[u32]) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_SMART_STREAM_EXP_PARAMS) {
                return Err(anyhow!(
                    "PARAM_SMART_STREAM_EXP_PARAMS is not available on this camera"
                ));
            }
            let entries = _exposures_ms.len() as u16;
            let mut ss_struct: *mut smart_stream_type = std::ptr::null_mut();

            // SAFETY: Pointers to valid buffers/structs; sizes match allocations.
            // pl_create_smart_stream_struct allocates, pl_release_smart_stream_struct frees.
            unsafe {
                if pl_create_smart_stream_struct(&mut ss_struct, entries) == 0 {
                    return Err(anyhow!(
                        "Failed to create smart stream struct: {}",
                        get_pvcam_error()
                    ));
                }

                // Copy exposures into the struct
                let params_slice =
                    std::slice::from_raw_parts_mut((*ss_struct).params, entries as usize);
                params_slice.copy_from_slice(_exposures_ms);

                // Upload to camera
                // NOTE: For PARAM_SMART_STREAM_EXP_PARAMS (TYPE_VOID_PTR), we pass the
                // pointer value (ss_struct) directly cast to *mut c_void.
                if pl_set_param(h, PARAM_SMART_STREAM_EXP_PARAMS, ss_struct as *mut _) == 0 {
                    let err = get_pvcam_error();
                    pl_release_smart_stream_struct(&mut ss_struct);
                    return Err(anyhow!("Failed to upload smart stream: {}", err));
                }

                pl_release_smart_stream_struct(&mut ss_struct);
            }
        }
        Ok(())
    }

    // =========================================================================
    // Host-Side Frame Processing (bd-46zc)
    // =========================================================================

    pub fn get_host_frame_rotate(_conn: &PvcamConnection) -> Result<FrameRotate> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is a valid camera handle. value is a stack-allocated i32.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_HOST_FRAME_ROTATE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get host frame rotate: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(FrameRotate::from_pvcam(value));
        }
        Ok(FrameRotate::None)
    }

    pub fn set_host_frame_rotate(_conn: &PvcamConnection, _rotate: FrameRotate) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _rotate.to_pvcam();
            // SAFETY: h is a valid camera handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_HOST_FRAME_ROTATE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set host frame rotate: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn get_host_frame_flip(_conn: &PvcamConnection) -> Result<FrameFlip> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
            // SAFETY: h is a valid camera handle. value is a stack-allocated i32.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_HOST_FRAME_FLIP,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get host frame flip: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(FrameFlip::from_pvcam(value));
        }
        Ok(FrameFlip::None)
    }

    pub fn set_host_frame_flip(_conn: &PvcamConnection, _flip: FrameFlip) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value = _flip.to_pvcam();
            // SAFETY: h is a valid camera handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_HOST_FRAME_FLIP, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set host frame flip: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn is_host_frame_summing_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut value: rs_bool = 0;
            // SAFETY: h is a valid camera handle. value is a stack-allocated rs_bool.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_HOST_FRAME_SUMMING_ENABLED,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Ok(false);
                }
            }
            return Ok(value != 0);
        }
        Ok(false)
    }

    pub fn set_host_frame_summing_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let value: rs_bool = if _enabled { 1 } else { 0 };
            // SAFETY: h is a valid camera handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_HOST_FRAME_SUMMING_ENABLED,
                    &value as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to set host frame summing enabled: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn get_host_frame_summing_count(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u32_param_impl(h, PARAM_HOST_FRAME_SUMMING_COUNT)
                .map_err(|e| anyhow!("Failed to get host frame summing count: {}", e));
        }
        Ok(1)
    }

    pub fn set_host_frame_summing_count(_conn: &PvcamConnection, _count: u32) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // SAFETY: h is a valid camera handle; value pointer valid for duration of call.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_HOST_FRAME_SUMMING_COUNT,
                    &_count as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to set host frame summing count: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // I/O Control (bd-6q0k)
    // =========================================================================

    /// Set the I/O signal address (selects which I/O line to control).
    ///
    /// Must be called before get/set_io_direction or get/set_io_state.
    /// The address selects the physical I/O line on the camera (0, 1, 2, etc.).
    pub fn set_io_address(_conn: &PvcamConnection, _address: u16) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_IO_ADDR) {
                return Err(anyhow!("PARAM_IO_ADDR is not available on this camera"));
            }
            let val = _address as i32;
            // SAFETY: h is a valid camera handle; val pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_IO_ADDR, &val as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set I/O address: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Set I/O signal direction for the currently selected I/O address.
    ///
    /// Internally sets PARAM_IO_ADDR first, then PARAM_IO_DIRECTION.
    pub fn set_io_direction(
        _conn: &PvcamConnection,
        _address: u16,
        _direction: &str,
    ) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // Select I/O address first
            Self::set_io_address(_conn, _address)?;

            if !Self::is_param_available(h, PARAM_IO_DIRECTION) {
                return Err(anyhow!(
                    "PARAM_IO_DIRECTION is not available on this camera"
                ));
            }
            let dir_val: i32 = match _direction {
                "Output" => 1,
                _ => 0, // Input
            };
            // SAFETY: h is a valid camera handle; dir_val pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_IO_DIRECTION, &dir_val as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set I/O direction: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Set I/O signal state for the currently selected I/O address.
    ///
    /// Internally sets PARAM_IO_ADDR first, then PARAM_IO_STATE.
    /// Value 0.0 = low, 1.0 = high.
    pub fn set_io_state(_conn: &PvcamConnection, _address: u16, _state: f64) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            // Select I/O address first
            Self::set_io_address(_conn, _address)?;

            if !Self::is_param_available(h, PARAM_IO_STATE) {
                return Err(anyhow!("PARAM_IO_STATE is not available on this camera"));
            }
            let state_val = if _state > 0.5 { 1.0f64 } else { 0.0f64 };
            // SAFETY: h is a valid camera handle; state_val pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_IO_STATE, &state_val as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set I/O state: {}", get_pvcam_error()));
                }
            }
        }
        Ok(())
    }

    /// Get I/O signal state for the currently selected I/O address.
    pub fn get_io_state(_conn: &PvcamConnection, _address: u16) -> Result<f64> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            Self::set_io_address(_conn, _address)?;

            if !Self::is_param_available(h, PARAM_IO_STATE) {
                return Err(anyhow!("PARAM_IO_STATE is not available on this camera"));
            }
            let mut value: f64 = 0.0;
            // SAFETY: h is a valid camera handle; value is a stack-allocated f64.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_IO_STATE,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get I/O state: {}", get_pvcam_error()));
                }
            }
            return Ok(value);
        }
        Ok(0.0)
    }

    // =========================================================================
    // I/O Script Control (bd-ncbd)
    // =========================================================================

    /// High-level I/O control using the PVCAM 3.x parameter-based API (bd-lkci).
    ///
    /// Combines `set_io_address`, `set_io_direction`, and `set_io_state` into a
    /// single call. This replaces the legacy `io_script_control` function.
    ///
    /// # Arguments
    /// - `address`: I/O line number (0, 1, 2, ...)
    /// - `direction`: "Input" or "Output"
    /// - `state`: Signal state (0.0 = low, 1.0 = high)
    pub fn io_control(
        conn: &PvcamConnection,
        address: u16,
        direction: &str,
        state: f64,
    ) -> Result<()> {
        Self::set_io_address(conn, address).context("setting I/O address")?;
        Self::set_io_direction(conn, address, direction).context("setting I/O direction")?;
        Self::set_io_state(conn, address, state).context("setting I/O state")?;
        Ok(())
    }

    // =========================================================================
    // I/O Diagnostics & Extended Parameters (bd-oqo7.6)
    // =========================================================================

    /// Get I/O port bit depth for the currently selected address.
    pub fn get_io_bitdepth(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_IO_BITDEPTH);
        }
        Ok(8)
    }

    /// Get I/O port type (TTL or DAC) for the currently selected address.
    pub fn get_io_type(_conn: &PvcamConnection) -> Result<IoType> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_IO_TYPE) {
                return Ok(IoType::Ttl);
            }
            let mut val: i32 = 0;
            // SAFETY: h is valid; val is writable i32 on stack.
            unsafe {
                if pl_get_param(h, PARAM_IO_TYPE, ATTR_CURRENT, &mut val as *mut _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to get PARAM_IO_TYPE: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(IoType::from_pvcam(val));
        }
        Ok(IoType::Ttl)
    }

    /// Get the current logic output mode.
    pub fn get_logic_output(_conn: &PvcamConnection) -> Result<LogicOutput> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_LOGIC_OUTPUT) {
                return Ok(LogicOutput::NotScan);
            }
            let mut val: i32 = 0;
            // SAFETY: h is valid; val is writable i32 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_LOGIC_OUTPUT,
                    ATTR_CURRENT,
                    &mut val as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get PARAM_LOGIC_OUTPUT: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(LogicOutput::from_pvcam(val));
        }
        Ok(LogicOutput::NotScan)
    }

    /// Set the logic output mode.
    pub fn set_logic_output(_conn: &PvcamConnection, _mode: LogicOutput) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_LOGIC_OUTPUT) {
                return Err(anyhow!(
                    "PARAM_LOGIC_OUTPUT is not available on this camera"
                ));
            }
            let val = _mode.to_pvcam();
            // SAFETY: h is valid; val pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_LOGIC_OUTPUT, &val as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set PARAM_LOGIC_OUTPUT: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Get whether the logic output signal is inverted.
    pub fn get_logic_output_invert(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_LOGIC_OUTPUT_INVERT) {
                return Ok(false);
            }
            let mut val: u16 = 0;
            // SAFETY: h is valid; val is writable u16 (rs_bool) on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_LOGIC_OUTPUT_INVERT,
                    ATTR_CURRENT,
                    &mut val as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get PARAM_LOGIC_OUTPUT_INVERT: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(val != 0);
        }
        Ok(false)
    }

    /// Set whether the logic output signal is inverted.
    pub fn set_logic_output_invert(_conn: &PvcamConnection, _invert: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_LOGIC_OUTPUT_INVERT) {
                return Err(anyhow!(
                    "PARAM_LOGIC_OUTPUT_INVERT is not available on this camera"
                ));
            }
            let val: u16 = u16::from(_invert);
            // SAFETY: h is valid; val pointer valid for duration of call.
            unsafe {
                if pl_set_param(h, PARAM_LOGIC_OUTPUT_INVERT, &val as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set PARAM_LOGIC_OUTPUT_INVERT: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Check whether the camera controller is alive and responsive.
    ///
    /// Returns `true` if the controller is powered on and running.
    /// A `false` return suggests hardware communication failure.
    pub fn get_controller_alive(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_CONTROLLER_ALIVE) {
                return Ok(true); // Assume alive if param not available
            }
            let mut val: u16 = 0;
            // SAFETY: h is valid; val is writable u16 (rs_bool) on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_CONTROLLER_ALIVE,
                    ATTR_CURRENT,
                    &mut val as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get PARAM_CONTROLLER_ALIVE: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(val != 0);
        }
        Ok(true)
    }

    /// Get Camera Control Subsystem status.
    ///
    /// Returns: 0=idle, 1=initializing, 2=running, 3=continuously clearing.
    pub fn get_ccs_status(_conn: &PvcamConnection) -> Result<i16> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_CCS_STATUS) {
                return Ok(0);
            }
            let mut val: i16 = 0;
            // SAFETY: h is valid; val is writable i16 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_CCS_STATUS,
                    ATTR_CURRENT,
                    &mut val as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get PARAM_CCS_STATUS: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(val);
        }
        Ok(0)
    }

    /// Get the minimum effective exposure time in seconds.
    ///
    /// Limited by necessary overhead like shifting data through the sensor.
    pub fn get_exp_min_time(_conn: &PvcamConnection) -> Result<f64> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            if !Self::is_param_available(h, PARAM_EXP_MIN_TIME) {
                return Ok(0.0);
            }
            let mut val: f64 = 0.0;
            // SAFETY: h is valid; val is writable f64 on stack.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_EXP_MIN_TIME,
                    ATTR_CURRENT,
                    &mut val as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to get PARAM_EXP_MIN_TIME: {}",
                        get_pvcam_error()
                    ));
                }
            }
            return Ok(val);
        }
        Ok(0.0)
    }

    // =========================================================================
    // Post-Processing Features (bd-oqo7.3)
    // =========================================================================

    /// Enumerate all available post-processing features on the camera.
    ///
    /// Logs each discovered feature at INFO level for diagnostics (bd-ldjy.1).
    pub fn enumerate_pp_features(_conn: &PvcamConnection) -> Result<Vec<PPFeature>> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let mut features = Vec::new();

            // First check ATTR_AVAIL explicitly
            let mut avail: rs_bool = 0;
            unsafe {
                let avail_result = pl_get_param(
                    h,
                    PARAM_PP_INDEX,
                    ATTR_AVAIL as i16,
                    &mut avail as *mut _ as *mut _,
                );
                tracing::info!(
                    "PARAM_PP_INDEX ATTR_AVAIL: result={}, avail={}, hcam={}",
                    avail_result,
                    avail,
                    h
                );
            }

            // Get number of PP features
            let mut count: u32 = 0;
            // SAFETY: h is valid; count is writable u32 on stack.
            unsafe {
                let count_result = pl_get_param(
                    h,
                    PARAM_PP_INDEX,
                    ATTR_COUNT,
                    &mut count as *mut _ as *mut _,
                );
                tracing::info!(
                    "PARAM_PP_INDEX ATTR_COUNT: result={}, count={}, err={}",
                    count_result,
                    count,
                    get_pvcam_error()
                );
                if count_result == 0 {
                    tracing::info!("PARAM_PP_INDEX not supported — no PP features available");
                    return Ok(Vec::new());
                }
            }

            tracing::info!(count, "Enumerating PP features");

            for feat_idx in 0..count {
                // PARAM_PP_INDEX is TYPE_INT16 — must pass int16*, not u32*
                let feat_idx_i16 = feat_idx as i16;
                // SAFETY: h valid; feat_idx_i16 pointer valid for call duration.
                unsafe {
                    if pl_set_param(h, PARAM_PP_INDEX, &feat_idx_i16 as *const _ as *mut _) == 0 {
                        tracing::warn!(feat_idx, "Failed to select PP feature index");
                        continue;
                    }
                }

                // Read feature name
                let mut name_buf = [0u8; 64];
                // SAFETY: h valid; name_buf large enough for PP feature names.
                unsafe {
                    let name_result = pl_get_param(
                        h,
                        PARAM_PP_FEAT_NAME,
                        ATTR_CURRENT,
                        name_buf.as_mut_ptr() as *mut _,
                    );
                    if name_result == 0 {
                        tracing::warn!(
                            feat_idx,
                            err_code = pl_error_code(),
                            err_msg = %get_pvcam_error(),
                            "Failed to read PP feature name"
                        );
                        continue;
                    }
                }
                let name = String::from_utf8_lossy(
                    &name_buf[..name_buf
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(name_buf.len())],
                )
                .to_string();

                // Read feature ID (TYPE_UNS16)
                let mut feat_id: u16 = 0;
                // SAFETY: h valid; feat_id writable u32 on stack.
                unsafe {
                    let _ = pl_get_param(
                        h,
                        PARAM_PP_FEAT_ID,
                        ATTR_CURRENT,
                        &mut feat_id as *mut _ as *mut _,
                    );
                }

                // Enumerate parameters within this feature
                let mut params = Vec::new();
                let mut param_count: u32 = 0;
                // SAFETY: h valid; param_count writable u32.
                unsafe {
                    if pl_get_param(
                        h,
                        PARAM_PP_PARAM_INDEX,
                        ATTR_COUNT,
                        &mut param_count as *mut _ as *mut _,
                    ) != 0
                    {
                        for param_idx in 0..param_count {
                            // SAFETY: h valid; param_idx pointer valid.
                            if pl_set_param(
                                h,
                                PARAM_PP_PARAM_INDEX,
                                &param_idx as *const _ as *mut _,
                            ) == 0
                            {
                                continue;
                            }

                            let mut pname_buf = [0u8; 64];
                            // SAFETY: h valid; pname_buf large enough.
                            if pl_get_param(
                                h,
                                PARAM_PP_PARAM_NAME,
                                ATTR_CURRENT,
                                pname_buf.as_mut_ptr() as *mut _,
                            ) == 0
                            {
                                continue;
                            }
                            let pname = String::from_utf8_lossy(
                                &pname_buf[..pname_buf
                                    .iter()
                                    .position(|&b| b == 0)
                                    .unwrap_or(pname_buf.len())],
                            )
                            .to_string();

                            let pid = Self::get_u16_param_impl(h, PARAM_PP_PARAM_ID)
                                .unwrap_or(param_idx as u16);
                            let pval = Self::get_u32_param_impl(h, PARAM_PP_PARAM).unwrap_or(0);
                            let pmin = Self::get_u32_param_attr_impl(h, PARAM_PP_PARAM, ATTR_MIN)
                                .unwrap_or(0);
                            let pmax = Self::get_u32_param_attr_impl(h, PARAM_PP_PARAM, ATTR_MAX)
                                .unwrap_or(u32::MAX);

                            params.push(PPParam {
                                name: pname,
                                index: param_idx as u16,
                                id: pid,
                                value: pval,
                                min: pmin,
                                max: pmax,
                            });
                        }
                    }
                }

                tracing::info!(
                    feat_idx,
                    feat_id,
                    name = %name,
                    param_count = params.len(),
                    "PP feature discovered"
                );

                features.push(PPFeature {
                    name,
                    index: feat_idx as u16,
                    id: feat_id as u16,
                    params,
                });
            }

            tracing::info!(
                total = features.len(),
                names = %features.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
                "PP feature enumeration complete"
            );

            return Ok(features);
        }
        Ok(Vec::new())
    }

    /// Check if PrimeEnhance is available on this camera.
    ///
    /// Uses case-insensitive, space/underscore-tolerant matching (bd-ldjy.1)
    /// to handle firmware naming variations ("PrimeEnhance", "PRIME ENHANCE", etc.).
    pub fn is_prime_enhance_available(_conn: &PvcamConnection) -> bool {
        #[cfg(feature = "pvcam_sdk")]
        if _conn.handle().is_some() {
            if let Ok(features) = Self::enumerate_pp_features(_conn) {
                return features.iter().any(|f| is_prime_enhance_name(&f.name));
            }
        }
        false
    }

    /// Get whether PrimeEnhance is currently enabled.
    pub fn get_prime_enhance_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if _conn.handle().is_some() {
            let features = Self::enumerate_pp_features(_conn)?;
            let pe_feat = features.iter().find(|f| is_prime_enhance_name(&f.name));

            if let Some(feat) = pe_feat {
                // Check enable param (usually first param or one containing "Enable")
                if let Some(ep) = feat
                    .params
                    .iter()
                    .find(|p| p.name.to_uppercase().contains("ENABLE"))
                {
                    return Ok(ep.value != 0);
                }
                if let Some(first) = feat.params.first() {
                    return Ok(first.value != 0);
                }
            }
        }
        Ok(false)
    }

    /// Enable or disable PrimeEnhance by name.
    pub fn set_prime_enhance(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let features = Self::enumerate_pp_features(_conn)?;
            let pe_feat = features.iter().find(|f| is_prime_enhance_name(&f.name));

            let feat = pe_feat
                .ok_or_else(|| anyhow!("PrimeEnhance feature not available on this camera"))?;

            // Select the feature
            let feat_idx = feat.index;
            // SAFETY: h valid; feat_idx pointer valid.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_PP_INDEX,
                    &(feat_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Find enable param
            let enable_param_idx = feat
                .params
                .iter()
                .find(|p| p.name.to_uppercase().contains("ENABLE"))
                .map(|p| p.index)
                .unwrap_or(0);

            // SAFETY: h valid; pointers valid for call duration.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_PP_PARAM_INDEX,
                    &enable_param_idx as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP param index: {}",
                        get_pvcam_error()
                    ));
                }
                let val: u32 = u32::from(_enabled);
                if pl_set_param(h, PARAM_PP_PARAM, &val as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set PrimeEnhance: {}", get_pvcam_error()));
                }
            }

            tracing::info!(
                enabled = _enabled,
                "PrimeEnhance {}",
                if _enabled { "enabled" } else { "disabled" }
            );
        }
        Ok(())
    }

    // =========================================================================
    // PrimeLocate Single-Molecule Localization (bd-oqo7.10)
    // =========================================================================

    /// Check if PrimeLocate is available on this camera.
    ///
    /// Uses case-insensitive, space/underscore-tolerant matching (bd-ldjy.1)
    /// to handle firmware naming variations ("PrimeLocate", "PRIME LOCATE", etc.).
    pub fn is_prime_locate_available(_conn: &PvcamConnection) -> bool {
        #[cfg(feature = "pvcam_sdk")]
        if _conn.handle().is_some() {
            if let Ok(features) = Self::enumerate_pp_features(_conn) {
                return features.iter().any(|f| is_prime_locate_name(&f.name));
            }
        }
        false
    }

    /// Get whether PrimeLocate is currently enabled.
    pub fn get_prime_locate_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_sdk")]
        if _conn.handle().is_some() {
            let features = Self::enumerate_pp_features(_conn)?;
            let pl_feat = features.iter().find(|f| is_prime_locate_name(&f.name));

            if let Some(feat) = pl_feat {
                if let Some(ep) = feat
                    .params
                    .iter()
                    .find(|p| p.name.to_uppercase().contains("ENABLE"))
                {
                    return Ok(ep.value != 0);
                }
                if let Some(first) = feat.params.first() {
                    return Ok(first.value != 0);
                }
            }
        }
        Ok(false)
    }

    /// Enable or disable PrimeLocate.
    ///
    /// When enabled, the camera FPGA evaluates frames and transmits only
    /// localized spot coordinates (x, y, intensity) instead of full pixel data.
    /// Frame buffers will contain [`LocalizationEvent`] arrays rather than images.
    pub fn set_prime_locate(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = _conn.handle() {
            let features = Self::enumerate_pp_features(_conn)?;
            let pl_feat = features.iter().find(|f| is_prime_locate_name(&f.name));

            let feat = pl_feat
                .ok_or_else(|| anyhow!("PrimeLocate feature not available on this camera"))?;

            let feat_idx = feat.index;
            // SAFETY: h valid; feat_idx pointer valid.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_PP_INDEX,
                    &(feat_idx as i16) as *const i16 as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            let enable_param_idx = feat
                .params
                .iter()
                .find(|p| p.name.to_uppercase().contains("ENABLE"))
                .map(|p| p.index)
                .unwrap_or(0);

            // SAFETY: h valid; pointers valid for call duration.
            unsafe {
                if pl_set_param(
                    h,
                    PARAM_PP_PARAM_INDEX,
                    &enable_param_idx as *const _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!(
                        "Failed to select PP param index: {}",
                        get_pvcam_error()
                    ));
                }
                let val: u32 = u32::from(_enabled);
                if pl_set_param(h, PARAM_PP_PARAM, &val as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set PrimeLocate: {}", get_pvcam_error()));
                }
            }

            tracing::info!(
                enabled = _enabled,
                "PrimeLocate {}",
                if _enabled { "enabled" } else { "disabled" }
            );
        }
        Ok(())
    }

    /// Parse localization events from a PrimeLocate frame buffer.
    ///
    /// When PrimeLocate is active, the frame buffer contains packed
    /// `LocalizationEvent` records instead of pixel data. Each event
    /// is 12 bytes: x (f32) + y (f32) + intensity (f32).
    ///
    /// Returns an empty Vec if the buffer doesn't contain valid localization data
    /// (e.g., PrimeLocate is disabled or the frame is a normal pixel frame).
    pub fn parse_localization_events(frame_data: &[u8]) -> Vec<LocalizationEvent> {
        const EVENT_SIZE: usize = 12; // 3 × f32
        if frame_data.len() < EVENT_SIZE {
            return Vec::new();
        }

        let n_events = frame_data.len() / EVENT_SIZE;
        let mut events = Vec::with_capacity(n_events);

        for i in 0..n_events {
            let offset = i * EVENT_SIZE;
            if offset + EVENT_SIZE > frame_data.len() {
                break;
            }
            let x = f32::from_le_bytes([
                frame_data[offset],
                frame_data[offset + 1],
                frame_data[offset + 2],
                frame_data[offset + 3],
            ]);
            let y = f32::from_le_bytes([
                frame_data[offset + 4],
                frame_data[offset + 5],
                frame_data[offset + 6],
                frame_data[offset + 7],
            ]);
            let intensity = f32::from_le_bytes([
                frame_data[offset + 8],
                frame_data[offset + 9],
                frame_data[offset + 10],
                frame_data[offset + 11],
            ]);

            // Basic sanity: NaN/Inf values indicate corrupt data
            if x.is_finite() && y.is_finite() && intensity.is_finite() && intensity > 0.0 {
                events.push(LocalizationEvent { x, y, intensity });
            }
        }

        events
    }

    // =========================================================================
    // Private Implementation Helpers
    // =========================================================================

    #[cfg(feature = "pvcam_sdk")]
    fn get_serial_number_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
        // SAFETY: h is valid; buf is writable array for string parameter.
        unsafe {
            // SAFETY: h is valid; buf is writable array for string parameter.
            if pl_get_param(
                h,
                PARAM_HEAD_SER_NUM_ALPHA,
                ATTR_CURRENT,
                buf.as_mut_ptr() as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get serial number: {}",
                    get_pvcam_error()
                ));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_CAM_FW_VERSION availability before access.
    #[cfg(feature = "pvcam_sdk")]
    fn get_firmware_version_impl(h: i16) -> Result<String> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_CAM_FW_VERSION) {
            return Err(anyhow!(
                "PARAM_CAM_FW_VERSION is not available on this camera"
            ));
        }

        let mut version: uns16 = 0;
        // SAFETY: h is valid; version is writable uns16 on stack.
        unsafe {
            // SAFETY: h is valid; version is writable uns16 on stack.
            if pl_get_param(
                h,
                PARAM_CAM_FW_VERSION,
                ATTR_CURRENT,
                &mut version as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get firmware version: {}",
                    get_pvcam_error()
                ));
            }
        }
        // PVCAM firmware version is encoded: major.minor in BCD or similar
        let major = (version >> 8) & 0xFF;
        let minor = version & 0xFF;
        Ok(format!("{}.{}", major, minor))
    }

    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_CHIP_NAME availability before access.
    #[cfg(feature = "pvcam_sdk")]
    fn get_chip_name_impl(h: i16) -> Result<String> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_CHIP_NAME) {
            return Err(anyhow!("PARAM_CHIP_NAME is not available on this camera"));
        }

        let mut buf = [0i8; 256];
        // SAFETY: h is valid; buf is writable array for string parameter.
        unsafe {
            // SAFETY: h is valid; buf is writable array for string parameter.
            if pl_get_param(h, PARAM_CHIP_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get chip name: {}", get_pvcam_error()));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    /// # SDK Pattern (bd-qijv)
    /// Checks PARAM_DD_VERSION availability before access.
    /// Returns device driver version as a formatted string (major.minor).
    #[cfg(feature = "pvcam_sdk")]
    fn get_device_driver_version_impl(h: i16) -> Result<String> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_DD_VERSION) {
            return Err(anyhow!("PARAM_DD_VERSION is not available on this camera"));
        }

        let mut version: uns16 = 0;
        // SAFETY: h is valid; version is writable uns16 on stack.
        unsafe {
            // SAFETY: h is valid; version is writable uns16 on stack.
            if pl_get_param(
                h,
                PARAM_DD_VERSION,
                ATTR_CURRENT,
                &mut version as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get device driver version: {}",
                    get_pvcam_error()
                ));
            }
        }
        // PVCAM device driver version is encoded: major.minor in BCD
        let major = (version >> 8) & 0xFF;
        let minor = version & 0xFF;
        Ok(format!("{}.{}", major, minor))
    }

    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_BIT_DEPTH availability before access.
    #[cfg(feature = "pvcam_sdk")]
    fn get_bit_depth_impl(h: i16) -> Result<u16> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_BIT_DEPTH) {
            return Err(anyhow!("PARAM_BIT_DEPTH is not available on this camera"));
        }

        let mut value: i16 = 0;
        // SAFETY: h is valid; value is writable i16 on stack.
        unsafe {
            // SAFETY: h is valid; value is writable i16 on stack.
            if pl_get_param(
                h,
                PARAM_BIT_DEPTH,
                ATTR_CURRENT,
                &mut value as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get bit depth: {}", get_pvcam_error()));
            }
        }
        Ok(value as u16)
    }

    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_PIX_TIME availability before access.
    #[cfg(feature = "pvcam_sdk")]
    fn get_pixel_time_impl(h: i16) -> Result<u32> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_PIX_TIME) {
            return Err(anyhow!("PARAM_PIX_TIME is not available on this camera"));
        }

        let mut value: uns16 = 0;
        // SAFETY: h is valid; value is writable uns16 on stack.
        unsafe {
            // SAFETY: h is valid; value is writable uns16 on stack.
            if pl_get_param(
                h,
                PARAM_PIX_TIME,
                ATTR_CURRENT,
                &mut value as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get pixel time: {}", get_pvcam_error()));
            }
        }
        Ok(value as u32)
    }

    #[cfg(feature = "pvcam_sdk")]
    fn get_pixel_size_impl(h: i16) -> Result<(u32, u32)> {
        let mut width: uns16 = 0;
        let mut height: uns16 = 0;
        // SAFETY: h is valid; width/height are writable uns16 on stack.
        unsafe {
            // SAFETY: h is valid; width/height are writable uns16 on stack.
            if pl_get_param(
                h,
                PARAM_PIX_SER_SIZE,
                ATTR_CURRENT,
                &mut width as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get pixel width: {}", get_pvcam_error()));
            }
            if pl_get_param(
                h,
                PARAM_PIX_PAR_SIZE,
                ATTR_CURRENT,
                &mut height as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get pixel height: {}", get_pvcam_error()));
            }
        }
        // Values are typically in 100ths of microns, convert to nm
        Ok((width as u32 * 10, height as u32 * 10))
    }

    /// # SDK Pattern (bd-sk6z)
    /// Checks PARAM_SER_SIZE and PARAM_PAR_SIZE availability before access.
    #[cfg(feature = "pvcam_sdk")]
    fn get_sensor_size_impl(h: i16) -> Result<(u32, u32)> {
        // SDK Pattern: Check availability before access
        if !Self::is_param_available(h, PARAM_SER_SIZE) {
            return Err(anyhow!("PARAM_SER_SIZE is not available on this camera"));
        }
        if !Self::is_param_available(h, PARAM_PAR_SIZE) {
            return Err(anyhow!("PARAM_PAR_SIZE is not available on this camera"));
        }

        let mut width: uns16 = 0;
        let mut height: uns16 = 0;
        // SAFETY: h is valid; width/height are writable uns16 on stack.
        unsafe {
            // SAFETY: h is valid; width/height are writable uns16 on stack.
            if pl_get_param(
                h,
                PARAM_SER_SIZE,
                ATTR_CURRENT,
                &mut width as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get sensor width: {}", get_pvcam_error()));
            }
            if pl_get_param(
                h,
                PARAM_PAR_SIZE,
                ATTR_CURRENT,
                &mut height as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get sensor height: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok((width as u32, height as u32))
    }

    #[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
    pub(crate) fn get_u16_param_impl(h: i16, param: u32) -> Result<u16> {
        let mut value: i32 = 0;
        // SAFETY: h is valid; value is writable i32 on stack.
        unsafe {
            // SAFETY: h is valid; value is writable i32 on stack.
            if pl_get_param(h, param, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to get parameter {}: {}",
                    param,
                    get_pvcam_error()
                ));
            }
        }
        Ok(value as u16)
    }

    #[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
    pub(crate) fn get_enum_count_impl(h: i16, param: u32) -> Result<u32> {
        let mut count: uns32 = 0;
        // SAFETY: h is valid; count is writable uns32 on stack.
        unsafe {
            // SAFETY: h is valid; count is writable uns32 on stack.
            if pl_get_param(h, param, ATTR_COUNT, &mut count as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to get enum count for {}: {}",
                    param,
                    get_pvcam_error()
                ));
            }
        }
        Ok(count)
    }

    #[cfg(feature = "pvcam_sdk")]
    pub(crate) fn get_enum_item_by_index_impl(
        h: i16,
        param: u32,
        index: u32,
    ) -> Result<(i32, String)> {
        let mut name_len: uns32 = 0;
        // SAFETY: h is valid; name_len is a writable uns32 on stack.
        unsafe {
            if pl_enum_str_length(h, param, index, &mut name_len) == 0 {
                return Err(anyhow!(
                    "Failed to get enum string length for {}[{}]: {}",
                    param,
                    index,
                    get_pvcam_error()
                ));
            }
        }

        let mut value: i32 = 0;
        let mut name = vec![0i8; (name_len.max(2) as usize).min(1024)];
        // SAFETY: h is valid; value/name are writable and size is bounded.
        unsafe {
            if pl_get_enum_param(
                h,
                param,
                index,
                &mut value,
                name.as_mut_ptr(),
                name.len() as uns32,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get enum value/string for {}[{}]: {}",
                    param,
                    index,
                    get_pvcam_error()
                ));
            }
        }

        // SAFETY: PVCAM writes a NUL-terminated enum string to `name`.
        let name = unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok((value, name))
    }

    #[cfg(feature = "pvcam_sdk")]
    pub(crate) fn get_enum_string_impl(h: i16, param: u32) -> Result<String> {
        // Get current enum value first (note: value != index for many PVCAM enums).
        let mut value: i32 = 0;
        // SAFETY: h is valid; value is writable i32 on stack.
        unsafe {
            // SAFETY: h is valid; value is writable i32 on stack.
            if pl_get_param(h, param, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get enum value: {}", get_pvcam_error()));
            }
        }

        let count = Self::get_enum_count_impl(h, param).unwrap_or(0);
        for idx in 0..count {
            if let Ok((entry_value, entry_name)) = Self::get_enum_item_by_index_impl(h, param, idx)
            {
                if entry_value == value {
                    return Ok(entry_name);
                }
            }
        }

        // Fallback: return raw enum value when we cannot resolve a display string.
        Ok(format!("{}", value))
    }

    #[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
    pub(crate) fn get_u32_param_impl(h: i16, param: u32) -> Result<u32> {
        let mut value: uns32 = 0;
        // SAFETY: h is valid; value is writable uns32 on stack.
        unsafe {
            // SAFETY: h is valid; value is writable uns32 on stack.
            if pl_get_param(h, param, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to get parameter {}: {}",
                    param,
                    get_pvcam_error()
                ));
            }
        }
        Ok(value)
    }

    #[cfg(any(feature = "pvcam_sdk", feature = "pvcam_hardware"))]
    pub(crate) fn get_u32_param_attr_impl(h: i16, param: u32, attr: i16) -> Result<u32> {
        let mut value: uns32 = 0;
        // SAFETY: h is valid; value is writable uns32 on stack.
        unsafe {
            if pl_get_param(h, param, attr, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to get parameter {} attr {}: {}",
                    param,
                    attr,
                    get_pvcam_error()
                ));
            }
        }
        Ok(value)
    }

    #[cfg(feature = "pvcam_sdk")]
    fn get_pp_feature_name_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
        // SAFETY: h is valid; buf is writable array for PP feature name string.
        unsafe {
            // SAFETY: h is valid; buf is writable array for PP feature name string.
            if pl_get_param(
                h,
                PARAM_PP_FEAT_NAME,
                ATTR_CURRENT,
                buf.as_mut_ptr() as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get PP feature name: {}",
                    get_pvcam_error()
                ));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    #[cfg(feature = "pvcam_sdk")]
    fn get_pp_param_name_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
        // SAFETY: h is valid; buf is writable array for PP parameter name string.
        unsafe {
            // SAFETY: h is valid; buf is writable array for PP parameter name string.
            if pl_get_param(
                h,
                PARAM_PP_PARAM_NAME,
                ATTR_CURRENT,
                buf.as_mut_ptr() as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get PP parameter name: {}",
                    get_pvcam_error()
                ));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prime_enhance_name_matches_canonical() {
        assert!(is_prime_enhance_name("PrimeEnhance"));
        assert!(is_prime_enhance_name("PRIME ENHANCE"));
        assert!(is_prime_enhance_name("Prime_Enhance"));
        assert!(is_prime_enhance_name("prime-enhance"));
    }

    #[test]
    fn prime_enhance_name_matches_sdk_denoising() {
        assert!(is_prime_enhance_name("Denoising"));
        assert!(is_prime_enhance_name("DENOISING"));
        assert!(is_prime_enhance_name("PP_FEATURE_DENOISING"));
        assert!(is_prime_enhance_name("denoising"));
    }

    #[test]
    fn prime_enhance_name_rejects_unrelated() {
        assert!(!is_prime_enhance_name("PrimeLocate"));
        assert!(!is_prime_enhance_name("Despeckle"));
        assert!(!is_prime_enhance_name("HDR"));
    }

    #[test]
    fn prime_locate_name_matches_canonical() {
        assert!(is_prime_locate_name("PrimeLocate"));
        assert!(is_prime_locate_name("PRIME LOCATE"));
        assert!(is_prime_locate_name("Prime_Locate"));
        assert!(is_prime_locate_name("prime-locate"));
    }

    #[test]
    fn prime_locate_name_matches_sdk_locate() {
        assert!(is_prime_locate_name("Locate"));
        assert!(is_prime_locate_name("LOCATE"));
        assert!(is_prime_locate_name("PP_FEATURE_LOCATE"));
        assert!(is_prime_locate_name("locate"));
    }

    #[test]
    fn prime_locate_name_rejects_unrelated() {
        assert!(!is_prime_locate_name("PrimeEnhance"));
        assert!(!is_prime_locate_name("Despeckle"));
        assert!(!is_prime_locate_name("HDR"));
    }
}
