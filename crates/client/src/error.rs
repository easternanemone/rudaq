//! Client error types.
//!
//! [`ClientError`] wraps `tonic::Status` and provides structured accessors
//! for the custom metadata headers set by the server's `error_mapping.rs`.
//!
//! This closes the error round-trip described in `common_traits::error`:
//!
//!   `DaqError` -> `tonic::Status` (with metadata) -> `ClientError` (with structured extraction)
//!
//! # Extracting structured error information
//!
//! When the server maps a `DaqError` to a gRPC `Status`, it attaches custom
//! metadata headers.  The [`ClientError`] methods [`daq_error_kind()`],
//! [`driver_type()`], and [`driver_kind()`] extract these headers:
//!
//! ```rust,ignore
//! use client::error::ClientError;
//!
//! match do_rpc_call().await {
//!     Err(ClientError::RpcStatus(ref status)) => {
//!         let err: &ClientError = &ClientError::from(status.clone());
//!         if let Some("driver") = err.daq_error_kind() {
//!             println!("Driver {} failed: {}", err.driver_type().unwrap_or("?"), status.message());
//!         }
//!     }
//!     _ => {}
//! }
//! ```

use thiserror::Error;

/// Well-known gRPC metadata header keys set by the server's error mapper.
/// These match the constants in `server::grpc::error_mapping`.
const ERROR_KIND_HEADER: &str = "x-daq-error-kind";
const DRIVER_TYPE_HEADER: &str = "x-daq-driver-type";
const DRIVER_KIND_HEADER: &str = "x-daq-driver-kind";

/// Result type alias using `ClientError`.
pub type Result<T> = std::result::Result<T, ClientError>;

/// Errors that can occur when using the DAQ client.
#[derive(Error, Debug)]
pub enum ClientError {
    /// Invalid URL format.
    #[error("Invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),

    /// gRPC transport error (connection failed, TLS error, etc.).
    /// Only available on native platforms (WASM uses browser fetch).
    #[cfg(not(target_arch = "wasm32"))]
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// gRPC status error (server returned an error).
    #[error("gRPC status error: {0}")]
    RpcStatus(#[from] tonic::Status),

    /// Connection failed with a descriptive message.
    #[error("Connection failed: {0}")]
    Connection(String),

    /// Timeout waiting for operation.
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Device not found.
    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

impl ClientError {
    /// Extract the DAQ error kind from gRPC metadata, if available.
    ///
    /// Returns the value of the `x-daq-error-kind` header set by the server
    /// when mapping a `DaqError` to a gRPC `Status`.  Typical values include
    /// `"driver"`, `"instrument"`, `"config"`, `"storage"`, `"processing"`, etc.
    ///
    /// Returns `None` if the error is not an `RpcStatus` variant or the
    /// metadata header is missing.
    pub fn daq_error_kind(&self) -> Option<&str> {
        self.rpc_metadata_str(ERROR_KIND_HEADER)
    }

    /// Extract the driver type from gRPC metadata, if available.
    ///
    /// Returns the value of the `x-daq-driver-type` header (e.g., `"mock_camera"`,
    /// `"pvcam"`, `"comedi"`).  Only present when the server error was a `DriverError`.
    ///
    /// Returns `None` if the error is not an `RpcStatus` variant or the
    /// metadata header is missing.
    pub fn driver_type(&self) -> Option<&str> {
        self.rpc_metadata_str(DRIVER_TYPE_HEADER)
    }

    /// Extract the driver error kind from gRPC metadata, if available.
    ///
    /// Returns the value of the `x-daq-driver-kind` header (e.g., `"communication"`,
    /// `"timeout"`, `"initialization"`).  Only present when the server error was a
    /// `DriverError`.
    ///
    /// Returns `None` if the error is not an `RpcStatus` variant or the
    /// metadata header is missing.
    pub fn driver_kind(&self) -> Option<&str> {
        self.rpc_metadata_str(DRIVER_KIND_HEADER)
    }

    /// Check if this error indicates a specific DAQ error category.
    ///
    /// Convenience method equivalent to `self.daq_error_kind() == Some(kind)`.
    pub fn is_daq_error_kind(&self, kind: &str) -> bool {
        self.daq_error_kind() == Some(kind)
    }

    /// Helper: extract a metadata string value from the inner `tonic::Status`.
    fn rpc_metadata_str(&self, key: &str) -> Option<&str> {
        match self {
            ClientError::RpcStatus(status) => {
                status.metadata().get(key).and_then(|v| v.to_str().ok())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_error_display() {
        let err = ClientError::Connection("test failure".to_string());
        assert!(err.to_string().contains("test failure"));
        assert!(err.to_string().contains("Connection failed"));
    }

    #[test]
    fn test_timeout_error_display() {
        let err = ClientError::Timeout("waiting for response".to_string());
        assert!(err.to_string().contains("waiting for response"));
        assert!(err.to_string().contains("Operation timed out"));
    }

    #[test]
    fn test_device_not_found_error_display() {
        let err = ClientError::DeviceNotFound("camera0".to_string());
        assert!(err.to_string().contains("camera0"));
        assert!(err.to_string().contains("Device not found"));
    }

    #[test]
    fn test_invalid_config_error_display() {
        let err = ClientError::InvalidConfig("bad port".to_string());
        assert!(err.to_string().contains("bad port"));
        assert!(err.to_string().contains("Invalid configuration"));
    }

    #[test]
    fn test_error_from_url_parse() {
        let url_err: std::result::Result<url::Url, _> = "not a url".parse();
        let client_err: ClientError = url_err.unwrap_err().into();
        assert!(matches!(client_err, ClientError::UrlParse(_)));
        assert!(client_err.to_string().contains("Invalid URL"));
    }

    #[test]
    fn test_error_from_tonic_status() {
        let status = tonic::Status::not_found("device missing");
        let client_err: ClientError = status.into();
        assert!(matches!(client_err, ClientError::RpcStatus(_)));
        assert!(client_err.to_string().contains("gRPC status error"));
    }

    #[test]
    fn test_result_type_alias() {
        // Verify Result<T> is usable
        let ok_result: Result<i32> = Ok(42);
        assert!(ok_result.is_ok());

        let err_result: Result<i32> = Err(ClientError::Connection("test".to_string()));
        assert!(err_result.is_err());
    }
}
