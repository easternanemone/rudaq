//! Error types for Comedi operations.
//!
//! This module provides a comprehensive error type that covers all possible
//! failure modes when interacting with Comedi devices.

use daq_core::error::{DaqError, DriverError, DriverErrorKind};
use std::ffi::CStr;
use std::fmt;
use thiserror::Error;

/// Result type alias for Comedi operations.
pub type Result<T> = std::result::Result<T, ComediError>;

/// Errors that can occur when working with Comedi devices.
#[derive(Error, Debug)]
pub enum ComediError {
    /// Device could not be opened (file not found, permissions, etc.)
    #[error("Failed to open device '{path}': {message}")]
    DeviceNotFound { path: String, message: String },

    /// Permission denied when accessing the device
    #[error("Permission denied for device '{path}'. Check udev rules or run as root.")]
    PermissionDenied { path: String },

    /// Device is already in use by another process
    #[error("Device '{path}' is busy (in use by another process)")]
    DeviceBusy { path: String },

    /// Invalid subdevice index
    #[error("Invalid subdevice {subdevice}: device has {max} subdevices")]
    InvalidSubdevice { subdevice: u32, max: u32 },

    /// Subdevice type mismatch (e.g., trying to use DIO subdevice as AI)
    #[error("Subdevice {subdevice} is type {actual:?}, expected {expected:?}")]
    SubdeviceTypeMismatch {
        subdevice: u32,
        expected: SubdeviceTypeError,
        actual: SubdeviceTypeError,
    },

    /// Invalid channel number
    #[error("Invalid channel {channel}: subdevice {subdevice} has {max} channels")]
    InvalidChannel {
        subdevice: u32,
        channel: u32,
        max: u32,
    },

    /// Invalid range index
    #[error("Invalid range {range}: channel has {max} ranges")]
    InvalidRange { range: u32, max: u32 },

    /// Buffer overflow during acquisition
    #[error("Buffer overflow: data acquisition too slow")]
    BufferOverflow,

    /// Buffer underrun during output
    #[error("Buffer underrun: data output too slow")]
    BufferUnderrun,

    /// Hardware error reported by the device
    #[error("Hardware error: {message}")]
    HardwareError { message: String },

    /// Command configuration error
    #[error("Invalid command configuration: {message}")]
    InvalidCommand { message: String },

    /// Calibration error
    #[error("Calibration error: {message}")]
    CalibrationError { message: String },

    /// I/O error from the operating system
    #[error("I/O error: {0}")]
    StdIoError(#[from] std::io::Error),

    /// I/O error with message
    #[error("I/O error: {message}")]
    IoError { message: String },

    /// Command test or execution error
    #[error("Command error (code {code}): {message}")]
    CommandError { code: i32, message: String },

    /// Low-level Comedi library error
    #[error("Comedi error ({errno}): {message}")]
    LibraryError { errno: i32, message: String },

    /// Null pointer returned from FFI
    #[error("Null pointer returned from Comedi function: {function}")]
    NullPointer { function: String },

    /// Operation not supported by this device/subdevice
    #[error("Operation not supported: {message}")]
    NotSupported { message: String },

    /// Invalid configuration or parameter
    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },
}

/// Subdevice type for error messages (avoids depending on full type enum in errors)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubdeviceTypeError {
    Unused,
    AnalogInput,
    AnalogOutput,
    DigitalInput,
    DigitalOutput,
    DigitalIO,
    Counter,
    Timer,
    Memory,
    Calibration,
    Processor,
    Serial,
    Pwm,
    Unknown(i32),
}

impl fmt::Display for SubdeviceTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unused => write!(f, "Unused"),
            Self::AnalogInput => write!(f, "Analog Input"),
            Self::AnalogOutput => write!(f, "Analog Output"),
            Self::DigitalInput => write!(f, "Digital Input"),
            Self::DigitalOutput => write!(f, "Digital Output"),
            Self::DigitalIO => write!(f, "Digital I/O"),
            Self::Counter => write!(f, "Counter"),
            Self::Timer => write!(f, "Timer"),
            Self::Memory => write!(f, "Memory"),
            Self::Calibration => write!(f, "Calibration"),
            Self::Processor => write!(f, "Processor"),
            Self::Serial => write!(f, "Serial"),
            Self::Pwm => write!(f, "PWM"),
            Self::Unknown(t) => write!(f, "Unknown({})", t),
        }
    }
}

impl ComediError {
    /// Create an error from the current Comedi errno.
    ///
    /// # Safety
    ///
    /// This function calls FFI functions to get the error state.
    pub(crate) unsafe fn from_errno() -> Self {
        let errno = comedi_sys::comedi_errno();
        let msg_ptr = comedi_sys::comedi_strerror(errno);
        let message = if msg_ptr.is_null() {
            "Unknown error".to_string()
        } else {
            CStr::from_ptr(msg_ptr).to_string_lossy().into_owned()
        };

        Self::LibraryError { errno, message }
    }

    /// Create an error from a specific errno value.
    ///
    /// # Safety
    ///
    /// This function calls FFI functions.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_errno_value(errno: i32) -> Self {
        let msg_ptr = comedi_sys::comedi_strerror(errno);
        let message = if msg_ptr.is_null() {
            "Unknown error".to_string()
        } else {
            CStr::from_ptr(msg_ptr).to_string_lossy().into_owned()
        };

        Self::LibraryError { errno, message }
    }

    /// Check if this is a "device not found" type error.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::DeviceNotFound { .. })
    }

    /// Check if this is a permission error.
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::PermissionDenied { .. })
    }

    /// Check if the device is busy.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::DeviceBusy { .. })
    }
}

impl From<ComediError> for DaqError {
    fn from(err: ComediError) -> Self {
        let kind = match &err {
            ComediError::DeviceNotFound { .. }
            | ComediError::InvalidSubdevice { .. }
            | ComediError::SubdeviceTypeMismatch { .. }
            | ComediError::InvalidChannel { .. }
            | ComediError::InvalidRange { .. }
            | ComediError::InvalidConfig { .. }
            | ComediError::NotSupported { .. }
            | ComediError::InvalidCommand { .. } => DriverErrorKind::Configuration,
            ComediError::PermissionDenied { .. } => DriverErrorKind::Permission,
            ComediError::DeviceBusy { .. }
            | ComediError::BufferOverflow
            | ComediError::BufferUnderrun
            | ComediError::HardwareError { .. } => DriverErrorKind::Hardware,
            ComediError::CalibrationError { .. } => DriverErrorKind::Initialization,
            ComediError::StdIoError(_)
            | ComediError::IoError { .. }
            | ComediError::CommandError { .. }
            | ComediError::LibraryError { .. }
            | ComediError::NullPointer { .. } => DriverErrorKind::Communication,
        };
        DaqError::Driver(DriverError::new("comedi", kind, err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ComediError::InvalidChannel {
            subdevice: 0,
            channel: 20,
            max: 16,
        };
        assert!(err.to_string().contains("20"));
        assert!(err.to_string().contains("16"));
    }

    #[test]
    fn test_subdevice_type_display() {
        assert_eq!(SubdeviceTypeError::AnalogInput.to_string(), "Analog Input");
        assert_eq!(SubdeviceTypeError::Counter.to_string(), "Counter");
    }
}
