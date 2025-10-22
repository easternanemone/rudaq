//! Shared measurement type for all V1 instruments

use crate::core::DataPoint;
use crate::measurement::{DataDistributor, Measure};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// A measurement type that all V1 instruments can use.
///
/// This provides a unified Measure implementation backed by a DataDistributor.
/// Uses Arc<DataDistributor> (without outer Mutex) since DataDistributor
/// implements interior mutability for thread-safe subscriber management.
#[derive(Clone)]
pub struct InstrumentMeasurement {
    distributor: Arc<DataDistributor<Arc<DataPoint>>>,
    id: String,
}

impl InstrumentMeasurement {
    /// Creates a new InstrumentMeasurement
    pub fn new(capacity: usize, id: String) -> Self {
        Self {
            distributor: Arc::new(DataDistributor::new(capacity)),
            id,
        }
    }

    /// Broadcast a data point to all subscribers.
    ///
    /// No longer requires locking at this level since DataDistributor
    /// implements interior mutability with minimal lock scope.
    pub async fn broadcast(&self, data: DataPoint) -> Result<()> {
        self.distributor.broadcast(Arc::new(data)).await
    }
}

#[async_trait]
impl Measure for InstrumentMeasurement {
    type Data = DataPoint;

    async fn measure(&mut self) -> Result<DataPoint> {
        // This method is not typically used for streaming instruments
        // The data flows through the DataDistributor instead
        let dp = DataPoint {
            timestamp: chrono::Utc::now(),
            instrument_id: self.id.clone(),
            channel: "placeholder".to_string(),
            value: 0.0,
            unit: "".to_string(),
            metadata: None,
        };
        Ok(dp)
    }

    async fn data_stream(&self) -> Result<mpsc::Receiver<Arc<DataPoint>>> {
        Ok(self.distributor.subscribe().await)
    }
}
