//! Custom error types for the application.
//!
//! This module defines the primary error type, `DaqError`, for the entire application.
//! Using the `thiserror` crate, it provides a centralized and consistent way to handle
//! different kinds of errors that can occur, from I/O and configuration issues to
//! instrument-specific problems.
//!
//! ## Error Hierarchy
//!
//! `DaqError` is an enum that consolidates various error sources:
//!
//! - **`Config`**: Wraps errors from the `config` crate, typically related to file parsing
//!   or format issues in the configuration files.
//! - **`Configuration`**: Represents semantic errors in the configuration, such as invalid
//!   values that pass parsing but are logically incorrect (e.g., an invalid IP address format).
//!   These are usually caught during the validation step.
//! - **`Io`**: Wraps standard `std::io::Error`, covering all file and network I/O issues.
//! - **`Tokio`**: Specifically for errors related to the Tokio runtime, though it also wraps
//!   `std::io::Error` as Tokio I/O operations are a common source.
//! - **`Instrument`**: A general category for errors originating from instrument drivers. This
//!   could be anything from a communication failure to an invalid command sent to the hardware.
//! - **`Processing`**: For errors that occur during data processing stages, such as filtering
//!   or analysis.
//! - **`FeatureNotEnabled`**: A specific error used when the code attempts to use functionality
//!   (like a specific instrument driver or storage format) that was not included at compile
//!   time via feature flags. This provides a clear message to the user on how to enable it.
//!
//! By using `#[from]`, `DaqError` can be seamlessly created from underlying error types,
//! simplifying error handling throughout the application with the `?` operator.

use thiserror::Error;

/// Convenience alias for results using the application error type.
pub type AppResult<T> = std::result::Result<T, DaqError>;

#[derive(Error, Debug)]
pub enum DaqError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("Configuration validation error: {0}")]
    Configuration(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Tokio runtime error: {0}")]
    Tokio(std::io::Error),

    #[error("Instrument error: {0}")]
    Instrument(String),

    #[error("Serial port not connected")]
    SerialPortNotConnected,

    #[error("Unexpected EOF from serial port")]
    SerialUnexpectedEof,

    #[error("Serial support not enabled. Rebuild with --features instrument_serial")]
    SerialFeatureDisabled,

    #[error("Data processing error: {0}")]
    Processing(String),

    #[error("Module does not support operation: {0}")]
    ModuleOperationNotSupported(String),

    #[error("Module is busy during operation")]
    ModuleBusyDuringOperation,

    #[error("No camera assigned to module")]
    CameraNotAssigned,

    #[error("Feature '{0}' is not enabled. Please build with --features {0}")]
    FeatureNotEnabled(String),

    #[error("Feature '{0}' is enabled but not yet implemented. {1}")]
    FeatureIncomplete(String, String),

    #[error("Shutdown failed with errors")]
    ShutdownFailed(Vec<DaqError>),

    #[error("Failed to send value update (no subscribers)")]
    #[cfg(feature = "v4")]
    ParameterNoSubscribers,

    #[error("Parameter is read-only")]
    #[cfg(feature = "v4")]
    ParameterReadOnly,

    #[error("Invalid choice for parameter")]
    #[cfg(feature = "v4")]
    ParameterInvalidChoice,

    #[error("No hardware reader connected")]
    #[cfg(feature = "v4")]
    ParameterNoHardwareReader,
}

// Note: Removed CoreDaqError conversions - daq_core crate deleted
// DaqError is now the only error type for the application

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DaqError::Instrument("laser failed".to_string());
        assert_eq!(err.to_string(), "Instrument error: laser failed");
    }

    #[test]
    fn test_shutdown_failed_error() {
        let err = DaqError::ShutdownFailed(vec![
            DaqError::Instrument("camera timeout".into()),
            DaqError::Processing("buffer drain".into()),
        ]);
        assert!(err.to_string().contains("Shutdown failed"));
    }
}
