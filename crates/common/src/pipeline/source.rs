//! A source of data measurements.
//!
//! Sources produce data and push it into a provided channel.

use tokio::sync::mpsc;

pub trait MeasurementSource: Send + Sync {
    /// The type of data produced (usually `common::core::Measurement`)
    type Output: Send + Clone + 'static;
    type Error: std::fmt::Debug + std::fmt::Display + Send + Sync + 'static;

    /// Register the output channel for the reliable path.
    ///
    /// This connects the source to the rest of the pipeline.
    /// The implementation should spawn a task to produce data into `tx`.
    /// Uses `&self` to allow usage with `Arc<dyn MeasurementSource>`.
    async fn register_output(&self, tx: mpsc::Sender<Self::Output>) -> Result<(), Self::Error>;
}
