//! Error types for configuration parsing and validation.

use thiserror::Error;

/// Errors that can occur during configuration parsing and validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("command '{command}' references unknown response '{name}'")]
    UnknownResponse { command: String, name: String },

    #[error("invalid regex pattern '{pattern}': {source}")]
    InvalidRegex {
        pattern: String,
        source: regex::Error,
    },

    #[error("invalid template '{name}': {reason}")]
    InvalidTemplate { name: String, reason: String },

    #[error("invalid formula '{formula}': {reason}")]
    InvalidFormula { formula: String, reason: String },

    #[error("invalid format string '{format}': {reason}")]
    InvalidFormat { format: String, reason: String },

    #[error("invalid baud rate: {0} (must be 300..=921600)")]
    InvalidBaudRate(u32),

    #[error("invalid timeout: {0}ms (must be 1..=60000)")]
    InvalidTimeout(u32),

    #[error("capability '{capability}' references missing {ref_type} '{ref_name}'")]
    MissingCapabilityMethod {
        capability: String,
        ref_type: String,
        ref_name: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("missing required field: '{0}'")]
    MissingField(String),

    #[error("unsupported schema version: {found} (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },

    #[error("{0}")]
    Other(String),
}

impl ConfigError {
    /// Create a new `Other` error with a message.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        let err = ConfigError::UnknownResponse {
            command: "get_pos".into(),
            name: "position_data".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("position_data"));
        assert!(msg.contains("get_pos"));

        let err = ConfigError::InvalidBaudRate(0);
        assert!(err.to_string().contains('0'));

        let err = ConfigError::UnsupportedSchemaVersion {
            found: 1,
            expected: 3,
        };
        assert!(err.to_string().contains('1'));
        assert!(err.to_string().contains('3'));
    }
}
