//! Error types for Andor SDK3 driver
//!
//! Provides error handling for both camera and spectrograph operations.

use thiserror::Error;

/// Result type for Andor operations
pub type AndorResult<T> = Result<T, AndorError>;

/// Errors that can occur during Andor SDK3 operations
#[derive(Error, Debug)]
pub enum AndorError {
    /// SDK function returned an error code
    #[error("Andor SDK error {code}: {message}")]
    SdkError { code: i32, message: String },

    /// Device not found or invalid index
    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    /// Feature/parameter not available on this device
    #[error("Feature not available: {0}")]
    FeatureNotAvailable(String),

    /// Feature is read-only
    #[error("Feature is read-only: {0}")]
    ReadOnly(String),

    /// Value out of valid range
    #[error("Value out of range for {feature}: {value} not in [{min}, {max}]")]
    OutOfRange {
        feature: String,
        value: f64,
        min: f64,
        max: f64,
    },

    /// Invalid enum string value
    #[error("Invalid enum value '{value}' for feature '{feature}'. Valid values: {valid}")]
    InvalidEnumValue {
        feature: String,
        value: String,
        valid: String,
    },

    /// Device is not initialized
    #[error("Device not initialized")]
    NotInitialized,

    /// Acquisition is not running
    #[error("Acquisition not running")]
    NotAcquiring,

    /// Timeout waiting for frame or operation
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Buffer allocation failed
    #[error("Buffer allocation failed: {0}")]
    BufferAllocation(String),

    /// Wide string conversion failed
    #[error("Wide string conversion failed: {0}")]
    StringConversion(String),

    /// General I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Other errors
    #[error("Other error: {0}")]
    Other(String),
}

impl AndorError {
    /// Create SDK error from code
    pub fn from_code(code: i32) -> Self {
        let message = error_code_to_string(code);
        Self::SdkError { code, message }
    }

    /// Check if error is a timeout
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Timeout(_) => true,
            Self::SdkError { code, .. } if *code == 13 => true,
            _ => false,
        }
    }

    /// Check if error is a feature not available error
    pub fn is_not_available(&self) -> bool {
        matches!(self, Self::FeatureNotAvailable(_))
    }
}

/// Convert Andor SDK3 error code to human-readable string
///
/// Based on AT_error_codes from atcore.h
fn error_code_to_string(code: i32) -> String {
    match code {
        0 => "Success".to_string(),
        1 => "Not initialized".to_string(),
        2 => "Not implemented".to_string(),
        3 => "Read only".to_string(),
        4 => "Not readable".to_string(),
        5 => "Not writable".to_string(),
        6 => "Out of range".to_string(),
        7 => "Index not available".to_string(),
        8 => "Index not implemented".to_string(),
        9 => "Exceeded max string length".to_string(),
        10 => "Connection lost".to_string(),
        11 => "No data".to_string(),
        12 => "Invalid handle".to_string(),
        13 => "Timed out".to_string(),
        14 => "Buffer full".to_string(),
        15 => "Invalid size".to_string(),
        16 => "Invalid alignment".to_string(),
        17 => "Communication error".to_string(),
        18 => "String not available".to_string(),
        19 => "String not implemented".to_string(),
        20 => "Null feature".to_string(),
        21 => "Null handle".to_string(),
        22 => "Null implemented".to_string(),
        23 => "Null read only".to_string(),
        100 => "Hardware overflow".to_string(),
        _ => format!("Unknown error code: {}", code),
    }
}

/// Convert SDK error code to anyhow::Error
pub fn sdk_result(code: i32) -> anyhow::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(AndorError::from_code(code).into())
    }
}
