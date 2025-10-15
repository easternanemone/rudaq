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
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::broadcast;

#[cfg(feature = "instrument_serial")]
use serialport::SerialPort;

/// Newport 1830-C instrument implementation
#[derive(Clone)]
pub struct Newport1830C {
    id: String,
    #[cfg(feature = "instrument_serial")]
    port: Option<Arc<tokio::sync::Mutex<Box<dyn SerialPort>>>>,
    sender: Option<broadcast::Sender<DataPoint>>,
}

impl Newport1830C {
    /// Creates a new Newport1830C instrument
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
            .ok_or_else(|| anyhow!("Not connected to Newport 1830-C '{}'", self.id))?;

        let mut port = port.blocking_lock();

        // Send command
        let cmd = format!("{}\r\n", command);
        port.write_all(cmd.as_bytes())
            .with_context(|| format!("Failed to send command to Newport 1830-C '{}'", self.id))?;

        // Read response
        let mut buffer = [0u8; 256];
        let mut response = String::new();
        let start = std::time::Instant::now();

        while start.elapsed() < std::time::Duration::from_secs(1) {
            if let Ok(n) = port.read(&mut buffer) {
                response.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if response.contains('\r') || response.contains('\n') {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Ok(response.trim().to_string())
    }
}

#[async_trait]
impl Instrument for Newport1830C {
    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Newport 1830-C: {}", self.id);

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
            .timeout(std::time::Duration::from_millis(100))
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for Newport 1830-C", port_name))?;

        self.port = Some(Arc::new(tokio::sync::Mutex::new(port)));

        // Configure wavelength if specified
        if let Some(wavelength) = instrument_config.get("wavelength").and_then(|v| v.as_float()) {
            self.send_command(&format!("PM:Lambda {}", wavelength))?;
            info!("Set wavelength to {} nm", wavelength);
        }

        // Configure range if specified
        if let Some(range) = instrument_config.get("range").and_then(|v| v.as_integer()) {
            self.send_command(&format!("PM:Range {}", range))?;
            info!("Set range to {}", range);
        }

        // Configure units if specified
        if let Some(units) = instrument_config.get("units").and_then(|v| v.as_integer()) {
            self.send_command(&format!("PM:Units {}", units))?;
            let unit_str = match units {
                0 => "Watts",
                1 => "dBm",
                2 => "dB",
                3 => "REL",
                _ => "Unknown",
            };
            info!("Set units to {}", unit_str);
        }

        // Create broadcast channel
        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(10.0);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs_f64(1.0 / polling_rate)
            );

            loop {
                interval.tick().await;

                // Query power measurement
                match instrument.send_command("PM:Power?") {
                    Ok(response) => {
                        if let Ok(value) = response.parse::<f64>() {
                            let dp = DataPoint {
                                timestamp: chrono::Utc::now(),
                                channel: format!("{}_power", instrument.id),
                                value,
                                unit: "W".to_string(),
                                metadata: None,
                            };

                            if sender.send(dp).is_err() {
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
    async fn connect(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        Err(anyhow!("Serial support not enabled. Rebuild with --features instrument_serial"))
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from Newport 1830-C: {}", self.id);
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
            .ok_or_else(|| anyhow!("Not connected to Newport 1830-C '{}'", self.id))
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                match key.as_str() {
                    "wavelength" => {
                        let wavelength: f64 = value.parse()
                            .with_context(|| format!("Invalid wavelength value: {}", value))?;
                        self.send_command(&format!("PM:Lambda {}", wavelength))?;
                        info!("Set Newport 1830-C wavelength to {} nm", wavelength);
                    }
                    "range" => {
                        let range: i32 = value.parse()
                            .with_context(|| format!("Invalid range value: {}", value))?;
                        self.send_command(&format!("PM:Range {}", range))?;
                        info!("Set Newport 1830-C range to {}", range);
                    }
                    "units" => {
                        let units: i32 = value.parse()
                            .with_context(|| format!("Invalid units value: {}", value))?;
                        self.send_command(&format!("PM:Units {}", units))?;
                        info!("Set Newport 1830-C units to {}", units);
                    }
                    _ => {
                        warn!("Unknown parameter '{}' for Newport 1830-C", key);
                    }
                }
            }
            InstrumentCommand::Execute(cmd, _) => {
                if cmd == "zero" {
                    self.send_command("PM:DS:Clear")?;
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
