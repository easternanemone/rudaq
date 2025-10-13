//! VISA instrument implementation.
//!
//! This module provides an `Instrument` implementation for devices that support
//! the VISA (Virtual Instrument Software Architecture) standard. It uses the
//! `visa-rs` crate to communicate with the VISA library.
//!
//! ## Configuration
//!
//! VISA instruments are configured in the `config/default.toml` file. Here is
//! an example configuration for a Rigol DS1054Z oscilloscope:
//!
//! ```toml
//! [instruments.visa_rigol]
//! name = "Rigol DS1054Z (VISA)"
//! resource_string = "TCPIP0::192.168.1.101::INSTR"
//! polling_rate_hz = 10.0
//! queries = { "voltage" = ":MEAS:VPP? CHAN1", "frequency" = ":MEAS:FREQ? CHAN1" }
//! ```
//!
//! - `resource_string`: The VISA resource string for the instrument.
//! - `polling_rate_hz`: The rate at which the instrument is polled for data.
//! - `queries`: A map of SCPI queries to be executed at each poll. The key is
//!   the channel name, and the value is the SCPI command.

use crate::{
    config::Settings,
    core::{DataPoint, Instrument},
    error::DaqError,
};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::broadcast;
use visa_rs::{prelude::*, session::Session};

/// An `Instrument` implementation for VISA devices.
#[derive(Clone)]
pub struct VisaInstrument {
    id: String,
    session: Option<Arc<Session>>,
    sender: Option<broadcast::Sender<DataPoint>>,
    // Add a default resource manager to handle the VISA session
    rm: Arc<DefaultRM>,
}

use std::io::{BufRead, BufReader, Write};

impl VisaInstrument {
    /// Creates a new `VisaInstrument` with the given resource name.
    pub fn new(id: &str) -> Result<Self, DaqError> {
        Ok(Self {
            id: id.to_string(),
            session: None,
            sender: None,
            rm: Arc::new(DefaultRM::new()?),
        })
    }

    /// Writes a SCPI command to the instrument.
    pub fn write(&self, command: &str) -> Result<(), DaqError> {
        self.session
            .as_ref()
            .ok_or_else(|| DaqError::Instrument("Not connected".to_string()))?
            .write_all(command.as_bytes())?;
        Ok(())
    }

    /// Writes a SCPI query to the instrument and returns the response.
    pub fn query(&self, command: &str) -> Result<String, DaqError> {
        self.write(command)?;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| DaqError::Instrument("Not connected".to_string()))?;
        let mut reader = BufReader::new(session.as_ref());
        let mut buf = String::new();
        reader.read_line(&mut buf)?;
        Ok(buf)
    }

    /// Reads a fixed number of bytes from the instrument.
    pub fn read_binary(&self, length: usize) -> Result<Vec<u8>, DaqError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| DaqError::Instrument("Not connected".to_string()))?;
        let mut reader = BufReader::new(session.as_ref());
        let mut buf = vec![0; length];
        reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Reads from the instrument until the buffer is empty.
    pub fn read_until_end(&self) -> Result<Vec<u8>, DaqError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| DaqError::Instrument("Not connected".to_string()))?;
        let mut reader = BufReader::new(session.as_ref());
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[async_trait]
impl Instrument for VisaInstrument {
    fn name(&self) -> String {
        self.id.clone()
    }

    async fn connect(&mut self, settings: &Arc<Settings>) -> Result<(), DaqError> {
        info!("Connecting to VISA instrument: {}", self.id);

        let instrument_config = settings
            .instruments
            .get(&self.id)
            .ok_or_else(|| DaqError::Instrument(format!("Configuration for {} not found", self.id)))?;

        let resource_string = instrument_config
            .get("resource_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DaqError::Instrument("resource_string not found in config".to_string()))?;

        let res = self
            .rm
            .open(&resource_string.try_into()?, AccessMode::NO_LOCK, TIMEOUT_IMMEDIATE)?;
        self.session = Some(Arc::new(res));
        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

        let polling_rate_hz = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .ok_or_else(|| DaqError::Instrument("polling_rate_hz not found in config".to_string()))?;

        let queries = instrument_config
            .get("queries")
            .and_then(|v| v.clone().try_into::<std::collections::HashMap<String, String>>().ok())
            .ok_or_else(|| DaqError::Instrument("queries not found in config".to_string()))?;

        let instrument = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate_hz));
            loop {
                interval.tick().await;
                for (channel, query_str) in &queries {
                    match instrument.query(query_str) {
                        Ok(response) => {
                            let value = response.trim().parse::<f64>().unwrap_or(0.0);
                            let dp = DataPoint {
                                timestamp: chrono::Utc::now(),
                                channel: channel.clone(),
                                value,
                                unit: "V".to_string(), // a default unit
                                metadata: None,
                            };
                            if sender.send(dp).is_err() {
                                warn!("No active receivers for VISA instrument data.");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("Failed to query instrument: {}", e);
                        }
                    }
                }
            }
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DaqError> {
        info!("Disconnecting from VISA instrument.");
        if let Some(session) = self.session.take() {
            drop(session);
        }
        self.sender = None;
        Ok(())
    }

    async fn data_stream(&mut self) -> Result<broadcast::Receiver<DataPoint>, DaqError> {
        self.sender
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or_else(|| DaqError::Instrument("Not connected".to_string()))
    }
}