//! Custom error types for the application.
use thiserror::Error;

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

    #[error("Data processing error: {0}")]
    Processing(String),

    #[error("Feature '{0}' is not enabled. Please build with --features {0}")]
    FeatureNotEnabled(String),
}
