
// src/measurement/mod.rs

use anyhow::Result;
use async_trait::async_trait;

use crate::core::DataPoint;

#[async_trait]
pub trait Measure: Send + Sync {
    type Data: Send + Clone;

    async fn measure(&mut self) -> Result<Self::Data>;
    async fn data_stream(&self) -> Result<tokio::sync::broadcast::Receiver<Self::Data>>;
}

pub mod power;
pub mod datapoint;
pub mod instrument_measurement;

pub use instrument_measurement::InstrumentMeasurement;
