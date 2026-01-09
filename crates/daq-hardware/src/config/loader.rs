//! Configuration loading utilities for device protocol definitions.
//!
//! This module provides functions to load device configurations from TOML files.
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_hardware::config::loader::{load_device_config, load_all_devices};
//! use std::path::Path;
//!
//! // Load a single device config
//! let config = load_device_config(Path::new("config/devices/ell14.toml"))?;
//!
//! // Load all device configs from a directory
//! let devices = load_all_devices(Path::new("config/devices/"))?;
//! ```

use super::schema::DeviceConfig;
use super::validation::validate_device_config;
use anyhow::{Context, Result};
use figment::{
    providers::{Format, Toml},
    Figment,
};
use serde_valid::Validate;
use std::path::Path;
use tracing::{debug, info, warn};

/// Error types for config loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    /// File not found
    #[error("Config file not found: {0}")]
    NotFound(String),

    /// File read error
    #[error("Failed to read config file: {0}")]
    ReadError(String),

    /// Parse error (invalid TOML)
    #[error("Failed to parse config: {0}")]
    ParseError(String),

    /// Validation error
    #[error("Config validation failed: {0}")]
    ValidationError(String),

    /// Schema validation error
    #[error("Schema validation failed:\n{0}")]
    SchemaValidationError(String),
}

/// Load a device configuration from a TOML file.
///
/// This function:
/// 1. Reads the TOML file
/// 2. Deserializes it into a `DeviceConfig`
/// 3. Validates the configuration (regex patterns, formulas, ranges)
///
/// # Arguments
///
/// * `path` - Path to the TOML configuration file
///
/// # Returns
///
/// * `Ok(DeviceConfig)` if loading and validation succeed
/// * `Err` if file cannot be read, parsed, or validation fails
///
/// # Example
///
/// ```rust,ignore
/// let config = load_device_config(Path::new("config/devices/ell14.toml"))?;
/// println!("Loaded device: {}", config.device.name);
/// ```
pub fn load_device_config(path: &Path) -> Result<DeviceConfig> {
    // Check file exists
    if !path.exists() {
        return Err(ConfigLoadError::NotFound(path.display().to_string()).into());
    }

    debug!("Loading device config from: {}", path.display());

    // Use Figment for flexible config loading
    let figment = Figment::new().merge(Toml::file(path));

    // Extract and deserialize
    let config: DeviceConfig = figment
        .extract()
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    // Run serde_valid validation
    if let Err(errors) = config.validate() {
        let error_messages: Vec<String> = errors
            .to_string()
            .lines()
            .map(|s| s.to_string())
            .collect();
        return Err(ConfigLoadError::SchemaValidationError(error_messages.join("\n")).into());
    }

    // Run custom cross-field validation
    if let Err(errors) = validate_device_config(&config) {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        return Err(ConfigLoadError::ValidationError(error_messages.join("\n")).into());
    }

    info!(
        "Loaded device config: {} ({})",
        config.device.name, config.device.protocol
    );

    Ok(config)
}

/// Load all device configurations from a directory.
///
/// This function scans a directory for `.toml` files and loads each one
/// as a device configuration. Files that fail to load are logged as warnings
/// but don't stop the loading of other files.
///
/// # Arguments
///
/// * `dir` - Path to the directory containing TOML configuration files
///
/// # Returns
///
/// * `Ok(Vec<DeviceConfig>)` - Vector of successfully loaded configs
/// * `Err` if the directory cannot be read
///
/// # Example
///
/// ```rust,ignore
/// let devices = load_all_devices(Path::new("config/devices/"))?;
/// for device in &devices {
///     println!("Found device: {}", device.device.name);
/// }
/// ```
pub fn load_all_devices(dir: &Path) -> Result<Vec<DeviceConfig>> {
    if !dir.exists() {
        return Err(ConfigLoadError::NotFound(dir.display().to_string()).into());
    }

    if !dir.is_dir() {
        return Err(anyhow::anyhow!(
            "Path is not a directory: {}",
            dir.display()
        ));
    }

    debug!("Loading all device configs from: {}", dir.display());

    let mut configs = Vec::new();

    let entries =
        std::fs::read_dir(dir).with_context(|| format!("Failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Only process .toml files
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }

        match load_device_config(&path) {
            Ok(config) => {
                configs.push(config);
            }
            Err(e) => {
                warn!("Failed to load config {}: {}", path.display(), e);
            }
        }
    }

    info!("Loaded {} device configurations from {}", configs.len(), dir.display());

    Ok(configs)
}

/// Load a device configuration from a TOML string.
///
/// Useful for testing or loading configs from embedded resources.
///
/// # Arguments
///
/// * `toml_content` - TOML configuration as a string
///
/// # Returns
///
/// * `Ok(DeviceConfig)` if parsing and validation succeed
/// * `Err` if parsing or validation fails
pub fn load_device_config_from_str(toml_content: &str) -> Result<DeviceConfig> {
    // Parse TOML directly
    let config: DeviceConfig =
        toml::from_str(toml_content).with_context(|| "Failed to parse TOML content")?;

    // Run serde_valid validation
    if let Err(errors) = config.validate() {
        let error_messages: Vec<String> = errors
            .to_string()
            .lines()
            .map(|s| s.to_string())
            .collect();
        return Err(ConfigLoadError::SchemaValidationError(error_messages.join("\n")).into());
    }

    // Run custom cross-field validation
    if let Err(errors) = validate_device_config(&config) {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        return Err(ConfigLoadError::ValidationError(error_messages.join("\n")).into());
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_VALID_CONFIG: &str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
"#;

    const FULL_ELL14_EXAMPLE: &str = r#"
[device]
name = "Thorlabs ELL14"
description = "Elliptec rotation mount with RS-485 multidrop support"
manufacturer = "Thorlabs"
model = "ELL14"
protocol = "elliptec"
category = "stage"
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
timeout_ms = 1000
terminator_tx = ""
terminator_rx = "\r\n"

[connection.bus]
type = "rs485"
address_format = "hex_char"
default_address = "0"

[parameters.address]
type = "string"
default = "0"
description = "Device address on RS-485 bus (0-F)"

[parameters.pulses_per_degree]
type = "float"
default = 398.2222
description = "Calibration factor"

[parameters.position_deg]
type = "float"
default = 0.0
range = [0.0, 360.0]
unit = "degrees"

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
description = "Move to absolute position"
parameters = { position_pulses = "int32" }

[commands.get_position]
template = "${address}gp"
description = "Query current position"
response = "position"

[commands.home]
template = "${address}ho0"
description = "Home to mechanical zero"

[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$"

[responses.position.fields.addr]
type = "string"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"
"#;

    #[test]
    fn test_load_minimal_config() {
        let config = load_device_config_from_str(MINIMAL_VALID_CONFIG).unwrap();
        assert_eq!(config.device.name, "Test Device");
        assert_eq!(config.device.protocol, "test");
    }

    #[test]
    fn test_load_full_ell14_config() {
        let config = load_device_config_from_str(FULL_ELL14_EXAMPLE).unwrap();
        assert_eq!(config.device.name, "Thorlabs ELL14");
        assert_eq!(config.device.manufacturer, "Thorlabs");
        assert_eq!(config.device.protocol, "elliptec");
        assert_eq!(config.connection.baud_rate, 9600);
        assert!(config.commands.contains_key("move_absolute"));
        assert!(config.commands.contains_key("get_position"));
        assert!(config.responses.contains_key("position"));
        assert!(config.conversions.contains_key("degrees_to_pulses"));
    }

    #[test]
    fn test_invalid_regex_rejected() {
        let invalid_config = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"

[responses]
bad_pattern = { pattern = "[" }
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("regex") || err.contains("pattern"),
            "Error should mention regex issue: {}",
            err
        );
    }

    #[test]
    fn test_invalid_formula_rejected() {
        // Use a truly invalid formula - unclosed parenthesis
        let invalid_config = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"

[conversions]
bad_formula = { formula = "round(" }
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("formula") || err.contains("Invalid") || err.contains("round("),
            "Error should mention formula issue: {}",
            err
        );
    }

    #[test]
    fn test_out_of_range_baud_rate_rejected() {
        let invalid_config = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
baud_rate = 2000000
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(
            result.is_err(),
            "Should reject baud rate > 921600, got: {:?}",
            result
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("baud") || err.contains("921600") || err.contains("maximum"),
            "Error should mention baud rate issue: {}",
            err
        );
    }

    #[test]
    fn test_out_of_range_timeout_rejected() {
        let invalid_config = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 100000
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(
            result.is_err(),
            "Should reject timeout > 60000, got: {:?}",
            result
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timeout") || err.contains("60000") || err.contains("maximum"),
            "Error should mention timeout issue: {}",
            err
        );
    }

    #[test]
    fn test_missing_required_fields_rejected() {
        // Missing 'name' field in [device]
        let invalid_config = r#"
[device]
protocol = "test"

[connection]
type = "serial"
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(
            result.is_err(),
            "Should reject missing 'name' field: {:?}",
            result
        );
        // The error should be from TOML parsing - just verify we get an error
        // The exact error message format varies by toml version
    }

    #[test]
    fn test_command_references_nonexistent_response() {
        let invalid_config = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"

[commands]
get_value = { template = "GV", response = "nonexistent_response" }
"#;
        let result = load_device_config_from_str(invalid_config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent_response"),
            "Error should mention missing response: {}",
            err
        );
    }

    #[test]
    fn test_default_values() {
        let config = load_device_config_from_str(MINIMAL_VALID_CONFIG).unwrap();

        // Check connection defaults
        assert_eq!(config.connection.baud_rate, 9600);
        assert_eq!(config.connection.data_bits, 8);
        assert_eq!(config.connection.stop_bits, 1);
        assert_eq!(config.connection.timeout_ms, 1000);
        assert_eq!(config.connection.terminator_rx, "\r\n");

        // Check empty collections
        assert!(config.parameters.is_empty());
        assert!(config.commands.is_empty());
        assert!(config.responses.is_empty());
    }
}
