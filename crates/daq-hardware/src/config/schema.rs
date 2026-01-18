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

    /// Initialization sequence (run on device connect)
    #[serde(default)]
    pub init_sequence: Vec<InitStep>,

    /// Default retry configuration (applies to all commands without explicit retry)
    #[serde(default)]
    #[validate]
    pub default_retry: Option<RetryConfig>,

    /// Rhai script definitions for complex logic that can't be expressed declaratively
    #[serde(default)]
    pub scripts: HashMap<String, ScriptDefinition>,

    /// Binary command definitions for protocols like Modbus RTU
    #[serde(default)]
    pub binary_commands: HashMap<String, BinaryCommandConfig>,

    /// Binary response parsing definitions
    #[serde(default)]
    pub binary_responses: HashMap<String, BinaryResponseConfig>,

    /// UI configuration for control panels and visualization
    #[serde(default)]
    #[validate]
    pub ui: Option<UiConfig>,
}

// =============================================================================
// Initialization Sequence
// =============================================================================

/// A single step in the device initialization sequence.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct InitStep {
    /// Name of the command to execute
    pub command: String,

    /// Optional parameters for the command
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,

    /// Expected response pattern for validation (optional)
    #[serde(default)]
    pub expect: Option<String>,

    /// Whether to fail initialization if this step fails
    #[serde(default = "default_required")]
    pub required: bool,

    /// Delay after this step (milliseconds)
    #[serde(default)]
    #[validate(maximum = 60000)]
    pub delay_ms: u32,

    /// Human-readable description
    #[serde(default)]
    pub description: String,
}

fn default_required() -> bool {
    true
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
// Retry Configuration
// =============================================================================

/// Retry configuration for commands.
///
/// Defines how failed commands should be retried with exponential backoff.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries)
    #[serde(default = "default_max_retries")]
    #[validate(maximum = 10)]
    pub max_retries: u8,

    /// Initial delay before first retry (milliseconds)
    #[serde(default = "default_initial_delay_ms")]
    #[validate(minimum = 10)]
    #[validate(maximum = 10000)]
    pub initial_delay_ms: u32,

    /// Maximum delay between retries (milliseconds)
    #[serde(default = "default_max_delay_ms")]
    #[validate(maximum = 60000)]
    pub max_delay_ms: u32,

    /// Multiplier for exponential backoff (e.g., 2.0 = double delay each retry)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,

    /// Error codes that should trigger a retry (if empty, retries on any error)
    #[serde(default)]
    pub retry_on_errors: Vec<String>,

    /// Error codes that should NOT trigger a retry (takes precedence over retry_on_errors)
    #[serde(default)]
    pub no_retry_on_errors: Vec<String>,
}

fn default_max_retries() -> u8 {
    3
}

fn default_initial_delay_ms() -> u32 {
    100
}

fn default_max_delay_ms() -> u32 {
    5000
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay_ms: default_initial_delay_ms(),
            max_delay_ms: default_max_delay_ms(),
            backoff_multiplier: default_backoff_multiplier(),
            retry_on_errors: Vec::new(),
            no_retry_on_errors: Vec::new(),
        }
    }
}

// =============================================================================
// Script Configuration
// =============================================================================

/// Rhai script definition for complex device operations.
///
/// Scripts extend the declarative config system for edge cases that can't be
/// expressed as simple command/response patterns. Use sparingly - prefer
/// declarative definitions when possible.
///
/// # Example
///
/// ```toml
/// [scripts.safe_move_with_correction]
/// description = "Move with automatic overshoot correction"
/// timeout_ms = 15000
/// script = """
///     let target = input;
///     driver.move_abs(target);
///     driver.wait_settled();
///     let actual = driver.position();
///     if abs(actual - target) > 1.0 {
///         driver.move_abs(target);  // Correction move
///     }
///     actual
/// """
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ScriptDefinition {
    /// Rhai script source code
    #[validate(min_length = 1)]
    #[validate(max_length = 65536)]
    pub script: String,

    /// Human-readable description of what the script does
    #[serde(default)]
    #[validate(max_length = 500)]
    pub description: String,

    /// Execution timeout in milliseconds (default: 30000 = 30 seconds)
    #[serde(default = "default_script_timeout_ms")]
    #[validate(minimum = 100)]
    #[validate(maximum = 300000)]
    pub timeout_ms: u32,

    /// Names of input parameters the script expects (for documentation/validation)
    #[serde(default)]
    pub inputs: Vec<String>,

    /// Expected return type for documentation (string, float, bool, none)
    #[serde(default)]
    pub returns: ScriptReturnType,
}

fn default_script_timeout_ms() -> u32 {
    30000 // 30 seconds
}

/// Expected return type of a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScriptReturnType {
    /// Script returns nothing (unit type)
    #[default]
    None,
    /// Script returns a string
    String,
    /// Script returns a floating-point number
    Float,
    /// Script returns a boolean
    Bool,
    /// Script returns an integer
    Int,
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

    /// Per-command timeout override (milliseconds).
    /// If not specified, uses the connection-level timeout.
    #[serde(default)]
    #[validate(maximum = 300000)]
    pub timeout_ms: Option<u32>,

    /// Retry configuration for this command
    #[serde(default)]
    #[validate]
    pub retry: Option<RetryConfig>,
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

    /// Error severity level
    #[serde(default)]
    pub severity: ErrorSeverity,

    /// Suggested recovery action
    #[serde(default)]
    pub recovery_action: Option<RecoveryAction>,
}

/// Error severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    /// Informational - operation may still succeed
    Info,
    /// Warning - operation succeeded with caveats
    Warning,
    /// Error - operation failed but device is OK
    #[default]
    Error,
    /// Critical - device may be in bad state, requires attention
    Critical,
    /// Fatal - device unusable until power cycle/reset
    Fatal,
}

/// Recovery actions for errors.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAction {
    /// Command to execute for recovery (optional)
    #[serde(default)]
    pub command: Option<String>,

    /// Whether to attempt automatic recovery
    #[serde(default)]
    pub auto_recover: bool,

    /// Delay before attempting recovery (milliseconds)
    #[serde(default)]
    #[validate(maximum = 60000)]
    pub delay_ms: u32,

    /// Human-readable instructions for manual recovery
    #[serde(default)]
    pub manual_instructions: Option<String>,
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
    /// Name of the command to execute (optional for polling-only or script-based methods)
    #[serde(default)]
    pub command: Option<String>,

    /// Name of the script to execute (alternative to command for complex logic)
    ///
    /// When set, the script is executed instead of the command. The script
    /// receives `input` as the input parameter and should return the result.
    #[serde(default)]
    pub script: Option<String>,

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

// =============================================================================
// Binary Protocol Configuration (Phase 6)
// =============================================================================

/// Binary command configuration for protocols like Modbus RTU.
///
/// Defines how to build a binary frame from fields with explicit types and endianness.
///
/// # Example
///
/// ```toml
/// [binary_commands.read_registers]
/// description = "Read holding registers (Modbus function 0x03)"
/// fields = [
///     { name = "address", type = "u8", value = "${device_address}" },
///     { name = "function", type = "u8", value = "0x03" },
///     { name = "start_register", type = "u16_be", value = "${start_register}" },
///     { name = "count", type = "u16_be", value = "${count}" },
/// ]
/// crc = { algorithm = "crc16_modbus", append = true }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct BinaryCommandConfig {
    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Ordered list of fields to build the frame
    pub fields: Vec<BinaryFieldConfig>,

    /// CRC configuration (optional)
    #[serde(default)]
    pub crc: Option<CrcConfig>,

    /// Whether this command expects a response
    #[serde(default = "default_expects_response")]
    pub expects_response: bool,

    /// Name of the binary response definition to use for parsing
    #[serde(default)]
    pub response: Option<String>,

    /// Per-command timeout override (milliseconds)
    #[serde(default)]
    #[validate(maximum = 300000)]
    pub timeout_ms: Option<u32>,

    /// Retry configuration for this command
    #[serde(default)]
    #[validate]
    pub retry: Option<RetryConfig>,
}

/// Binary field configuration for frame building.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct BinaryFieldConfig {
    /// Field name (for documentation and debugging)
    pub name: String,

    /// Field data type (includes endianness)
    #[serde(rename = "type")]
    pub field_type: BinaryFieldType,

    /// Value template (can include `${param}` substitutions or hex literals like `0x03`)
    #[serde(default)]
    pub value: Option<String>,

    /// Fixed byte array value (alternative to value template)
    #[serde(default)]
    pub bytes: Option<Vec<u8>>,

    /// For variable-length fields: expression for length (e.g., "${count} * 2")
    #[serde(default)]
    pub length: Option<String>,
}

/// Binary field types with explicit endianness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BinaryFieldType {
    /// 8-bit unsigned integer
    U8,
    /// 8-bit signed integer
    I8,
    /// 16-bit unsigned, big-endian (network byte order)
    U16Be,
    /// 16-bit unsigned, little-endian
    U16Le,
    /// 16-bit signed, big-endian
    I16Be,
    /// 16-bit signed, little-endian
    I16Le,
    /// 32-bit unsigned, big-endian
    U32Be,
    /// 32-bit unsigned, little-endian
    U32Le,
    /// 32-bit signed, big-endian
    I32Be,
    /// 32-bit signed, little-endian
    I32Le,
    /// 32-bit float, big-endian (IEEE 754)
    F32Be,
    /// 32-bit float, little-endian (IEEE 754)
    F32Le,
    /// 64-bit unsigned, big-endian
    U64Be,
    /// 64-bit unsigned, little-endian
    U64Le,
    /// Fixed-length byte array
    Bytes,
    /// ASCII string (no null terminator)
    AsciiString,
    /// Null-terminated ASCII string
    AsciiStringZ,
}

impl BinaryFieldType {
    /// Returns the fixed size in bytes, or None for variable-length types.
    pub fn fixed_size(&self) -> Option<usize> {
        match self {
            Self::U8 | Self::I8 => Some(1),
            Self::U16Be | Self::U16Le | Self::I16Be | Self::I16Le => Some(2),
            Self::U32Be | Self::U32Le | Self::I32Be | Self::I32Le => Some(4),
            Self::F32Be | Self::F32Le => Some(4),
            Self::U64Be | Self::U64Le => Some(8),
            Self::Bytes | Self::AsciiString | Self::AsciiStringZ => None,
        }
    }
}

/// CRC configuration for binary protocols.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CrcConfig {
    /// CRC algorithm to use
    pub algorithm: CrcAlgorithm,

    /// Whether to append CRC to outgoing frames
    #[serde(default = "default_true")]
    pub append: bool,

    /// Whether to validate CRC on incoming frames
    #[serde(default = "default_true")]
    pub validate: bool,

    /// Byte order for multi-byte CRC values
    #[serde(default)]
    pub byte_order: ByteOrder,
}

fn default_true() -> bool {
    true
}

/// Supported CRC algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrcAlgorithm {
    /// CRC-16 Modbus (polynomial 0x8005, init 0xFFFF, reflect in/out)
    Crc16Modbus,
    /// CRC-16 CCITT (polynomial 0x1021, init 0xFFFF)
    Crc16Ccitt,
    /// CRC-16 CCITT with init 0x0000 (X.25)
    Crc16CcittFalse,
    /// CRC-16 XMODEM (polynomial 0x1021, init 0x0000)
    Crc16Xmodem,
    /// CRC-32 (Ethernet, ZIP)
    Crc32,
    /// CRC-32C (Castagnoli, iSCSI)
    Crc32C,
    /// Simple 8-bit checksum (sum of bytes mod 256)
    Checksum8,
    /// XOR of all bytes
    Xor8,
    /// Longitudinal Redundancy Check (XOR of all bytes)
    Lrc,
}

/// Byte order for multi-byte values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ByteOrder {
    /// Big-endian (most significant byte first)
    BigEndian,
    /// Little-endian (least significant byte first)
    #[default]
    LittleEndian,
}

/// Binary response configuration for parsing binary frames.
///
/// # Example
///
/// ```toml
/// [binary_responses.read_registers_response]
/// fields = [
///     { name = "address", type = "u8", position = 0 },
///     { name = "function", type = "u8", position = 1 },
///     { name = "byte_count", type = "u8", position = 2 },
///     { name = "data", type = "bytes", start = 3, length_field = "byte_count" },
/// ]
/// crc = { algorithm = "crc16_modbus", validate = true }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct BinaryResponseConfig {
    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Field definitions for parsing (in order)
    pub fields: Vec<BinaryResponseFieldConfig>,

    /// CRC configuration for validation
    #[serde(default)]
    pub crc: Option<CrcConfig>,

    /// Minimum expected response length (bytes)
    #[serde(default)]
    #[validate(maximum = 65535)]
    pub min_length: Option<u16>,

    /// Maximum expected response length (bytes)
    #[serde(default)]
    #[validate(maximum = 65535)]
    pub max_length: Option<u16>,
}

/// Binary response field configuration for parsing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct BinaryResponseFieldConfig {
    /// Field name (used for extracting values)
    pub name: String,

    /// Field data type
    #[serde(rename = "type")]
    pub field_type: BinaryFieldType,

    /// Fixed position in the frame (0-based byte offset)
    #[serde(default)]
    pub position: Option<usize>,

    /// Start position for variable-length fields
    #[serde(default)]
    pub start: Option<usize>,

    /// Fixed length for variable-length fields
    #[serde(default)]
    pub length: Option<usize>,

    /// Name of another field that contains the length
    #[serde(default)]
    pub length_field: Option<String>,

    /// Expected value (for validation, hex string like "0x03")
    #[serde(default)]
    pub expected: Option<String>,

    /// Whether this field is the error code indicator
    #[serde(default)]
    pub is_error_code: bool,
}

// =============================================================================
// UI Configuration (Control Panels)
// =============================================================================

/// Top-level UI configuration for a device.
///
/// Defines how the device should be displayed and controlled in the GUI.
///
/// # Example
///
/// ```toml
/// [ui]
/// icon = "laser"
/// color = "#FF5733"
///
/// [ui.control_panel]
/// layout = "vertical"
/// sections = [
///     { type = "motion", label = "Position", show_jog = true },
///     { type = "preset_buttons", label = "Presets", presets = [0.0, 45.0, 90.0] },
/// ]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    /// Icon identifier for the device (e.g., "laser", "motor", "camera")
    #[serde(default)]
    pub icon: Option<String>,

    /// Color for the device in hex format (e.g., "#FF5733")
    #[serde(default)]
    pub color: Option<String>,

    /// Control panel configuration
    #[serde(default)]
    #[validate]
    pub control_panel: Option<ControlPanelConfig>,

    /// Status display configuration
    #[serde(default)]
    #[validate]
    pub status_display: Option<StatusDisplayConfig>,
}

/// Control panel layout configuration.
///
/// Defines the sections and layout of the device control panel in the GUI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ControlPanelConfig {
    /// Layout direction for sections
    #[serde(default)]
    pub layout: PanelLayout,

    /// Ordered list of control sections to display
    #[serde(default)]
    pub sections: Vec<ControlSection>,

    /// Width hint for the panel (pixels, 0 = auto)
    #[serde(default)]
    #[validate(maximum = 2000)]
    pub width: u16,

    /// Whether to show the device header with name/status
    #[serde(default = "default_true")]
    pub show_header: bool,

    /// Whether to allow collapsing sections
    #[serde(default)]
    pub collapsible: bool,
}

/// Panel layout direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayout {
    /// Sections stacked vertically (default)
    #[default]
    Vertical,
    /// Sections arranged horizontally
    Horizontal,
    /// Grid layout with configurable columns
    Grid,
}

/// A control section in the device control panel.
///
/// Uses tagged union for different section types.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlSection {
    /// Motion control section with position display and jog buttons
    Motion(MotionSectionConfig),

    /// Preset position buttons
    PresetButtons(PresetButtonsSectionConfig),

    /// Custom action button (triggers a command)
    CustomAction(CustomActionSectionConfig),

    /// Camera/frame producer controls
    Camera(CameraSectionConfig),

    /// Shutter control toggle
    Shutter(ShutterSectionConfig),

    /// Wavelength tuning control
    Wavelength(WavelengthSectionConfig),

    /// Generic parameter display/edit
    Parameter(ParameterSectionConfig),

    /// Read-only status display
    StatusDisplay(StatusDisplaySectionConfig),

    /// Power meter / sensor reading display
    Sensor(SensorSectionConfig),

    /// Separator/spacer between sections
    Separator(SeparatorConfig),

    /// Custom section with user-defined widgets
    Custom(CustomSectionConfig),
}

/// Motion control section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct MotionSectionConfig {
    /// Section label
    #[serde(default = "default_motion_label")]
    pub label: String,

    /// Show jog buttons (+/- step movement)
    #[serde(default = "default_true")]
    pub show_jog: bool,

    /// Jog step sizes available (e.g., [0.1, 1.0, 10.0])
    #[serde(default = "default_jog_steps")]
    pub jog_steps: Vec<f64>,

    /// Show home button
    #[serde(default)]
    pub show_home: bool,

    /// Show stop button
    #[serde(default = "default_true")]
    pub show_stop: bool,

    /// Position display precision (decimal places)
    #[serde(default = "default_precision")]
    #[validate(maximum = 10)]
    pub precision: u8,

    /// Unit label for position display
    #[serde(default)]
    pub unit: Option<String>,
}

fn default_motion_label() -> String {
    "Position".to_string()
}

fn default_jog_steps() -> Vec<f64> {
    vec![0.1, 1.0, 10.0]
}

fn default_precision() -> u8 {
    3
}

/// Preset buttons section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PresetButtonsSectionConfig {
    /// Section label
    #[serde(default = "default_presets_label")]
    pub label: String,

    /// Preset values (position or parameter value)
    #[serde(default)]
    pub presets: Vec<PresetValue>,

    /// Arrange buttons vertically instead of horizontally
    #[serde(default)]
    pub vertical: bool,
}

fn default_presets_label() -> String {
    "Presets".to_string()
}

/// A preset button value with optional label.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PresetValue {
    /// Simple numeric preset
    Number(f64),
    /// Labeled preset with value
    Labeled { label: String, value: f64 },
}

/// Custom action button configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CustomActionSectionConfig {
    /// Button label
    pub label: String,

    /// Command to execute when clicked
    pub command: String,

    /// Optional parameters to pass to the command
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,

    /// Button style/color hint
    #[serde(default)]
    pub style: ButtonStyle,

    /// Confirmation message (if set, prompts before executing)
    #[serde(default)]
    pub confirm: Option<String>,
}

/// Button style hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    /// Default button style
    #[default]
    Default,
    /// Primary action button (highlighted)
    Primary,
    /// Secondary/subtle button
    Secondary,
    /// Danger/destructive action (red)
    Danger,
    /// Success/positive action (green)
    Success,
}

/// Camera control section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CameraSectionConfig {
    /// Section label
    #[serde(default = "default_camera_label")]
    pub label: String,

    /// Show exposure control
    #[serde(default = "default_true")]
    pub show_exposure: bool,

    /// Show gain control
    #[serde(default)]
    pub show_gain: bool,

    /// Show binning control
    #[serde(default)]
    pub show_binning: bool,

    /// Show ROI controls
    #[serde(default)]
    pub show_roi: bool,

    /// Show histogram
    #[serde(default)]
    pub show_histogram: bool,

    /// Show frame statistics (min/max/mean)
    #[serde(default = "default_true")]
    pub show_stats: bool,
}

fn default_camera_label() -> String {
    "Camera".to_string()
}

/// Shutter control section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ShutterSectionConfig {
    /// Section label
    #[serde(default = "default_shutter_label")]
    pub label: String,

    /// Show as toggle switch instead of buttons
    #[serde(default)]
    pub toggle_style: bool,
}

fn default_shutter_label() -> String {
    "Shutter".to_string()
}

/// Wavelength tuning section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct WavelengthSectionConfig {
    /// Section label
    #[serde(default = "default_wavelength_label")]
    pub label: String,

    /// Show wavelength slider
    #[serde(default = "default_true")]
    pub show_slider: bool,

    /// Wavelength presets (nm)
    #[serde(default)]
    pub presets: Vec<f64>,

    /// Show color indicator based on wavelength
    #[serde(default = "default_true")]
    pub show_color: bool,
}

fn default_wavelength_label() -> String {
    "Wavelength".to_string()
}

/// Generic parameter section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ParameterSectionConfig {
    /// Section label
    #[serde(default)]
    pub label: String,

    /// Parameter name to display/edit
    pub parameter: String,

    /// Widget type for editing
    #[serde(default)]
    pub widget: ParameterWidget,

    /// Read-only display (no editing)
    #[serde(default)]
    pub read_only: bool,
}

/// Widget type for parameter editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterWidget {
    /// Automatic based on parameter type
    #[default]
    Auto,
    /// Text input field
    TextInput,
    /// Numeric slider
    Slider,
    /// Numeric spinner with +/- buttons
    Spinner,
    /// Toggle switch (for booleans)
    Toggle,
    /// Dropdown/combo box (for enums/choices)
    Dropdown,
}

/// Status display section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct StatusDisplaySectionConfig {
    /// Section label
    #[serde(default)]
    pub label: String,

    /// Parameters to display as status
    #[serde(default)]
    pub parameters: Vec<String>,

    /// Show as compact inline display
    #[serde(default)]
    pub compact: bool,
}

/// Sensor reading display section configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct SensorSectionConfig {
    /// Section label
    #[serde(default = "default_sensor_label")]
    pub label: String,

    /// Display precision (decimal places)
    #[serde(default = "default_precision")]
    #[validate(maximum = 10)]
    pub precision: u8,

    /// Unit label
    #[serde(default)]
    pub unit: Option<String>,

    /// Show trend graph
    #[serde(default)]
    pub show_trend: bool,

    /// Auto-refresh interval (milliseconds, 0 = manual)
    #[serde(default)]
    #[validate(maximum = 60000)]
    pub refresh_ms: u32,
}

fn default_sensor_label() -> String {
    "Reading".to_string()
}

/// Separator/spacer configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct SeparatorConfig {
    /// Height in pixels (0 = default)
    #[serde(default)]
    #[validate(maximum = 100)]
    pub height: u8,

    /// Show visible line
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// Custom section for advanced use cases.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CustomSectionConfig {
    /// Section label
    #[serde(default)]
    pub label: String,

    /// Custom widget identifier
    pub widget: String,

    /// Widget-specific configuration
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

/// Status display configuration (outside control panel).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct StatusDisplayConfig {
    /// Parameters to show in device tree summary
    #[serde(default)]
    pub summary_params: Vec<String>,

    /// Format string for summary (uses parameter names)
    #[serde(default)]
    pub summary_format: Option<String>,

    /// Show connection status indicator
    #[serde(default = "default_true")]
    pub show_connection: bool,
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

    #[test]
    fn test_binary_field_type_serialization() {
        let field = BinaryFieldType::U16Be;
        let json = serde_json::to_string(&field).unwrap();
        assert_eq!(json, "\"u16_be\"");

        let parsed: BinaryFieldType = serde_json::from_str("\"i32_le\"").unwrap();
        assert_eq!(parsed, BinaryFieldType::I32Le);
    }

    #[test]
    fn test_binary_field_type_fixed_size() {
        assert_eq!(BinaryFieldType::U8.fixed_size(), Some(1));
        assert_eq!(BinaryFieldType::U16Be.fixed_size(), Some(2));
        assert_eq!(BinaryFieldType::U32Le.fixed_size(), Some(4));
        assert_eq!(BinaryFieldType::U64Be.fixed_size(), Some(8));
        assert_eq!(BinaryFieldType::Bytes.fixed_size(), None);
        assert_eq!(BinaryFieldType::AsciiString.fixed_size(), None);
    }

    #[test]
    fn test_crc_algorithm_serialization() {
        let crc = CrcAlgorithm::Crc16Modbus;
        let json = serde_json::to_string(&crc).unwrap();
        assert_eq!(json, "\"crc16_modbus\"");

        let parsed: CrcAlgorithm = serde_json::from_str("\"crc32\"").unwrap();
        assert_eq!(parsed, CrcAlgorithm::Crc32);
    }

    #[test]
    fn test_byte_order_default() {
        let order: ByteOrder = Default::default();
        assert_eq!(order, ByteOrder::LittleEndian);
    }

    // =============================================================================
    // UI Config Tests
    // =============================================================================

    #[test]
    fn test_ui_config_deserialization() {
        let toml_str = r##"
            icon = "laser"
            color = "#FF5733"

            [control_panel]
            layout = "vertical"
            show_header = true
        "##;

        let config: UiConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.icon, Some("laser".to_string()));
        assert_eq!(config.color, Some("#FF5733".to_string()));
        assert!(config.control_panel.is_some());
        let panel = config.control_panel.unwrap();
        assert_eq!(panel.layout, PanelLayout::Vertical);
        assert!(panel.show_header);
    }

    #[test]
    fn test_control_section_motion() {
        let toml = r#"
            type = "motion"
            label = "Rotation"
            show_jog = true
            jog_steps = [0.1, 1.0, 10.0]
            precision = 2
            unit = "degrees"
        "#;

        let section: ControlSection = toml::from_str(toml).unwrap();
        match section {
            ControlSection::Motion(cfg) => {
                assert_eq!(cfg.label, "Rotation");
                assert!(cfg.show_jog);
                assert_eq!(cfg.jog_steps, vec![0.1, 1.0, 10.0]);
                assert_eq!(cfg.precision, 2);
                assert_eq!(cfg.unit, Some("degrees".to_string()));
            }
            _ => panic!("Expected Motion section"),
        }
    }

    #[test]
    fn test_control_section_preset_buttons() {
        let toml = r#"
            type = "preset_buttons"
            label = "Presets"
            presets = [0.0, 45.0, 90.0, 180.0]
        "#;

        let section: ControlSection = toml::from_str(toml).unwrap();
        match section {
            ControlSection::PresetButtons(cfg) => {
                assert_eq!(cfg.label, "Presets");
                assert_eq!(cfg.presets.len(), 4);
            }
            _ => panic!("Expected PresetButtons section"),
        }
    }

    #[test]
    fn test_control_section_camera() {
        let toml = r#"
            type = "camera"
            label = "Camera Controls"
            show_exposure = true
            show_histogram = true
            show_binning = false
        "#;

        let section: ControlSection = toml::from_str(toml).unwrap();
        match section {
            ControlSection::Camera(cfg) => {
                assert_eq!(cfg.label, "Camera Controls");
                assert!(cfg.show_exposure);
                assert!(cfg.show_histogram);
                assert!(!cfg.show_binning);
            }
            _ => panic!("Expected Camera section"),
        }
    }

    #[test]
    fn test_control_section_wavelength() {
        let toml = r#"
            type = "wavelength"
            label = "Tuning"
            show_slider = true
            presets = [700.0, 800.0, 900.0, 1000.0]
            show_color = true
        "#;

        let section: ControlSection = toml::from_str(toml).unwrap();
        match section {
            ControlSection::Wavelength(cfg) => {
                assert_eq!(cfg.label, "Tuning");
                assert!(cfg.show_slider);
                assert_eq!(cfg.presets, vec![700.0, 800.0, 900.0, 1000.0]);
            }
            _ => panic!("Expected Wavelength section"),
        }
    }

    #[test]
    fn test_preset_value_variants() {
        // Simple number
        let simple: PresetValue = serde_json::from_str("45.0").unwrap();
        matches!(simple, PresetValue::Number(45.0));

        // Labeled preset
        let labeled: PresetValue =
            serde_json::from_str(r#"{"label": "Home", "value": 0.0}"#).unwrap();
        match labeled {
            PresetValue::Labeled { label, value } => {
                assert_eq!(label, "Home");
                assert_eq!(value, 0.0);
            }
            _ => panic!("Expected Labeled preset"),
        }
    }

    #[test]
    fn test_button_style_serialization() {
        let style = ButtonStyle::Danger;
        let json = serde_json::to_string(&style).unwrap();
        assert_eq!(json, "\"danger\"");

        let parsed: ButtonStyle = serde_json::from_str("\"primary\"").unwrap();
        assert_eq!(parsed, ButtonStyle::Primary);
    }

    #[test]
    fn test_panel_layout_serialization() {
        let layout = PanelLayout::Horizontal;
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(json, "\"horizontal\"");

        let parsed: PanelLayout = serde_json::from_str("\"grid\"").unwrap();
        assert_eq!(parsed, PanelLayout::Grid);
    }

    #[test]
    fn test_full_control_panel_config() {
        let toml = r#"
            layout = "vertical"
            show_header = true
            collapsible = false

            [[sections]]
            type = "motion"
            label = "Position"
            show_jog = true

            [[sections]]
            type = "separator"
            visible = true

            [[sections]]
            type = "preset_buttons"
            label = "Quick Positions"
            presets = [0.0, 90.0, 180.0]
        "#;

        let config: ControlPanelConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.layout, PanelLayout::Vertical);
        assert!(config.show_header);
        assert!(!config.collapsible);
        assert_eq!(config.sections.len(), 3);

        // Check first section is Motion
        matches!(&config.sections[0], ControlSection::Motion(_));
        // Check second section is Separator
        matches!(&config.sections[1], ControlSection::Separator(_));
        // Check third section is PresetButtons
        matches!(&config.sections[2], ControlSection::PresetButtons(_));
    }
}
