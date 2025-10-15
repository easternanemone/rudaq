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
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::broadcast;

#[cfg(feature = "instrument_serial")]
use serialport::SerialPort;

/// MaiTai laser instrument implementation
#[derive(Clone)]
pub struct MaiTai {
    id: String,
    #[cfg(feature = "instrument_serial")]
    port: Option<Arc<tokio::sync::Mutex<Box<dyn SerialPort>>>>,
    sender: Option<broadcast::Sender<DataPoint>>,
}

impl MaiTai {
    /// Creates a new MaiTai instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            port: None,
            sender: None,
        }
    }

    #[cfg(feature = "instrument_serial")]
    fn send_command(&self, command: &str) -> Result<String> {
        use std::io::{Read, Write};

        let port = self.port.as_ref()
            .ok_or_else(|| anyhow!("Not connected to MaiTai '{}'", self.id))?;

        let mut port = port.blocking_lock();

        // Send command (MaiTai uses CR as terminator)
        let cmd = format!("{}\r", command);
        port.write_all(cmd.as_bytes())
            .with_context(|| format!("Failed to send command to MaiTai '{}'", self.id))?;

        // Read response
        let mut buffer = [0u8; 256];
        let mut response = String::new();
        let start = std::time::Instant::now();

        while start.elapsed() < std::time::Duration::from_secs(2) {
            if let Ok(n) = port.read(&mut buffer) {
                response.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if response.contains('\r') {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Ok(response.trim().to_string())
    }

    #[cfg(feature = "instrument_serial")]
    fn query_value(&self, command: &str) -> Result<f64> {
        let response = self.send_command(command)?;
        // Remove command echo if present
        let value_str = response.split(':').last().unwrap_or(&response);
        value_str.trim().parse::<f64>()
            .with_context(|| format!("Failed to parse response '{}' as float", response))
    }
}

#[async_trait]
impl Instrument for MaiTai {
    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to MaiTai laser: {}", self.id);

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
            .unwrap_or(9600) as u32;

        // Open serial port
        let port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(500))
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for MaiTai", port_name))?;

        self.port = Some(Arc::new(tokio::sync::Mutex::new(port)));

        // Verify connection with identity query
        let id_response = self.send_command("*IDN?")?;
        info!("MaiTai identity: {}", id_response);

        // Set wavelength if specified
        if let Some(wavelength) = instrument_config.get("wavelength").and_then(|v| v.as_float()) {
            self.send_command(&format!("WAVELENGTH:{}", wavelength))?;
            info!("Set wavelength to {} nm", wavelength);
        }

        // Create broadcast channel
        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(1.0);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs_f64(1.0 / polling_rate)
            );

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Query wavelength
                if let Ok(wavelength) = instrument.query_value("WAVELENGTH?") {
                    let dp = DataPoint {
                        timestamp,
                        channel: format!("{}_wavelength", instrument.id),
                        value: wavelength,
                        unit: "nm".to_string(),
                        metadata: None,
                    };
                    if sender.send(dp).is_err() {
                        warn!("No active receivers for MaiTai data");
                        break;
                    }
                }

                // Query power
                if let Ok(power) = instrument.query_value("POWER?") {
                    let dp = DataPoint {
                        timestamp,
                        channel: format!("{}_power", instrument.id),
                        value: power,
                        unit: "W".to_string(),
                        metadata: None,
                    };
                    let _ = sender.send(dp);
                }

                // Query shutter state
                if let Ok(shutter) = instrument.query_value("SHUTTER?") {
                    let dp = DataPoint {
                        timestamp,
                        channel: format!("{}_shutter", instrument.id),
                        value: shutter,
                        unit: "state".to_string(),
                        metadata: None,
                    };
                    let _ = sender.send(dp);
                }
            }
        });

        info!("MaiTai laser '{}' connected successfully", self.id);
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn connect(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        Err(anyhow!("Serial support not enabled. Rebuild with --features instrument_serial"))
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from MaiTai laser: {}", self.id);
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
            .ok_or_else(|| anyhow!("Not connected to MaiTai '{}'", self.id))
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                match key.as_str() {
                    "wavelength" => {
                        let wavelength: f64 = value.parse()
                            .with_context(|| format!("Invalid wavelength value: {}", value))?;
                        self.send_command(&format!("WAVELENGTH:{}", wavelength))?;
                        info!("Set MaiTai wavelength to {} nm", wavelength);
                    }
                    "shutter" => {
                        let cmd = match value.as_str() {
                            "open" => "SHUTTER:1",
                            "close" => "SHUTTER:0",
                            _ => return Err(anyhow!("Invalid shutter value: {}", value)),
                        };
                        self.send_command(cmd)?;
                        info!("MaiTai shutter: {}", value);
                    }
                    "laser" => {
                        let cmd = match value.as_str() {
                            "on" => "ON",
                            "off" => "OFF",
                            _ => return Err(anyhow!("Invalid laser value: {}", value)),
                        };
                        self.send_command(cmd)?;
                        info!("MaiTai laser: {}", value);
                    }
                    _ => {
                        warn!("Unknown parameter '{}' for MaiTai", key);
                    }
                }
            }
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
