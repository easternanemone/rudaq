//! Semantic mapping from DaqError to gRPC Status codes (bd-cxvg).
//!
//! This module provides a centralized, well-documented mapping from internal
//! `DaqError` variants to appropriate gRPC `Status` codes. The mappings follow
//! gRPC best practices and semantic guidelines.
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

use daq_core::error::DaqError;
use tonic::{Code, Status};

/// Map a DaqError to an appropriate gRPC Status.
///
/// This function provides semantic mapping from internal error types to
/// gRPC status codes that clients can interpret meaningfully.
///
/// # Examples
///
/// ```
/// use daq_core::error::DaqError;
/// use daq_server::grpc::map_daq_error_to_status;
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
        DaqError::Configuration(msg) => {
            Status::new(Code::InvalidArgument, format!("Configuration error: {}", msg))
        }

        // Hardware/connection errors → Unavailable
        // Resource is temporarily unavailable, client may retry
        DaqError::Instrument(msg) => {
            Status::new(Code::Unavailable, format!("Instrument error: {}", msg))
        }
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
        DaqError::ModuleOperationNotSupported(op) => {
            Status::new(Code::Unimplemented, format!("Operation not supported: {}", op))
        }
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
    }
}

/// Extension trait for converting Result<T, DaqError> to Result<T, Status>
pub trait DaqResultExt<T> {
    /// Convert a DaqError result to a tonic Status result
    fn map_daq_err(self) -> Result<T, Status>;
}

impl<T> DaqResultExt<T> for Result<T, DaqError> {
    fn map_daq_err(self) -> Result<T, Status> {
        self.map_err(map_daq_error_to_status)
    }
}
