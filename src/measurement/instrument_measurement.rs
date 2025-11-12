//! Shared measurement type for V1 instruments during Phase 2 migration.
//!
//! Provides a legacy `Measure` implementation backed by `DataDistributor`
//! so existing instrument drivers and tests can continue to operate while
//! the actor-based runtime evolves. New code should prefer V2/V3 measurement
//! pathways that emit `daq_core::Measurement` directly.

use crate::core::DataPoint;
use crate::measurement::{DataDistributor, Measure};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;
use daq_core::timestamp::Timestamp;

/// Compatibility measurement wrapper for legacy instruments.
#[derive(Clone)]
pub struct InstrumentMeasurement {
    distributor: Arc<DataDistributor<Arc<DataPoint>>>,
    id: String,
}

impl InstrumentMeasurement {
    /// Create a new measurement broadcaster with the provided channel capacity.
    pub fn new(capacity: usize, id: String) -> Self {
        Self {
            distributor: Arc::new(DataDistributor::new(capacity)),
            id,
        }
    }

    /// Broadcast a data point to all subscribers.
    pub async fn broadcast(&self, data: DataPoint) -> Result<()> {
        self.distributor.broadcast(Arc::new(data)).await
    }
}

#[async_trait]
impl Measure for InstrumentMeasurement {
    type Data = DataPoint;

    async fn measure(&mut self) -> Result<DataPoint> {
        // Minimal placeholder for APIs that still call `measure()` directly.
        // Primary data path should use `data_stream()`.
        Ok(DataPoint {
            timestamp: Timestamp::now_system(),
            instrument_id: self.id.clone(),
            channel: "placeholder".to_string(),
            value: 0.0,
            unit: "".to_string(),
            metadata: None,
        })
    }

    async fn data_stream(&self) -> Result<mpsc::Receiver<Arc<DataPoint>>> {
        Ok(self.distributor.subscribe("instrument_measurement").await)
    }
}
