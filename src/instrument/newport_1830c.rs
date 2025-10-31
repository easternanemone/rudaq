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
//! port = "/dev/ttyS0"  # Native RS-232 port
//! baud_rate = 9600
//! attenuator = 0  # 0=off, 1=on
//! filter = 2      # 1=Slow, 2=Medium, 3=Fast
//! ```
//!
//! ## Important Notes
//!
//! - Newport 1830-C uses SIMPLE single-letter commands, NOT SCPI
//! - Does NOT support wavelength or units configuration via commands
//! - Does NOT require hardware flow control (unlike ESP300)
//! - Commands: D? (power), A0/A1 (attenuator), F1/F2/F3 (filter)
//! - Terminator: LF (\n) only

#[cfg(feature = "instrument_serial")]
use crate::adapters::serial::SerialAdapter;
use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
    instrument::capabilities::power_measurement_capability_id,
    measurement::InstrumentMeasurement,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::any::TypeId;
use std::sync::Arc;

/// Newport 1830-C instrument implementation
#[derive(Clone)]
pub struct Newport1830C {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    measurement: Option<InstrumentMeasurement>,
    // Track current parameter values for validation and state management
    current_attenuator: Option<i32>,
    current_filter: Option<i32>,
}

impl Newport1830C {
    /// Creates a new Newport1830C instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            adapter: None,
            measurement: None,
            current_attenuator: None,
            current_filter: None,
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
            "\n", // LF terminator only
            Duration::from_millis(500),
            b'\n',
        )
        .await
    }

    /// Send a configuration command without waiting for response
    /// Newport 1830-C doesn't respond to configuration commands like A0, A1, F1, F2, F3
    #[cfg(feature = "instrument_serial")]
    async fn send_config_command(&self, command: &str) -> Result<()> {
        use crate::adapters::Adapter;

        let mut adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to Newport 1830-C '{}'", self.id))?
            .clone();

        let command_with_term = format!("{}\n", command);
        adapter
            .write(command_with_term.into_bytes())
            .await
            .with_context(|| format!("Failed to write command '{}' to Newport 1830-C", command))?;

        // Small delay to allow meter to process command
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        Ok(())
    }

    /// Parse power measurement response from meter
    /// Handles scientific notation like "5E-9", "+.75E-9"
    fn parse_power_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();

        // Check for error responses or empty
        if trimmed.is_empty() {
            return Err(anyhow!("Empty power response"));
        }
        if trimmed.contains("ERR") || trimmed.contains("OVER") || trimmed.contains("UNDER") {
            return Err(anyhow!("Meter error response: {}", trimmed));
        }

        // Parse the value (handles scientific notation)
        trimmed
            .parse::<f64>()
            .with_context(|| format!("Failed to parse power response: '{}'", trimmed))
    }

    /// Validate attenuator code: 0=off, 1=on
    fn validate_attenuator(code: i32) -> Result<()> {
        if code < 0 || code > 1 {
            Err(anyhow!(
                "Attenuator code {} invalid. Valid codes: 0 (off), 1 (on)",
                code
            ))
        } else {
            Ok(())
        }
    }

    /// Validate filter code: 1=Slow, 2=Medium, 3=Fast
    fn validate_filter(code: i32) -> Result<()> {
        if code < 1 || code > 3 {
            Err(anyhow!(
                "Filter code {} invalid. Valid codes: 1 (Slow), 2 (Medium), 3 (Fast)",
                code
            ))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl Instrument for Newport1830C {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    fn capabilities(&self) -> Vec<TypeId> {
        vec![power_measurement_capability_id()]
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

        // Configure attenuator if specified
        if let Some(attenuator) = instrument_config
            .get("attenuator")
            .and_then(|v| v.as_integer())
        {
            let attenuator_code = attenuator as i32;
            Self::validate_attenuator(attenuator_code)?;
            self.send_config_command(&format!("A{}", attenuator_code))
                .await?;
            self.current_attenuator = Some(attenuator_code);
            info!("Set attenuator to {}", attenuator_code);
        }

        // Configure filter if specified
        if let Some(filter) = instrument_config.get("filter").and_then(|v| v.as_integer()) {
            let filter_code = filter as i32;
            Self::validate_filter(filter_code)?;
            self.send_config_command(&format!("F{}", filter_code))
                .await?;
            self.current_filter = Some(filter_code);
            info!("Set filter to {}", filter_code);
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

                // Query power measurement with retry logic
                let mut last_error = None;
                let mut read_success = false;

                for attempt in 0..3 {
                    match instrument.send_command_async("D?").await {
                        Ok(response) => {
                            match instrument.parse_power_response(&response) {
                                Ok(value) => {
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
                                        return; // Exit task if no receivers
                                    }

                                    read_success = true;
                                    break; // Success, exit retry loop
                                }
                                Err(e) => {
                                    last_error = Some(e);
                                    if attempt < 2 {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(
                                            100 * (attempt + 1),
                                        ))
                                        .await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            last_error = Some(e);
                            if attempt < 2 {
                                tokio::time::sleep(tokio::time::Duration::from_millis(
                                    100 * (attempt + 1),
                                ))
                                .await;
                            }
                        }
                    }
                }

                if !read_success {
                    if let Some(e) = last_error {
                        warn!("Failed to read from Newport 1830-C after retries: {}", e);
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
                "attenuator" => {
                    let attenuator: i32 = value
                        .as_i64()
                        .map(|v| v as i32)
                        .with_context(|| format!("Invalid attenuator value: {}", value))?;
                    Self::validate_attenuator(attenuator)?;
                    self.send_config_command(&format!("A{}", attenuator))
                        .await?;
                    self.current_attenuator = Some(attenuator);
                    info!("Set Newport 1830-C attenuator to {}", attenuator);
                }
                "filter" => {
                    let filter: i32 = value
                        .as_i64()
                        .map(|v| v as i32)
                        .with_context(|| format!("Invalid filter value: {}", value))?;
                    Self::validate_filter(filter)?;
                    self.send_config_command(&format!("F{}", filter))
                        .await?;
                    self.current_filter = Some(filter);
                    info!("Set Newport 1830-C filter to {}", filter);
                }
                _ => {
                    warn!("Unknown parameter '{}' for Newport 1830-C", key);
                }
            },
            InstrumentCommand::Execute(cmd, _) => {
                if cmd == "zero" {
                    // Newport 1830-C has CS (Clear Status) command
                    self.send_command_async("CS").await?;
                    info!("Newport 1830-C status cleared");
                }
            }
            InstrumentCommand::Capability {
                capability,
                operation,
                parameters: _,
            } => {
                if capability == power_measurement_capability_id() {
                    match operation.as_str() {
                        "start_sampling" => {
                            info!("Newport 1830-C: start_sampling capability command received");
                            // Already continuously sampling in polling loop
                        }
                        "stop_sampling" => {
                            info!("Newport 1830-C: stop_sampling capability command received");
                            // Could set a flag to pause sampling, but for now just acknowledge
                        }
                        _ => {
                            warn!(
                                "Unknown PowerMeasurement operation '{}' for Newport 1830-C",
                                operation
                            );
                        }
                    }
                } else {
                    warn!("Unsupported capability {:?} for Newport 1830-C", capability);
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