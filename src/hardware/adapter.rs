
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

/// An error that can occur when interacting with a hardware adapter.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Not connected")]
    NotConnected,

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}


#[async_trait]
pub trait HardwareAdapter: Send + Sync {
    /// Get the name of the adapter.
    fn name(&self) -> &str;

    /// Get the default configuration for the adapter.
    fn default_config(&self) -> Value;

    /// Validate a configuration for the adapter.
    fn validate_config(&self, config: &Value) -> Result<()>;

    /// Connect to the hardware.
    async fn connect(&mut self, config: &Value) -> Result<()>;

    /// Disconnect from the hardware.
    async fn disconnect(&mut self) -> Result<()>;

    /// Send a command to the hardware.
    async fn send(&mut self, command: &str) -> Result<()>;

    /// Query the hardware.
    async fn query(&mut self, query: &str) -> Result<String>;
}
