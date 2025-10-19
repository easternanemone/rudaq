//! Thorlabs Elliptec ELL14 rotation mount driver
//!
//! This module provides an `Instrument` implementation for Elliptec ELL14
//! rotation mounts using RS-485 multidrop serial communication.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.elliptec_rotators]
//! type = "elliptec"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! device_addresses = [0, 1]  # Multiple devices on same bus
//! polling_rate_hz = 2.0
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
use tokio::sync::broadcast;


/// Elliptec ELL14 instrument implementation
#[derive(Clone)]
pub struct Elliptec {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    sender: Option<broadcast::Sender<DataPoint>>,
    device_addresses: Vec<u8>,
    measurement: Option<InstrumentMeasurement>,
}

impl Elliptec {
    /// Creates a new Elliptec instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            adapter: None,
            sender: None,
            device_addresses: vec![0], // Default to address 0
            measurement: None,
        }
    }

    #[cfg(feature = "instrument_serial")]
    async fn send_command_async(&self, address: u8, command: &str) -> Result<String> {
        use super::serial_helper;
        use std::time::Duration;

        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to Elliptec '{}'", self.id))?
            .clone();

        // Elliptec protocol: address + command
        let cmd = format!("{}{}", address, command);

        serial_helper::send_command_async(
            adapter,
            &self.id,
            &cmd,
            "",
            Duration::from_millis(500),
            b'\r',
        )
        .await
    }

    #[cfg(feature = "instrument_serial")]
    async fn get_position(&self, address: u8) -> Result<f64> {
        // 'gp' command - get position
        let response = self.send_command_async(address, "gp").await?;

        // Response format: "0PO12345678" where address=0, PO is response code, 12345678 is hex position
        if response.len() < 10 {
            return Err(anyhow!("Invalid position response: {}", response));
        }

        let hex_pos = &response[3..]; // Skip address and "PO"
        let raw_pos = u32::from_str_radix(hex_pos, 16)
            .with_context(|| format!("Failed to parse hex position: {}", hex_pos))?;

        // Convert to degrees (ELL14 specific conversion)
        // Full rotation = 143360 counts = 360 degrees
        let degrees = (raw_pos as f64 / 143360.0) * 360.0;
        Ok(degrees)
    }

    #[cfg(feature = "instrument_serial")]
    async fn set_position(&self, address: u8, degrees: f64) -> Result<()> {
        // Convert degrees to counts
        let counts = ((degrees / 360.0) * 143360.0) as u32;
        let hex_pos = format!("{:08X}", counts);

        // 'ma' command - move absolute
        self.send_command_async(address, &format!("ma{}", hex_pos))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Instrument for Elliptec {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Elliptec rotators: {}", id);
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

        // Get device addresses
        self.device_addresses = instrument_config
            .get("device_addresses")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_integer())
                    .map(|i| i as u8)
                    .collect()
            })
            .unwrap_or_else(|| vec![0]);

        info!("Elliptec device addresses: {:?}", self.device_addresses);

        // Open serial port
        let port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(100))
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for Elliptec", port_name))?;

        self.adapter = Some(SerialAdapter::new(port));

        // Query device info for each address
        for &addr in &self.device_addresses {
            let response = self.send_command_async(addr, "in").await?;
            info!("Elliptec device {} info: {}", addr, response);
        }

        // Create broadcast channel with configured capacity
        let capacity = settings.application.broadcast_channel_capacity;
        let (sender, _) = broadcast::channel(capacity);
        self.sender = Some(sender.clone());
        self.measurement = Some(InstrumentMeasurement::new(sender.clone(), self.id.clone()));

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(2.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Poll each device
                for &addr in &instrument.device_addresses {
                    match instrument.get_position(addr).await {
                        Ok(position) => {
                            let dp = DataPoint {
                                timestamp,
                                instrument_id: instrument.id.clone(),
                                channel: format!("device{}_position", addr),
                                value: position,
                                unit: "deg".to_string(),
                                metadata: Some(serde_json::json!({"device_address": addr})),
                            };

                            if sender.send(dp).is_err() {
                                warn!("No active receivers for Elliptec data");
                                return;
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to read position from Elliptec device {}: {}",
                                addr, e
                            );
                        }
                    }
                }
            }
        });

        info!("Elliptec rotators '{}' connected successfully", self.id);
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
        info!("Disconnecting from Elliptec rotators: {}", self.id);
        #[cfg(feature = "instrument_serial")]
        {
            self.adapter = None;
        }
        self.sender = None;
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement
            .as_ref()
            .expect("Elliptec measurement not initialised")
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                // Parse device_address:parameter format
                let parts: Vec<&str> = key.split(':').collect();
                if parts.len() == 2 {
                    let addr: u8 = parts[0]
                        .parse()
                        .with_context(|| format!("Invalid device address: {}", parts[0]))?;

                    if parts[1] == "position" {
                        let degrees: f64 = value
                            .parse()
                            .with_context(|| format!("Invalid position value: {}", value))?;
                        self.set_position(addr, degrees).await?;
                        info!("Set Elliptec device {} to {} degrees", addr, degrees);
                    }
                } else {
                    warn!("Unknown parameter '{}' for Elliptec", key);
                }
            }
            InstrumentCommand::Execute(cmd, args) => {
                if cmd == "home" {
                    if args.is_empty() {
                        // Home all devices
                        for &addr in &self.device_addresses {
                            self.send_command_async(addr, "ho").await?;
                        }
                        info!("Homed all Elliptec devices");
                    } else {
                        // Home specific device
                        let addr: u8 = args[0]
                            .parse()
                            .with_context(|| format!("Invalid device address: {}", args[0]))?;
                        self.send_command_async(addr, "ho").await?;
                        info!("Homed Elliptec device {}", addr);
                    }
                }
            }
            _ => {
                warn!("Unsupported command type for Elliptec");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn handle_command(&mut self, _command: InstrumentCommand) -> Result<()> {
        Err(anyhow!("Serial support not enabled"))
    }
}
