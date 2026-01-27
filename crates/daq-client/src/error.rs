//! Client error types.

use thiserror::Error;

/// Result type alias using ClientError.
pub type Result<T> = std::result::Result<T, ClientError>;

/// Errors that can occur when using the DAQ client.
#[derive(Error, Debug)]
pub enum ClientError {
    /// Invalid URL format.
    #[error("Invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),

    /// gRPC transport error (connection failed, TLS error, etc.).
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
        assert_eq!(ok_result.unwrap(), 42);

        let err_result: Result<i32> = Err(ClientError::Connection("test".to_string()));
        assert!(err_result.is_err());
    }
}
