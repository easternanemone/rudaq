//! A processor that transforms data.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[async_trait]
pub trait MeasurementProcessor: Send + Sync {
    type Input: Send + 'static;
    type Output: Send + 'static;
    type Error: std::fmt::Debug + std::fmt::Display + Send + Sync + 'static;

    /// Connect input and output.
    fn register(
        &mut self,
        rx: mpsc::Receiver<Self::Input>,
        tx: mpsc::Sender<Self::Output>,
    ) -> Result<JoinHandle<()>, Self::Error>;
}
