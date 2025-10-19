
// src/measurement/mod.rs

use anyhow::Result;
use async_trait::async_trait;

use crate::core::DataPoint;

/// Fan-out data distributor for efficient multi-consumer broadcasting with backpressure.
///
/// Replaces tokio::sync::broadcast to prevent silent data loss from lagging receivers.
/// Each subscriber gets a dedicated mpsc channel, providing isolation and true backpressure.
pub struct DataDistributor<T: Clone> {
    subscribers: Vec<tokio::sync::mpsc::Sender<T>>,
    capacity: usize,
}

impl<T: Clone> DataDistributor<T> {
    /// Creates a new DataDistributor with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            subscribers: Vec::new(),
            capacity,
        }
    }

    /// Subscribe to the data stream, returns a new mpsc::Receiver
    pub fn subscribe(&mut self) -> tokio::sync::mpsc::Receiver<T> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.capacity);
        self.subscribers.push(tx);
        rx
    }

    /// Broadcast data to all subscribers with automatic dead subscriber cleanup
    pub async fn broadcast(&mut self, data: T) -> Result<()> {
        let mut dead_indices = Vec::new();

        for (i, sender) in self.subscribers.iter().enumerate() {
            if sender.send(data.clone()).await.is_err() {
                dead_indices.push(i);
            }
        }

        // Remove dead subscribers in reverse order to maintain indices
        for i in dead_indices.iter().rev() {
            self.subscribers.swap_remove(*i);
        }

        Ok(())
    }

    /// Returns the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

#[async_trait]
pub trait Measure: Send + Sync {
    type Data: Send + Sync + Clone;

    async fn measure(&mut self) -> Result<Self::Data>;
    async fn data_stream(&self) -> Result<tokio::sync::mpsc::Receiver<std::sync::Arc<Self::Data>>>;
}

pub mod power;
pub mod datapoint;
pub mod instrument_measurement;

pub use instrument_measurement::InstrumentMeasurement;
