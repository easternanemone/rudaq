//! Shared measurement type for all V1 instruments

use crate::core::DataPoint;
use crate::measurement::{DataDistributor, Measure};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// A measurement type that all V1 instruments can use
/// This provides a unified Measure implementation backed by a DataDistributor
#[derive(Clone)]
pub struct InstrumentMeasurement {
    distributor: Arc<Mutex<DataDistributor<DataPoint>>>,
    id: String,
}

impl InstrumentMeasurement {
    /// Creates a new InstrumentMeasurement
    pub fn new(capacity: usize, id: String) -> Self {
        Self {
            distributor: Arc::new(Mutex::new(DataDistributor::new(capacity))),
            id,
        }
    }

    /// Broadcast a data point to all subscribers
    pub async fn broadcast(&self, data: DataPoint) -> Result<()> {
        let mut dist = self.distributor.lock().await;
        dist.broadcast(data).await
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

    async fn data_stream(&self) -> Result<mpsc::Receiver<DataPoint>> {
        let mut dist = self.distributor.lock().await;
        Ok(dist.subscribe())
    }
}
