//! Custom error types for the application.
//!
//! This module defines the primary error type, [`DaqError`], for the entire application.
//! Using the `thiserror` crate, it provides a centralized and consistent way to handle
//! different kinds of errors that can occur, from I/O and configuration issues to
//! instrument-specific problems.
//!
//! # Error Hierarchy and Boundary Contracts
//!
//! The rust-daq project uses a layered error strategy, with different conventions at
//! each architectural boundary:
//!
//! ## 1. `DaqError` -- the canonical application error enum (this module)
//!
//! [`DaqError`] is a `thiserror`-derived enum that consolidates all known failure modes
//! across the system.  It lives in `common-traits` (re-exported via `common::error`) so
//! every crate in the workspace can construct and match on it.
//!
//! Structured sub-errors [`DriverError`] and [`StorageError`] carry typed "kind" enums
//! (`DriverErrorKind`, `StorageErrorKind`) so that mapping layers (e.g., gRPC) can
//! select the right status code without string parsing.
//!
//! ## 2. Capability trait methods return `anyhow::Result`
//!
//! All capability trait methods (in `capabilities.rs`) return `anyhow::Result<T>` rather
//! than `Result<T, DaqError>`.  This gives driver authors maximum flexibility: they can
//! use any error type internally and attach context with `anyhow::Context`.
//!
//! **Best practice for drivers:**
//!
//! ```rust,ignore
//! use anyhow::Context;
//! use common::error::{DriverError, DriverErrorKind};
//!
//! async fn move_to(&self, pos: f64) -> anyhow::Result<()> {
//!     self.serial
//!         .send(format!("MOVE {pos}"))
//!         .await
//!         .context(DriverError::new("my_stage", DriverErrorKind::Communication, "serial send failed"))?;
//!     Ok(())
//! }
//! ```
//!
//! By attaching a `DriverError` (or `DaqError`) as the *context*, the error chain
//! retains both the original low-level cause and a structured error the server can
//! downcast at the gRPC boundary.
//!
//! ## 3. gRPC boundary -- `error_mapping.rs` in the `server` crate
//!
//! The server's `map_daq_error_to_status()` function converts `DaqError` into
//! `tonic::Status`.  When the error originates from an `anyhow::Error` (the common
//! case for capability trait results), the server first tries a downcast chain:
//!
//!   1. `downcast_ref::<DaqError>()` -- full variant-level mapping
//!   2. `downcast_ref::<DriverError>()` -- driver kind mapping
//!   3. `downcast_ref::<StorageError>()` -- storage kind mapping
//!   4. Fallback: string-based `Code::Internal`
//!
//! Custom gRPC metadata headers (`x-daq-error-kind`, `x-daq-driver-type`,
//! `x-daq-driver-kind`) are set so that the client can recover structured
//! information without parsing the human-readable message.
//!
//! ## 4. `ClientError` in the `client` crate
//!
//! `ClientError` wraps `tonic::Status` and provides accessor methods
//! (`daq_error_kind()`, `device_id()`, `driver_kind()`) that extract the
//! metadata headers set by the server, closing the round-trip:
//!
//!   `DaqError` -> `tonic::Status` (with metadata) -> `ClientError` (with structured extraction)
//!
//! ## Error Categories
//!
//! Errors fall into three broad categories:
//!
//! 1. **Configuration Errors** -- `Config`, `Configuration`, `FeatureNotEnabled`
//!    - Occur during startup or configuration reload
//!    - Permanent errors requiring config file changes or rebuild
//!    - Recovery: Fix configuration and restart
//!
//! 2. **Hardware/Communication Errors** -- `Instrument`, `Driver`, `SerialPortNotConnected`
//!    - Occur during instrument communication
//!    - May be transient (network glitch) or permanent (hardware failure)
//!    - Recovery: Retry with backoff or check hardware connections
//!
//! 3. **Runtime Errors** -- `Processing`, `ModuleBusyDuringOperation`, resource limits
//!    - Occur during normal operation
//!    - Usually transient or state-related
//!    - Recovery: Retry after state change or abort operation

use thiserror::Error;

// =============================================================================
// gRPC error metadata header keys (shared contract between server + client)
// =============================================================================

/// gRPC metadata header for the DaqError variant name (e.g., "driver", "instrument").
pub const GRPC_ERROR_KIND_HEADER: &str = "x-daq-error-kind";
/// gRPC metadata header for the driver type (e.g., "pvcam", "andor").
pub const GRPC_DRIVER_TYPE_HEADER: &str = "x-daq-driver-type";
/// gRPC metadata header for the driver error kind (e.g., "communication", "timeout").
pub const GRPC_DRIVER_KIND_HEADER: &str = "x-daq-driver-kind";

// =============================================================================
// ErrorKind — typed error category for gRPC metadata (replaces ad-hoc strings)
// =============================================================================

/// High-level error category transmitted via the `x-daq-error-kind` gRPC metadata
/// header.
///
/// Each variant maps 1:1 to a `DaqError` variant (or group of variants).  The
/// wire representation is a lowercase-with-underscores ASCII string (e.g.,
/// `ErrorKind::SerialEof` ↔ `"serial_eof"`).
///
/// # Round-trip
///
/// - **Server side**: `map_daq_error_to_status()` passes an `ErrorKind` variant to
///   `status_with_metadata()`, which serialises it via `as_str()`.
/// - **Client side**: `ClientError::daq_error_kind()` returns `Option<ErrorKind>` by
///   parsing the header with `ErrorKind::from_str()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Config,
    Configuration,
    Instrument,
    Driver,
    Serial,
    ModuleBusy,
    SerialEof,
    SerialDisabled,
    FrameDimensions,
    FrameTooLarge,
    ResponseTooLarge,
    ScriptTooLarge,
    SizeOverflow,
    ModuleUnsupported,
    CameraNotAssigned,
    FeatureNotEnabled,
    FeatureIncomplete,
    ShutdownFailed,
    ParameterNoSubscribers,
    ParameterReadOnly,
    ParameterInvalidChoice,
    ParameterNoReader,
    Io,
    Tokio,
    Processing,
    Storage,
    Hdf5,
    Arrow,
    Serde,
    TaskJoin,
    Unknown,
}

impl ErrorKind {
    /// Wire-format string, suitable for gRPC metadata values.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Configuration => "configuration",
            Self::Instrument => "instrument",
            Self::Driver => "driver",
            Self::Serial => "serial",
            Self::ModuleBusy => "module_busy",
            Self::SerialEof => "serial_eof",
            Self::SerialDisabled => "serial_disabled",
            Self::FrameDimensions => "frame_dimensions",
            Self::FrameTooLarge => "frame_too_large",
            Self::ResponseTooLarge => "response_too_large",
            Self::ScriptTooLarge => "script_too_large",
            Self::SizeOverflow => "size_overflow",
            Self::ModuleUnsupported => "module_unsupported",
            Self::CameraNotAssigned => "camera_not_assigned",
            Self::FeatureNotEnabled => "feature_not_enabled",
            Self::FeatureIncomplete => "feature_incomplete",
            Self::ShutdownFailed => "shutdown_failed",
            Self::ParameterNoSubscribers => "parameter_no_subscribers",
            Self::ParameterReadOnly => "parameter_read_only",
            Self::ParameterInvalidChoice => "parameter_invalid_choice",
            Self::ParameterNoReader => "parameter_no_reader",
            Self::Io => "io",
            Self::Tokio => "tokio",
            Self::Processing => "processing",
            Self::Storage => "storage",
            Self::Hdf5 => "hdf5",
            Self::Arrow => "arrow",
            Self::Serde => "serde",
            Self::TaskJoin => "task_join",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ErrorKind {
    type Err = UnknownErrorKind;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "config" => Ok(Self::Config),
            "configuration" => Ok(Self::Configuration),
            "instrument" => Ok(Self::Instrument),
            "driver" => Ok(Self::Driver),
            "serial" => Ok(Self::Serial),
            "module_busy" => Ok(Self::ModuleBusy),
            "serial_eof" => Ok(Self::SerialEof),
            "serial_disabled" => Ok(Self::SerialDisabled),
            "frame_dimensions" => Ok(Self::FrameDimensions),
            "frame_too_large" => Ok(Self::FrameTooLarge),
            "response_too_large" => Ok(Self::ResponseTooLarge),
            "script_too_large" => Ok(Self::ScriptTooLarge),
            "size_overflow" => Ok(Self::SizeOverflow),
            "module_unsupported" => Ok(Self::ModuleUnsupported),
            "camera_not_assigned" => Ok(Self::CameraNotAssigned),
            "feature_not_enabled" => Ok(Self::FeatureNotEnabled),
            "feature_incomplete" => Ok(Self::FeatureIncomplete),
            "shutdown_failed" => Ok(Self::ShutdownFailed),
            "parameter_no_subscribers" => Ok(Self::ParameterNoSubscribers),
            "parameter_read_only" => Ok(Self::ParameterReadOnly),
            "parameter_invalid_choice" => Ok(Self::ParameterInvalidChoice),
            "parameter_no_reader" => Ok(Self::ParameterNoReader),
            "io" => Ok(Self::Io),
            "tokio" => Ok(Self::Tokio),
            "processing" => Ok(Self::Processing),
            "storage" => Ok(Self::Storage),
            "hdf5" => Ok(Self::Hdf5),
            "arrow" => Ok(Self::Arrow),
            "serde" => Ok(Self::Serde),
            "task_join" => Ok(Self::TaskJoin),
            "unknown" => Ok(Self::Unknown),
            _ => Err(UnknownErrorKind(s.to_string())),
        }
    }
}

/// Error returned when parsing an unrecognised `ErrorKind` wire string.
#[derive(Debug, Clone, Error)]
#[error("unknown error kind: {0}")]
pub struct UnknownErrorKind(pub String);

// =============================================================================
// Storage Errors
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageErrorKind {
    Hdf5,
    Arrow,
    RingBuffer,
    Io,
    Configuration,
    Serialization,
    Other,
}

impl std::fmt::Display for StorageErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            StorageErrorKind::Hdf5 => "hdf5",
            StorageErrorKind::Arrow => "arrow",
            StorageErrorKind::RingBuffer => "ring_buffer",
            StorageErrorKind::Io => "io",
            StorageErrorKind::Configuration => "configuration",
            StorageErrorKind::Serialization => "serialization",
            StorageErrorKind::Other => "other",
        };
        write!(f, "{label}")
    }
}

#[derive(Error, Debug, Clone)]
#[error("Storage {kind} error: {message}")]
pub struct StorageError {
    pub kind: StorageErrorKind,
    pub message: String,
}

impl StorageError {
    pub fn new(kind: StorageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

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
    /// Device is busy and cannot accept the operation right now.
    Busy,
    /// Referenced device or resource was not found.
    NotFound,
    /// Safety-critical error (e.g. interlock violation, laser fault).
    Safety,
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
            DriverErrorKind::Busy => "busy",
            DriverErrorKind::NotFound => "not_found",
            DriverErrorKind::Safety => "safety",
            DriverErrorKind::Unknown => "unknown",
        };
        write!(f, "{label}")
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
/// use common::error::{DaqError, AppResult};
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
    #[error("Configuration error: {0}")]
    Config(String),

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
    /// ```rust,ignore
    /// use common::error::DaqError;
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

    /// Tokio task join error.
    #[error("Task join error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),

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
    /// ```rust,ignore
    /// use common::error::DaqError;
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

    /// Structured storage error
    #[error("{0}")]
    Storage(#[from] StorageError),

    /// HDF5 error.
    #[cfg(feature = "storage_hdf5")]
    #[error("HDF5 error: {0}")]
    Hdf5(#[from] hdf5::Error),

    /// Arrow error.
    #[cfg(feature = "storage_arrow")]
    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

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
    /// ```rust,ignore
    /// use common::error::DaqError;
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

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

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

// Note: Removed CoreDaqError conversions - common crate deleted
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
        assert!(
            err.to_string()
                .contains("Driver 'mock_camera' initialization error")
        );
    }
}
