//! Common types and enums for Andor SDK3 driver

use serde::{Deserialize, Serialize};
use std::fmt;

/// Trigger mode for camera acquisition
///
/// Maps to SDK3 "TriggerMode" enum feature values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    /// Internal (free-running) mode
    Internal,
    /// External trigger input (edge-triggered)
    External,
    /// Software trigger via AT_Command("SoftwareTrigger")
    Software,
    /// External start: external edge starts acquisition, internal timing continues
    ExternalStart,
    /// External exposure: external signal controls both start and duration
    ExternalExposure,
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internal => write!(f, "Internal"),
            Self::External => write!(f, "External"),
            Self::Software => write!(f, "Software"),
            Self::ExternalStart => write!(f, "External Start"),
            Self::ExternalExposure => write!(f, "External Exposure"),
        }
    }
}

impl TryFrom<&str> for TriggerMode {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "Internal" => Ok(Self::Internal),
            "External" => Ok(Self::External),
            "Software" => Ok(Self::Software),
            "External Start" | "ExternalStart" => Ok(Self::ExternalStart),
            "External Exposure" | "ExternalExposure" => Ok(Self::ExternalExposure),
            _ => Err(format!("Invalid trigger mode: {s}")),
        }
    }
}

/// Gate mode for MCP (Multi-Channel Plate) intensifier.
///
/// SDK3 enum values: "CW On"(0), "CW Off"(1), "Fire Only"(2),
/// "Gate Only"(3), "Fire and Gate"(4), "DDG"(5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateMode {
    /// Continuous Wave On - MCP gate always open
    CWOn,
    /// Continuous Wave Off - MCP gate always closed
    CWOff,
    /// Fire Only - external fire trigger controls gate
    FireOnly,
    /// Gate Only - external DIRECT GATE TTL input controls gate
    GateOnly,
    /// Fire and Gate - external fire trigger + direct gate
    FireAndGate,
    /// Digital Delay Generator - internal DDG controls timing
    DDG,
}

impl fmt::Display for GateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CWOn => write!(f, "CW On"),
            Self::CWOff => write!(f, "CW Off"),
            Self::FireOnly => write!(f, "Fire Only"),
            Self::GateOnly => write!(f, "Gate Only"),
            Self::FireAndGate => write!(f, "Fire and Gate"),
            Self::DDG => write!(f, "DDG"),
        }
    }
}

impl TryFrom<&str> for GateMode {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "CW On" | "CW" | "CWOn" => Ok(Self::CWOn),
            "CW Off" | "CWOff" => Ok(Self::CWOff),
            "Fire Only" | "FireOnly" => Ok(Self::FireOnly),
            "Gate Only" | "GateOnly" => Ok(Self::GateOnly),
            "Fire and Gate" | "FireAndGate" => Ok(Self::FireAndGate),
            "DDG" => Ok(Self::DDG),
            _ => Err(format!("Invalid gate mode: {s}")),
        }
    }
}

/// Insertion delay mode for the iStar intensifier.
///
/// Controls the minimum delay between trigger arrival and gate opening.
/// SDK3 enum feature "InsertionDelay".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertionDelay {
    /// Normal insertion delay (~40ns)
    Normal,
    /// Fast insertion delay (<19ns)
    Fast,
}

impl fmt::Display for InsertionDelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Fast => write!(f, "Fast"),
        }
    }
}

impl TryFrom<&str> for InsertionDelay {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "Normal" => Ok(Self::Normal),
            "Fast" => Ok(Self::Fast),
            _ => Err(format!("Invalid insertion delay: {s}")),
        }
    }
}

/// Device lifecycle state for database persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    /// Camera not connected
    Offline,
    /// Camera opening / initializing SDK
    Initializing,
    /// Camera ready, cooled, idle
    Ready,
    /// Actively acquiring frames
    Streaming,
    /// Error state (SDK error, temperature fault, etc.)
    Error,
    /// Shutting down gracefully
    ShuttingDown,
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline => write!(f, "offline"),
            Self::Initializing => write!(f, "initializing"),
            Self::Ready => write!(f, "ready"),
            Self::Streaming => write!(f, "streaming"),
            Self::Error => write!(f, "error"),
            Self::ShuttingDown => write!(f, "shutting_down"),
        }
    }
}

impl TryFrom<&str> for DeviceState {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, <Self as TryFrom<&str>>::Error> {
        match s {
            "offline" => Ok(Self::Offline),
            "initializing" => Ok(Self::Initializing),
            "ready" => Ok(Self::Ready),
            "streaming" => Ok(Self::Streaming),
            "error" => Ok(Self::Error),
            "shutting_down" | "ShuttingDown" => Ok(Self::ShuttingDown),
            _ => Err(format!("Invalid device state: {s}")),
        }
    }
}

/// Electronic shuttering mode for SDK3 cameras.
///
/// Controls how the sensor readout is performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElectronicShutteringMode {
    /// Rolling shutter — rows exposed sequentially
    Rolling,
    /// Global shutter — all rows exposed simultaneously
    Global,
}

impl fmt::Display for ElectronicShutteringMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rolling => write!(f, "Rolling"),
            Self::Global => write!(f, "Global"),
        }
    }
}

impl TryFrom<&str> for ElectronicShutteringMode {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "Rolling" => Ok(Self::Rolling),
            "Global" => Ok(Self::Global),
            _ => Err(format!("Invalid electronic shuttering mode: {s}")),
        }
    }
}

/// Cooling status of the camera sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoolingStatus {
    /// Cooling is off
    Off,
    /// Cooling is stabilising (not yet at target)
    Stabilising,
    /// Cooling has reached and is maintaining target temperature
    Stabilised,
    /// Cooler fault detected
    Fault,
}

/// Spectrograph grating index (1-3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grating {
    Grating1 = 1,
    Grating2 = 2,
    Grating3 = 3,
}

impl fmt::Display for Grating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", *self as i32)
    }
}

impl TryFrom<i32> for Grating {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Grating1),
            2 => Ok(Self::Grating2),
            3 => Ok(Self::Grating3),
            _ => Err(format!("Invalid grating index: {value}")),
        }
    }
}

/// Flipper mirror position
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlipperMirror {
    /// Direct output (mirror out of beam path)
    Direct,
    /// Side output (mirror in beam path)
    Side,
}

impl fmt::Display for FlipperMirror {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => write!(f, "Direct"),
            Self::Side => write!(f, "Side"),
        }
    }
}

impl TryFrom<i32> for FlipperMirror {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Direct),
            1 => Ok(Self::Side),
            _ => Err(format!("Invalid flipper mirror position: {value}")),
        }
    }
}

/// Camera information from SDK
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CameraInfo {
    pub model: String,
    pub serial_number: String,
    pub firmware_version: String,
    pub sensor_width: u32,
    pub sensor_height: u32,
    pub features: FeatureSupport,
}

/// Tracks which optional SDK3 features are implemented on this camera.
///
/// Queried once at init via AT_IsImplemented. Allows conditional feature
/// access without repeated FFI calls.
#[derive(Debug, Clone, Default)]
pub struct FeatureSupport {
    pub mcp_gain: bool,
    pub gate_mode: bool,
    pub ddg_output_delay: bool,
    pub ddg_output_width: bool,
    pub sensor_cooling: bool,
    pub sensor_temperature: bool,
    pub pixel_encoding: bool,
    pub external_trigger_modes: bool,
    pub electronic_shuttering_mode: bool,
    pub frame_count: bool,
    // New features (bd-zg9e)
    pub mcp_intelligate: bool,
    pub mcp_voltage: bool,
    pub insertion_delay: bool,
    pub metadata_ddg_info: bool,
    pub metadata_mcp_gain: bool,
    pub metadata_frame_info: bool,
    pub camera_acquiring: bool,
    pub baseline_level: bool,
    pub software_trigger: bool,
}

/// Known Andor SDK3 camera models and their expected feature sets.
///
/// This is used for documentation, testing, and mock mode configuration.
/// At runtime, actual feature support is determined by AT_IsImplemented.
///
/// Reference: Andor SDK3 manual, per-camera feature matrices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraModel {
    /// iStar sCMOS — intensified camera with MCP, DDG, gate modes
    IStar,
    /// Zyla sCMOS — general purpose scientific camera
    Zyla,
    /// Neo sCMOS — high-end cooled camera
    Neo,
    /// Marana sCMOS — large format back-illuminated camera
    Marana,
    /// Sona — back-illuminated sCMOS
    Sona,
    /// Balor — large format, high speed
    Balor,
    /// SimCam — SDK simulated camera for testing
    SimCam,
}

impl CameraModel {
    /// Return the expected feature profile for this camera model.
    ///
    /// These are the features *expected* to be available based on SDK
    /// documentation. Actual availability is determined at runtime
    /// via `AT_IsImplemented`.
    pub fn expected_features(&self) -> FeatureSupport {
        match self {
            Self::IStar => FeatureSupport {
                mcp_gain: true,
                gate_mode: true,
                ddg_output_delay: true,
                ddg_output_width: true,
                sensor_cooling: true,
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
                mcp_intelligate: true,
                mcp_voltage: true,
                insertion_delay: true,
                metadata_ddg_info: true,
                metadata_mcp_gain: true,
                metadata_frame_info: true,
                camera_acquiring: true,
                baseline_level: true,
                software_trigger: true,
            },
            Self::Zyla | Self::Neo | Self::Marana | Self::Sona | Self::Balor => FeatureSupport {
                mcp_gain: false,
                gate_mode: false,
                ddg_output_delay: false,
                ddg_output_width: false,
                sensor_cooling: true,
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
                camera_acquiring: true,
                baseline_level: true,
                software_trigger: true,
                ..Default::default()
            },
            Self::SimCam => FeatureSupport {
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
                camera_acquiring: true,
                ..Default::default()
            },
        }
    }

    /// Try to identify camera model from its SDK model string.
    pub fn from_model_string(model: &str) -> Option<Self> {
        let lower = model.to_lowercase();
        if lower.contains("istar") {
            Some(Self::IStar)
        } else if lower.contains("zyla") {
            Some(Self::Zyla)
        } else if lower.contains("neo") {
            Some(Self::Neo)
        } else if lower.contains("marana") {
            Some(Self::Marana)
        } else if lower.contains("sona") {
            Some(Self::Sona)
        } else if lower.contains("balor") {
            Some(Self::Balor)
        } else if lower.contains("simcam") {
            Some(Self::SimCam)
        } else {
            None
        }
    }
}

/// Spectrograph information from SDK
#[derive(Debug, Clone)]
pub struct SpectrographInfo {
    pub model: String,
    pub serial_number: String,
    pub num_gratings: usize,
}

/// Grating information
#[derive(Debug, Clone)]
pub struct GratingInfo {
    pub lines_per_mm: f64,
    pub blaze_wavelength_nm: f64,
}

/// Wavelength calibration array
///
/// Maps pixel index to wavelength in nanometers.
/// Obtained from spectrograph GetCalibration() function.
#[derive(Debug, Clone)]
pub struct WavelengthCalibration {
    pub wavelengths_nm: Vec<f64>,
    pub num_pixels: usize,
}

impl WavelengthCalibration {
    /// Create new calibration from wavelength array
    pub fn new(wavelengths: Vec<f64>) -> Self {
        let num_pixels = wavelengths.len();
        Self {
            wavelengths_nm: wavelengths,
            num_pixels,
        }
    }

    /// Get wavelength for pixel index
    pub fn wavelength(&self, pixel: usize) -> Option<f64> {
        self.wavelengths_nm.get(pixel).copied()
    }

    /// Get wavelength range
    pub fn range(&self) -> Option<(f64, f64)> {
        if self.wavelengths_nm.is_empty() {
            return None;
        }
        let min = self
            .wavelengths_nm
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let max = self
            .wavelengths_nm
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        Some((min, max))
    }
}

/// Wavelength limits for a specific grating
#[derive(Debug, Clone, Copy)]
pub struct WavelengthLimits {
    pub min_nm: f64,
    pub max_nm: f64,
}

/// Slit port identifier for the Shamrock spectrograph
///
/// The Shamrock has up to 4 motorized slit ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SlitPort {
    /// Input slit, side port (port 1)
    InputSide = 1,
    /// Input slit, direct port (port 2)
    InputDirect = 2,
    /// Output slit, side port (port 3)
    OutputSide = 3,
    /// Output slit, direct port (port 4)
    OutputDirect = 4,
}

impl fmt::Display for SlitPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputSide => write!(f, "InputSide"),
            Self::InputDirect => write!(f, "InputDirect"),
            Self::OutputSide => write!(f, "OutputSide"),
            Self::OutputDirect => write!(f, "OutputDirect"),
        }
    }
}

impl TryFrom<i32> for SlitPort {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::InputSide),
            2 => Ok(Self::InputDirect),
            3 => Ok(Self::OutputSide),
            4 => Ok(Self::OutputDirect),
            _ => Err(format!("Invalid slit port: {value}")),
        }
    }
}

/// Filter wheel position (1-indexed)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterPosition(pub i32);

impl fmt::Display for FilterPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Filter {}", self.0)
    }
}
