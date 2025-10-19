
// src/measurement/power.rs

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait PowerMeasure {
    async fn read_power(&mut self) -> Result<f64>;
}
