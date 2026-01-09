//! Schema definitions for declarative device configuration.
//!
//! This module defines the Rust types for TOML-based device protocol definitions.
//! These schemas enable config-driven hardware drivers without code changes.
//!
//! # Schema Structure
//!
//! ```toml
//! [device]           # Device identity and metadata
//! [connection]       # Communication settings
//! [parameters]       # Device-specific parameters
//! [commands]         # Command definitions
//! [responses]        # Response parsing definitions
//! [conversions]      # Unit conversion formulas
//! [error_codes]      # Error code mapping (optional)
//! [validation]       # Validation rules
//! [trait_mapping]    # Maps trait methods to commands
//! ```
//!
//! # Example
//!
//! See `config/devices/ell14.toml` for a complete example.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_valid::Validate;
use std::collections::HashMap;

use super::validation::{validate_evalexpr_formula, validate_regex_pattern};

// =============================================================================
// Top-Level Config
// =============================================================================

/// Complete device configuration loaded from TOML.
///
/// This is the top-level struct that contains all device protocol definitions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    /// Device identity and metadata
    #[validate]
    pub device: DeviceIdentity,

    /// Connection/communication settings
    #[validate]
    pub connection: ConnectionConfig,

    /// Device-specific parameters
    #[serde(default)]
    pub parameters: HashMap<String, ParameterConfig>,

    /// Command definitions
    #[serde(default)]
    pub commands: HashMap<String, CommandConfig>,

    /// Response parsing definitions
    #[serde(default)]
    pub responses: HashMap<String, ResponseConfig>,

    /// Unit conversion formulas
    #[serde(default)]
    pub conversions: HashMap<String, ConversionConfig>,

    /// Error code mapping (optional)
    #[serde(default)]
    pub error_codes: HashMap<String, ErrorCodeConfig>,

    /// Validation rules for parameters
    #[serde(default)]
    pub validation: HashMap<String, ValidationRuleConfig>,

    /// Trait method to command mapping
    #[serde(default)]
    pub trait_mapping: HashMap<String, TraitMappingConfig>,
}

// =============================================================================
// Device Identity
// =============================================================================

/// Device identity and metadata.
///
/// Defines what the device is and what capabilities it supports.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    /// Human-readable device name
    #[validate(min_length = 1)]
    #[validate(max_length = 100)]
    pub name: String,

    /// Device description
    #[serde(default)]
    #[validate(max_length = 500)]
    pub description: String,

    /// Manufacturer name
    #[serde(default)]
    #[validate(max_length = 100)]
    pub manufacturer: String,

    /// Model number/name
    #[serde(default)]
    #[validate(max_length = 100)]
    pub model: String,

    /// Protocol identifier (e.g., "elliptec", "scpi", "esp300")
    #[validate(min_length = 1)]
    #[validate(max_length = 50)]
    pub protocol: String,

    /// Device category
    #[serde(default)]
    pub category: DeviceCategory,

    /// List of capability traits this device implements
    #[serde(default)]
    pub capabilities: Vec<CapabilityType>,
}

/// Device category classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCategory {
    /// Motion control stages, rotators, actuators
    Stage,
    /// Power meters, thermometers, voltmeters
    Sensor,
    /// Lasers, LEDs, lamps
    Source,
    /// Cameras, detectors
    Detector,
    /// Shutters, filter wheels, beam blocks
    Modulator,
    /// Lock-in amplifiers, oscilloscopes, analyzers
    Analyzer,
    /// DAQ boards, digitizers
    DataAcquisition,
    /// Unspecified or custom device
    #[default]
    Other,
}

/// Capability traits that a device can implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum CapabilityType {
    /// Single-axis motion control
    Movable,
    /// Read scalar values
    Readable,
    /// Set parameter values
    Settable,
    /// Control shutter state
    ShutterControl,
    /// Tune wavelength
    WavelengthTunable,
    /// Produce image frames
    FrameProducer,
    /// External triggering
    Triggerable,
    /// Exposure control
    ExposureControl,
    /// Emission control (on/off)
    EmissionControl,
    /// Stage lifecycle (stage/unstage)
    Stageable,
    /// Direct command interface
    Commandable,
    /// Parameter observation/subscription
    Parameterized,
}

// =============================================================================
// Connection Configuration
// =============================================================================

/// Communication settings for the device.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    /// Connection type
    #[serde(rename = "type")]
    pub connection_type: ConnectionType,

    /// Serial baud rate (300-921600)
    #[serde(default = "default_baud_rate")]
    #[validate(minimum = 300)]
    #[validate(maximum = 921600)]
    pub baud_rate: u32,

    /// Data bits (5, 6, 7, or 8)
    #[serde(default = "default_data_bits")]
    #[validate(minimum = 5)]
    #[validate(maximum = 8)]
    pub data_bits: u8,

    /// Parity setting
    #[serde(default)]
    pub parity: ParitySetting,

    /// Stop bits (1 or 2)
    #[serde(default = "default_stop_bits")]
    #[validate(minimum = 1)]
    #[validate(maximum = 2)]
    pub stop_bits: u8,

    /// Flow control setting
    #[serde(default)]
    pub flow_control: FlowControlSetting,

    /// Read/write timeout in milliseconds (1-60000)
    #[serde(default = "default_timeout_ms")]
    #[validate(minimum = 1)]
    #[validate(maximum = 60000)]
    pub timeout_ms: u32,

    /// Command terminator (sent after commands)
    #[serde(default)]
    pub terminator_tx: String,

    /// Response terminator (expected in responses)
    #[serde(default = "default_terminator_rx")]
    pub terminator_rx: String,

    /// Optional serial port path (can be set at runtime)
    #[serde(default)]
    pub port_path: Option<String>,

    /// Bus configuration for multidrop protocols (RS-485)
    #[serde(default)]
    #[validate]
    pub bus: Option<BusConfig>,
}

fn default_baud_rate() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_stop_bits() -> u8 {
    1
}
fn default_timeout_ms() -> u32 {
    1000
}
fn default_terminator_rx() -> String {
    "\r\n".to_string()
}

/// Connection type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    /// Serial port (RS-232 or USB-serial)
    #[default]
    Serial,
    /// RS-485 multidrop serial
    Rs485,
    /// TCP/IP socket
    Tcp,
    /// UDP socket
    Udp,
}

/// Parity bit setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParitySetting {
    #[default]
    None,
    Odd,
    Even,
}

/// Flow control setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowControlSetting {
    #[default]
    None,
    Software,
    Hardware,
}

/// Bus configuration for multidrop protocols.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct BusConfig {
    /// Bus type
    #[serde(rename = "type")]
    pub bus_type: BusType,

    /// Address format (how addresses are encoded in commands)
    #[serde(default)]
    pub address_format: AddressFormat,

    /// Default device address on the bus
    #[serde(default = "default_address")]
    pub default_address: String,
}

fn default_address() -> String {
    "0".to_string()
}

/// Bus type for multidrop protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BusType {
    #[default]
    Rs485,
    Gpib,
    Can,
}

/// Address format in commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddressFormat {
    /// Single hex character (0-9, A-F)
    #[default]
    HexChar,
    /// Decimal integer
    Decimal,
    /// Two-character hex (00-FF)
    HexByte,
}

// =============================================================================
// Command Configuration
// =============================================================================

/// Definition of a device command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    /// Command template with parameter placeholders.
    ///
    /// Use `${param_name}` for substitution.
    /// Use `${param_name:format}` for formatted output (e.g., `${value:08X}` for hex).
    #[validate(min_length = 1)]
    pub template: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Parameter definitions for this command
    #[serde(default)]
    pub parameters: HashMap<String, CommandParameterType>,

    /// Name of the response definition to use for parsing
    #[serde(default)]
    pub response: Option<String>,

    /// Whether this command expects a response
    #[serde(default = "default_expects_response")]
    pub expects_response: bool,

    /// Delay after sending this command (milliseconds)
    #[serde(default)]
    #[validate(maximum = 60000)]
    pub delay_ms: u32,
}

fn default_expects_response() -> bool {
    true
}

/// Parameter type for command templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandParameterType {
    /// String value
    String,
    /// 32-bit signed integer
    Int32,
    /// 64-bit signed integer
    Int64,
    /// 32-bit unsigned integer
    Uint32,
    /// 64-bit unsigned integer
    Uint64,
    /// Floating point value
    Float,
    /// Boolean value
    Bool,
}

// =============================================================================
// Response Configuration
// =============================================================================

/// Definition of a response parsing pattern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ResponseConfig {
    /// Regex pattern with named capture groups.
    ///
    /// Example: `^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$`
    #[serde(default)]
    #[validate(custom(validate_regex_pattern))]
    pub pattern: Option<String>,

    /// Delimiter for delimiter-based parsing (alternative to regex)
    #[serde(default)]
    pub delimiter: Option<String>,

    /// Field definitions for parsing
    #[serde(default)]
    pub fields: HashMap<String, ResponseFieldConfig>,

    /// Fixed-position field definitions (alternative to regex)
    #[serde(default)]
    pub fixed_fields: Vec<FixedFieldConfig>,
}

/// Field configuration for response parsing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ResponseFieldConfig {
    /// Field data type
    #[serde(rename = "type")]
    pub field_type: FieldType,

    /// For hex integer types, whether to interpret as signed
    #[serde(default)]
    pub signed: bool,

    /// Unit of measurement (informational)
    #[serde(default)]
    pub unit: Option<String>,

    /// Index for delimiter-based parsing (0-based)
    #[serde(default)]
    pub index: Option<usize>,
}

/// Fixed-position field for fixed-width protocols.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct FixedFieldConfig {
    /// Field name
    pub name: String,

    /// Start position (0-based, inclusive)
    pub start: usize,

    /// End position (exclusive)
    pub end: usize,

    /// Field data type
    #[serde(rename = "type")]
    pub field_type: FieldType,

    /// Expected value (for validation)
    #[serde(default)]
    pub expected: Option<String>,
}

/// Data types for response fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// String value (default)
    #[default]
    String,
    /// Signed integer (parsed from decimal)
    Int,
    /// Unsigned integer (parsed from decimal)
    Uint,
    /// Floating point number
    Float,
    /// Boolean value
    Bool,
    /// 8-bit unsigned from hex string
    HexU8,
    /// 16-bit unsigned from hex string
    HexU16,
    /// 32-bit unsigned from hex string
    HexU32,
    /// 64-bit unsigned from hex string
    HexU64,
    /// 32-bit signed from hex string (two's complement)
    HexI32,
    /// 64-bit signed from hex string (two's complement)
    HexI64,
}

// =============================================================================
// Conversion Configuration
// =============================================================================

/// Unit conversion formula configuration.
///
/// Uses `evalexpr` syntax for expression evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ConversionConfig {
    /// Conversion formula expression.
    ///
    /// Supports arithmetic: `+`, `-`, `*`, `/`, `%`
    /// Supports functions: `round()`, `floor()`, `ceil()`, `abs()`
    /// Supports variables from parameters.
    ///
    /// Example: `round(degrees * pulses_per_degree)`
    #[validate(custom(validate_evalexpr_formula))]
    pub formula: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,
}

// For simpler inline conversion definitions
impl From<String> for ConversionConfig {
    fn from(formula: String) -> Self {
        Self {
            formula,
            description: String::new(),
        }
    }
}

// =============================================================================
// Parameter Configuration
// =============================================================================

/// Device parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ParameterConfig {
    /// Parameter data type
    #[serde(rename = "type")]
    pub param_type: ParameterType,

    /// Default value (as JSON value for flexibility)
    #[serde(default)]
    pub default: serde_json::Value,

    /// Minimum value (for numeric types)
    #[serde(default)]
    pub min: Option<f64>,

    /// Maximum value (for numeric types)
    #[serde(default)]
    pub max: Option<f64>,

    /// Valid range [min, max] (shorthand for min/max)
    #[serde(default)]
    pub range: Option<(f64, f64)>,

    /// Unit of measurement
    #[serde(default)]
    pub unit: Option<String>,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Allowed discrete values (for enum-like parameters)
    #[serde(default)]
    pub choices: Vec<serde_json::Value>,

    /// Regex pattern for string validation
    #[serde(default)]
    #[validate(custom(validate_regex_pattern))]
    pub pattern: Option<String>,

    /// Whether this parameter is read-only
    #[serde(default)]
    pub read_only: bool,
}

/// Parameter data types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    /// String value
    #[default]
    String,
    /// Signed integer
    Int,
    /// Unsigned integer
    Uint,
    /// Floating point number
    Float,
    /// Boolean value
    Bool,
}

// =============================================================================
// Error Code Configuration
// =============================================================================

/// Error code definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ErrorCodeConfig {
    /// Short error name/identifier
    #[validate(min_length = 1)]
    pub name: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Whether this error is recoverable
    #[serde(default)]
    pub recoverable: bool,
}

// =============================================================================
// Validation Rule Configuration
// =============================================================================

/// Validation rule for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ValidationRuleConfig {
    /// Valid range [min, max]
    #[serde(default)]
    pub range: Option<(f64, f64)>,

    /// Unit of measurement
    #[serde(default)]
    pub unit: Option<String>,

    /// Regex pattern for validation
    #[serde(default)]
    #[validate(custom(validate_regex_pattern))]
    pub pattern: Option<String>,

    /// Custom error message
    #[serde(default)]
    pub error_message: Option<String>,
}

// =============================================================================
// Trait Mapping Configuration
// =============================================================================

/// Mapping from capability trait methods to device commands.
///
/// This struct uses `#[serde(flatten)]` to capture trait method mappings,
/// so we cannot use `deny_unknown_fields`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
pub struct TraitMappingConfig {
    /// Mappings for individual trait methods
    #[serde(flatten)]
    pub methods: HashMap<String, TraitMethodMapping>,
}

/// Mapping for a single trait method.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct TraitMethodMapping {
    /// Name of the command to execute (optional for polling-only methods)
    #[serde(default)]
    pub command: Option<String>,

    /// Conversion to apply to input value
    #[serde(default)]
    pub input_conversion: Option<String>,

    /// Name of the command parameter to set with converted input
    #[serde(default)]
    pub input_param: Option<String>,

    /// Name of the trait method parameter (input)
    #[serde(default)]
    pub from_param: Option<String>,

    /// Conversion to apply to output value
    #[serde(default)]
    pub output_conversion: Option<String>,

    /// Name of the response field to extract
    #[serde(default)]
    pub output_field: Option<String>,

    /// For polling operations: command to poll
    #[serde(default)]
    pub poll_command: Option<String>,

    /// For polling: success condition expression
    #[serde(default)]
    pub success_condition: Option<String>,

    /// For polling: interval between polls (milliseconds)
    #[serde(default)]
    #[validate(maximum = 60000)]
    pub poll_interval_ms: Option<u32>,

    /// For polling: timeout (milliseconds)
    #[serde(default)]
    #[validate(maximum = 300000)]
    pub timeout_ms: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_category_serialization() {
        let category = DeviceCategory::Stage;
        let json = serde_json::to_string(&category).unwrap();
        assert_eq!(json, "\"stage\"");

        let parsed: DeviceCategory = serde_json::from_str("\"sensor\"").unwrap();
        assert_eq!(parsed, DeviceCategory::Sensor);
    }

    #[test]
    fn test_capability_type_serialization() {
        let cap = CapabilityType::Movable;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"Movable\"");
    }

    #[test]
    fn test_field_type_serialization() {
        let field = FieldType::HexI32;
        let json = serde_json::to_string(&field).unwrap();
        assert_eq!(json, "\"hex_i32\"");
    }
}
