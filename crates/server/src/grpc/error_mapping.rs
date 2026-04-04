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
//!     if let Some(daq_err) = err.downcast::<DaqError>().ok() {
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

use common::error::{DaqError, DriverError, StorageError};
use std::str::FromStr;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Code, Status};

/// Metadata header carrying the high-level error category (e.g., "driver", "instrument",
/// "config", "storage").  Always set on mapped errors.
pub const ERROR_KIND_HEADER: &str = "x-daq-error-kind";
/// Metadata header carrying the driver type string (e.g., "mock_camera", "comedi").
/// Only set when the error is a `DriverError`.
pub const DRIVER_TYPE_HEADER: &str = "x-daq-driver-type";
/// Metadata header carrying the `DriverErrorKind` variant (e.g., "communication", "timeout").
/// Only set when the error is a `DriverError`.
pub const DRIVER_KIND_HEADER: &str = "x-daq-driver-kind";

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
    error_kind: &'static str,
    driver: Option<&DriverError>,
) -> Status {
    let mut status = Status::new(code, message.into());
    let metadata = status.metadata_mut();
    insert_metadata(metadata, ERROR_KIND_HEADER, error_kind);
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
    // Try DaqError first (most specific application error)
    if let Ok(daq_err) = err.downcast::<DaqError>() {
        return map_daq_error_to_status(daq_err);
    }
    // Try DriverError (common from driver trait methods)
    if let Ok(driver_err) = err.downcast::<DriverError>() {
        return map_daq_error_to_status(DaqError::Driver(driver_err));
    }
    // Try StorageError
    if let Ok(storage_err) = err.downcast::<StorageError>() {
        return map_daq_error_to_status(DaqError::Storage(storage_err));
    }
    // Fallback: opaque internal error with the full anyhow display chain
    status_with_metadata(Code::Internal, err.to_string(), "unknown", None)
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
/// let status = map_daq_error_to_status(err);
/// assert_eq!(status.code(), Code::Unavailable);
/// ```
pub fn map_daq_error_to_status(err: DaqError) -> Status {
    match err {
        // Configuration errors → InvalidArgument
        // Client provided bad configuration that cannot be accepted
        DaqError::Config(e) => Status::new(Code::InvalidArgument, format!("Config error: {}", e)),
        DaqError::Configuration(msg) => Status::new(
            Code::InvalidArgument,
            format!("Configuration error: {}", msg),
        ),

        // Hardware/connection errors → Unavailable
        // Resource is temporarily unavailable, client may retry
        DaqError::Instrument(msg) => status_with_metadata(
            Code::Unavailable,
            format!("Instrument error: {}", msg),
            "instrument",
            None,
        ),
        DaqError::Driver(ref err) => match err.kind {
            common::error::DriverErrorKind::Configuration
            | common::error::DriverErrorKind::InvalidParameter => {
                status_with_metadata(Code::InvalidArgument, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::Initialization => status_with_metadata(
                Code::FailedPrecondition,
                err.to_string(),
                "driver",
                Some(err),
            ),
            common::error::DriverErrorKind::Communication
            | common::error::DriverErrorKind::Hardware => {
                status_with_metadata(Code::Unavailable, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::Timeout => {
                status_with_metadata(Code::DeadlineExceeded, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::Permission => {
                status_with_metadata(Code::PermissionDenied, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::Busy => {
                status_with_metadata(Code::Unavailable, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::NotFound => {
                status_with_metadata(Code::NotFound, err.to_string(), "driver", Some(err))
            }
            common::error::DriverErrorKind::Safety => status_with_metadata(
                Code::FailedPrecondition,
                err.to_string(),
                "driver",
                Some(err),
            ),
            common::error::DriverErrorKind::Shutdown | common::error::DriverErrorKind::Unknown => {
                status_with_metadata(Code::Internal, err.to_string(), "driver", Some(err))
            }
        },
        DaqError::SerialPortNotConnected => {
            Status::new(Code::Unavailable, "Serial port not connected")
        }
        DaqError::ModuleBusyDuringOperation => {
            Status::new(Code::Unavailable, "Module busy during operation")
        }

        // Serial protocol errors
        DaqError::SerialUnexpectedEof => {
            Status::new(Code::Aborted, "Serial communication: unexpected EOF")
        }
        DaqError::SerialFeatureDisabled => {
            Status::new(Code::Unimplemented, "Serial feature is disabled")
        }

        // Resource limit errors → ResourceExhausted
        DaqError::FrameDimensionsTooLarge {
            width,
            height,
            max_dimension,
        } => Status::new(
            Code::ResourceExhausted,
            format!(
                "Frame dimensions {}x{} exceed maximum {}",
                width, height, max_dimension
            ),
        ),
        DaqError::FrameTooLarge { bytes, max_bytes } => Status::new(
            Code::ResourceExhausted,
            format!("Frame size {} bytes exceeds maximum {}", bytes, max_bytes),
        ),
        DaqError::ResponseTooLarge { bytes, max_bytes } => Status::new(
            Code::ResourceExhausted,
            format!(
                "Response size {} bytes exceeds maximum {}",
                bytes, max_bytes
            ),
        ),
        DaqError::ScriptTooLarge { bytes, max_bytes } => Status::new(
            Code::ResourceExhausted,
            format!("Script size {} bytes exceeds maximum {}", bytes, max_bytes),
        ),
        DaqError::SizeOverflow { context } => Status::new(
            Code::ResourceExhausted,
            format!("Size overflow in {}", context),
        ),

        // Module state errors → FailedPrecondition or Unimplemented
        DaqError::ModuleOperationNotSupported(op) => Status::new(
            Code::Unimplemented,
            format!("Operation not supported: {}", op),
        ),
        DaqError::CameraNotAssigned => {
            Status::new(Code::FailedPrecondition, "Camera not assigned to module")
        }

        // Feature availability → Unimplemented
        DaqError::FeatureNotEnabled(feature) => Status::new(
            Code::Unimplemented,
            format!("Feature not enabled: {}", feature),
        ),
        DaqError::FeatureIncomplete(feature, reason) => Status::new(
            Code::Unimplemented,
            format!("Feature '{}' incomplete: {}", feature, reason),
        ),

        // Shutdown errors → Internal (aggregated failures)
        DaqError::ShutdownFailed(errors) => {
            let messages: Vec<String> = errors.into_iter().map(|e| e.to_string()).collect();
            Status::new(
                Code::Internal,
                format!("Shutdown failed: {}", messages.join("; ")),
            )
        }

        // Parameter errors
        DaqError::ParameterNoSubscribers => Status::new(
            Code::FailedPrecondition,
            "No subscribers for parameter update",
        ),
        DaqError::ParameterReadOnly => {
            Status::new(Code::PermissionDenied, "Parameter is read-only")
        }
        DaqError::ParameterInvalidChoice => {
            Status::new(Code::InvalidArgument, "Invalid parameter choice")
        }
        DaqError::ParameterNoHardwareReader => Status::new(
            Code::FailedPrecondition,
            "Parameter has no hardware reader configured",
        ),

        // I/O errors → Internal
        // These are server-side failures that shouldn't happen in normal operation
        DaqError::Io(e) => Status::new(Code::Internal, format!("I/O error: {}", e)),
        DaqError::Tokio(e) => Status::new(Code::Internal, format!("Tokio I/O error: {}", e)),

        // Processing errors → Internal
        DaqError::Processing(msg) => {
            Status::new(Code::Internal, format!("Processing error: {}", msg))
        }

        // Storage errors
        DaqError::Storage(e) => match &e.kind {
            common::error::StorageErrorKind::Configuration => Status::new(
                Code::InvalidArgument,
                format!("Storage configuration error: {}", e.message),
            ),
            common::error::StorageErrorKind::Io => {
                Status::new(Code::Internal, format!("Storage I/O error: {}", e.message))
            }
            _ => Status::new(Code::Internal, format!("Storage error: {}", e.message)),
        },

        // Feature-specific errors
        #[cfg(feature = "storage_hdf5")]
        DaqError::Hdf5(e) => Status::new(Code::Internal, format!("HDF5 error: {}", e)),
        #[cfg(feature = "storage_arrow")]
        DaqError::Arrow(e) => Status::new(Code::Internal, format!("Arrow error: {}", e)),

        DaqError::Serde(e) => Status::new(Code::Internal, format!("Serialization error: {}", e)),
        DaqError::TaskJoin(e) => Status::new(Code::Internal, format!("Task join error: {}", e)),
    }
}

/// Extension trait for converting Result<T, DaqError> to Result<T, Status>
pub trait DaqResultExt<T> {
    /// Convert a DaqError result to a tonic Status result
    #[allow(clippy::result_large_err)] // tonic::Status (176 bytes) is the standard gRPC error type
    fn map_daq_err(self) -> Result<T, Status>;
}

impl<T> DaqResultExt<T> for Result<T, DaqError> {
    fn map_daq_err(self) -> Result<T, Status> {
        self.map_err(map_daq_error_to_status)
    }
}
