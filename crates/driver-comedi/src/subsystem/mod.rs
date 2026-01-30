//! Comedi subsystem implementations.
//!
//! This module contains safe wrappers for the various Comedi subsystem types:
//!
//! - [`analog_input`] - Analog input (AI) channels
//! - [`analog_output`] - Analog output (AO) channels
//! - [`digital_io`] - Digital I/O (DIO) channels
//! - [`counter`] - Counter/timer channels

pub mod analog_input;
pub mod analog_output;
pub mod counter;
pub mod digital_io;

use comedi_sys::comedi_range;

/// Voltage range information for a channel.
#[derive(Debug, Clone, Copy)]
pub struct Range {
    /// Range index (for Comedi API calls)
    pub index: u32,
    /// Minimum voltage
    pub min: f64,
    /// Maximum voltage
    pub max: f64,
    /// Unit (0 = volts, 1 = mA, 2 = none)
    pub unit: u32,
}

impl Default for Range {
    fn default() -> Self {
        Self {
            index: 0,
            min: -10.0,
            max: 10.0,
            unit: 0,
        }
    }
}

impl Range {
    /// Create a new range with the specified parameters.
    pub fn new(index: u32, min: f64, max: f64) -> Self {
        Self {
            index,
            min,
            max,
            unit: 0, // Default to volts
        }
    }

    /// Create a range from a Comedi range pointer.
    ///
    /// # Safety
    ///
    /// The pointer must be valid and point to a valid comedi_range struct.
    pub(crate) unsafe fn from_ptr(index: u32, ptr: *const comedi_range) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }

        let range = &*ptr;
        Some(Self {
            index,
            min: range.min,
            max: range.max,
            unit: range.unit,
        })
    }

    /// Get the span (max - min) of this range.
    pub fn span(&self) -> f64 {
        self.max - self.min
    }

    /// Check if this is a bipolar range (includes negative values).
    pub fn is_bipolar(&self) -> bool {
        self.min < 0.0
    }

    /// Check if this is a unipolar range (0 to max).
    pub fn is_unipolar(&self) -> bool {
        self.min >= 0.0
    }

    /// Human-readable description of the range.
    pub fn description(&self) -> String {
        let unit_str = match self.unit {
            0 => "V",
            1 => "mA",
            _ => "",
        };
        format!("{:.3} to {:.3} {}", self.min, self.max, unit_str)
    }
}

/// Analog reference type for measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum AnalogReference {
    /// Ground reference (single-ended, referenced to ground)
    #[default]
    Ground = comedi_sys::AREF_GROUND,
    /// Common reference (single-ended, referenced to common)
    Common = comedi_sys::AREF_COMMON,
    /// Differential (measures difference between two inputs)
    Differential = comedi_sys::AREF_DIFF,
    /// Other/board-specific reference
    Other = comedi_sys::AREF_OTHER,
}

impl AnalogReference {
    /// Convert to raw Comedi value.
    pub fn to_raw(self) -> u32 {
        self as u32
    }
}
