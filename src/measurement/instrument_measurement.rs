//! Shared measurement type for all V1 instruments

use crate::core::DataPoint;
use crate::measurement::Measure;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::broadcast;

/// A measurement type that all V1 instruments can use
/// This provides a unified Measure implementation backed by a broadcast channel
#[derive(Clone)]
pub struct InstrumentMeasurement {
    sender: broadcast::Sender<DataPoint>,
    id: String,
}

impl InstrumentMeasurement {
    /// Creates a new InstrumentMeasurement
    pub fn new(sender: broadcast::Sender<DataPoint>, id: String) -> Self {
        Self { sender, id }
    }
}

#[async_trait]
impl Measure for InstrumentMeasurement {
    type Data = DataPoint;

    async fn measure(&mut self) -> Result<DataPoint> {
        // This method is not typically used for streaming instruments
        // The data flows through the broadcast channel instead
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

    async fn data_stream(&self) -> Result<broadcast::Receiver<DataPoint>> {
        Ok(self.sender.subscribe())
    }
}
