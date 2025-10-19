//! A basic skeleton for an SCPI-based instrument.
use crate::{
    config::Settings,
    core::{DataPoint, Instrument},
    measurement::InstrumentMeasurement,
};
use anyhow::Result;
use async_trait::async_trait;
use log::info;
use std::sync::Arc;


pub struct ScpiInstrument {
    id: String,
    measurement: Option<InstrumentMeasurement>,
}

impl Default for ScpiInstrument {
    fn default() -> Self {
        Self::new()
    }
}

impl ScpiInstrument {
    pub fn new() -> Self {
        Self {
            id: String::new(),
            measurement: None,
        }
    }
}

#[async_trait]
impl Instrument for ScpiInstrument {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        "SCPI Instrument".to_string()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to SCPI Instrument {}...", id);
        // TODO: Implement connection logic (e.g., open serial port)
        // let config = settings.instruments.get("scpi_keithley").unwrap();
        // let port = config.get("port").unwrap().as_str().unwrap();
        // let baud_rate = config.get("baud_rate").unwrap().as_integer().unwrap() as u32;
        // let port = serialport::new(port, baud_rate).open()...
        info!("SCPI connection is a placeholder.");
        self.id = id.to_string();
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from SCPI Instrument.");
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement
            .as_ref()
            .expect("SCPI instrument measurement not initialised")
    }
}
