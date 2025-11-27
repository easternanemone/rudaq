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
pub mod dependencies;
use anyhow::{Context, Result};
use config::Config;
use figment::{providers::Serialized, Figment, Provider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod versioning;

impl Provider for Settings {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Library Defaults")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        Serialized::defaults(Settings::default()).data()
    }
}

/// Configuration for V3 instruments (Phase 3)
///
/// Matches the structure used in InstrumentManagerV3 for consistency.
/// The `type` field in TOML maps to `type_name` in code.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstrumentConfigV3 {
    /// Unique identifier for this instrument instance
    pub id: String,

    /// Instrument type name (must match factory registry key)
    #[serde(rename = "type")]
    pub type_name: String,

    /// Type-specific configuration settings
    ///
    /// Captures all extra TOML fields for flexible per-instrument config.
    /// Each instrument factory is responsible for parsing its own settings.
    #[serde(flatten)]
    pub settings: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    pub log_level: String,
    pub application: ApplicationSettings,
    pub storage: StorageSettings,
    pub instruments: HashMap<String, toml::Value>,
    pub processors: Option<HashMap<String, Vec<ProcessorConfig>>>,

    /// V3 instruments configuration (Phase 3)
    ///
    /// Backward compatible: missing [[instruments_v3]] sections result in empty vec
    #[serde(default)]
    pub instruments_v3: Vec<InstrumentConfigV3>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            application: ApplicationSettings::default(),
            storage: StorageSettings::default(),
            instruments: HashMap::new(),
            processors: None,
            instruments_v3: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ApplicationSettings {
    pub broadcast_channel_capacity: usize,
    pub command_channel_capacity: usize,
    pub data_distributor: DataDistributorSettings,
    pub timeouts: TimeoutSettings,
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            broadcast_channel_capacity: default_broadcast_capacity(),
            command_channel_capacity: default_command_capacity(),
            data_distributor: DataDistributorSettings::default(),
            timeouts: TimeoutSettings::default(),
        }
    }
}

fn default_broadcast_capacity() -> usize {
    1024
}

fn default_command_capacity() -> usize {
    32
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProcessorConfig {
    pub r#type: String,
    #[serde(flatten)]
    pub config: toml::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct StorageSettings {
    pub default_path: String,
    pub default_format: String,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            default_path: "./data".to_string(),
            default_format: "hdf5".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct DataDistributorSettings {
    pub subscriber_capacity: usize,
    pub warn_drop_rate_percent: f64,
    pub error_saturation_percent: f64,
    pub metrics_window_secs: u64,
}

impl Default for DataDistributorSettings {
    fn default() -> Self {
        Self {
            subscriber_capacity: default_broadcast_capacity(),
            warn_drop_rate_percent: 1.0,
            error_saturation_percent: 90.0,
            metrics_window_secs: 10,
        }
    }
}

/// Timeout configuration for system operations.
///
/// Each timeout is stored in milliseconds for ease of configuration and
/// alignment with other numeric settings. Defaults match the historical
/// hardcoded values to preserve existing behavior.
///
/// # Driver Timeout Usage
///
/// Different drivers use different timeout fields based on their communication method:
///
/// - **Serial Instruments** (MaiTai, Newport, ESP300, ELL14):
///   - Use `serial_read_timeout_ms` and `serial_write_timeout_ms` for port operations
///   - Use `scpi_command_timeout_ms` for SCPI command execution (if applicable)
///   - Use `instrument_measurement_timeout_ms` for long-running measurements
///
/// - **Network Instruments** (SCPI over TCP/IP):
///   - Use `network_client_timeout_ms` for socket operations
///   - Use `scpi_command_timeout_ms` for SCPI command execution
///   - Use `instrument_measurement_timeout_ms` for long-running measurements
///
/// - **All Instruments**:
///   - Use `instrument_connect_timeout_ms` during initialization/connection
///   - Use `instrument_shutdown_timeout_ms` during cleanup/disconnection
///
/// # Historical Driver Defaults (Before Configurable Timeouts)
///
/// For reference, these were the hardcoded values in driver implementations:
///
/// - MaiTai: 5000ms (instrument_measurement_timeout_ms)
/// - Newport: 500ms (serial_read_timeout_ms)
/// - ESP300: 5000ms (scpi_command_timeout_ms)
/// - ELL14: 500ms (serial_read_timeout_ms)
///
/// These defaults are now configurable via the `[application.timeouts]` section in config.toml
/// or via environment variables like `RUSTDAQ_APPLICATION__TIMEOUTS__SERIAL_READ_TIMEOUT_MS=2000`.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct TimeoutSettings {
    /// Serial port read timeout in milliseconds (100-30000ms, default: 1000ms).
    ///
    /// Used by serial instruments (MaiTai, Newport, ESP300, ELL14) for blocking read operations.
    /// Increase for slow instruments or noisy serial lines.
    pub serial_read_timeout_ms: u64,

    /// Serial port write timeout in milliseconds (100-30000ms, default: 1000ms).
    ///
    /// Used by serial instruments for blocking write operations.
    /// Increase if experiencing write timeouts on slow serial connections.
    pub serial_write_timeout_ms: u64,

    /// SCPI command execution timeout in milliseconds (500-60000ms, default: 2000ms).
    ///
    /// Used by SCPI-compatible instruments (network and serial) for command execution.
    /// Increase for instruments with slow response times or complex commands.
    pub scpi_command_timeout_ms: u64,

    /// Network client operation timeout in milliseconds (1000-120000ms, default: 5000ms).
    ///
    /// Used by network-connected instruments for TCP/IP socket operations (connect, read, write).
    /// Increase for instruments on slow or unreliable networks.
    pub network_client_timeout_ms: u64,

    /// Network resource cleanup timeout in milliseconds (1000-120000ms, default: 10000ms).
    ///
    /// Used when closing network connections and cleaning up resources.
    /// Increase if experiencing incomplete shutdowns or resource leaks.
    pub network_cleanup_timeout_ms: u64,

    /// Instrument connection/initialization timeout in milliseconds (1000-60000ms, default: 5000ms).
    ///
    /// Used during instrument connect(), initialize(), and initial handshake operations.
    /// Increase for instruments with slow startup or extensive self-test procedures.
    pub instrument_connect_timeout_ms: u64,

    /// Instrument shutdown/disconnect timeout in milliseconds (1000-60000ms, default: 6000ms).
    ///
    /// Used during instrument disconnect() and cleanup operations.
    /// Increase if instruments need time to safely power down or save state.
    pub instrument_shutdown_timeout_ms: u64,

    /// Instrument measurement timeout in milliseconds (1000-60000ms, default: 5000ms).
    ///
    /// Used for long-running measurement operations (e.g., spectroscopy scans, multi-sample acquisitions).
    /// Increase for instruments with slow integration times or large data transfers.
    /// Historical defaults: MaiTai 5000ms, ESP300 5000ms.
    pub instrument_measurement_timeout_ms: u64,
}

impl Default for TimeoutSettings {
    fn default() -> Self {
        Self {
            serial_read_timeout_ms: 1_000,
            serial_write_timeout_ms: 1_000,
            scpi_command_timeout_ms: 2_000,
            network_client_timeout_ms: 5_000,
            network_cleanup_timeout_ms: 10_000,
            instrument_connect_timeout_ms: 5_000,
            instrument_shutdown_timeout_ms: 6_000,
            instrument_measurement_timeout_ms: 5_000,
        }
    }
}

impl TimeoutSettings {
    /// Validate that all timeout values fall within the supported ranges.
    pub fn validate(&self) -> Result<()> {
        validate_timeout_range(
            self.serial_read_timeout_ms,
            100,
            30_000,
            "serial_read_timeout_ms",
        )?;
        validate_timeout_range(
            self.serial_write_timeout_ms,
            100,
            30_000,
            "serial_write_timeout_ms",
        )?;
        validate_timeout_range(
            self.scpi_command_timeout_ms,
            500,
            60_000,
            "scpi_command_timeout_ms",
        )?;
        validate_timeout_range(
            self.network_client_timeout_ms,
            1_000,
            120_000,
            "network_client_timeout_ms",
        )?;
        validate_timeout_range(
            self.network_cleanup_timeout_ms,
            1_000,
            120_000,
            "network_cleanup_timeout_ms",
        )?;
        validate_timeout_range(
            self.instrument_connect_timeout_ms,
            1_000,
            60_000,
            "instrument_connect_timeout_ms",
        )?;
        validate_timeout_range(
            self.instrument_shutdown_timeout_ms,
            1_000,
            60_000,
            "instrument_shutdown_timeout_ms",
        )?;
        validate_timeout_range(
            self.instrument_measurement_timeout_ms,
            1_000,
            60_000,
            "instrument_measurement_timeout_ms",
        )?;

        Ok(())
    }
}

fn validate_timeout_range(value: u64, min: u64, max: u64, name: &str) -> Result<()> {
    if value < min || value > max {
        anyhow::bail!(
            "Timeout '{}' = {}ms is out of valid range ({}ms - {}ms). Check [application.timeouts] in config.",
            name,
            value,
            min,
            max
        );
    }

    Ok(())
}

impl Settings {
    /// Load configuration using Figment (V5 architecture)
    ///
    /// This method implements the layered configuration approach:
    /// 1. Base Layer: Hardcoded defaults from `Settings::default()`
    /// 2. File Layer: config/config.toml (optional, warns if missing)
    /// 3. Environment Layer: Environment variables prefixed with `RUSTDAQ_`
    ///
    /// The layering ensures that every field has a valid value, with each
    /// layer overriding the previous one.
    ///
    /// # Environment Variables
    ///
    /// All configuration fields can be overridden via environment variables using the
    /// `RUSTDAQ_` prefix. Nested fields use double underscores. Examples:
    ///
    /// - `RUSTDAQ_LOG_LEVEL=debug` → sets `log_level`
    /// - `RUSTDAQ_APPLICATION__TIMEOUTS__SERIAL_READ_TIMEOUT_MS=2000` → sets `application.timeouts.serial_read_timeout_ms`
    /// - `RUSTDAQ_STORAGE__DEFAULT_PATH=/data` → sets `storage.default_path`
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to a config file. If None, uses "config/config.toml"
    pub fn load_v5(config_path: Option<std::path::PathBuf>) -> Result<Self> {
        use figment::providers::{Env, Format, Toml};

        // Layer 1: Start with defaults
        // The Provider trait implementation allows Settings to act as a data source
        let mut figment = Figment::from(Settings::default());

        // Layer 2: Config File (optional)
        // Priority: Explicit path > config.toml > skip if neither exists
        let file_path = config_path.unwrap_or_else(|| "config/config.toml".into());

        if file_path.exists() {
            figment = figment.merge(Toml::file(&file_path));
        } else {
            // Warn but don't fail - defaults are sufficient
            eprintln!(
                "⚠️  Config file not found: {}. Using defaults.",
                file_path.display()
            );
        }

        // Layer 3: Environment Variables
        // Prefix: RUSTDAQ_
        // Nested fields use double underscores: RUSTDAQ_APPLICATION__TIMEOUTS__SERIAL_READ_TIMEOUT_MS
        figment = figment.merge(Env::prefixed("RUSTDAQ_").split("__"));

        // Extract (deserialize) the final configuration
        let settings: Settings = figment
            .extract()
            .context("Failed to extract configuration from Figment")?;

        // Validate the configuration
        settings
            .validate()
            .context("Configuration validation failed")?;

        Ok(settings)
    }

    /// Load configuration using the legacy config crate (backward compatibility)
    ///
    /// This method is deprecated and will be removed once the Figment migration is complete.
    /// Use `load_v5()` for new code.
    pub fn new(config_name: Option<&str>) -> Result<Self> {
        Self::new_legacy(config_name)
    }

    pub fn new_legacy(config_name: Option<&str>) -> Result<Self> {
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
        // Check for ID collisions between V1 and V3 instruments
        use std::collections::HashSet;
        let mut all_ids = HashSet::new();

        // Check V1 instrument IDs
        for id in self.instruments.keys() {
            if !all_ids.insert(id) {
                anyhow::bail!("Duplicate instrument ID: {}", id);
            }
        }

        // Check V3 instrument IDs
        for inst_v3 in &self.instruments_v3 {
            if !all_ids.insert(&inst_v3.id) {
                anyhow::bail!("Duplicate instrument ID (V3): {}", inst_v3.id);
            }
        }

        is_not_empty(&self.log_level)
            .map_err(anyhow::Error::msg)
            .context("log_level cannot be empty")?;
        let valid_log_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_log_levels.contains(&self.log_level.to_lowercase().as_str()) {
            anyhow::bail!("Invalid log level: {}", self.log_level);
        }

        self.application
            .timeouts
            .validate()
            .context("Invalid timeout configuration")?;

        // Validate channel capacities
        is_in_range(self.application.broadcast_channel_capacity, 64..=65536)
            .map_err(anyhow::Error::msg)
            .context("broadcast_channel_capacity must be between 64 and 65536")?;
        is_in_range(self.application.command_channel_capacity, 8..=4096)
            .map_err(anyhow::Error::msg)
            .context("command_channel_capacity must be between 8 and 4096")?;

        let distributor = &self.application.data_distributor;
        is_in_range(distributor.subscriber_capacity, 64..=65536)
            .map_err(anyhow::Error::msg)
            .context("data_distributor.subscriber_capacity must be between 64 and 65536")?;
        if !(0.0..=100.0).contains(&distributor.warn_drop_rate_percent) {
            anyhow::bail!("data_distributor.warn_drop_rate_percent must be between 0 and 100");
        }
        if !(0.0..=100.0).contains(&distributor.error_saturation_percent) {
            anyhow::bail!("data_distributor.error_saturation_percent must be between 0 and 100");
        }
        is_in_range(distributor.metrics_window_secs, 1..=3600)
            .map_err(anyhow::Error::msg)
            .context("data_distributor.metrics_window_secs must be between 1 and 3600")?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_load_v5_with_defaults() {
        // Test that load_v5() successfully loads with default values
        // Use a non-existent path to ensure we're testing defaults-only
        let settings = Settings::load_v5(Some("config/nonexistent.toml".into()));
        // Will warn about missing config file, but should succeed with defaults
        if let Err(ref e) = settings {
            eprintln!("Error loading settings: {:#}", e);
        }
        assert!(settings.is_ok());

        let settings = settings.unwrap();
        assert_eq!(settings.log_level, "info");
        assert_eq!(settings.storage.default_path, "./data");
        assert_eq!(settings.storage.default_format, "hdf5");
        assert_eq!(settings.application.broadcast_channel_capacity, 1024);
        assert_eq!(settings.application.command_channel_capacity, 32);

        // Verify timeouts are set to defaults
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 1_000);
        assert_eq!(settings.application.timeouts.scpi_command_timeout_ms, 2_000);
        assert_eq!(
            settings.application.timeouts.instrument_connect_timeout_ms,
            5_000
        );
    }

    #[test]
    fn test_load_v5_validates_defaults() {
        // Test that load_v5() validates the configuration
        // Since we're using defaults, this should always pass
        let settings = Settings::load_v5(Some("config/nonexistent.toml".into()));
        if let Err(ref e) = settings {
            eprintln!("Error loading settings: {:#}", e);
        }
        assert!(settings.is_ok());

        // Verify that the returned settings pass validation
        let settings = settings.unwrap();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_parse_instruments_v3() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024
            command_channel_capacity = 32

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]

            [[instruments_v3]]
            id = "test_pm"
            type = "MockPowerMeterV3"
            sampling_rate = 10.0
            wavelength_nm = 532.0

            [[instruments_v3]]
            id = "test_stage"
            type = "MockStageV3"
            axis = "x"
            range_mm = 100.0
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.instruments_v3.len(), 2);
        assert_eq!(settings.instruments_v3[0].id, "test_pm");
        assert_eq!(settings.instruments_v3[0].type_name, "MockPowerMeterV3");
        assert_eq!(settings.instruments_v3[1].id, "test_stage");
        assert_eq!(settings.instruments_v3[1].type_name, "MockStageV3");

        // Verify settings captured extra fields
        let pm_settings = &settings.instruments_v3[0].settings;
        assert!(pm_settings.get("sampling_rate").is_some());
        assert!(pm_settings.get("wavelength_nm").is_some());
    }

    #[test]
    fn test_empty_instruments_v3() {
        // Test backward compatibility - config without [[instruments_v3]] should work
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024
            command_channel_capacity = 32

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.instruments_v3.len(), 0);
    }

    #[test]
    fn test_duplicate_id_v1_v3_fails() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024
            command_channel_capacity = 32

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments.duplicate_id]
            type = "mock"

            [[instruments_v3]]
            id = "duplicate_id"
            type = "MockPowerMeterV3"
            sampling_rate = 10.0
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        let result = settings.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Duplicate instrument ID"));
    }

    #[test]
    fn test_serial_read_timeout_too_short() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 50;
        let result = settings.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("serial_read_timeout_ms"));
        assert!(err_msg.contains("50ms"));
        assert!(err_msg.contains("100ms - 30000ms"));
    }

    #[test]
    fn test_serial_read_timeout_too_long() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 40_000;
        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("serial_read_timeout_ms"));
    }

    #[test]
    fn test_serial_read_timeout_valid_range() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 100;
        assert!(settings.validate().is_ok());
        settings.serial_read_timeout_ms = 30_000;
        assert!(settings.validate().is_ok());
        settings.serial_read_timeout_ms = 5_000;
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_scpi_command_timeout_too_short() {
        let mut settings = TimeoutSettings::default();
        settings.scpi_command_timeout_ms = 400;
        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("scpi_command_timeout_ms"));
    }

    #[test]
    fn test_scpi_command_timeout_valid_range() {
        let mut settings = TimeoutSettings::default();
        settings.scpi_command_timeout_ms = 500;
        assert!(settings.validate().is_ok());
        settings.scpi_command_timeout_ms = 60_000;
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_network_timeouts_valid_range() {
        let mut settings = TimeoutSettings::default();
        settings.network_client_timeout_ms = 1_000;
        settings.network_cleanup_timeout_ms = 1_000;
        assert!(settings.validate().is_ok());
        settings.network_client_timeout_ms = 120_000;
        settings.network_cleanup_timeout_ms = 120_000;
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_instrument_lifecycle_timeouts_valid_range() {
        let mut settings = TimeoutSettings::default();
        settings.instrument_connect_timeout_ms = 1_000;
        settings.instrument_shutdown_timeout_ms = 1_000;
        settings.instrument_measurement_timeout_ms = 1_000;
        assert!(settings.validate().is_ok());
        settings.instrument_connect_timeout_ms = 60_000;
        settings.instrument_shutdown_timeout_ms = 60_000;
        settings.instrument_measurement_timeout_ms = 60_000;
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_default_timeouts_are_valid() {
        let settings = TimeoutSettings::default();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_missing_timeout_section_uses_defaults() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024
            command_channel_capacity = 32

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 1_000);
        assert_eq!(settings.application.timeouts.scpi_command_timeout_ms, 2_000);
        assert_eq!(
            settings.application.timeouts.network_client_timeout_ms,
            5_000
        );
        assert_eq!(
            settings.application.timeouts.instrument_connect_timeout_ms,
            5_000
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_partial_timeout_section() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024
            command_channel_capacity = 32

            [application.timeouts]
            serial_read_timeout_ms = 5000

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 5_000);
        assert_eq!(settings.application.timeouts.serial_write_timeout_ms, 1_000);
        assert_eq!(settings.application.timeouts.scpi_command_timeout_ms, 2_000);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_empty_timeout_section_uses_defaults() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        let defaults = TimeoutSettings::default();
        assert_eq!(
            settings.application.timeouts.serial_read_timeout_ms,
            defaults.serial_read_timeout_ms
        );
        assert_eq!(
            settings.application.timeouts.scpi_command_timeout_ms,
            defaults.scpi_command_timeout_ms
        );
    }

    #[test]
    fn test_custom_timeouts_load_correctly() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]
            serial_read_timeout_ms = 3000
            serial_write_timeout_ms = 2500
            scpi_command_timeout_ms = 8000
            network_client_timeout_ms = 15000
            network_cleanup_timeout_ms = 20000
            instrument_connect_timeout_ms = 10000
            instrument_shutdown_timeout_ms = 12000
            instrument_measurement_timeout_ms = 25000

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();

        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 3_000);
        assert_eq!(settings.application.timeouts.serial_write_timeout_ms, 2_500);
        assert_eq!(settings.application.timeouts.scpi_command_timeout_ms, 8_000);
        assert_eq!(
            settings.application.timeouts.network_client_timeout_ms,
            15_000
        );
        assert_eq!(
            settings.application.timeouts.network_cleanup_timeout_ms,
            20_000
        );
        assert_eq!(
            settings.application.timeouts.instrument_connect_timeout_ms,
            10_000
        );
        assert_eq!(
            settings.application.timeouts.instrument_shutdown_timeout_ms,
            12_000
        );
        assert_eq!(
            settings
                .application
                .timeouts
                .instrument_measurement_timeout_ms,
            25_000
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_invalid_timeout_fails_config_load() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]
            serial_read_timeout_ms = 50

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        let result = settings.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = format!("{:#}", err);
        assert!(err_msg.contains("serial_read_timeout_ms"));
        assert!(err_msg.contains("50ms"));
        assert!(err_msg.contains("100ms - 30000ms"));
    }

    #[test]
    fn test_timeout_conversion_to_duration() {
        let settings = TimeoutSettings::default();
        let serial_timeout = Duration::from_millis(settings.serial_read_timeout_ms);
        assert_eq!(serial_timeout, Duration::from_secs(1));
        let scpi_timeout = Duration::from_millis(settings.scpi_command_timeout_ms);
        assert_eq!(scpi_timeout, Duration::from_secs(2));
        let connect_timeout = Duration::from_millis(settings.instrument_connect_timeout_ms);
        assert_eq!(connect_timeout, Duration::from_secs(5));
    }

    #[test]
    fn test_settings_new_with_valid_config() {
        let settings = Settings::new(Some("default"));
        assert!(settings.is_ok());
        if let Ok(settings) = settings {
            assert!(
                settings.application.timeouts.serial_read_timeout_ms >= 100
                    && settings.application.timeouts.serial_read_timeout_ms <= 30_000
            );
        }
    }

    #[test]
    fn test_timeout_at_exact_boundaries() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 100;
        settings.scpi_command_timeout_ms = 500;
        settings.network_client_timeout_ms = 1_000;
        settings.instrument_connect_timeout_ms = 1_000;
        assert!(settings.validate().is_ok());

        settings.serial_read_timeout_ms = 30_000;
        settings.scpi_command_timeout_ms = 60_000;
        settings.network_client_timeout_ms = 120_000;
        settings.instrument_connect_timeout_ms = 60_000;
        assert!(settings.validate().is_ok());

        settings.serial_read_timeout_ms = 99;
        assert!(settings.validate().is_err());
        settings.serial_read_timeout_ms = 30_001;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn test_multiple_invalid_timeouts() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 50;
        settings.scpi_command_timeout_ms = 400;
        settings.network_client_timeout_ms = 500;
        let result = settings.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("serial_read_timeout_ms")
                || err_msg.contains("scpi_command_timeout_ms")
                || err_msg.contains("network_client_timeout_ms")
        );
    }

    #[test]
    fn test_zero_timeout_fails_validation() {
        let mut settings = TimeoutSettings::default();
        settings.serial_read_timeout_ms = 0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn test_slow_spectrometer_config() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]
            instrument_measurement_timeout_ms = 35000
            scpi_command_timeout_ms = 10000

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(
            settings
                .application
                .timeouts
                .instrument_measurement_timeout_ms,
            35_000
        );
        assert_eq!(
            settings.application.timeouts.scpi_command_timeout_ms,
            10_000
        );
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 1_000);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_debug_mode_long_timeouts() {
        let toml_content = r#"
            log_level = "debug"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]
            serial_read_timeout_ms = 30000
            serial_write_timeout_ms = 30000
            scpi_command_timeout_ms = 60000
            network_client_timeout_ms = 60000
            instrument_connect_timeout_ms = 60000
            instrument_shutdown_timeout_ms = 60000
            instrument_measurement_timeout_ms = 60000

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 30_000);
        assert_eq!(
            settings.application.timeouts.instrument_connect_timeout_ms,
            60_000
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_fast_mock_instruments_config() {
        let toml_content = r#"
            log_level = "info"

            [application]
            broadcast_channel_capacity = 1024

            [application.timeouts]
            serial_read_timeout_ms = 200
            instrument_connect_timeout_ms = 1000
            instrument_measurement_timeout_ms = 1000

            [storage]
            default_path = "./data"
            default_format = "csv"

            [instruments]
        "#;

        let settings: Settings = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.application.timeouts.serial_read_timeout_ms, 200);
        assert_eq!(
            settings.application.timeouts.instrument_connect_timeout_ms,
            1_000
        );
        assert!(settings.validate().is_ok());
    }
}
