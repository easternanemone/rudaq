
// src/measurement/mod.rs

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::core::DataPoint;

/// Fan-out data distributor for efficient multi-consumer broadcasting with backpressure.
///
/// Replaces tokio::sync::broadcast to prevent silent data loss from lagging receivers.
/// Each subscriber gets a dedicated mpsc channel, providing isolation and true backpressure.
///
/// Uses interior mutability (Mutex) to avoid requiring Arc<Mutex<DataDistributor>> wrapper,
/// following actor model principles by minimizing lock scope.
pub struct DataDistributor<T: Clone> {
    subscribers: Mutex<Vec<tokio::sync::mpsc::Sender<T>>>,
    capacity: usize,
}

impl<T: Clone> DataDistributor<T> {
    /// Creates a new DataDistributor with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            capacity,
        }
    }

    /// Subscribe to the data stream, returns a new mpsc::Receiver
    pub async fn subscribe(&self) -> tokio::sync::mpsc::Receiver<T> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.capacity);
        let mut subscribers = self.subscribers.lock().await;
        subscribers.push(tx);
        rx
    }

    /// Broadcast data to all subscribers with automatic dead subscriber cleanup
    pub async fn broadcast(&self, data: T) -> Result<()> {
        let mut subscribers = self.subscribers.lock().await;
        let mut dead_indices = Vec::new();

        for (i, sender) in subscribers.iter().enumerate() {
            if sender.send(data.clone()).await.is_err() {
                dead_indices.push(i);
            }
        }

        // Remove dead subscribers in reverse order to maintain indices
        for i in dead_indices.iter().rev() {
            subscribers.swap_remove(*i);
        }

        Ok(())
    }

    /// Returns the number of active subscribers
    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.lock().await.len()
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
