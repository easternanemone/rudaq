//! Stage 2: Validated intermediate representation with newtypes.
//!
//! Every type in this module has been validated at construction time. A
//! `DeviceManifest` is guaranteed to be internally consistent: all command
//! refs, response refs, and conversion refs point to entries that exist.

use crate::format_parser::FormatSegment;
use crate::transform::TransformPipeline;
use std::collections::HashMap;

/// A validated, internally-consistent device manifest.
///
/// All cross-references have been verified: command refs in capabilities
/// point to entries in `commands`, response refs in commands point to
/// entries in `responses`, and conversion refs point to entries in
/// `conversions`.
#[derive(Debug)]
pub struct DeviceManifest {
    /// Device metadata.
    pub device: DeviceInfo,

    /// Validated connection configuration.
    pub connection: ConnectionConfig,

    /// Validated command definitions, keyed by name.
    pub commands: HashMap<String, CommandConfig>,

    /// Validated response parsers, keyed by name.
    pub responses: HashMap<String, ResponseParser>,

    /// Validated conversion formulas, keyed by name.
    pub conversions: HashMap<String, ValidatedFormula>,

    /// Validated capability configurations.
    pub capabilities: CapabilitySet,

    /// Device parameters with their default numeric values.
    /// Extracted from `[parameters]` section for use in formula evaluation.
    pub parameters: HashMap<String, f64>,

    /// Rich parameter metadata parsed from the `[parameters]` section.
    /// Preserves type, range, unit, description, and read_only from the
    /// TOML manifest for downstream DB/UI consumption.
    pub parameter_metadata: Vec<ManifestParameterMeta>,

    /// Initialization sequence to run when the driver connects.
    pub init_sequence: Vec<InitCommand>,

    /// Optional UI schema/configuration from `[ui]`.
    pub ui: Option<toml::Value>,
}

/// Metadata about a single parameter defined in the TOML manifest.
///
/// Extracted from `[parameters.<name>]` tables for DB/UI consumption.
/// This is a TOML deserialization type that converts to the authoritative
/// [`common::observable::ParameterMetadata`] via the `From` impl (bd-20ap).
#[derive(Debug, Clone)]
pub struct ManifestParameterMeta {
    /// Parameter name (e.g., "position_deg").
    pub name: String,
    /// Data type: "float", "int", "bool", "string", "enum".
    pub dtype: String,
    /// Default value as f64 (for numeric types).
    pub default_value: Option<f64>,
    /// Minimum of allowed range.
    pub min_value: Option<f64>,
    /// Maximum of allowed range.
    pub max_value: Option<f64>,
    /// Physical unit (e.g., "degrees", "nm", "mW").
    pub unit: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Whether the parameter is read-only.
    pub read_only: bool,
}

fn parameter_metadata_from_manifest(
    m: &ManifestParameterMeta,
) -> common::observable::ParameterMetadata {
    common::observable::ParameterMetadata {
        name: m.name.clone(),
        description: m.description.clone(),
        units: m.unit.clone(),
        read_only: m.read_only,
        dtype: m.dtype.clone(),
        min_value: m.min_value,
        max_value: m.max_value,
        step: None,
        default_value: m.default_value,
        enum_values: Vec::new(),
        group_name: None,
    }
}

impl From<ManifestParameterMeta> for common::observable::ParameterMetadata {
    fn from(m: ManifestParameterMeta) -> Self {
        parameter_metadata_from_manifest(&m)
    }
}

impl From<&ManifestParameterMeta> for common::observable::ParameterMetadata {
    fn from(m: &ManifestParameterMeta) -> Self {
        parameter_metadata_from_manifest(m)
    }
}

/// Device metadata.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub capability_names: Vec<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
}

/// Validated parity mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialParity {
    None,
    Odd,
    Even,
}

/// Validated flow control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

/// Optional serial port configuration beyond baud rate.
///
/// When all fields are `None`, defaults to 8N1 with no flow control.
#[derive(Debug, Clone, Default)]
pub struct SerialConfig {
    /// Data bits per character (5, 6, 7, or 8). Default: 8.
    pub data_bits: Option<u8>,
    /// Parity mode. Default: None.
    pub parity: Option<SerialParity>,
    /// Stop bits: 1 or 2. Default: 1.
    pub stop_bits: Option<u8>,
    /// Flow control mode. Default: None.
    pub flow_control: Option<SerialFlowControl>,
}

/// A single command in the device initialization sequence.
#[derive(Debug, Clone)]
pub struct InitCommand {
    /// The command string to send (a raw template source).
    pub command: String,
    /// Optional delay in milliseconds after sending this command.
    pub delay_ms: Option<u32>,
    /// Whether the device sends a response that must be drained.
    pub expects_response: bool,
}

/// Validated connection configuration.
#[derive(Debug, Clone)]
pub enum ConnectionConfig {
    Serial {
        baud_rate: BaudRate,
        timeout: Timeout,
        terminator: Option<String>,
        serial_config: SerialConfig,
    },
    Tcp {
        host: String,
        port: u16,
        timeout: Timeout,
        terminator: Option<String>,
    },
    Udp {
        host: String,
        port: u16,
        timeout: Timeout,
    },
    /// USB TMC transport (Linux `/dev/usbtmcN`).
    Usbtmc {
        timeout: Timeout,
        terminator: Option<String>,
    },
}

impl ConnectionConfig {
    /// Get the timeout for this connection.
    pub fn timeout(&self) -> Timeout {
        match self {
            Self::Serial { timeout, .. } => *timeout,
            Self::Tcp { timeout, .. } => *timeout,
            Self::Udp { timeout, .. } => *timeout,
            Self::Usbtmc { timeout, .. } => *timeout,
        }
    }
}

/// Validated baud rate (300..=921_600).
#[derive(Debug, Clone, Copy)]
pub struct BaudRate(u32);

impl BaudRate {
    /// Create a validated baud rate.
    ///
    /// # Errors
    /// Returns error if the value is not in range 300..=921_600.
    pub fn new(value: u32) -> Result<Self, String> {
        if (300..=921_600).contains(&value) {
            Ok(Self(value))
        } else {
            Err(format!(
                "baud rate {value} out of range (must be 300..=921600)"
            ))
        }
    }

    /// Get the inner value.
    pub fn value(self) -> u32 {
        self.0
    }
}

/// Validated timeout in milliseconds (1..=60_000).
#[derive(Debug, Clone, Copy)]
pub struct Timeout(u32);

impl Timeout {
    /// Create a validated timeout.
    ///
    /// # Errors
    /// Returns error if the value is not in range 1..=60_000.
    pub fn new(value: u32) -> Result<Self, String> {
        if (1..=60_000).contains(&value) {
            Ok(Self(value))
        } else {
            Err(format!(
                "timeout {value}ms out of range (must be 1..=60000)"
            ))
        }
    }

    /// Get the inner value in milliseconds.
    pub fn value(self) -> u32 {
        self.0
    }

    /// Get as a `std::time::Duration`.
    pub fn as_duration(self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.0))
    }
}

/// A validated command configuration.
#[derive(Debug)]
pub struct CommandConfig {
    /// Validated MiniJinja template.
    pub template: ValidatedTemplate,

    /// Parameter type declarations.
    pub parameters: HashMap<String, String>,

    /// Optional reference to a response parser.
    pub response: Option<ResponseRef>,

    /// Optional SCPI auto-parse type.
    pub response_type: Option<ScpiResponseType>,

    /// Whether this command expects a response.
    pub expects_response: bool,

    /// Human-readable description from the TOML manifest.
    pub description: Option<String>,
}

/// A verified-to-exist reference to a command name.
#[derive(Debug, Clone)]
pub struct CommandRef(pub String);

/// A verified-to-exist reference to a response name.
#[derive(Debug, Clone)]
pub struct ResponseRef(pub String);

/// A verified-to-exist reference to a conversion name.
#[derive(Debug, Clone)]
pub struct ConversionRef(pub String);

/// SCPI auto-parse response types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScpiResponseType {
    Float,
    Integer,
    String,
    Boolean,
    ArrayFloat,
}

impl ScpiResponseType {
    /// Parse a SCPI response type from a string label.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "float" => Some(Self::Float),
            "integer" | "int" => Some(Self::Integer),
            "string" | "str" => Some(Self::String),
            "boolean" | "bool" => Some(Self::Boolean),
            "array_float" | "arrayfloat" => Some(Self::ArrayFloat),
            _ => None,
        }
    }
}

/// Validated response parser - one of three tiers.
#[derive(Debug)]
pub enum ResponseParser {
    /// Tier 1: Format string parser.
    Format(ValidatedFormat),
    /// Tier 2: Transform pipeline.
    Transform(TransformPipeline),
    /// Tier 3: Regex with named capture groups.
    Regex(ValidatedRegex),
}

/// A validated format string, pre-parsed into segments.
#[derive(Debug)]
pub struct ValidatedFormat {
    /// The original format string.
    pub source: String,
    /// Pre-parsed segments.
    pub segments: Vec<FormatSegment>,
}

/// A validated MiniJinja template, verified to parse.
#[derive(Debug)]
pub struct ValidatedTemplate {
    /// The original template source.
    pub source: String,
}

/// A validated evalexpr formula, verified to parse.
#[derive(Debug)]
pub struct ValidatedFormula {
    /// The original formula source.
    pub source: String,
}

/// A validated regex, verified to compile.
#[derive(Debug)]
pub struct ValidatedRegex {
    /// The original regex pattern.
    pub source: String,
    /// The compiled regex.
    pub regex: regex::Regex,
}

/// All validated capability configurations.
#[derive(Debug, Default)]
pub struct CapabilitySet {
    pub movable: Option<MovableConfig>,
    pub readable: Option<ReadableConfig>,
    pub settable: Option<SettableConfig>,
    pub shutter_control: Option<ShutterControlConfig>,
    pub wavelength_tunable: Option<WavelengthTunableConfig>,
    pub emission_control: Option<EmissionControlConfig>,
    pub spectrum_readable: Option<SpectrumReadableConfig>,
}

/// Validated Movable capability configuration.
#[derive(Debug)]
pub struct MovableConfig {
    pub move_abs: Option<MethodConfig>,
    pub move_rel: Option<MethodConfig>,
    pub position: Option<MethodConfig>,
    pub stop: Option<MethodConfig>,
    pub wait_settled: Option<WaitSettledConfig>,
}

/// Validated Readable capability configuration.
#[derive(Debug)]
pub struct ReadableConfig {
    pub read: Option<MethodConfig>,
}

/// Validated Settable capability configuration.
#[derive(Debug)]
pub struct SettableConfig {
    pub set: Option<MethodConfig>,
}

/// Validated ShutterControl capability configuration.
#[derive(Debug)]
pub struct ShutterControlConfig {
    pub open: Option<MethodConfig>,
    pub close: Option<MethodConfig>,
    pub is_open: Option<MethodConfig>,
}

/// Validated WavelengthTunable capability configuration.
#[derive(Debug)]
pub struct WavelengthTunableConfig {
    pub set_wavelength: Option<MethodConfig>,
    pub get_wavelength: Option<MethodConfig>,
}

/// Validated EmissionControl capability configuration.
#[derive(Debug)]
pub struct EmissionControlConfig {
    pub enable: Option<MethodConfig>,
    pub disable: Option<MethodConfig>,
    pub is_enabled: Option<MethodConfig>,
}

/// Validated SpectrumReadable capability configuration (1D detectors).
#[derive(Debug)]
pub struct SpectrumReadableConfig {
    pub read_spectrum: Option<MethodConfig>,
    pub spectrum_length: usize,
    pub value_units: String,
    pub axis_units: Option<String>,
}

/// A validated method-to-command mapping.
#[derive(Debug)]
pub struct MethodConfig {
    /// Reference to the command to invoke.
    pub command: CommandRef,

    /// Optional input conversion reference.
    pub input_conversion: Option<ConversionRef>,

    /// Parameter name to pass converted input as.
    pub input_param: Option<String>,

    /// Parameter name to extract input from.
    pub from_param: Option<String>,

    /// Optional output conversion reference.
    pub output_conversion: Option<ConversionRef>,

    /// Field name to extract from parsed response.
    pub output_field: Option<String>,
}

/// Validated wait/poll settling configuration.
#[derive(Debug)]
pub struct WaitSettledConfig {
    /// Reference to the command to poll.
    pub poll_command: CommandRef,

    /// Success condition expression.
    pub success_condition: String,

    /// Poll interval.
    pub poll_interval_ms: u32,

    /// Total timeout.
    pub timeout_ms: u32,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- BaudRate ----

    #[test]
    fn test_baud_rate_valid_minimum() {
        let b = BaudRate::new(300).unwrap();
        assert_eq!(b.value(), 300);
    }

    #[test]
    fn test_baud_rate_valid_maximum() {
        let b = BaudRate::new(921_600).unwrap();
        assert_eq!(b.value(), 921_600);
    }

    #[test]
    fn test_baud_rate_common_values() {
        for &rate in &[9600u32, 19200, 38400, 57600, 115200] {
            BaudRate::new(rate).unwrap_or_else(|e| panic!("{rate} should be valid: {e}"));
        }
    }

    #[test]
    fn test_baud_rate_below_minimum() {
        let err = BaudRate::new(299).unwrap_err();
        assert!(err.contains("out of range"), "error message: {err}");
    }

    #[test]
    fn test_baud_rate_zero_is_invalid() {
        assert!(BaudRate::new(0).is_err());
    }

    #[test]
    fn test_baud_rate_above_maximum() {
        let err = BaudRate::new(921_601).unwrap_err();
        assert!(err.contains("out of range"), "error message: {err}");
    }

    #[test]
    fn test_baud_rate_copy() {
        let b = BaudRate::new(9600).unwrap();
        let c = b;
        assert_eq!(b.value(), c.value());
    }

    // ---- Timeout ----

    #[test]
    fn test_timeout_valid_minimum() {
        let t = Timeout::new(1).unwrap();
        assert_eq!(t.value(), 1);
    }

    #[test]
    fn test_timeout_valid_maximum() {
        let t = Timeout::new(60_000).unwrap();
        assert_eq!(t.value(), 60_000);
    }

    #[test]
    fn test_timeout_zero_is_invalid() {
        assert!(Timeout::new(0).is_err());
    }

    #[test]
    fn test_timeout_above_maximum() {
        let err = Timeout::new(60_001).unwrap_err();
        assert!(err.contains("out of range"), "error message: {err}");
    }

    #[test]
    fn test_timeout_as_duration() {
        let t = Timeout::new(1000).unwrap();
        assert_eq!(t.as_duration(), std::time::Duration::from_millis(1000));
    }

    #[test]
    fn test_timeout_as_duration_minimum() {
        let t = Timeout::new(1).unwrap();
        assert_eq!(t.as_duration(), std::time::Duration::from_millis(1));
    }

    #[test]
    fn test_timeout_copy() {
        let t = Timeout::new(500).unwrap();
        let c = t;
        assert_eq!(t.value(), c.value());
    }

    // ---- ScpiResponseType ----

    #[test]
    fn test_scpi_response_type_parse_float() {
        assert_eq!(ScpiResponseType::parse("float"), Some(ScpiResponseType::Float));
        assert_eq!(ScpiResponseType::parse("FLOAT"), Some(ScpiResponseType::Float));
    }

    #[test]
    fn test_scpi_response_type_parse_integer() {
        assert_eq!(
            ScpiResponseType::parse("integer"),
            Some(ScpiResponseType::Integer)
        );
        assert_eq!(ScpiResponseType::parse("int"), Some(ScpiResponseType::Integer));
        assert_eq!(ScpiResponseType::parse("INT"), Some(ScpiResponseType::Integer));
    }

    #[test]
    fn test_scpi_response_type_parse_string() {
        assert_eq!(
            ScpiResponseType::parse("string"),
            Some(ScpiResponseType::String)
        );
        assert_eq!(ScpiResponseType::parse("str"), Some(ScpiResponseType::String));
    }

    #[test]
    fn test_scpi_response_type_parse_boolean() {
        assert_eq!(
            ScpiResponseType::parse("boolean"),
            Some(ScpiResponseType::Boolean)
        );
        assert_eq!(
            ScpiResponseType::parse("bool"),
            Some(ScpiResponseType::Boolean)
        );
    }

    #[test]
    fn test_scpi_response_type_parse_array_float() {
        assert_eq!(
            ScpiResponseType::parse("array_float"),
            Some(ScpiResponseType::ArrayFloat)
        );
        assert_eq!(
            ScpiResponseType::parse("arrayfloat"),
            Some(ScpiResponseType::ArrayFloat)
        );
    }

    #[test]
    fn test_scpi_response_type_parse_unknown() {
        assert_eq!(ScpiResponseType::parse("unknown_type"), None);
        assert_eq!(ScpiResponseType::parse(""), None);
    }

    #[test]
    fn test_scpi_response_type_equality() {
        assert_eq!(ScpiResponseType::Float, ScpiResponseType::Float);
        assert_ne!(ScpiResponseType::Float, ScpiResponseType::Integer);
    }
}
