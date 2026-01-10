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

use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_hardware")]
use anyhow::anyhow;
use anyhow::Result;

#[cfg(feature = "pvcam_hardware")]
use crate::components::connection::get_pvcam_error;
#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use std::ffi::CStr;

// =============================================================================
// Data Structures
// =============================================================================

/// Comprehensive camera information (bd-565x)
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Camera serial number (alphanumeric)
    pub serial_number: String,
    /// Firmware version string
    pub firmware_version: String,
    /// Sensor chip name (e.g., "GS2020" for Prime BSI)
    pub chip_name: String,
    /// Current sensor temperature in Celsius
    pub temperature_c: f64,
    /// Bit depth for current speed mode
    pub bit_depth: u16,
    /// Pixel readout time in nanoseconds
    pub pixel_time_ns: u32,
    /// Pixel size in nanometers (width, height)
    pub pixel_size_nm: (u32, u32),
    /// Sensor size in pixels (width, height)
    pub sensor_size: (u32, u32),
    /// Current gain mode name
    pub gain_name: String,
    /// Current speed mode name
    pub speed_name: String,
    /// Current readout port name
    pub port_name: String,
    /// Current gain index
    pub gain_index: u16,
    /// Current speed table index
    pub speed_index: u16,
}

#[derive(Debug, Clone)]
pub struct GainMode {
    pub index: u16,
    pub name: String,
}

/// Speed mode entry from the camera's speed table (bd-v54z)
#[derive(Debug, Clone)]
pub struct SpeedMode {
    /// Speed table index
    pub index: u16,
    /// Display name (e.g., "100 MHz")
    pub name: String,
    /// Pixel readout time in nanoseconds
    pub pixel_time_ns: u32,
    /// Bit depth at this speed
    pub bit_depth: u16,
    /// Associated readout port index
    pub port_index: u16,
}

/// Readout port entry (bd-v54z)
#[derive(Debug, Clone)]
pub struct ReadoutPort {
    /// Port index
    pub index: u16,
    /// Port name (e.g., "Sensitivity", "Speed")
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
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FanSpeed::High,
            1 => FanSpeed::Medium,
            2 => FanSpeed::Low,
            3 => FanSpeed::Off,
            _ => FanSpeed::High,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FanSpeed::High => 0,
            FanSpeed::Medium => 1,
            FanSpeed::Low => 2,
            FanSpeed::Off => 3,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "High" => FanSpeed::High,
            "Medium" => FanSpeed::Medium,
            "Low" => FanSpeed::Low,
            "Off" => FanSpeed::Off,
            _ => FanSpeed::High,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FanSpeed::High => "High",
            FanSpeed::Medium => "Medium",
            FanSpeed::Low => "Low",
            FanSpeed::Off => "Off",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["High".into(), "Medium".into(), "Low".into(), "Off".into()]
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
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => CentroidsMode::Locate,
            1 => CentroidsMode::Track,
            2 => CentroidsMode::Blob,
            _ => CentroidsMode::Locate,
        }
    }

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
// Smart Streaming Types (bd-0zge)
// =============================================================================

/// A single entry in a hardware-timed Smart Streaming sequence (bd-0zge)
///
/// Smart Streaming allows loading a sequence of varying exposure times
/// directly onto the camera FPGA, eliminating USB communication jitter
/// between frames. Useful for HDR imaging and time-lapse with varying exposures.
#[derive(Debug, Clone, Copy)]
pub struct SmartStreamEntry {
    /// Exposure time in milliseconds for this frame
    pub exposure_ms: u32,
}

/// Smart Streaming mode options (bd-0zge)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartStreamMode {
    /// Exposures only - varying exposure times per frame
    Exposures,
    /// Interleaved mode (if supported)
    Interleaved,
}

impl SmartStreamMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => SmartStreamMode::Exposures,
            1 => SmartStreamMode::Interleaved,
            _ => SmartStreamMode::Exposures,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            SmartStreamMode::Exposures => 0,
            SmartStreamMode::Interleaved => 1,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Exposures" => SmartStreamMode::Exposures,
            "Interleaved" => SmartStreamMode::Interleaved,
            _ => SmartStreamMode::Exposures,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SmartStreamMode::Exposures => "Exposures",
            SmartStreamMode::Interleaved => "Interleaved",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["Exposures".into(), "Interleaved".into()]
    }
}

// =============================================================================
// Shutter Control Types (bd-e8ah)
// =============================================================================

/// Shutter open mode settings (bd-e8ah)
///
/// Controls the physical shutter behavior or TTL "Expose Out" signal
/// for triggering external light sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutterMode {
    /// Normal operation - shutter opens during exposure
    Normal,
    /// Shutter always open (for external shutter control)
    Open,
    /// Shutter always closed (for dark frames)
    Closed,
    /// No shutter installed/disabled
    None,
    /// Open before trigger (pre-open mode)
    PreOpen,
}

impl ShutterMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ShutterMode::Normal,
            1 => ShutterMode::Open,
            2 => ShutterMode::Closed,
            3 => ShutterMode::None,
            4 => ShutterMode::PreOpen,
            _ => ShutterMode::Normal,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ShutterMode::Normal => 0,
            ShutterMode::Open => 1,
            ShutterMode::Closed => 2,
            ShutterMode::None => 3,
            ShutterMode::PreOpen => 4,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Normal" => ShutterMode::Normal,
            "Open" => ShutterMode::Open,
            "Closed" => ShutterMode::Closed,
            "None" => ShutterMode::None,
            "PreOpen" => ShutterMode::PreOpen,
            _ => ShutterMode::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ShutterMode::Normal => "Normal",
            ShutterMode::Open => "Open",
            ShutterMode::Closed => "Closed",
            ShutterMode::None => "None",
            ShutterMode::PreOpen => "PreOpen",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Normal".into(),
            "Open".into(),
            "Closed".into(),
            "None".into(),
            "PreOpen".into(),
        ]
    }
}

/// Shutter status reported by camera (bd-e8ah)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutterStatus {
    /// Shutter is closed
    Closed,
    /// Shutter is open
    Open,
    /// Shutter is opening
    Opening,
    /// Shutter is closing
    Closing,
    /// Shutter fault detected
    Fault,
    /// Status unknown
    Unknown,
}

impl ShutterStatus {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ShutterStatus::Closed,
            1 => ShutterStatus::Open,
            2 => ShutterStatus::Opening,
            3 => ShutterStatus::Closing,
            4 => ShutterStatus::Fault,
            _ => ShutterStatus::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ShutterStatus::Closed => "Closed",
            ShutterStatus::Open => "Open",
            ShutterStatus::Opening => "Opening",
            ShutterStatus::Closing => "Closing",
            ShutterStatus::Fault => "Fault",
            ShutterStatus::Unknown => "Unknown",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Closed".into(),
            "Open".into(),
            "Opening".into(),
            "Closing".into(),
            "Fault".into(),
            "Unknown".into(),
        ]
    }
}

// =============================================================================
// Triggering & Exposure Mode Types (bd-iai9)
// =============================================================================

/// Exposure mode settings (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureMode {
    /// Internal timing - camera controls exposure
    Timed,
    /// Strobe mode - external strobe signal
    Strobe,
    /// Bulb mode - exposure controlled by external signal duration
    Bulb,
    /// Trigger first - wait for trigger, then start exposure
    TriggerFirst,
    /// External edge trigger
    EdgeTrigger,
}

impl ExposureMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposureMode::Timed,
            1 => ExposureMode::Strobe,
            2 => ExposureMode::Bulb,
            3 => ExposureMode::TriggerFirst,
            4 => ExposureMode::EdgeTrigger,
            _ => ExposureMode::Timed,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposureMode::Timed => 0,
            ExposureMode::Strobe => 1,
            ExposureMode::Bulb => 2,
            ExposureMode::TriggerFirst => 3,
            ExposureMode::EdgeTrigger => 4,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Timed" => ExposureMode::Timed,
            "Strobe" => ExposureMode::Strobe,
            "Bulb" => ExposureMode::Bulb,
            "TriggerFirst" => ExposureMode::TriggerFirst,
            "EdgeTrigger" => ExposureMode::EdgeTrigger,
            _ => ExposureMode::Timed,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExposureMode::Timed => "Timed",
            ExposureMode::Strobe => "Strobe",
            ExposureMode::Bulb => "Bulb",
            ExposureMode::TriggerFirst => "TriggerFirst",
            ExposureMode::EdgeTrigger => "EdgeTrigger",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Timed".into(),
            "Strobe".into(),
            "Bulb".into(),
            "TriggerFirst".into(),
            "EdgeTrigger".into(),
        ]
    }
}

/// Clear mode settings for CCD clearing (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearMode {
    /// Never clear
    Never,
    /// Clear before each exposure
    PreExposure,
    /// Clear before sequence
    PreSequence,
    /// Clear after sequence
    PostSequence,
    /// Clear before and after exposure
    PrePostSequence,
    /// Clear before each frame in sequence
    PreExposurePostSequence,
}

impl ClearMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ClearMode::Never,
            1 => ClearMode::PreExposure,
            2 => ClearMode::PreSequence,
            3 => ClearMode::PostSequence,
            4 => ClearMode::PrePostSequence,
            5 => ClearMode::PreExposurePostSequence,
            _ => ClearMode::PreExposure,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ClearMode::Never => 0,
            ClearMode::PreExposure => 1,
            ClearMode::PreSequence => 2,
            ClearMode::PostSequence => 3,
            ClearMode::PrePostSequence => 4,
            ClearMode::PreExposurePostSequence => 5,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "Never" => ClearMode::Never,
            "PreExposure" => ClearMode::PreExposure,
            "PreSequence" => ClearMode::PreSequence,
            "PostSequence" => ClearMode::PostSequence,
            "PrePostSequence" => ClearMode::PrePostSequence,
            "PreExposurePostSequence" => ClearMode::PreExposurePostSequence,
            _ => ClearMode::PreExposure,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ClearMode::Never => "Never",
            ClearMode::PreExposure => "PreExposure",
            ClearMode::PreSequence => "PreSequence",
            ClearMode::PostSequence => "PostSequence",
            ClearMode::PrePostSequence => "PrePostSequence",
            ClearMode::PreExposurePostSequence => "PreExposurePostSequence",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "Never".into(),
            "PreExposure".into(),
            "PreSequence".into(),
            "PostSequence".into(),
            "PrePostSequence".into(),
            "PreExposurePostSequence".into(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureResolution {
    Milliseconds,
    Microseconds,
    Seconds,
}

impl ExposureResolution {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposureResolution::Milliseconds,
            1 => ExposureResolution::Microseconds,
            2 => ExposureResolution::Seconds,
            _ => ExposureResolution::Milliseconds,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposureResolution::Milliseconds => 0,
            ExposureResolution::Microseconds => 1,
            ExposureResolution::Seconds => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRotate {
    None,
    Rotate90CW,
    Rotate180CW,
    Rotate270CW,
}

impl FrameRotate {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FrameRotate::None,
            1 => FrameRotate::Rotate90CW,
            2 => FrameRotate::Rotate180CW,
            3 => FrameRotate::Rotate270CW,
            _ => FrameRotate::None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FrameRotate::None => 0,
            FrameRotate::Rotate90CW => 1,
            FrameRotate::Rotate180CW => 2,
            FrameRotate::Rotate270CW => 3,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "None" => FrameRotate::None,
            "90 CW" => FrameRotate::Rotate90CW,
            "180 CW" => FrameRotate::Rotate180CW,
            "270 CW" => FrameRotate::Rotate270CW,
            _ => FrameRotate::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FrameRotate::None => "None",
            FrameRotate::Rotate90CW => "90 CW",
            FrameRotate::Rotate180CW => "180 CW",
            FrameRotate::Rotate270CW => "270 CW",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "None".into(),
            "90 CW".into(),
            "180 CW".into(),
            "270 CW".into(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFlip {
    None,
    FlipX,
    FlipY,
    FlipXY,
}

impl FrameFlip {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FrameFlip::None,
            1 => FrameFlip::FlipX,
            2 => FrameFlip::FlipY,
            3 => FrameFlip::FlipXY,
            _ => FrameFlip::None,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            FrameFlip::None => 0,
            FrameFlip::FlipX => 1,
            FrameFlip::FlipY => 2,
            FrameFlip::FlipXY => 3,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "None" => FrameFlip::None,
            "X" => FrameFlip::FlipX,
            "Y" => FrameFlip::FlipY,
            "XY" => FrameFlip::FlipXY,
            _ => FrameFlip::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FrameFlip::None => "None",
            FrameFlip::FlipX => "X",
            FrameFlip::FlipY => "Y",
            FrameFlip::FlipXY => "XY",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec!["None".into(), "X".into(), "Y".into(), "XY".into()]
    }
}

/// Expose out mode - controls the expose_out signal (bd-iai9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposeOutMode {
    /// First row exposure timing
    FirstRow,
    /// All rows exposure timing
    AllRows,
    /// Any row exposure timing
    AnyRow,
    /// Rolling shutter mode
    RollingShutter,
    /// Line output mode
    LineOutput,
}

impl ExposeOutMode {
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => ExposeOutMode::FirstRow,
            1 => ExposeOutMode::AllRows,
            2 => ExposeOutMode::AnyRow,
            3 => ExposeOutMode::RollingShutter,
            4 => ExposeOutMode::LineOutput,
            _ => ExposeOutMode::FirstRow,
        }
    }

    pub fn to_pvcam(self) -> i32 {
        match self {
            ExposeOutMode::FirstRow => 0,
            ExposeOutMode::AllRows => 1,
            ExposeOutMode::AnyRow => 2,
            ExposeOutMode::RollingShutter => 3,
            ExposeOutMode::LineOutput => 4,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "FirstRow" => ExposeOutMode::FirstRow,
            "AllRows" => ExposeOutMode::AllRows,
            "AnyRow" => ExposeOutMode::AnyRow,
            "RollingShutter" => ExposeOutMode::RollingShutter,
            "LineOutput" => ExposeOutMode::LineOutput,
            _ => ExposeOutMode::FirstRow,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExposeOutMode::FirstRow => "FirstRow",
            ExposeOutMode::AllRows => "AllRows",
            ExposeOutMode::AnyRow => "AnyRow",
            ExposeOutMode::RollingShutter => "RollingShutter",
            ExposeOutMode::LineOutput => "LineOutput",
        }
    }

    pub fn all_choices() -> Vec<String> {
        vec![
            "FirstRow".into(),
            "AllRows".into(),
            "AnyRow".into(),
            "RollingShutter".into(),
            "LineOutput".into(),
        ]
    }
}

// =============================================================================
// Feature Logic
// =============================================================================

pub struct PvcamFeatures;

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
    #[cfg(feature = "pvcam_hardware")]
    pub fn is_param_available(hcam: i16, param_id: u32) -> bool {
        let mut avail: rs_bool = 0;
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
    #[cfg(feature = "pvcam_hardware")]
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_TEMP) {
                return Err(anyhow!("PARAM_TEMP is not available on this camera"));
            }

            let mut temp_raw: i16 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(_conn.mock_state.lock().unwrap().temperature_c);

        #[cfg(feature = "pvcam_hardware")]
        Ok(-40.0)
    }

    /// Set temperature setpoint in Celsius
    ///
    /// # SDK Pattern (bd-ng5p)
    /// Checks PARAM_TEMP_SETPOINT availability before access.
    pub fn set_temperature_setpoint(_conn: &PvcamConnection, _celsius: f64) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            // SDK Pattern: Check availability before access
            if !Self::is_param_available(h, PARAM_TEMP_SETPOINT) {
                return Err(anyhow!(
                    "PARAM_TEMP_SETPOINT is not available on this camera"
                ));
            }

            let temp_raw = (_celsius * 100.0) as i16;
            unsafe {
                // SAFETY: h is a valid open handle; temp_raw pointer valid for duration of call.
                if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set temperature: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
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
        #[cfg(feature = "pvcam_hardware")]
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_serial_number_impl(h);
        }
        Ok("MOCK-001".to_string())
    }

    /// Get firmware version string (bd-565x)
    pub fn get_firmware_version(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_firmware_version_impl(h);
        }
        Ok("1.0.0".to_string())
    }

    /// Get sensor chip name (bd-565x)
    pub fn get_chip_name(_conn: &PvcamConnection) -> Result<String> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_chip_name_impl(h);
        }
        Ok("MockSensor".to_string())
    }

    /// Get current bit depth (bd-565x)
    pub fn get_bit_depth(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_bit_depth_impl(h);
        }
        Ok(16)
    }

    /// Get pixel readout time in nanoseconds (bd-565x)
    pub fn get_pixel_time(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_hardware")]
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(FanSpeed::from_pvcam(state.fan_speed))
        }

        #[cfg(feature = "pvcam_hardware")]
        Ok(FanSpeed::High)
    }

    /// Set fan speed (bd-glia)
    pub fn set_fan_speed(_conn: &PvcamConnection, _speed: FanSpeed) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _speed.to_pvcam();
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_FAN_SPEED_SETPOINT, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set fan speed: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
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
    pub fn get_speed_index(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_SPDTAB_INDEX)
                .map_err(|e| anyhow!("Failed to get speed index: {}", e));
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(_conn.mock_state.lock().unwrap().speed_index);

        #[cfg(feature = "pvcam_hardware")]
        Ok(0)
    }

    /// Set speed table index (bd-v54z)
    ///
    /// Changes the readout speed. Valid indices can be obtained from `list_speed_modes()`.
    pub fn set_speed_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _index as i32;
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set speed index: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.speed_index = _index;
        }
        Ok(())
    }

    /// Get current readout port index (bd-v54z)
    pub fn get_readout_port(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_READOUT_PORT)
                .map_err(|e| anyhow!("Failed to get readout port: {}", e));
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(_conn.mock_state.lock().unwrap().readout_port_index);

        #[cfg(feature = "pvcam_hardware")]
        Ok(0)
    }

    /// Set readout port (bd-v54z)
    ///
    /// Valid ports can be obtained from `list_readout_ports()`.
    pub fn set_readout_port(_conn: &PvcamConnection, _port: u16) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _port as i32;
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_READOUT_PORT, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set readout port: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.readout_port_index = _port;
        }
        Ok(())
    }

    /// List available speed modes (bd-v54z)
    ///
    /// Returns all speed table entries with their properties.
    pub fn list_speed_modes(_conn: &PvcamConnection) -> Result<Vec<SpeedMode>> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_SPDTAB_INDEX)?;
            let mut modes = Vec::with_capacity(count as usize);

            // Save current speed index to restore after enumeration
            let current_speed = Self::get_u16_param_impl(h, PARAM_SPDTAB_INDEX).unwrap_or(0);

            for i in 0..count {
                // Set speed index to enumerate its properties
                let idx = i as i32;
                unsafe {
                    // SAFETY: h is valid; setting speed index to enumerate properties.
                    if pl_set_param(h, PARAM_SPDTAB_INDEX, &idx as *const _ as *mut _) != 0 {
                        modes.push(SpeedMode {
                            index: i as u16,
                            name: Self::get_enum_string_impl(h, PARAM_SPDTAB_INDEX)
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
    pub fn list_readout_ports(_conn: &PvcamConnection) -> Result<Vec<ReadoutPort>> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_READOUT_PORT)?;
            let mut ports = Vec::with_capacity(count as usize);

            // Save current port to restore after enumeration
            let current_port = Self::get_u16_param_impl(h, PARAM_READOUT_PORT).unwrap_or(0);

            for i in 0..count {
                let idx = i as i32;
                unsafe {
                    // SAFETY: h is valid; setting port to enumerate properties.
                    if pl_set_param(h, PARAM_READOUT_PORT, &idx as *const _ as *mut _) != 0 {
                        ports.push(ReadoutPort {
                            index: i as u16,
                            name: Self::get_enum_string_impl(h, PARAM_READOUT_PORT)
                                .unwrap_or_else(|_| format!("Port {}", i)),
                        });
                    }
                }
            }

            // Restore original port
            let restore = current_port as i32;
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
    pub fn get_gain_index(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_GAIN_INDEX)
                .map_err(|e| anyhow!("Failed to get gain index: {}", e));
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(_conn.mock_state.lock().unwrap().gain_index);

        #[cfg(feature = "pvcam_hardware")]
        Ok(0)
    }

    /// Set gain index (bd-doju)
    pub fn set_gain_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _index as i32;
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_GAIN_INDEX, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set gain index: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.gain_index = _index;
        }
        Ok(())
    }

    /// List available gain modes (bd-doju)
    pub fn list_gain_modes(_conn: &PvcamConnection) -> Result<Vec<GainMode>> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_GAIN_INDEX)?;
            let mut modes = Vec::with_capacity(count as usize);

            // Save current gain to restore after enumeration
            let current_gain = Self::get_u16_param_impl(h, PARAM_GAIN_INDEX).unwrap_or(0);

            for i in 0..count {
                let idx = i as i32;
                unsafe {
                    // SAFETY: h is valid; setting gain index to enumerate properties.
                    if pl_set_param(h, PARAM_GAIN_INDEX, &idx as *const _ as *mut _) != 0 {
                        modes.push(GainMode {
                            index: i as u16,
                            name: Self::get_enum_string_impl(h, PARAM_GAIN_INDEX)
                                .unwrap_or_else(|_| format!("Gain {}", i)),
                        });
                    }
                }
            }

            // Restore original gain
            let restore = current_gain as i32;
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
    pub fn get_exposure_mode(_conn: &PvcamConnection) -> Result<ExposureMode> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
            return Ok(ExposureMode::from_pvcam(value));
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ExposureMode::from_pvcam(state.exposure_mode))
        }

        #[cfg(feature = "pvcam_hardware")]
        Ok(ExposureMode::Timed)
    }

    /// Set exposure mode (bd-iai9)
    pub fn set_exposure_mode(_conn: &PvcamConnection, _mode: ExposureMode) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_EXPOSURE_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to set exposure mode: {}",
                        get_pvcam_error()
                    ));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.exposure_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// Get current clear mode (bd-iai9)
    pub fn get_clear_mode(_conn: &PvcamConnection) -> Result<ClearMode> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ClearMode::from_pvcam(state.clear_mode))
        }

        #[cfg(feature = "pvcam_hardware")]
        Ok(ClearMode::PreExposure)
    }

    /// Set clear mode (bd-iai9)
    pub fn set_clear_mode(_conn: &PvcamConnection, _mode: ClearMode) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
            unsafe {
                // SAFETY: h is valid handle; value pointer valid for duration of call.
                if pl_set_param(h, PARAM_CLEAR_MODE, &value as *const _ as *mut _) == 0 {
                    return Err(anyhow!("Failed to set clear mode: {}", get_pvcam_error()));
                }
            }
        }
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.clear_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// Get expose out mode (bd-iai9)
    pub fn get_expose_out_mode(_conn: &PvcamConnection) -> Result<ExposeOutMode> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let state = _conn.mock_state.lock().unwrap();
            Ok(ExposeOutMode::from_pvcam(state.expose_out_mode))
        }

        #[cfg(feature = "pvcam_hardware")]
        Ok(ExposeOutMode::FirstRow)
    }

    /// Set expose out mode (bd-iai9)
    pub fn set_expose_out_mode(_conn: &PvcamConnection, _mode: ExposeOutMode) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.expose_out_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    // =========================================================================
    // Exposure Resolution (bd-i2k7.1)
    // =========================================================================

    /// Get current exposure resolution
    pub fn get_exposure_resolution(_conn: &PvcamConnection) -> Result<ExposureResolution> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _res.to_pvcam();
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_EXP_RES_INDEX)
                .map_err(|e| anyhow!("Failed to get exposure resolution index: {}", e));
        }
        Ok(0)
    }

    /// Set exposure resolution index
    pub fn set_exposure_resolution_index(_conn: &PvcamConnection, _index: u16) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _index as i32;
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
    // ADC & Sensor Parameters (bd-i2k7.2, bd-i2k7.3)
    // =========================================================================

    /// Get current ADC offset (bd-i2k7.2)
    pub fn get_adc_offset(_conn: &PvcamConnection) -> Result<i16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i16 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
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
        #[cfg(feature = "pvcam_hardware")]
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_PREMASK)
                .map_err(|e| anyhow!("Failed to get pre-mask: {}", e));
        }
        Ok(0)
    }

    pub fn get_post_mask(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_POSTMASK)
                .map_err(|e| anyhow!("Failed to get post-mask: {}", e));
        }
        Ok(0)
    }

    pub fn get_pre_scan(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u16_param_impl(h, PARAM_PRESCAN)
                .map_err(|e| anyhow!("Failed to get pre-scan: {}", e));
        }
        Ok(0)
    }

    pub fn get_post_scan(_conn: &PvcamConnection) -> Result<u16> {
        #[cfg(feature = "pvcam_hardware")]
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: f64 = 0.0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
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
        Ok(0)
    }

    /// Get post-trigger delay in microseconds (bd-ejx3)
    pub fn get_post_trigger_delay_us(_conn: &PvcamConnection) -> Result<u32> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i64 = 0;
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
        Ok(0)
    }

    // =========================================================================
    // Shutter Control (bd-e8ah)
    // =========================================================================

    /// Get current shutter status (bd-e8ah)
    pub fn get_shutter_status(_conn: &PvcamConnection) -> Result<ShutterStatus> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(ShutterMode::from_pvcam(
            _conn.mock_state.lock().unwrap().shutter_mode,
        ));

        #[cfg(feature = "pvcam_hardware")]
        Ok(ShutterMode::Normal)
    }

    /// Set shutter open mode (bd-e8ah)
    ///
    /// Controls the physical shutter behavior or TTL output signal.
    pub fn set_shutter_mode(_conn: &PvcamConnection, _mode: ShutterMode) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: uns32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: uns32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _delay_us as uns32;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _delay_us as uns32;
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
    pub fn list_pp_features(_conn: &PvcamConnection) -> Result<Vec<PPFeature>> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_PP_INDEX)?;
            let mut features = Vec::with_capacity(count as usize);

            for i in 0..count {
                // Select the PP feature
                let idx = i as i32;
                unsafe {
                    // SAFETY: h is valid; setting PP index to enumerate features.
                    if pl_set_param(h, PARAM_PP_INDEX, &idx as *const _ as *mut _) == 0 {
                        continue;
                    }
                }

                // Get feature info
                let feature = PPFeature {
                    index: i as u16,
                    id: Self::get_u16_param_impl(h, PARAM_PP_FEAT_ID).unwrap_or(0),
                    name: Self::get_pp_feature_name_impl(h)
                        .unwrap_or_else(|_| format!("Feature {}", i)),
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
            },
            PPFeature {
                index: 1,
                id: 2,
                name: "PrimeLocate".to_string(),
            },
        ])
    }

    /// Get parameters for a specific PP feature (bd-we5p)
    ///
    /// First call `select_pp_feature()` to select the feature, then call this.
    pub fn list_pp_params(_conn: &PvcamConnection, _feature_index: u16) -> Result<Vec<PPParam>> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            // Select the feature first
            let feat_idx = _feature_index as i32;
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
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
                unsafe {
                    // SAFETY: h is valid; selecting PP parameter.
                    if pl_set_param(h, PARAM_PP_PARAM_INDEX, &param_idx as *const _ as *mut _) == 0
                    {
                        continue;
                    }
                }

                // Get parameter info
                let param = PPParam {
                    index: i as u16,
                    id: Self::get_u16_param_impl(h, PARAM_PP_PARAM_ID).unwrap_or(0),
                    name: Self::get_pp_param_name_impl(h)
                        .unwrap_or_else(|_| format!("Param {}", i)),
                    value: Self::get_u32_param_impl(h, PARAM_PP_PARAM).unwrap_or(0),
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
            },
            PPParam {
                index: 1,
                id: 2,
                name: "Threshold".to_string(),
                value: 100,
            },
        ])
    }

    /// Set a PP parameter value (bd-we5p)
    ///
    /// Select feature and parameter first using their indices.
    pub fn set_pp_param(
        _conn: &PvcamConnection,
        _feature_index: u16,
        _param_index: u16,
        _value: u32,
    ) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            // Select feature
            let feat_idx = _feature_index as i32;
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Select parameter
            let param_idx = _param_index as i32;
            unsafe {
                // SAFETY: h is valid; selecting PP parameter.
                if pl_set_param(h, PARAM_PP_PARAM_INDEX, &param_idx as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to select PP parameter: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Set value
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
    pub fn get_pp_param(
        _conn: &PvcamConnection,
        _feature_index: u16,
        _param_index: u16,
    ) -> Result<u32> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            // Select feature
            let feat_idx = _feature_index as i32;
            unsafe {
                // SAFETY: h is valid; selecting PP feature.
                if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                    return Err(anyhow!(
                        "Failed to select PP feature: {}",
                        get_pvcam_error()
                    ));
                }
            }

            // Select parameter
            let param_idx = _param_index as i32;
            unsafe {
                // SAFETY: h is valid; selecting PP parameter.
                if pl_set_param(h, PARAM_PP_PARAM_INDEX, &param_idx as *const _ as *mut _) == 0 {
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
        Self::set_pp_param(_conn, _feature_index, 0, if _enabled { 1 } else { 0 })
    }

    /// Reset all post-processing features to defaults
    pub fn reset_pp_features(_conn: &PvcamConnection) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_BINNING_SER)?;
            let mut factors = Vec::with_capacity(count as usize);

            for i in 0..count {
                let idx = i as i32;
                let mut value: i32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let count = Self::get_enum_count_impl(h, PARAM_BINNING_PAR)?;
            let mut factors = Vec::with_capacity(count as usize);

            for i in 0..count {
                let idx = i as i32;
                let mut value: i32 = 0;
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

    /// Get current binning as (serial, parallel) (bd-fqi8)
    pub fn get_binning(_conn: &PvcamConnection) -> Result<(u16, u16)> {
        #[cfg(feature = "pvcam_hardware")]
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
    pub fn is_metadata_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: rs_bool = 0;
            unsafe {
                // SAFETY: h is valid; value is writable rs_bool on stack.
                if pl_get_param(
                    h,
                    PARAM_METADATA_ENABLED,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    // Parameter may not be available - return false
                    return Ok(false);
                }
            }
            return Ok(value != 0);
        }
        Ok(false)
    }

    /// Enable or disable frame metadata (bd-ne6a)
    ///
    /// **WARNING:** Frame metadata is currently disabled during acquisition (see acquisition.rs).
    /// When enabled, frame buffers contain header data before pixel data which requires
    /// parsing with pl_md_frame_decode. Without proper parsing, this corrupts image data.
    ///
    /// # Future Work (Gemini SDK Review)
    ///
    /// To fully support metadata:
    /// 1. Add pl_md_create_frame_struct and pl_md_frame_decode to pvcam-sys bindings
    /// 2. Create md_frame struct to hold decoded metadata
    /// 3. Update frame_loop_hardware to detect metadata-enabled mode
    /// 4. Parse frames using pl_md_frame_decode to extract:
    ///    - Hardware timestamps (microsecond precision from FPGA)
    ///    - Hardware frame count (absolute reference for loss detection)
    ///    - Pixel data offset/size
    /// 5. Remove the force-disable in start_stream once parsing is implemented
    pub fn set_metadata_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value: rs_bool = if _enabled { 1 } else { 0 };
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
    pub fn get_centroids_config(_conn: &PvcamConnection) -> Result<CentroidsConfig> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mode = {
                let mut value: i32 = 0;
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
    pub fn set_centroids_config(_conn: &PvcamConnection, _config: &CentroidsConfig) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
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

    // =========================================================================
    // Smart Streaming (bd-0zge)
    // =========================================================================

    /// Check if Smart Streaming is enabled (bd-0zge)
    pub fn is_smart_stream_enabled(_conn: &PvcamConnection) -> Result<bool> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: rs_bool = 0;
            unsafe {
                // SAFETY: h is valid; value is writable rs_bool on stack.
                if pl_get_param(
                    h,
                    PARAM_SMART_STREAM_MODE_ENABLED,
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

    /// Enable or disable Smart Streaming (bd-0zge)
    ///
    /// When enabled, the camera will cycle through a pre-configured sequence
    /// of exposure times without software intervention.
    pub fn set_smart_stream_enabled(_conn: &PvcamConnection, _enabled: bool) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value: rs_bool = if _enabled { 1 } else { 0 };
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.smart_stream_enabled = _enabled;
        }
        Ok(())
    }

    /// Get current Smart Streaming mode (bd-0zge)
    pub fn get_smart_stream_mode(_conn: &PvcamConnection) -> Result<SmartStreamMode> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(not(feature = "pvcam_hardware"))]
        return Ok(SmartStreamMode::from_pvcam(
            _conn.mock_state.lock().unwrap().smart_stream_mode,
        ));

        #[cfg(feature = "pvcam_hardware")]
        Ok(SmartStreamMode::Exposures)
    }

    /// Set Smart Streaming mode (bd-0zge)
    pub fn set_smart_stream_mode(_conn: &PvcamConnection, _mode: SmartStreamMode) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _mode.to_pvcam();
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
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            let mut state = _conn.mock_state.lock().unwrap();
            state.smart_stream_mode = _mode.to_pvcam();
        }
        Ok(())
    }

    /// Upload smart streaming exposure sequence to camera hardware
    pub fn upload_smart_stream(_conn: &PvcamConnection, _exposures_ms: &[u32]) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let entries = _exposures_ms.len() as u16;
            let mut ss_struct: *mut smart_stream_type = std::ptr::null_mut();

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
                // NOTE: pl_set_param expects the ADDRESS of the value to set. For TYPE_VOID_PTR
                // params like PARAM_SMART_STREAM_EXP_PARAMS, the "value" is a pointer, so we must
                // pass &ss_struct (address of the pointer), not ss_struct (the pointer value).
                if pl_set_param(
                    h,
                    PARAM_SMART_STREAM_EXP_PARAMS,
                    &ss_struct as *const _ as *mut _,
                ) == 0
                {
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _rotate.to_pvcam();
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: i32 = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value = _flip.to_pvcam();
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let mut value: rs_bool = 0;
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            let value: rs_bool = if _enabled { 1 } else { 0 };
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
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
            return Self::get_u32_param_impl(h, PARAM_HOST_FRAME_SUMMING_COUNT)
                .map_err(|e| anyhow!("Failed to get host frame summing count: {}", e));
        }
        Ok(1)
    }

    pub fn set_host_frame_summing_count(_conn: &PvcamConnection, _count: u32) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = _conn.handle() {
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
    // Private Implementation Helpers
    // =========================================================================

    #[cfg(feature = "pvcam_hardware")]
    fn get_serial_number_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_firmware_version_impl(h: i16) -> Result<String> {
        let mut version: uns16 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_chip_name_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
        unsafe {
            // SAFETY: h is valid; buf is writable array for string parameter.
            if pl_get_param(h, PARAM_CHIP_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get chip name: {}", get_pvcam_error()));
            }
            Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    #[cfg(feature = "pvcam_hardware")]
    fn get_bit_depth_impl(h: i16) -> Result<u16> {
        let mut value: i16 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_pixel_time_impl(h: i16) -> Result<u32> {
        let mut value: uns16 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_pixel_size_impl(h: i16) -> Result<(u32, u32)> {
        let mut width: uns16 = 0;
        let mut height: uns16 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_sensor_size_impl(h: i16) -> Result<(u32, u32)> {
        let mut width: uns16 = 0;
        let mut height: uns16 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_u16_param_impl(h: i16, param: u32) -> Result<u16> {
        let mut value: i32 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_enum_count_impl(h: i16, param: u32) -> Result<u32> {
        let mut count: uns32 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_enum_string_impl(h: i16, param: u32) -> Result<String> {
        // Get current value first
        let mut value: i32 = 0;
        unsafe {
            // SAFETY: h is valid; value is writable i32 on stack.
            if pl_get_param(h, param, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get enum value: {}", get_pvcam_error()));
            }
        }

        // Get string for this enum value
        let mut buf = [0i8; 256];
        unsafe {
            // SAFETY: h is valid; buf is writable; value is the enum index.
            if pl_enum_str_length(h, param, value as u32, std::ptr::null_mut()) != 0 {
                if pl_get_enum_param(
                    h,
                    param,
                    value as u32,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr(),
                    256,
                ) != 0
                {
                    return Ok(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
                }
            }
        }
        // Fallback: return value as string
        Ok(format!("{}", value))
    }

    #[cfg(feature = "pvcam_hardware")]
    fn get_u32_param_impl(h: i16, param: u32) -> Result<u32> {
        let mut value: uns32 = 0;
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_pp_feature_name_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
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

    #[cfg(feature = "pvcam_hardware")]
    fn get_pp_param_name_impl(h: i16) -> Result<String> {
        let mut buf = [0i8; 256];
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
