//! Error types for NI-DAQmx driver

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NiDaqError {
    #[error("Python error: {0}")]
    Python(#[from] pyo3::PyErr),

    #[error("Task configuration error: {0}")]
    Configuration(String),

    #[error("Device not armed")]
    NotArmed,

    #[error("Task already running")]
    AlreadyRunning,

    #[error("Task not running")]
    NotRunning,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Hardware error: {0}")]
    Hardware(String),
}

pub type Result<T> = std::result::Result<T, NiDaqError>;
