//! A Tee processor that splits a stream into a Reliable path and a Lossy path.
//!
//! - **Reliable Path**: Uses `mpsc::Sender`. Supports backpressure. If full, the source slows down.
//! - **Lossy Path**: Uses `broadcast::Sender`. Drops messages if receivers lag.

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::pipeline::sink::MeasurementSink;

pub struct Tee<T> {
    reliable_tx: Option<mpsc::Sender<T>>,
    lossy_tx: broadcast::Sender<T>,
}

impl<T> Tee<T> {
    /// Create a new Tee.
    ///
    /// # Arguments
    /// * `lossy_tx` - The broadcast channel for the lossy path (e.g., to gRPC server)
    pub fn new(lossy_tx: broadcast::Sender<T>) -> Self {
        Self {
            reliable_tx: None,
            lossy_tx,
        }
    }

    /// Connect the reliable output path.
    pub fn connect_reliable(&mut self, tx: mpsc::Sender<T>) {
        self.reliable_tx = Some(tx);
    }
}

#[async_trait]
impl<T> MeasurementSink for Tee<T>
where
    T: Send + Clone + 'static,
{
    type Input = T;
    type Error = anyhow::Error;

    fn register_input(&mut self, mut rx: mpsc::Receiver<T>) -> Result<JoinHandle<()>, Self::Error> {
        let reliable_tx = self.reliable_tx.clone();
        let lossy_tx = self.lossy_tx.clone();

        let handle = tokio::spawn(async move {
            while let Some(item) = rx.recv().await {
                // 1. Send to Reliable Path (Backpressure enforced here)
                // We await this send, which pushes backpressure upstream to the source
                if let Some(ref tx) = reliable_tx
                    && tx.send(item.clone()).await.is_err()
                {
                    // Reliable receiver closed (e.g., storage full/error)
                    // We should probably stop the pipeline or log error
                    tracing::error!("Reliable pipeline path closed unexpectedly");
                    break;
                }

                // 2. Send to Lossy Path (Fire and forget)
                // We ignore errors (no receivers) and don't await capacity
                let _ = lossy_tx.send(item);
            }
        });

        Ok(handle)
    }
}
