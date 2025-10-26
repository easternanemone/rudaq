// src/measurement/mod.rs

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::core::DataPoint;

/// Fan-out data distributor for efficient multi-consumer broadcasting without backpressure.
///
/// Uses non-blocking try_send() to prevent slow subscribers from blocking fast ones.
/// Each subscriber gets a dedicated mpsc channel, providing isolation. Messages are dropped
/// if a subscriber's channel is full (logged as warning).
///
/// Uses interior mutability (Mutex) to avoid requiring Arc<Mutex<DataDistributor>> wrapper,
/// following actor model principles by minimizing lock scope.
pub struct DataDistributor<T: Clone> {
    subscribers: Mutex<Vec<(String, tokio::sync::mpsc::Sender<T>)>>,
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

    /// Subscribe to the data stream with a named identifier, returns a new mpsc::Receiver
    ///
    /// The name is used for observability - logs and metrics will identify subscribers
    /// by this name when messages are dropped or subscribers disconnect.
    pub async fn subscribe(&self, name: impl Into<String>) -> tokio::sync::mpsc::Receiver<T> {
        let (tx, rx) = tokio::sync::mpsc::channel(self.capacity);
        let mut subscribers = self.subscribers.lock().await;
        subscribers.push((name.into(), tx));
        rx
    }

    /// Broadcast data to all subscribers with automatic dead subscriber cleanup.
    ///
    /// Uses non-blocking try_send() to prevent slow subscribers from blocking fast ones.
    /// Messages are dropped if a subscriber's channel is full (logged as warning).
    /// Dead subscribers (closed channels) are automatically removed.
    pub async fn broadcast(&self, data: T) -> Result<()> {
        let mut subscribers = self.subscribers.lock().await;
        let mut disconnected_indices = Vec::new();

        for (i, (name, sender)) in subscribers.iter().enumerate() {
            match sender.try_send(data.clone()) {
                Ok(_) => {
                    // Success - instant send without blocking
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    log::warn!(
                        "Subscriber '{}' channel full (capacity: {}). Dropping measurement.",
                        name,
                        self.capacity
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    log::info!("Subscriber '{}' disconnected. Removing from subscriber list.", name);
                    disconnected_indices.push(i);
                }
            }
        }

        // Remove disconnected subscribers in reverse order to maintain indices
        for i in disconnected_indices.iter().rev() {
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

pub mod datapoint;
pub mod instrument_measurement;
pub mod power;

pub use instrument_measurement::InstrumentMeasurement;