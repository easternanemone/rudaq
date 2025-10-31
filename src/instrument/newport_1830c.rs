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
    // Removed sender field - using InstrumentMeasurement with DataDistributor
    measurement: Option<InstrumentMeasurement>,
    // Track current units for measurement labeling
    current_units: String,
    // Track current parameter values for validation and state management
    current_wavelength: Option<f64>,
    current_range: Option<i32>,
    current_units_code: Option<i32>,
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
            current_units: "W".to_string(),
            current_wavelength: None,
            current_range: None,
            current_units_code: None,
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
    
    /// Parse power measurement response from meter
    /// Handles scientific notation, whitespace, and error responses
    fn parse_power_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();
        
        // Check for error responses
        if trimmed.contains("ERR") || trimmed.contains("OVER") || trimmed.contains("UNDER") {
            return Err(anyhow!("Meter error response: {}", trimmed));
        }
        
        // Parse the value (handles scientific notation like "1.234E-03")
        trimmed.parse::<f64>()
            .with_context(|| format!("Failed to parse power response: '{}'", trimmed))
    }

    /// Validate wavelength is within typical Newport 1830-C range
    /// Range depends on photodetector model (typically 400-1700nm)
    fn validate_wavelength(nm: f64) -> Result<()> {
        if nm < 400.0 || nm > 1700.0 {
            Err(anyhow!(
                "Wavelength {} nm out of range (400-1700 nm). Range depends on photodetector model.",
                nm
            ))
        } else {
            Ok(())
        }
    }

    /// Validate range code
    /// Valid codes: 0 (autorange), 1-8 (manual ranges)
    fn validate_range(code: i32) -> Result<()> {
        if code < 0 || code > 8 {
            Err(anyhow!(
                "Range code {} invalid. Valid codes: 0 (auto), 1-8 (manual ranges)",
                code
            ))
        } else {
            Ok(())
        }
    }

    /// Validate units code
    /// Valid codes: 0=Watts, 1=dBm, 2=dB, 3=REL
    fn validate_units(code: i32) -> Result<()> {
        if code < 0 || code > 3 {
            Err(anyhow!(
                "Units code {} invalid. Valid codes: 0=Watts, 1=dBm, 2=dB, 3=REL",
                code
            ))
        } else {
            Ok(())
        }
    }

    /// Convert units code to string
    fn units_code_to_string(code: i32) -> &'static str {
        match code {
            0 => "W",
            1 => "dBm",
            2 => "dB",
            3 => "REL",
            _ => "W",
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

        // Configure wavelength if specified
        if let Some(wavelength) = instrument_config
            .get("wavelength")
            .and_then(|v| v.as_float())
        {
            Self::validate_wavelength(wavelength)?;
            self.send_command_async(&format!("PM:Lambda {}", wavelength))
                .await?;
            self.current_wavelength = Some(wavelength);
            info!("Set wavelength to {} nm", wavelength);
        }

        // Configure range if specified
        if let Some(range) = instrument_config.get("range").and_then(|v| v.as_integer()) {
            let range_code = range as i32;
            Self::validate_range(range_code)?;
            self.send_command_async(&format!("PM:Range {}", range_code))
                .await?;
            self.current_range = Some(range_code);
            info!("Set range to {}", range_code);
        }

        // Configure units if specified
        if let Some(units) = instrument_config.get("units").and_then(|v| v.as_integer()) {
            let units_code = units as i32;
            Self::validate_units(units_code)?;
            self.send_command_async(&format!("PM:Units {}", units_code))
                .await?;
            self.current_units_code = Some(units_code);
            let unit_str = Self::units_code_to_string(units_code);
            self.current_units = unit_str.to_string();
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

                // Query power measurement with retry logic
                let mut last_error = None;
                let mut read_success = false;
                
                for attempt in 0..3 {
                    match instrument.send_command_async("PM:Power?").await {
                        Ok(response) => {
                            match instrument.parse_power_response(&response) {
                                Ok(value) => {
                                    let dp = DataPoint {
                                        timestamp: chrono::Utc::now(),
                                        instrument_id: instrument.id.clone(),
                                        channel: "power".to_string(),
                                        value,
                                        unit: instrument.current_units.clone(),
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
                                        tokio::time::sleep(
                                            tokio::time::Duration::from_millis(100 * (attempt + 1))
                                        ).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            last_error = Some(e);
                            if attempt < 2 {
                                tokio::time::sleep(
                                    tokio::time::Duration::from_millis(100 * (attempt + 1))
                                ).await;
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
                "wavelength" => {
                    let wavelength: f64 = value
                        .as_f64()
                        .with_context(|| format!("Invalid wavelength value: {}", value))?;
                    Self::validate_wavelength(wavelength)?;
                    self.send_command_async(&format!("PM:Lambda {}", wavelength))
                        .await?;
                    self.current_wavelength = Some(wavelength);
                    info!("Set Newport 1830-C wavelength to {} nm", wavelength);
                }
                "range" => {
                    let range: i32 = value
                        .as_i64()
                        .map(|v| v as i32)
                        .with_context(|| format!("Invalid range value: {}", value))?;
                    Self::validate_range(range)?;
                    self.send_command_async(&format!("PM:Range {}", range))
                        .await?;
                    self.current_range = Some(range);
                    info!("Set Newport 1830-C range to {}", range);
                }
                "units" => {
                    let units: i32 = value
                        .as_i64()
                        .map(|v| v as i32)
                        .with_context(|| format!("Invalid units value: {}", value))?;
                    Self::validate_units(units)?;
                    self.send_command_async(&format!("PM:Units {}", units))
                        .await?;
                    self.current_units_code = Some(units);
                    self.current_units = Self::units_code_to_string(units).to_string();
                    info!("Set Newport 1830-C units to {}", Self::units_code_to_string(units));
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
            InstrumentCommand::Capability {
                capability,
                operation,
                parameters,
            } => {
                if capability == power_measurement_capability_id() {
                    match operation.as_str() {
                        "start_sampling" => {
                            info!("Newport 1830-C: start_sampling capability command received");
                            // Already continuously sampling in polling loop
                            Ok(())
                        }
                        "stop_sampling" => {
                            info!("Newport 1830-C: stop_sampling capability command received");
                            // Could set a flag to pause sampling, but for now just acknowledge
                            Ok(())
                        }
                        "set_range" => {
                            if let Some(range_value) = parameters.first().and_then(|p| p.as_f64()) {
                                let range_code = range_value as i32;
                                self.send_command_async(&format!("PM:Range {}", range_code))
                                    .await?;
                                info!("Set Newport 1830-C range to {} via capability", range_code);
                                Ok(())
                            } else {
                                Err(anyhow!("set_range requires a numeric range parameter"))
                            }
                        }
                        _ => {
                            warn!(
                                "Unknown PowerMeasurement operation '{}' for Newport 1830-C",
                                operation
                            );
                            Ok(())
                        }
                    }?;
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