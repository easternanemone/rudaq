//! Spectra-Physics MaiTai tunable Ti:Sapphire laser driver
//!
//! This module provides an `Instrument` implementation for the MaiTai laser
//! using RS-232 serial communication.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.maitai_laser]
//! type = "maitai"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! wavelength = 800.0  # nm
//! polling_rate_hz = 1.0
//! ```

use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
    measurement::InstrumentMeasurement,
};
#[cfg(feature = "instrument_serial")]
use crate::adapters::serial::SerialAdapter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;



/// MaiTai laser instrument implementation
#[derive(Clone)]
pub struct MaiTai {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    // Removed sender field - using InstrumentMeasurement with DataDistributor
    measurement: Option<InstrumentMeasurement>,
}

impl MaiTai {
    /// Creates a new MaiTai instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            adapter: None,
            // No sender field
            measurement: None,
        }
    }

    #[cfg(feature = "instrument_serial")]
    async fn send_command_async(&self, command: &str) -> Result<String> {
        use super::serial_helper;
        use std::time::Duration;

        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to MaiTai '{}'", self.id))?
            .clone();

        serial_helper::send_command_async(
            adapter,
            &self.id,
            command,
            "\r",
            Duration::from_secs(2),
            b'\r',
        )
        .await
    }

    #[cfg(feature = "instrument_serial")]
    async fn query_value(&self, command: &str) -> Result<f64> {
        let response = self.send_command_async(command).await?;
        // Remove command echo if present
        let value_str = response.split(':').next_back().unwrap_or(&response);
        value_str
            .trim()
            .parse::<f64>()
            .with_context(|| format!("Failed to parse response '{}' as float", response))
    }
}

#[async_trait]
impl Instrument for MaiTai {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to MaiTai laser: {}", id);
        self.id = id.to_string();

        let instrument_config = settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Configuration for '{}' not found", id))?;

        let port_name = instrument_config
            .get("port")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("'port' not found in config for '{}'", self.id))?;

        let baud_rate = instrument_config
            .get("baud_rate")
            .and_then(|v| v.as_integer())
            .unwrap_or(9600) as u32;

        // Open serial port
        let port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(500))
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for MaiTai", port_name))?;

        self.adapter = Some(SerialAdapter::new(port));

        // Verify connection with identity query
        let id_response = self.send_command_async("*IDN?").await?;
        info!("MaiTai identity: {}", id_response);

        // Set wavelength if specified
        if let Some(wavelength) = instrument_config
            .get("wavelength")
            .and_then(|v| v.as_float())
        {
            self.send_command_async(&format!("WAVELENGTH:{}", wavelength))
                .await?;
            info!("Set wavelength to {} nm", wavelength);
        }

        // Create broadcast channel with configured capacity
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        // No sender field
        self.measurement = Some(measurement.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(1.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Query wavelength
                if let Ok(wavelength) = instrument.query_value("WAVELENGTH?").await {
                    let dp = DataPoint {
                        timestamp,
                        instrument_id: instrument.id.clone(),
                        channel: "wavelength".to_string(),
                        value: wavelength,
                        unit: "nm".to_string(),
                        metadata: None,
                    };
                    if measurement.broadcast(dp).await.is_err() {
                        warn!("No active receivers for MaiTai data");
                        break;
                    }
                }

                // Query power
                if let Ok(power) = instrument.query_value("POWER?").await {
                    let dp = DataPoint {
                        timestamp,
                        instrument_id: instrument.id.clone(),
                        channel: "power".to_string(),
                        value: power,
                        unit: "W".to_string(),
                        metadata: None,
                    };
                    let _ = measurement.broadcast(dp).await;
                }

                // Query shutter state
                if let Ok(shutter) = instrument.query_value("SHUTTER?").await {
                    let dp = DataPoint {
                        timestamp,
                        instrument_id: instrument.id.clone(),
                        channel: "shutter".to_string(),
                        value: shutter,
                        unit: "state".to_string(),
                        metadata: None,
                    };
                    let _ = measurement.broadcast(dp).await;
                }
            }
        });

        info!("MaiTai laser '{}' connected successfully", self.id);
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn connect(&mut self, id: &str, _settings: &Arc<Settings>) -> Result<()> {
        self.id = id.to_string();
        Err(anyhow!(
            "Serial support not enabled. Rebuild with --features instrument_serial"
        ))
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from MaiTai laser: {}", self.id);
        #[cfg(feature = "instrument_serial")]
        {
            self.adapter = None;
        }
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement
            .as_ref()
            .expect("MaiTai measurement not initialised")
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => match key.as_str() {
                "wavelength" => {
                    let wavelength: f64 = value
                        .parse()
                        .with_context(|| format!("Invalid wavelength value: {}", value))?;
                    self.send_command_async(&format!("WAVELENGTH:{}", wavelength))
                        .await?;
                    info!("Set MaiTai wavelength to {} nm", wavelength);
                }
                "shutter" => {
                    let cmd = match value.as_str() {
                        "open" => "SHUTTER:1",
                        "close" => "SHUTTER:0",
                        _ => return Err(anyhow!("Invalid shutter value: {}", value)),
                    };
                    self.send_command_async(cmd).await?;
                    info!("MaiTai shutter: {}", value);
                }
                "laser" => {
                    let cmd = match value.as_str() {
                        "on" => "ON",
                        "off" => "OFF",
                        _ => return Err(anyhow!("Invalid laser value: {}", value)),
                    };
                    self.send_command_async(cmd).await?;
                    info!("MaiTai laser: {}", value);
                }
                _ => {
                    warn!("Unknown parameter '{}' for MaiTai", key);
                }
            },
            _ => {
                warn!("Unsupported command type for MaiTai");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn handle_command(&mut self, _command: InstrumentCommand) -> Result<()> {
        Err(anyhow!("Serial support not enabled"))
    }
}
