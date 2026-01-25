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

// =============================================================================
// Driver Errors
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverErrorKind {
    Initialization,
    Configuration,
    Communication,
    Shutdown,
    Hardware,
    Timeout,
    Permission,
    InvalidParameter,
    Unknown,
}

impl std::fmt::Display for DriverErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            DriverErrorKind::Initialization => "initialization",
            DriverErrorKind::Configuration => "configuration",
            DriverErrorKind::Communication => "communication",
            DriverErrorKind::Shutdown => "shutdown",
            DriverErrorKind::Hardware => "hardware",
            DriverErrorKind::Timeout => "timeout",
            DriverErrorKind::Permission => "permission",
            DriverErrorKind::InvalidParameter => "invalid_parameter",
            DriverErrorKind::Unknown => "unknown",
        };
        write!(f, "{}", label)
    }
}

#[derive(Error, Debug, Clone)]
#[error("Driver '{driver_type}' {kind} error: {message}")]
pub struct DriverError {
    pub driver_type: String,
    pub kind: DriverErrorKind,
    pub message: String,
}

impl DriverError {
    pub fn new(
        driver_type: impl Into<String>,
        kind: DriverErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            driver_type: driver_type.into(),
            kind,
            message: message.into(),
        }
    }
}

/// Convenience alias for results using the application error type.
pub type AppResult<T> = std::result::Result<T, DaqError>;

/// Primary error type for the DAQ application.
///
/// This enum consolidates all error types that can occur during data acquisition,
/// from configuration parsing to hardware communication and data processing.
///
/// # Error Categories
///
/// Errors fall into three broad categories:
///
/// 1. **Configuration Errors** - `Config`, `Configuration`, `FeatureNotEnabled`
///    - Occur during startup or configuration reload
///    - Permanent errors requiring config file changes or rebuild
///    - Recovery: Fix configuration and restart
///
/// 2. **Hardware/Communication Errors** - `Instrument`, `SerialPortNotConnected`, etc.
///    - Occur during instrument communication
///    - May be transient (network glitch) or permanent (hardware failure)
///    - Recovery: Retry with backoff or check hardware connections
///
/// 3. **Runtime Errors** - `Processing`, `ModuleBusyDuringOperation`, etc.
///    - Occur during normal operation
///    - Usually transient or state-related
///    - Recovery: Retry after state change or abort operation
///
/// # Example
///
/// ```rust,ignore
/// use daq_core::error::{DaqError, AppResult};
///
/// fn configure_instrument() -> AppResult<()> {
///     // Config parsing errors automatically convert to DaqError::Config
///     let settings = load_config()?;
///
///     // Instrument errors wrap device-specific errors
///     connect_instrument(&settings)
///         .map_err(|e| DaqError::Instrument(e.to_string()))?;
///
///     Ok(())
/// }
/// ```
#[derive(Error, Debug)]
pub enum DaqError {
    /// Configuration file parsing failed.
    ///
    /// Occurs when loading TOML/YAML configuration files that have syntax errors,
    /// missing required fields, or type mismatches.
    ///
    /// **Error Type**: Permanent - requires fixing the configuration file.
    ///
    /// **Recovery Strategy**: Abort startup, display error to user, fix config file.
    ///
    /// **Source**: Wraps `config::ConfigError` from the `config` crate.
    ///
    /// # Example
    ///
    /// ```toml
    /// # Invalid TOML - missing closing quote
    /// device_name = "camera
    /// ```
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    /// Configuration validation failed.
    ///
    /// Occurs when configuration values parse correctly but fail semantic validation
    /// (e.g., negative exposure time, invalid IP address format, port out of range).
    ///
    /// **Error Type**: Permanent - requires fixing the configuration values.
    ///
    /// **Recovery Strategy**: Abort startup, display validation error message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use daq_core::error::DaqError;
    ///
    /// fn validate_exposure(exposure_seconds: f64) -> Result<(), DaqError> {
    ///     if exposure_seconds < 0.0 {
    ///         return Err(DaqError::Configuration(
    ///             "exposure_seconds must be positive".into()
    ///         ));
    ///     }
    ///     Ok(())
    /// }
    /// ```
    #[error("Configuration validation error: {0}")]
    Configuration(String),

    /// Standard I/O operation failed.
    ///
    /// Occurs during file operations, network I/O, or other standard I/O operations.
    /// Common causes include permission denied, file not found, disk full, or
    /// network timeouts.
    ///
    /// **Error Type**: Can be transient (network timeout) or permanent (permission denied).
    ///
    /// **Recovery Strategy**:
    /// - For `ErrorKind::NotFound` or `PermissionDenied`: Abort and report to user
    /// - For `ErrorKind::TimedOut` or `WouldBlock`: Retry with exponential backoff
    /// - For other kinds: Log and decide based on context
    ///
    /// **Source**: Wraps `std::io::Error`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Tokio async runtime error.
    ///
    /// Occurs during async I/O operations in the Tokio runtime, such as async file
    /// operations, TCP/UDP communication, or timer operations.
    ///
    /// **Error Type**: Can be transient (temporary network issue) or permanent
    /// (runtime shutdown, resource exhaustion).
    ///
    /// **Recovery Strategy**: Similar to `Io` errors - inspect the wrapped `std::io::Error`
    /// and retry for transient errors, abort for permanent errors.
    ///
    /// **Source**: Wraps `std::io::Error` from Tokio operations.
    #[error("Tokio runtime error: {0}")]
    Tokio(std::io::Error),

    /// Instrument hardware error.
    ///
    /// Occurs when communicating with hardware instruments (cameras, stages, lasers).
    /// Causes include command failures, invalid responses, hardware faults, or
    /// communication protocol errors.
    ///
    /// **Error Type**: Depends on cause:
    /// - Transient: Communication glitch, temporary hardware busy state
    /// - Permanent: Hardware fault, incompatible firmware, device disconnected
    ///
    /// **Recovery Strategy**:
    /// - Retry 2-3 times with short delay for transient errors
    /// - Check hardware connections and power for permanent errors
    /// - May require device power cycle or manual intervention
    ///
    /// # Example
    ///
    /// ```rust
    /// use daq_core::error::DaqError;
    ///
    /// const CAMERA_FAULT: u32 = 0x01;
    ///
    /// fn check_camera_status(status_code: u32) -> Result<(), DaqError> {
    ///     if status_code == CAMERA_FAULT {
    ///         return Err(DaqError::Instrument(
    ///             format!("Camera fault code: {:#x}", status_code)
    ///         ));
    ///     }
    ///     Ok(())
    /// }
    /// ```
    #[error("Instrument error: {0}")]
    Instrument(String),

    /// Structured driver error with category
    #[error("{0}")]
    Driver(DriverError),

    /// Serial port is not connected.
    ///
    /// Occurs when attempting operations on a serial port that hasn't been
    /// opened or has been closed. This typically indicates a programming error
    /// (using port before connecting) or handling after disconnect.
    ///
    /// **Error Type**: Permanent for current operation - requires reconnection.
    ///
    /// **Recovery Strategy**: Call the port's connect/open method before retrying.
    /// If reconnection fails, check hardware and cable connections.
    #[error("Serial port not connected")]
    SerialPortNotConnected,

    /// Serial port reached end-of-file unexpectedly.
    ///
    /// Occurs when the serial device disconnects mid-communication or sends incomplete
    /// data. This typically indicates the hardware was unplugged or powered off.
    ///
    /// **Error Type**: Permanent - device disconnected.
    ///
    /// **Recovery Strategy**: Abort current operation, attempt to detect and
    /// reopen port. May require user to reconnect hardware.
    #[error("Unexpected EOF from serial port")]
    SerialUnexpectedEof,

    /// Serial support not compiled into binary.
    ///
    /// Occurs when code attempts to use serial port functionality but the
    /// application was built without the `instrument_serial` feature flag.
    ///
    /// **Error Type**: Permanent - requires rebuild.
    ///
    /// **Recovery Strategy**: Rebuild application with:
    /// ```bash
    /// cargo build --features instrument_serial
    /// ```
    #[error("Serial support not enabled. Rebuild with --features instrument_serial")]
    SerialFeatureDisabled,

    /// Data processing operation failed.
    ///
    /// Occurs during post-acquisition data processing such as FFT computation,
    /// filtering, background subtraction, or analysis pipeline failures.
    ///
    /// **Error Type**: Usually transient - often due to invalid input data or
    /// numerical issues (NaN, overflow).
    ///
    /// **Recovery Strategy**:
    /// - Skip the problematic data frame and continue
    /// - Log the error with context for debugging
    /// - Check for systematic data issues if frequent
    #[error("Data processing error: {0}")]
    Processing(String),

    /// Requested frame dimensions exceed supported limits.
    #[error("Frame dimensions {width}x{height} exceed maximum {max_dimension} per dimension")]
    FrameDimensionsTooLarge {
        width: u32,
        height: u32,
        max_dimension: u32,
    },

    /// Calculating a size overflowed usize.
    #[error("Size overflow while computing {context}")]
    SizeOverflow { context: &'static str },

    /// Frame payload exceeds maximum allowed size.
    #[error("Frame size {bytes} bytes exceeds maximum {max_bytes} bytes")]
    FrameTooLarge { bytes: usize, max_bytes: usize },

    /// Response payload exceeds maximum allowed size.
    #[error("Response size {bytes} bytes exceeds maximum {max_bytes} bytes")]
    ResponseTooLarge { bytes: usize, max_bytes: usize },

    /// Script payload exceeds maximum allowed size.
    #[error("Script size {bytes} bytes exceeds maximum {max_bytes} bytes")]
    ScriptTooLarge { bytes: usize, max_bytes: usize },

    /// Module does not support the requested operation.
    ///
    /// Occurs when calling a capability method on a module that doesn't implement
    /// that capability (e.g., calling `set_exposure()` on a power meter module).
    ///
    /// **Error Type**: Permanent - indicates programming error or misconfiguration.
    ///
    /// **Recovery Strategy**: Check module capabilities before calling operations.
    /// Fix calling code to only use supported operations.
    ///
    /// # Example
    ///
    /// ```rust
    /// use daq_core::error::DaqError;
    ///
    /// fn acquire_frame_from_power_meter() -> Result<(), DaqError> {
    ///     // Power meter doesn't support frame acquisition
    ///     Err(DaqError::ModuleOperationNotSupported(
    ///         "Power meters do not produce frames".into()
    ///     ))
    /// }
    /// ```
    #[error("Module does not support operation: {0}")]
    ModuleOperationNotSupported(String),

    /// Module is busy and cannot accept new operations.
    ///
    /// Occurs when attempting to start a new operation while the module is
    /// still executing a previous operation (e.g., starting acquisition while
    /// already acquiring, moving stage during an active move).
    ///
    /// **Error Type**: Transient - resolves when current operation completes.
    ///
    /// **Recovery Strategy**: Wait for current operation to complete, then retry.
    /// Use status polling or completion callbacks to coordinate operations.
    #[error("Module is busy during operation")]
    ModuleBusyDuringOperation,

    /// No camera has been assigned to this module.
    ///
    /// Occurs when attempting camera operations on a module that requires a camera
    /// but none has been assigned in the configuration.
    ///
    /// **Error Type**: Permanent - requires configuration update.
    ///
    /// **Recovery Strategy**: Update configuration to assign a camera to the module,
    /// then reload configuration or restart application.
    #[error("No camera assigned to module")]
    CameraNotAssigned,

    /// Required feature not enabled at compile time.
    ///
    /// Occurs when attempting to use functionality (hardware driver, storage format,
    /// network protocol) that wasn't included in the build due to missing feature flags.
    ///
    /// **Error Type**: Permanent - requires rebuild with appropriate features.
    ///
    /// **Recovery Strategy**: Rebuild with the required feature flag.
    /// The error message includes the specific feature name to enable.
    ///
    /// # Example
    ///
    /// ```bash
    /// # Enable HDF5 storage support
    /// cargo build --features storage_hdf5
    ///
    /// # Enable all hardware drivers
    /// cargo build --features all_hardware
    /// ```
    #[error("Feature '{0}' is not enabled. Please build with --features {0}")]
    FeatureNotEnabled(String),

    /// Feature is enabled but implementation is incomplete.
    ///
    /// Occurs when a feature flag is enabled but the implementation is still
    /// under development. This is used during the V5 migration to mark
    /// work-in-progress code paths.
    ///
    /// **Error Type**: Permanent - requires code implementation.
    ///
    /// **Recovery Strategy**: Either:
    /// - Wait for feature completion in future release
    /// - Use alternative code path if available
    /// - Disable the feature flag and use legacy implementation
    ///
    /// The second string parameter provides context about what's missing.
    #[error("Feature '{0}' is enabled but not yet implemented. {1}")]
    FeatureIncomplete(String, String),

    /// Application shutdown encountered errors.
    ///
    /// Occurs during graceful shutdown when one or more subsystems fail to
    /// clean up properly (e.g., camera fails to stop acquisition, file handles
    /// fail to close, hardware fails to return to safe state).
    ///
    /// **Error Type**: Permanent - shutdown already in progress.
    ///
    /// **Recovery Strategy**: Log all errors for diagnostics. Proceed with
    /// forceful shutdown if needed. Manual hardware inspection may be required.
    ///
    /// Contains a vector of all errors encountered during shutdown for complete
    /// error reporting.
    #[error("Shutdown failed with errors")]
    ShutdownFailed(Vec<DaqError>),

    /// Failed to send parameter update (no active subscribers).
    ///
    /// Occurs when attempting to broadcast a parameter change but no modules
    /// or components are subscribed to receive updates. This is typically a
    /// benign condition indicating nothing is listening.
    ///
    /// **Error Type**: Transient - subscribers may connect later.
    ///
    /// **Recovery Strategy**: This is often not a true error. Log at debug level
    /// and continue. If subscribers are expected, verify subscription setup.
    #[error("Failed to send value update (no subscribers)")]
    ParameterNoSubscribers,

    /// Attempted to modify a read-only parameter.
    ///
    /// Occurs when code attempts to write to a parameter marked as read-only
    /// in the configuration. Examples include hardware-determined values like
    /// sensor temperature or calculated values like total frame count.
    ///
    /// **Error Type**: Permanent - indicates programming error or misconfiguration.
    ///
    /// **Recovery Strategy**: Fix calling code to avoid writes to read-only parameters.
    /// Check parameter metadata before attempting writes.
    #[error("Parameter is read-only")]
    ParameterReadOnly,

    /// Invalid choice for enumerated parameter.
    ///
    /// Occurs when setting a parameter to a value not in its allowed choices
    /// (e.g., setting trigger mode to "invalid" when only "software", "hardware",
    /// "external" are allowed).
    ///
    /// **Error Type**: Permanent - indicates invalid input data.
    ///
    /// **Recovery Strategy**: Query valid choices and select from allowed values.
    /// Validate user input against parameter constraints.
    #[error("Invalid choice for parameter")]
    ParameterInvalidChoice,

    /// No hardware reader connected for parameter.
    ///
    /// Occurs when attempting to read a hardware-backed parameter but no
    /// hardware interface has been registered. This indicates incomplete
    /// module initialization.
    ///
    /// **Error Type**: Permanent - requires proper module setup.
    ///
    /// **Recovery Strategy**: Ensure hardware interface is registered during
    /// module initialization before attempting parameter reads.
    #[error("No hardware reader connected")]
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

    #[test]
    fn test_driver_error_display() {
        let err = DaqError::Driver(DriverError::new(
            "mock_camera",
            DriverErrorKind::Initialization,
            "failed to connect",
        ));
        assert!(err
            .to_string()
            .contains("Driver 'mock_camera' initialization error"));
    }
}
