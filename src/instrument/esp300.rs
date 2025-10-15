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

use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::broadcast;

#[cfg(feature = "instrument_serial")]
use serialport::SerialPort;

/// Newport ESP300 instrument implementation
#[derive(Clone)]
pub struct ESP300 {
    id: String,
    #[cfg(feature = "instrument_serial")]
    port: Option<Arc<tokio::sync::Mutex<Box<dyn SerialPort>>>>,
    sender: Option<broadcast::Sender<DataPoint>>,
    num_axes: u8,
}

impl ESP300 {
    /// Creates a new ESP300 instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            port: None,
            sender: None,
            num_axes: 3, // ESP300 has 3 axes
        }
    }

    #[cfg(feature = "instrument_serial")]
    fn send_command(&self, command: &str) -> Result<String> {
        use super::serial_helper;
        use std::time::Duration;

        let port = self.port.as_ref()
            .ok_or_else(|| anyhow!("Not connected to ESP300 '{}'", self.id))?;

        let mut port = port.blocking_lock();

        serial_helper::send_command(
            &mut port,
            &self.id,
            command,
            "\r\n",
            Duration::from_secs(1),
            '\n',
        )
    }

    #[cfg(feature = "instrument_serial")]
    fn get_position(&self, axis: u8) -> Result<f64> {
        let response = self.send_command(&format!("{}TP", axis))?;
        response.parse::<f64>()
            .with_context(|| format!("Failed to parse position response: {}", response))
    }

    #[cfg(feature = "instrument_serial")]
    fn get_velocity(&self, axis: u8) -> Result<f64> {
        let response = self.send_command(&format!("{}TV", axis))?;
        response.parse::<f64>()
            .with_context(|| format!("Failed to parse velocity response: {}", response))
    }

    #[cfg(feature = "instrument_serial")]
    fn move_absolute(&self, axis: u8, position: f64) -> Result<()> {
        self.send_command(&format!("{}PA{}", axis, position))?;
        Ok(())
    }

    #[cfg(feature = "instrument_serial")]
    fn move_relative(&self, axis: u8, distance: f64) -> Result<()> {
        self.send_command(&format!("{}PR{}", axis, distance))?;
        Ok(())
    }
}

#[async_trait]
impl Instrument for ESP300 {
    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to ESP300 motion controller: {}", self.id);

        let instrument_config = settings
            .instruments
            .get(&self.id)
            .ok_or_else(|| anyhow!("Configuration for '{}' not found", self.id))?;

        let port_name = instrument_config
            .get("port")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("'port' not found in config for '{}'", self.id))?;

        let baud_rate = instrument_config
            .get("baud_rate")
            .and_then(|v| v.as_integer())
            .unwrap_or(19200) as u32;

        // Open serial port with hardware flow control
        let mut port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(500))
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for ESP300", port_name))?;

        // ESP300 requires hardware flow control (RTS/CTS)
        port.write_request_to_send(true)
            .context("Failed to enable RTS")?;
        port.write_data_terminal_ready(true)
            .context("Failed to enable DTR")?;

        self.port = Some(Arc::new(tokio::sync::Mutex::new(port)));

        // Query controller version
        let version = self.send_command("VE?")?;
        info!("ESP300 version: {}", version);

        // Configure axes if specified
        for axis in 1..=self.num_axes {
            let axis_key = format!("axis{}", axis);
            if let Some(axis_config) = instrument_config.get(&axis_key) {
                // Set units
                if let Some(units) = axis_config.get("units").and_then(|v| v.as_integer()) {
                    self.send_command(&format!("{}SN{}", axis, units))?;
                    info!("Set axis {} units to {}", axis, units);
                }

                // Set velocity
                if let Some(vel) = axis_config.get("velocity").and_then(|v| v.as_float()) {
                    self.send_command(&format!("{}VA{}", axis, vel))?;
                    info!("Set axis {} velocity to {} units/s", axis, vel);
                }

                // Set acceleration
                if let Some(accel) = axis_config.get("acceleration").and_then(|v| v.as_float()) {
                    self.send_command(&format!("{}AC{}", axis, accel))?;
                    info!("Set axis {} acceleration to {} units/s²", axis, accel);
                }
            }
        }

        // Create broadcast channel
        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(5.0);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs_f64(1.0 / polling_rate)
            );

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Poll each axis
                for axis in 1..=instrument.num_axes {
                    // Get position
                    if let Ok(position) = instrument.get_position(axis) {
                        let dp = DataPoint {
                            timestamp,
                            channel: format!("{}_axis{}_position", instrument.id, axis),
                            value: position,
                            unit: "units".to_string(),
                            metadata: Some(serde_json::json!({"axis": axis})),
                        };
                        if sender.send(dp).is_err() {
                            warn!("No active receivers for ESP300 data");
                            return;
                        }
                    }

                    // Get velocity
                    if let Ok(velocity) = instrument.get_velocity(axis) {
                        let dp = DataPoint {
                            timestamp,
                            channel: format!("{}_axis{}_velocity", instrument.id, axis),
                            value: velocity,
                            unit: "units/s".to_string(),
                            metadata: Some(serde_json::json!({"axis": axis})),
                        };
                        let _ = sender.send(dp);
                    }
                }
            }
        });

        info!("ESP300 motion controller '{}' connected successfully", self.id);
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn connect(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        Err(anyhow!("Serial support not enabled. Rebuild with --features instrument_serial"))
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from ESP300 motion controller: {}", self.id);
        #[cfg(feature = "instrument_serial")]
        {
            self.port = None;
        }
        self.sender = None;
        Ok(())
    }

    async fn data_stream(&mut self) -> Result<broadcast::Receiver<DataPoint>> {
        self.sender
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or_else(|| anyhow!("Not connected to ESP300 '{}'", self.id))
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                // Parse axis:parameter format (e.g., "1:position", "2:velocity")
                let parts: Vec<&str> = key.split(':').collect();
                if parts.len() == 2 {
                    let axis: u8 = parts[0].parse()
                        .with_context(|| format!("Invalid axis number: {}", parts[0]))?;

                    match parts[1] {
                        "position" => {
                            let position: f64 = value.parse()
                                .with_context(|| format!("Invalid position value: {}", value))?;
                            self.move_absolute(axis, position)?;
                            info!("ESP300 axis {} move to {} mm", axis, position);
                        }
                        "velocity" => {
                            let velocity: f64 = value.parse()
                                .with_context(|| format!("Invalid velocity value: {}", value))?;
                            self.send_command(&format!("{}VA{}", axis, velocity))?;
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
                            let axis: u8 = args[0].parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            let distance: f64 = args[1].parse()
                                .with_context(|| format!("Invalid distance: {}", args[1]))?;
                            self.move_relative(axis, distance)?;
                            info!("ESP300 axis {} move relative {} mm", axis, distance);
                        }
                    }
                    "stop" => {
                        if !args.is_empty() {
                            let axis: u8 = args[0].parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            self.send_command(&format!("{}ST", axis))?;
                            info!("ESP300 axis {} stopped", axis);
                        }
                    }
                    "home" => {
                        if args.is_empty() {
                            // Home all axes
                            for axis in 1..=self.num_axes {
                                self.send_command(&format!("{}OR", axis))?;
                            }
                            info!("Homed all ESP300 axes");
                        } else {
                            let axis: u8 = args[0].parse()
                                .with_context(|| format!("Invalid axis: {}", args[0]))?;
                            self.send_command(&format!("{}OR", axis))?;
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
