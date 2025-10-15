//! Configuration management for the DAQ application.
//!
//! This module defines the data structures for the application's configuration,
//! which is loaded from TOML files. It uses the `config` crate to handle file
//! loading and deserialization and `serde` for the data structures.
//!
//! ## Schema
//!
//! The configuration is structured as follows:
//!
//! - **`log_level`**: A string representing the logging verbosity (e.g., "info", "debug").
//! - **`storage`**: A table containing storage settings.
//!   - `default_path`: The directory where data files are saved.
//!   - `default_format`: The default file format for saving data (e.g., "csv", "hdf5").
//! - **`instruments`**: A map where each key is a unique instrument ID and the value is a
//!   TOML table defining the instrument's properties. The `type` field within this table
//!   is mandatory and determines which instrument driver is used. Other fields are
//!   specific to the instrument type.
//! - **`processors`**: An optional map where each key corresponds to a data channel. The value
//!   is a list of processor configurations to be applied to the data from that channel.
//!   Each processor has a `type` and its own specific configuration.
//!
//! ## Validation
//!
//! The `Settings::new` function loads and deserializes the configuration. After loading,
//! it calls the `validate` method, which performs a series of checks on the configuration
//! values to ensure they are valid. This includes:
//!
//! - Checking that required fields are not empty.
//! - Validating log levels against a predefined list.
//! - Ensuring file paths are valid.
//! - Validating network parameters like IP addresses and port numbers.
//! - Checking that numerical values (like sample rates) are within reasonable ranges.
//!
//! Validation logic is implemented in the `validation` module. If validation fails,
//! the application will not start, preventing runtime errors due to misconfiguration.

use crate::validation::{is_in_range, is_not_empty, is_valid_ip, is_valid_path, is_valid_port};
use anyhow::{Context, Result};
use config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub log_level: String,
    pub storage: StorageSettings,
    pub instruments: HashMap<String, toml::Value>,
    pub processors: Option<HashMap<String, Vec<ProcessorConfig>>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProcessorConfig {
    pub r#type: String,
    #[serde(flatten)]
    pub config: toml::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StorageSettings {
    pub default_path: String,
    pub default_format: String,
}

impl Settings {
    pub fn new(config_name: Option<&str>) -> Result<Self> {
        let config_path = format!("config/{}", config_name.unwrap_or("default"));
        let s = Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()
            .with_context(|| format!("Failed to load configuration from '{}'", config_path))?;

        let settings: Settings = s
            .try_deserialize()
            .context("Failed to deserialize configuration")?;
        settings.validate()?;
        Ok(settings)
    }
}

impl Settings {
    fn validate(&self) -> Result<()> {
        is_not_empty(&self.log_level)
            .map_err(anyhow::Error::msg)
            .context("log_level cannot be empty")?;
        let valid_log_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_log_levels.contains(&self.log_level.to_lowercase().as_str()) {
            anyhow::bail!("Invalid log level: {}", self.log_level);
        }

        is_valid_path(&self.storage.default_path)
            .map_err(anyhow::Error::msg)
            .context("Invalid storage default_path")?;
        is_not_empty(&self.storage.default_format)
            .map_err(anyhow::Error::msg)
            .context("storage default_format cannot be empty")?;

        for (name, instrument) in &self.instruments {
            self.validate_instrument(name, instrument)?;
        }

        Ok(())
    }

    fn validate_instrument(&self, name: &str, instrument: &toml::Value) -> Result<()> {
        if let Some(resource_string) = instrument.get("resource_string").and_then(|v| v.as_str()) {
            if resource_string.starts_with("TCPIP") {
                let parts: Vec<&str> = resource_string.split("::").collect();
                if parts.len() >= 2 {
                    let ip_address = parts[1];
                    is_valid_ip(ip_address)
                        .map_err(anyhow::Error::msg)
                        .with_context(|| {
                            format!("Invalid IP address for {}: {}", name, ip_address)
                        })?;
                }
            }
        }

        if let Some(sample_rate) = instrument.get("sample_rate_hz").and_then(|v| v.as_float()) {
            is_in_range(sample_rate, 0.1..=1_000_000.0)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("Invalid sample_rate_hz for {}", name))?;
        }

        if let Some(num_samples) = instrument.get("num_samples").and_then(|v| v.as_integer()) {
            is_in_range(num_samples, 1..=1_000_000)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("Invalid num_samples for {}", name))?;
        }

        if let Some(address) = instrument.get("address").and_then(|v| v.as_str()) {
            is_valid_ip(address)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("Invalid IP address for {}", name))?;
        }

        if let Some(port) = instrument.get("port").and_then(|v| v.as_integer()) {
            is_valid_port(port as u16)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("Invalid port for {}", name))?;
        }

        Ok(())
    }
}
