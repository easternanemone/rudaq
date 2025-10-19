//! Newport 1830-C Optical Power Meter driver
//!
//! This module provides an `Instrument` implementation for the Newport 1830-C
//! optical power meter using RS-232 serial communication.
//!
//! ## Configuration
//!
//! The Newport 1830-C is configured in the `config/default.toml` file:
//!
//! ```toml
//! [instruments.power_meter_1]
//! type = "newport_1830c"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! wavelength = 1550.0  # nm
//! range = 0  # 0=autorange
//! units = 0  # 0=Watts, 1=dBm, 2=dB, 3=REL
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



/// Newport 1830-C instrument implementation
#[derive(Clone)]
pub struct Newport1830C {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    // Removed sender field - using InstrumentMeasurement with DataDistributor
    measurement: Option<InstrumentMeasurement>,
}

impl Newport1830C {
    /// Creates a new Newport1830C instrument
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
            .ok_or_else(|| anyhow!("Not connected to Newport 1830-C '{}'", self.id))?
            .clone();

        serial_helper::send_command_async(
            adapter,
            &self.id,
            command,
            "\r\n",
            Duration::from_secs(1),
            b'\n',
        )
        .await
    }
}

#[async_trait]
impl Instrument for Newport1830C {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Newport 1830-C: {}", id);
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
            .timeout(std::time::Duration::from_millis(100))
            .open()
            .with_context(|| {
                format!(
                    "Failed to open serial port '{}' for Newport 1830-C",
                    port_name
                )
            })?;

        self.adapter = Some(SerialAdapter::new(port));

        // Configure wavelength if specified
        if let Some(wavelength) = instrument_config
            .get("wavelength")
            .and_then(|v| v.as_float())
        {
            self.send_command_async(&format!("PM:Lambda {}", wavelength))
                .await?;
            info!("Set wavelength to {} nm", wavelength);
        }

        // Configure range if specified
        if let Some(range) = instrument_config.get("range").and_then(|v| v.as_integer()) {
            self.send_command_async(&format!("PM:Range {}", range))
                .await?;
            info!("Set range to {}", range);
        }

        // Configure units if specified
        if let Some(units) = instrument_config.get("units").and_then(|v| v.as_integer()) {
            self.send_command_async(&format!("PM:Units {}", units))
                .await?;
            let unit_str = match units {
                0 => "Watts",
                1 => "dBm",
                2 => "dB",
                3 => "REL",
                _ => "Unknown",
            };
            info!("Set units to {}", unit_str);
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
            .unwrap_or(10.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            loop {
                interval.tick().await;

                // Query power measurement
                match instrument.send_command_async("PM:Power?").await {
                    Ok(response) => {
                        if let Ok(value) = response.parse::<f64>() {
                            let dp = DataPoint {
                                timestamp: chrono::Utc::now(),
                                instrument_id: instrument.id.clone(),
                                channel: "power".to_string(),
                                value,
                                unit: "W".to_string(),
                                metadata: None,
                            };

                            if measurement.broadcast(dp).await.is_err() {
                                warn!("No active receivers for Newport 1830-C data");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read from Newport 1830-C: {}", e);
                    }
                }
            }
        });

        info!("Newport 1830-C '{}' connected successfully", self.id);
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
        info!("Disconnecting from Newport 1830-C: {}", self.id);
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
            .expect("Newport 1830-C measurement not initialised")
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => match key.as_str() {
                "wavelength" => {
                    let wavelength: f64 = value
                        .parse()
                        .with_context(|| format!("Invalid wavelength value: {}", value))?;
                    self.send_command_async(&format!("PM:Lambda {}", wavelength))
                        .await?;
                    info!("Set Newport 1830-C wavelength to {} nm", wavelength);
                }
                "range" => {
                    let range: i32 = value
                        .parse()
                        .with_context(|| format!("Invalid range value: {}", value))?;
                    self.send_command_async(&format!("PM:Range {}", range))
                        .await?;
                    info!("Set Newport 1830-C range to {}", range);
                }
                "units" => {
                    let units: i32 = value
                        .parse()
                        .with_context(|| format!("Invalid units value: {}", value))?;
                    self.send_command_async(&format!("PM:Units {}", units))
                        .await?;
                    info!("Set Newport 1830-C units to {}", units);
                }
                _ => {
                    warn!("Unknown parameter '{}' for Newport 1830-C", key);
                }
            },
            InstrumentCommand::Execute(cmd, _) => {
                if cmd == "zero" {
                    self.send_command_async("PM:DS:Clear").await?;
                    info!("Newport 1830-C zeroed");
                }
            }
            _ => {
                warn!("Unsupported command type for Newport 1830-C");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn handle_command(&mut self, _command: InstrumentCommand) -> Result<()> {
        Err(anyhow!("Serial support not enabled"))
    }
}
