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
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;
use tokio::sync::broadcast;
use visa_rs::prelude::*;
use visa_rs::prelude::*;
use std::ffi::CString;

/// An `Instrument` implementation for VISA devices.
#[derive(Clone)]
pub struct VisaInstrument {
    id: String,
    sender: Option<broadcast::Sender<DataPoint>>,
}

use std::io::{Read, Write};

impl VisaInstrument {
    /// Creates a new `VisaInstrument` with the given resource name.
    pub fn new(id: &str) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            sender: None,
        })
    }
}

#[async_trait]
impl Instrument for VisaInstrument {
    fn name(&self) -> String {
        self.id.clone()
    }

    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to VISA instrument: {}", id);
        self.id = id.to_string();

        let instrument_config = settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Configuration for '{}' not found", id))?;

        let resource_string = instrument_config
            .get("resource_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("'resource_string' not found in config for '{}'", self.id))?;

        let rm = DefaultRM::new()?;
        let c_string = CString::new(resource_string).context("Failed to create CString")?;
        let visa_string = visa_rs::VisaString::from(c_string);
        let mut session = rm.open(&visa_string, AccessMode::NO_LOCK, TIMEOUT_IMMEDIATE)?;
        let _instrument = VisaInstrument::new(&self.id)?;

        let (sender, _) = broadcast::channel(1024);
        self.sender = Some(sender.clone());

        let polling_rate_hz = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .ok_or_else(|| anyhow!("'polling_rate_hz' not found in config for '{}'", self.id))?;

        let queries = instrument_config
            .get("queries")
            .and_then(|v| {
                v.clone()
                    .try_into::<std::collections::HashMap<String, String>>()
                    .ok()
            })
            .ok_or_else(|| anyhow!("'queries' not found in config for '{}'", self.id))?;

        let id_clone = id.to_string();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate_hz));
            loop {
                interval.tick().await;
                for (channel, query_str) in &queries {
                    let mut buf = [0u8; 1024];
                    let result = session.write_all(query_str.as_bytes()).and_then(|()| session.read(&mut buf));
                    match result {
                        Ok(bytes_read) => {
                            let response = String::from_utf8_lossy(&buf[..bytes_read]);
                            let value = response.trim().parse::<f64>().unwrap_or(0.0);
                            let dp = DataPoint {
                                timestamp: chrono::Utc::now(),
                                instrument_id: id_clone.clone(),
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

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from VISA instrument.");
        self.sender = None;
        Ok(())
    }

    async fn data_stream(&mut self) -> Result<broadcast::Receiver<DataPoint>> {
        self.sender
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or_else(|| anyhow!("Not connected to instrument '{}'", self.id))
    }
}
