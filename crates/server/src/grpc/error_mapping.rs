//! Semantic mapping from `DaqError` to gRPC Status codes (bd-cxvg).
//!
//! This module is the server-side half of the error round-trip described in
//! `common_traits::error`.  It converts structured `DaqError` values into
//! `tonic::Status` responses with:
//!
//! 1. An appropriate gRPC status code (see Mapping Philosophy below).
//! 2. Custom metadata headers for structured client-side recovery:
//!    - `x-daq-error-kind`  -- the high-level error category (e.g., "driver", "instrument")
//!    - `x-daq-driver-type` -- the driver type string, when the error is a `DriverError`
//!    - `x-daq-driver-kind` -- the `DriverErrorKind` variant name (e.g., "communication")
//!
//! The `client` crate's `ClientError` type provides accessor methods that extract
//! these headers, completing the round-trip:
//!
//!   `DaqError` -> `map_daq_error_to_status()` -> wire -> `ClientError::daq_error_kind()`
//!
//! When the server receives an `anyhow::Error` from a capability trait method, the
//! service handler should attempt to downcast before calling this mapper:
//!
//! ```rust,ignore
//! use common::error::DaqError;
//! use server::grpc::map_daq_error_to_status;
//!
//! fn anyhow_to_status(err: anyhow::Error) -> tonic::Status {
//!     // Try structured downcast first
//!     if let Some(daq_err) = err.downcast_ref::<DaqError>() {
//!         return map_daq_error_to_status(daq_err);
//!     }
//!     // Fallback: opaque internal error
//!     tonic::Status::internal(err.to_string())
//! }
//! ```
//!
//! # Mapping Philosophy
//!
//! - **InvalidArgument**: Client sent bad input (config errors, invalid choices)
//! - **FailedPrecondition**: System state doesn't allow operation (missing camera, no subscribers)
//! - **Unavailable**: Resource temporarily unavailable (hardware faults, connection issues, busy)
//! - **ResourceExhausted**: Limits exceeded (frame too large, script too large)
//! - **Unimplemented**: Feature not enabled or incomplete
//! - **PermissionDenied**: Client lacks permission (read-only parameters)
//! - **Internal**: Server-side bugs (I/O errors, processing failures)
//! - **Aborted**: Operation was aborted (unexpected EOF)
//! - **DeadlineExceeded**: Driver timeout
//! - **NotFound**: Referenced device or resource not found

use common::error::{DaqError, DriverError, ErrorKind, StorageError};
use std::str::FromStr;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Code, Status};

// Re-export the shared header constants from common-traits for backward compat.
pub use common::error::{
    GRPC_DRIVER_KIND_HEADER as DRIVER_KIND_HEADER, GRPC_DRIVER_TYPE_HEADER as DRIVER_TYPE_HEADER,
    GRPC_ERROR_KIND_HEADER as ERROR_KIND_HEADER,
};

fn sanitize_metadata_value(value: &str) -> String {
    let sanitized: String = value.chars().filter(|c| c.is_ascii()).collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn insert_metadata(metadata: &mut MetadataMap, key: &'static str, value: &str) {
    let sanitized = sanitize_metadata_value(value);
    if let Ok(val) = MetadataValue::from_str(&sanitized) {
        metadata.insert(key, val);
    }
}

/// Build a `Status` with the `x-daq-error-kind` header and optional driver metadata.
fn status_with_metadata(
    code: Code,
    message: impl Into<String>,
    error_kind: ErrorKind,
    driver: Option<&DriverError>,
) -> Status {
    let mut status = Status::new(code, message.into());
    let metadata = status.metadata_mut();
    insert_metadata(metadata, ERROR_KIND_HEADER, error_kind.as_str());
    if let Some(driver) = driver {
        insert_metadata(metadata, DRIVER_TYPE_HEADER, &driver.driver_type);
        insert_metadata(metadata, DRIVER_KIND_HEADER, &driver.kind.to_string());
    }
    status
}

/// Convert an `anyhow::Error` into a `tonic::Status` by attempting structured downcasts.
///
/// The downcast order ensures the most specific error type wins:
///
/// 1. `DaqError` -- full variant-level mapping via [`map_daq_error_to_status`]
/// 2. `DriverError` -- wraps in `DaqError::Driver` then maps
/// 3. `StorageError` -- wraps in `DaqError::Storage` then maps
/// 4. Fallback -- `Code::Internal` with the anyhow display chain
///
/// Service handlers receiving `anyhow::Result` from capability traits should use
/// this function instead of manually converting to `Status`.
pub fn anyhow_to_status(err: anyhow::Error) -> Status {
    // Walk the full error chain so that `anyhow::Context` wrappers don't hide
    // structured errors.  The first recognized type wins.
    for cause in err.chain() {
        if let Some(daq_err) = cause.downcast_ref::<DaqError>() {
            return map_daq_error_to_status(daq_err);
        }
        if let Some(driver_err) = cause.downcast_ref::<DriverError>() {
            return map_driver_error_to_status(driver_err);
        }
        if let Some(storage_err) = cause.downcast_ref::<StorageError>() {
            return map_storage_error_to_status(storage_err);
        }
    }
    // Fallback: opaque internal error with the full anyhow display chain
    status_with_metadata(Code::Internal, err.to_string(), ErrorKind::Unknown, None)
}

/// Map a DaqError to an appropriate gRPC Status.
///
/// This function provides semantic mapping from internal error types to
/// gRPC status codes that clients can interpret meaningfully.
///
/// # Examples
///
/// ```
/// use common::error::DaqError;
/// use server::grpc::map_daq_error_to_status;
/// use tonic::Code;
///
/// let err = DaqError::SerialPortNotConnected;
/// let status = map_daq_error_to_status(&err);
/// assert_eq!(status.code(), Code::Unavailable);
/// ```
pub fn map_daq_error_to_status(err: &DaqError) -> Status {
    match err {
        // Configuration errors -> InvalidArgument
        // Client provided bad configuration that cannot be accepted
        DaqError::Config(e) => status_with_metadata(
            Code::InvalidArgument,
            format!("Config error: {e}"),
            ErrorKind::Config,
            None,
        ),
        DaqError::Configuration(msg) => status_with_metadata(
            Code::InvalidArgument,
            format!("Configuration error: {msg}"),
            ErrorKind::Configuration,
            None,
        ),

        // Hardware/connection errors -> Unavailable
        // Resource is temporarily unavailable, client may retry
        DaqError::Instrument(msg) => status_with_metadata(
            Code::Unavailable,
            format!("Instrument error: {msg}"),
            ErrorKind::Instrument,
            None,
        ),
        DaqError::Driver(driver_err) => {
            use common::error::DriverErrorKind;
            let code = match driver_err.kind {
                DriverErrorKind::Configuration | DriverErrorKind::InvalidParameter => {
                    Code::InvalidArgument
                }
                DriverErrorKind::Initialization | DriverErrorKind::Safety => {
                    Code::FailedPrecondition
                }
                DriverErrorKind::Communication
                | DriverErrorKind::Hardware
                | DriverErrorKind::Busy => Code::Unavailable,
                DriverErrorKind::Timeout => Code::DeadlineExceeded,
                DriverErrorKind::Permission => Code::PermissionDenied,
                DriverErrorKind::NotFound => Code::NotFound,
                DriverErrorKind::Shutdown | DriverErrorKind::Unknown => Code::Internal,
            };
            status_with_metadata(
                code,
                driver_err.to_string(),
                ErrorKind::Driver,
                Some(driver_err),
            )
        }
        DaqError::SerialPortNotConnected => status_with_metadata(
            Code::Unavailable,
            "Serial port not connected",
            ErrorKind::Serial,
            None,
        ),
        DaqError::ModuleBusyDuringOperation => status_with_metadata(
            Code::Unavailable,
            "Module busy during operation",
            ErrorKind::ModuleBusy,
            None,
        ),

        // Serial protocol errors
        DaqError::SerialUnexpectedEof => status_with_metadata(
            Code::Aborted,
            "Serial communication: unexpected EOF",
            ErrorKind::SerialEof,
            None,
        ),
        DaqError::SerialFeatureDisabled => status_with_metadata(
            Code::Unimplemented,
            "Serial feature is disabled",
            ErrorKind::SerialDisabled,
            None,
        ),

        // Resource limit errors -> ResourceExhausted
        DaqError::FrameDimensionsTooLarge {
            width,
            height,
            max_dimension,
        } => status_with_metadata(
            Code::ResourceExhausted,
            format!("Frame dimensions {width}x{height} exceed maximum {max_dimension}"),
            ErrorKind::FrameDimensions,
            None,
        ),
        DaqError::FrameTooLarge { bytes, max_bytes } => status_with_metadata(
            Code::ResourceExhausted,
            format!("Frame size {bytes} bytes exceeds maximum {max_bytes}"),
            ErrorKind::FrameTooLarge,
            None,
        ),
        DaqError::ResponseTooLarge { bytes, max_bytes } => status_with_metadata(
            Code::ResourceExhausted,
            format!("Response size {bytes} bytes exceeds maximum {max_bytes}"),
            ErrorKind::ResponseTooLarge,
            None,
        ),
        DaqError::ScriptTooLarge { bytes, max_bytes } => status_with_metadata(
            Code::ResourceExhausted,
            format!("Script size {bytes} bytes exceeds maximum {max_bytes}"),
            ErrorKind::ScriptTooLarge,
            None,
        ),
        DaqError::SizeOverflow { context } => status_with_metadata(
            Code::ResourceExhausted,
            format!("Size overflow in {context}"),
            ErrorKind::SizeOverflow,
            None,
        ),

        // Module state errors -> FailedPrecondition or Unimplemented
        DaqError::ModuleOperationNotSupported(op) => status_with_metadata(
            Code::Unimplemented,
            format!("Operation not supported: {op}"),
            ErrorKind::ModuleUnsupported,
            None,
        ),
        DaqError::CameraNotAssigned => status_with_metadata(
            Code::FailedPrecondition,
            "Camera not assigned to module",
            ErrorKind::CameraNotAssigned,
            None,
        ),

        // Feature availability -> Unimplemented
        DaqError::FeatureNotEnabled(feature) => status_with_metadata(
            Code::Unimplemented,
            format!("Feature not enabled: {feature}"),
            ErrorKind::FeatureNotEnabled,
            None,
        ),
        DaqError::FeatureIncomplete(feature, reason) => status_with_metadata(
            Code::Unimplemented,
            format!("Feature '{feature}' incomplete: {reason}"),
            ErrorKind::FeatureIncomplete,
            None,
        ),

        // Shutdown errors -> Internal (aggregated failures)
        DaqError::ShutdownFailed(errors) => {
            let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            status_with_metadata(
                Code::Internal,
                format!("Shutdown failed: {}", messages.join("; ")),
                ErrorKind::ShutdownFailed,
                None,
            )
        }

        // Parameter errors
        DaqError::ParameterNoSubscribers => status_with_metadata(
            Code::FailedPrecondition,
            "No subscribers for parameter update",
            ErrorKind::ParameterNoSubscribers,
            None,
        ),
        DaqError::ParameterReadOnly => status_with_metadata(
            Code::PermissionDenied,
            "Parameter is read-only",
            ErrorKind::ParameterReadOnly,
            None,
        ),
        DaqError::ParameterInvalidChoice => status_with_metadata(
            Code::InvalidArgument,
            "Invalid parameter choice",
            ErrorKind::ParameterInvalidChoice,
            None,
        ),
        DaqError::ParameterNoHardwareReader => status_with_metadata(
            Code::FailedPrecondition,
            "Parameter has no hardware reader configured",
            ErrorKind::ParameterNoReader,
            None,
        ),

        // I/O errors -> Internal
        // These are server-side failures that shouldn't happen in normal operation
        DaqError::Io(e) => status_with_metadata(
            Code::Internal,
            format!("I/O error: {e}"),
            ErrorKind::Io,
            None,
        ),
        DaqError::Tokio(e) => status_with_metadata(
            Code::Internal,
            format!("Tokio I/O error: {e}"),
            ErrorKind::Tokio,
            None,
        ),

        // Processing errors -> Internal
        DaqError::Processing(msg) => status_with_metadata(
            Code::Internal,
            format!("Processing error: {msg}"),
            ErrorKind::Processing,
            None,
        ),

        // Storage errors
        DaqError::Storage(e) => {
            let code = match e.kind {
                common::error::StorageErrorKind::Configuration => Code::InvalidArgument,
                _ => Code::Internal,
            };
            let msg = match e.kind {
                common::error::StorageErrorKind::Configuration => {
                    format!("Storage configuration error: {}", e.message)
                }
                common::error::StorageErrorKind::Io => {
                    format!("Storage I/O error: {}", e.message)
                }
                _ => format!("Storage error: {}", e.message),
            };
            status_with_metadata(code, msg, ErrorKind::Storage, None)
        }

        // Feature-specific errors
        #[cfg(feature = "storage_hdf5")]
        DaqError::Hdf5(e) => status_with_metadata(
            Code::Internal,
            format!("HDF5 error: {e}"),
            ErrorKind::Hdf5,
            None,
        ),
        #[cfg(feature = "storage_arrow")]
        DaqError::Arrow(e) => status_with_metadata(
            Code::Internal,
            format!("Arrow error: {e}"),
            ErrorKind::Arrow,
            None,
        ),

        DaqError::Serde(e) => status_with_metadata(
            Code::Internal,
            format!("Serialization error: {e}"),
            ErrorKind::Serde,
            None,
        ),
        DaqError::TaskJoin(e) => status_with_metadata(
            Code::Internal,
            format!("Task join error: {e}"),
            ErrorKind::TaskJoin,
            None,
        ),
    }
}

/// Map a standalone `DriverError` to gRPC Status without cloning.
fn map_driver_error_to_status(driver_err: &DriverError) -> Status {
    use common::error::DriverErrorKind;
    let code = match driver_err.kind {
        DriverErrorKind::Configuration | DriverErrorKind::InvalidParameter => Code::InvalidArgument,
        DriverErrorKind::Initialization | DriverErrorKind::Safety => Code::FailedPrecondition,
        DriverErrorKind::Communication | DriverErrorKind::Hardware | DriverErrorKind::Busy => {
            Code::Unavailable
        }
        DriverErrorKind::Timeout => Code::DeadlineExceeded,
        DriverErrorKind::Permission => Code::PermissionDenied,
        DriverErrorKind::NotFound => Code::NotFound,
        DriverErrorKind::Shutdown | DriverErrorKind::Unknown => Code::Internal,
    };
    status_with_metadata(
        code,
        driver_err.to_string(),
        ErrorKind::Driver,
        Some(driver_err),
    )
}

/// Map a standalone `StorageError` to gRPC Status without cloning.
fn map_storage_error_to_status(storage_err: &StorageError) -> Status {
    let code = match storage_err.kind {
        common::error::StorageErrorKind::Configuration => Code::InvalidArgument,
        _ => Code::Internal,
    };
    let msg = match storage_err.kind {
        common::error::StorageErrorKind::Configuration => {
            format!("Storage configuration error: {}", storage_err.message)
        }
        common::error::StorageErrorKind::Io => {
            format!("Storage I/O error: {}", storage_err.message)
        }
        _ => format!("Storage error: {}", storage_err.message),
    };
    status_with_metadata(code, msg, ErrorKind::Storage, None)
}

/// Extension trait for converting `Result<T, DaqError>` to `Result<T, Status>`.
pub trait DaqResultExt<T> {
    /// Convert a `DaqError` result to a tonic `Status` result.
    #[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
    fn map_daq_err(self) -> Result<T, Status>;
}

impl<T> DaqResultExt<T> for Result<T, DaqError> {
    fn map_daq_err(self) -> Result<T, Status> {
        self.map_err(|e| map_daq_error_to_status(&e))
    }
}

/// Extension trait for converting `Result<T, anyhow::Error>` to `Result<T, Status>`.
///
/// Uses the downcast chain in [`anyhow_to_status`] to recover structured error types
/// before falling back to an opaque `Code::Internal` status.
pub trait AnyhowResultExt<T> {
    /// Convert an anyhow result to a tonic `Status` result via downcast chain.
    #[allow(clippy::result_large_err)]
    fn map_anyhow_err(self) -> Result<T, Status>;
}

impl<T> AnyhowResultExt<T> for Result<T, anyhow::Error> {
    fn map_anyhow_err(self) -> Result<T, Status> {
        self.map_err(anyhow_to_status)
    }
}
