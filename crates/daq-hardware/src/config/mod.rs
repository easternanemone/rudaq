//! Declarative device configuration module.
//!
//! This module provides TOML-based device protocol definitions that enable
//! config-driven hardware drivers without code changes.
//!
//! # Architecture
//!
//! The configuration system consists of three layers:
//!
//! 1. **Schema** - Rust types representing device protocol definitions
//! 2. **Validation** - Custom validators for regex patterns, formulas, and cross-field rules
//! 3. **Loader** - Functions to load and validate configurations from TOML files
//!
//! # Schema Overview
//!
//! A device configuration consists of several sections:
//!
//! - `[device]` - Device identity (name, manufacturer, model, protocol, capabilities)
//! - `[connection]` - Serial/network settings (baud rate, parity, timeout)
//! - `[parameters]` - Device-specific parameters with types and defaults
//! - `[commands]` - Command templates with placeholders
//! - `[responses]` - Response parsing patterns (regex, delimiter, fixed)
//! - `[conversions]` - Unit conversion formulas (evalexpr syntax)
//! - `[error_codes]` - Error code definitions (optional)
//! - `[validation]` - Parameter validation rules
//! - `[trait_mapping]` - Maps capability traits to commands
//!
//! # Example Configuration
//!
//! ```toml
//! [device]
//! name = "Thorlabs ELL14"
//! manufacturer = "Thorlabs"
//! model = "ELL14"
//! protocol = "elliptec"
//! category = "stage"
//! capabilities = ["Movable", "Parameterized"]
//!
//! [connection]
//! type = "serial"
//! baud_rate = 9600
//! timeout_ms = 1000
//! terminator_rx = "\r\n"
//!
//! [connection.bus]
//! type = "rs485"
//! address_format = "hex_char"
//! default_address = "0"
//!
//! [parameters]
//! address = { type = "string", default = "0", description = "Device address" }
//! pulses_per_degree = { type = "float", default = 398.2222 }
//!
//! [commands]
//! move_absolute = { template = "${address}ma${position:08X}" }
//! get_position = { template = "${address}gp", response = "position" }
//!
//! [responses]
//! position = { pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$" }
//!
//! [responses.position.fields]
//! addr = { type = "string" }
//! pulses = { type = "hex_i32", signed = true }
//!
//! [conversions]
//! degrees_to_pulses = { formula = "round(degrees * pulses_per_degree)" }
//! pulses_to_degrees = { formula = "pulses / pulses_per_degree" }
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_hardware::config::loader::load_device_config;
//! use std::path::Path;
//!
//! // Load and validate a device configuration
//! let config = load_device_config(Path::new("config/devices/ell14.toml"))?;
//!
//! // Access configuration data
//! println!("Device: {}", config.device.name);
//! println!("Baud rate: {}", config.connection.baud_rate);
//!
//! // List supported commands
//! for (name, cmd) in &config.commands {
//!     println!("Command {}: {}", name, cmd.template);
//! }
//! ```
//!
//! # JSON Schema Generation
//!
//! The schema types derive `JsonSchema` from the `schemars` crate,
//! enabling JSON Schema generation for IDE support:
//!
//! ```rust,ignore
//! use daq_hardware::config::schema::DeviceConfig;
//! use schemars::schema_for;
//!
//! let schema = schema_for!(DeviceConfig);
//! let json = serde_json::to_string_pretty(&schema)?;
//! std::fs::write("config/schemas/device.schema.json", json)?;
//! ```

pub mod loader;
pub mod schema;
pub mod validation;

// Re-exports for convenience
pub use loader::{load_all_devices, load_device_config, load_device_config_from_str};
pub use schema::{
    AddressFormat, BusConfig, BusType, CapabilityType, CommandConfig, CommandParameterType,
    ConnectionConfig, ConnectionType, ConversionConfig, DeviceCategory, DeviceConfig,
    DeviceIdentity, ErrorCodeConfig, FieldType, FlowControlSetting, ParameterConfig,
    ParameterType, ParitySetting, ResponseConfig, ResponseFieldConfig, TraitMappingConfig,
    TraitMethodMapping, ValidationRuleConfig,
};

/// Generate JSON schema for DeviceConfig.
///
/// This can be used to create a schema file for IDE autocompletion.
///
/// # Example
///
/// ```rust,ignore
/// let schema_json = daq_hardware::config::generate_json_schema()?;
/// std::fs::write("config/schemas/device.schema.json", schema_json)?;
/// ```
pub fn generate_json_schema() -> Result<String, serde_json::Error> {
    let schema = schemars::schema_for!(DeviceConfig);
    serde_json::to_string_pretty(&schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_json_schema() {
        let schema = generate_json_schema().unwrap();
        assert!(schema.contains("DeviceConfig"));
        assert!(schema.contains("device"));
        assert!(schema.contains("connection"));
        assert!(schema.contains("parameters"));
        assert!(schema.contains("commands"));
        assert!(schema.contains("responses"));
        // Verify it's valid JSON
        let _: serde_json::Value = serde_json::from_str(&schema).unwrap();
    }

    /// Write the JSON schema to a file.
    ///
    /// Run with: `cargo test -p daq-hardware write_json_schema_file -- --ignored --nocapture`
    #[test]
    #[ignore = "Run manually to regenerate schema file"]
    fn write_json_schema_file() {
        let schema = generate_json_schema().unwrap();

        // Find workspace root by looking for Cargo.toml with [workspace]
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // Go up from crates/daq-hardware to workspace root
        path.pop();
        path.pop();

        let schema_dir = path.join("config").join("schemas");
        std::fs::create_dir_all(&schema_dir).unwrap();

        let schema_path = schema_dir.join("device.schema.json");
        std::fs::write(&schema_path, &schema).unwrap();
        println!("Wrote JSON schema to: {}", schema_path.display());
    }
}
