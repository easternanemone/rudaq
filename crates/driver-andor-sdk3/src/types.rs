//! Common types and enums for Andor SDK3 driver

use serde::{Deserialize, Serialize};
use std::fmt;

/// Trigger mode for camera acquisition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    /// Internal (free-running) mode
    Internal,
    /// External trigger input
    External,
    /// Software trigger
    Software,
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internal => write!(f, "Internal"),
            Self::External => write!(f, "External"),
            Self::Software => write!(f, "Software"),
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
            _ => Err(format!("Invalid trigger mode: {}", s)),
        }
    }
}

/// Gate mode for MCP (Micro-Channel Plate) intensifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateMode {
    /// Continuous Wave - MCP always active
    CW,
    /// Digital Delay Generator - use DDG timing control
    DDG,
}

impl fmt::Display for GateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CW => write!(f, "CW"),
            Self::DDG => write!(f, "DDG"),
        }
    }
}

impl TryFrom<&str> for GateMode {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "CW" => Ok(Self::CW),
            "DDG" => Ok(Self::DDG),
            _ => Err(format!("Invalid gate mode: {}", s)),
        }
    }
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
            _ => Err(format!("Invalid grating index: {}", value)),
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
            _ => Err(format!("Invalid flipper mirror position: {}", value)),
        }
    }
}

/// Camera information from SDK
#[derive(Debug, Clone)]
pub struct CameraInfo {
    pub model: String,
    pub serial_number: String,
    pub firmware_version: String,
    pub sensor_width: u32,
    pub sensor_height: u32,
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
