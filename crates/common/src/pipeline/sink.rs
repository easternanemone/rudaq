//! A sink that consumes measurements.
//!
//! Sinks are the endpoints of the pipeline (e.g., Storage, Network).

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[async_trait]
pub trait MeasurementSink: Send + Sync {
    type Input: Send + 'static;
    type Error: std::fmt::Debug + std::fmt::Display + Send + Sync + 'static;

    /// Register the input channel.
    ///
    /// The sink should spawn a task to consume from `rx`.
    /// Returns a JoinHandle to monitor the sink task.
    fn register_input(
        &mut self,
        rx: mpsc::Receiver<Self::Input>,
    ) -> Result<JoinHandle<()>, Self::Error>;
}
