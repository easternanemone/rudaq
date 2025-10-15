//! Core traits and data types for the DAQ application.
use crate::config::Settings;
use crate::error::DaqError;
use crate::metadata::Metadata;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// A single data point captured from an instrument.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub channel: String,
    pub value: f64,
    pub unit: String,
    /// Optional metadata for this specific data point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A handle to a running instrument task.
pub struct InstrumentHandle {
    pub task: JoinHandle<anyhow::Result<()>>,
}

/// Trait for any scientific instrument.
///
/// This trait defines the common interface for all instruments, allowing them
/// to be managed and controlled in a generic way.
#[async_trait]
pub trait Instrument: Send + Sync {
    /// Returns the name of the instrument.
    fn name(&self) -> String;

    /// Connects to the instrument and prepares it for data acquisition.
    async fn connect(&mut self, settings: &Arc<Settings>) -> Result<(), DaqError>;

    /// Disconnects from the instrument.
    async fn disconnect(&mut self) -> Result<(), DaqError>;

    /// Returns a stream of `DataPoint`s from the instrument.
    async fn data_stream(&mut self) -> Result<broadcast::Receiver<DataPoint>, DaqError>;
}

/// Trait for a data processor.
///
/// Data processors can be chained to form a processing pipeline.
pub trait DataProcessor: Send + Sync {
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint>;
}

/// Trait for a data storage writer.
#[async_trait]
pub trait StorageWriter: Send + Sync {
    /// Initializes the storage (e.g., creates a file, opens a database connection).
    async fn init(&mut self, settings: &Arc<Settings>) -> Result<(), DaqError>;

    /// Sets the experiment-level metadata for this storage session.
    /// This should be called once after `init` and before the first `write`.
    async fn set_metadata(&mut self, metadata: &Metadata) -> Result<(), DaqError>;

    /// Writes a batch of data points to the storage.
    async fn write(&mut self, data: &[DataPoint]) -> Result<(), DaqError>;

    /// Finalizes the storage (e.g., closes the file).
    async fn shutdown(&mut self) -> Result<(), DaqError>;
}
