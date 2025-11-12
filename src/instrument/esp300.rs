//! Newport ESP300 3-axis motion controller driver
//!
//! This module provides an `Instrument` implementation for the Newport ESP300
//! motion controller using RS-232 serial communication with hardware flow control.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.motion_controller]
//! type = "esp300"
//! port = "/dev/ttyUSB0"
//! baud_rate = 19200
//! polling_rate_hz = 5.0
//!
//! [instruments.motion_controller.axis1]
//! units = 1  # millimeters
//! velocity = 5.0  # mm/s
//! acceleration = 10.0  # mm/s²
//! ```

#[cfg(feature = "instrument_serial")]
use crate::adapters::serial::SerialAdapter;
use crate::{
    config::{Settings, TimeoutSettings},
    core::{DataPoint, Instrument, InstrumentCommand},
    measurement::InstrumentMeasurement,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;

/// Newport ESP300 instrument implementation
#[derive(Clone)]
pub struct ESP300 {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    #[cfg(feature = "instrument_serial")]
    command_timeout: Duration,
    // Removed sender field - using InstrumentMeasurement with DataDistributor
    num_axes: u8,
    measurement: Option<InstrumentMeasurement>,
}

impl ESP300 {
    /// Creates a new ESP300 instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            adapter: None,
            #[cfg(feature = "instrument_serial")]
            command_timeout: default_scpi_timeout(),
            // No sender field
            num_axes: 3, // ESP300 has 3 axes
            measurement: None,
        }
    }

    #[cfg(feature = "instrument_serial")]
    async fn send_command_async(&self, command: &str) -> Result<String> {
        use super::serial_helper;

        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to ESP300 '{}'", self.id))?
            .clone();

        serial_helper::send_command_async(
            adapter,
            &self.id,
            command,
            "\r\n",
            self.command_timeout,
            b'\n',
        )
        .await
    }

    #[cfg(feature = "instrument_serial")]
    async fn get_position(&self, axis: u8) -> Result<f64> {
        let response = self.send_command_async(&format!("{}TP", axis)).await?;
        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse position response: {}", response))
    }

    #[cfg(feature = "instrument_serial")]
    async fn get_velocity(&self, axis: u8) -> Result<f64> {
        let response = self.send_command_async(&format!("{}TV", axis)).await?;
        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse velocity response: {}", response))
    }

    #[cfg(feature = "instrument_serial")]
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()> {
        self.send_command_async(&format!("{}PA{}", axis, position))
            .await?;
        Ok(())
    }

    #[cfg(feature = "instrument_serial")]
    async fn move_relative(&self, axis: u8, distance: f64) -> Result<()> {
        self.send_command_async(&format!("{}PR{}", axis, distance))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Instrument for ESP300 {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement.as_ref().unwrap()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to ESP300 motion controller: {}", id);
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
            .unwrap_or(19200) as u32;

        #[cfg(feature = "instrument_serial")]
        {
            self.command_timeout =
                Duration::from_millis(settings.application.timeouts.scpi_command_timeout_ms);
        }

        // Open serial port with NO flow control
        // NOTE: Despite documentation suggesting RTS/CTS, hardware testing confirmed
        // that ESP300 v3.04 works correctly with FlowControl::None (validated 2025-11-02)
        let port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(500))
            .flow_control(serialport::FlowControl::None)
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for ESP300", port_name))?;

        self.adapter = Some(SerialAdapter::new(port));

        // Query controller version
        let version = self.send_command_async("VE?").await?;
        info!("ESP300 version: {}", version);

        // Configure axes if specified
        let mut batch = self.adapter.as_mut().unwrap().start_batch();
        for axis in 1..=self.num_axes {
            let axis_key = format!("axis{}", axis);
            if let Some(axis_config) = instrument_config.get(&axis_key) {
                // Set units
                if let Some(units) = axis_config.get("units").and_then(|v| v.as_integer()) {
                    batch.queue(format!("{}SN{}", axis, units));
                    info!("Set axis {} units to {}", axis, units);
                }

                // Set velocity
                if let Some(vel) = axis_config.get("velocity").and_then(|v| v.as_float()) {
                    batch.queue(format!("{}VA{}", axis, vel));
                    info!("Set axis {} velocity to {} units/s", axis, vel);
                }

                // Set acceleration
                if let Some(accel) = axis_config.get("acceleration").and_then(|v| v.as_float()) {
                    batch.queue(format!("{}AC{}", axis, accel));
                    info!("Set axis {} acceleration to {} units/s²", axis, accel);
                }
            }
        }
        batch.flush().await?;

        // Create measurement distributor with configured capacity
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(5.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Poll each axis
                for axis in 1..=instrument.num_axes {
                    // Get position
                    if let Ok(position) = instrument.get_position(axis).await {
                        let dp = DataPoint {
                            timestamp,
                            instrument_id: instrument.id.clone(),
                            channel: format!("axis{}_position", axis),
                            value: position,
                            unit: "units".to_string(),
                            metadata: Some(serde_json::json!({"axis": axis})),
                        };
                        if measurement.broadcast(dp).await.is_err() {
                            warn!("No active receivers for ESP300 data");
                            return;
                        }
                    }

                    // Get velocity
                    if let Ok(velocity) = instrument.get_velocity(axis).await {
                        let dp = DataPoint {
                            timestamp,
                            instrument_id: instrument.id.clone(),
                            channel: format!("axis{}_velocity", axis),
                            value: velocity,
                            unit: "units/s".to_string(),
                            metadata: Some(serde_json::json!({"axis": axis})),
                        };
                        let _ = measurement.broadcast(dp).await;
                    }
                }
            }
        });

        info!(
            "ESP300 motion controller '{}' connected successfully",
            self.id
        );
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
        info!("Disconnecting from ESP300 motion controller: {}", self.id);
        #[cfg(feature = "instrument_serial")]
        {
            self.adapter = None;
        }
        self.measurement = None;
        Ok(())
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                // Parse axis:parameter format (e.g., "1:position", "2:velocity")
                let parts: Vec<&str> = key.split(':').collect();
                if parts.len() == 2 {
                    let axis: u8 = parts[0]
                        .parse()
                        .with_context(|| format!("Invalid axis number: {}", parts[0]))?;

                    match parts[1] {
                        "position" => {
                            let position: f64 = value
                                .as_f64()
                                .with_context(|| format!("Invalid position value: {}", value))?;
                            self.move_absolute(axis, position).await?;
                            info!("ESP300 axis {} move to {} mm", axis, position);
                        }
                        "velocity" => {
                            let velocity: f64 = value
                                .as_f64()
                                .with_context(|| format!("Invalid velocity value: {}", value))?;
                            self.send_command_async(&format!("{}VA{}", axis, velocity))
                                .await?;
                            info!("ESP300 axis {} velocity set to {} mm/s", axis, velocity);
                        }
                        _ => {
                            warn!("Unknown parameter '{}' for ESP300", key);
                        }
                    }
                } else {
                    warn!("Unknown parameter '{}' for ESP300", key);
                }
            }
            InstrumentCommand::Execute(cmd, args) => {
                match cmd.as_str() {
                    "move_relative" => {
                        if args.len() >= 2 {
                            let axis: u8 = args[0]
                                .parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            let distance: f64 = args[1]
                                .parse()
                                .with_context(|| format!("Invalid distance: {}", args[1]))?;
                            self.move_relative(axis, distance).await?;
                            info!("ESP300 axis {} move relative {} mm", axis, distance);
                        }
                    }
                    "stop" => {
                        if !args.is_empty() {
                            let axis: u8 = args[0]
                                .parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            self.send_command_async(&format!("{}ST", axis)).await?;
                            info!("ESP300 axis {} stopped", axis);
                        }
                    }
                    "home" => {
                        if args.is_empty() {
                            // Home all axes
                            let mut batch = self.adapter.as_mut().unwrap().start_batch();
                            for axis in 1..=self.num_axes {
                                batch.queue(format!("{}OR", axis));
                            }
                            batch.flush().await?;
                            info!("Homed all ESP300 axes");
                        } else {
                            let axis: u8 = args[0]
                                .parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            self.send_command_async(&format!("{}OR", axis)).await?;
                            info!("ESP300 axis {} homed", axis);
                        }
                    }
                    _ => {
                        warn!("Unknown command '{}' for ESP300", cmd);
                    }
                }
            }
            _ => {
                warn!("Unsupported command type for ESP300");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn handle_command(&mut self, _command: InstrumentCommand) -> Result<()> {
        Err(anyhow!("Serial support not enabled"))
    }
}

#[cfg(feature = "instrument_serial")]
fn default_scpi_timeout() -> Duration {
    Duration::from_millis(TimeoutSettings::default().scpi_command_timeout_ms)
}
