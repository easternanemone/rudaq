//! Pipeline traits for data acquisition
//!
//! Defines the core abstractions for the "Mullet Strategy" pipeline:
//! - **Source**: Produces measurements
//! - **Sink**: Consumes measurements
//! - **Processor**: Transforms measurements
//! - **Tee**: Splits stream into Reliable (mpsc) and Lossy (broadcast) paths
//!
//! # Architecture
//!
//! ```text
//! [Source] --> [Tee] --(mpsc)--> [Storage Sink] (Reliable, Backpressure)
//!                |
//!                --(broadcast)--> [Network Sink] (Lossy, Droppable)
//! ```

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// A source of data measurements.
///
/// Sources produce data and push it into a provided channel.
#[async_trait]
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

/// A sink that consumes measurements.
///
/// Sinks are the endpoints of the pipeline (e.g., Storage, Network).
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

/// A processor that transforms data.
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

/// A Tee processor that splits a stream into a Reliable path and a Lossy path.
///
/// - **Reliable Path**: Uses `mpsc::Sender`. Supports backpressure. If full, the source slows down.
/// - **Lossy Path**: Uses `broadcast::Sender`. Drops messages if receivers lag.
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
                if let Some(ref tx) = reliable_tx {
                    if tx.send(item.clone()).await.is_err() {
                        // Reliable receiver closed (e.g., storage full/error)
                        // We should probably stop the pipeline or log error
                        tracing::error!("Reliable pipeline path closed unexpectedly");
                        break;
                    }
                }

                // 2. Send to Lossy Path (Fire and forget)
                // We ignore errors (no receivers) and don't await capacity
                let _ = lossy_tx.send(item);
            }
        });

        Ok(handle)
    }
}
