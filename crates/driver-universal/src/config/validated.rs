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
    /// Optional inline UI declaration (bd-jcb4x G1). When present, feeds
    /// Phase 2 G2 auto-synthesis of [`ControlPanelConfig`]. `None` means the
    /// synthesizer falls back to dtype/range-driven defaults.
    pub ui_hint: Option<ParameterUiHint>,
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

    /// Optional inline UI declaration (bd-jcb4x G1). When present, feeds
    /// Phase 2 G2 auto-synthesis of [`ControlPanelConfig`]. Has no effect on
    /// runtime driver behavior.
    pub ui_hint: Option<CommandUiHint>,
}

/// Validated widget-shape hint for an inline UI declaration (bd-jcb4x G1).
///
/// Each variant corresponds to one of the renderer's concrete widget types.
/// Unknown strings from the manifest produce a validation error in stage 2;
/// this enum is what Phase 2 G2 synthesis keys off to pick an egui widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetHint {
    /// Slider widget: bounded numeric with min/max range (needs a range).
    Slider,
    /// Boolean toggle / checkbox.
    Toggle,
    /// Dropdown selecting one of a fixed set of enum values.
    EnumSelect,
    /// Plain numeric text input (no slider).
    NumericInput,
    /// Plain text line edit (for string parameters).
    TextInput,
    /// Action button (for commands with no input).
    Button,
}

impl WidgetHint {
    /// Parse a widget-hint string from the manifest. Returns `None` for
    /// unrecognised values; callers report that as a `ConfigError`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "slider" => Some(Self::Slider),
            "toggle" | "checkbox" => Some(Self::Toggle),
            "enum_select" | "enum" | "select" | "dropdown" => Some(Self::EnumSelect),
            "numeric_input" | "numeric" | "number" => Some(Self::NumericInput),
            "text_input" | "text" | "string" => Some(Self::TextInput),
            "button" | "action" => Some(Self::Button),
            _ => None,
        }
    }

    /// Canonical string label, matching the primary spelling accepted by
    /// [`WidgetHint::parse`]. Used for diagnostics and round-tripping.
    pub fn label(self) -> &'static str {
        match self {
            Self::Slider => "slider",
            Self::Toggle => "toggle",
            Self::EnumSelect => "enum_select",
            Self::NumericInput => "numeric_input",
            Self::TextInput => "text_input",
            Self::Button => "button",
        }
    }

    /// All valid widget-hint string values, for documenting the schema /
    /// producing helpful error messages.
    pub const ALL_LABELS: &'static [&'static str] = &[
        "slider",
        "toggle",
        "enum_select",
        "numeric_input",
        "text_input",
        "button",
    ];
}

/// Validated inline UI declaration for a command (bd-jcb4x G1).
///
/// Every field is optional — Phase 2 G2 synthesis applies sensible defaults
/// when fields are omitted. Used only by the synthesizer; runtime ignores
/// this completely.
#[derive(Debug, Clone)]
pub struct CommandUiHint {
    /// Human-readable label for the generated widget. Falls back to the
    /// command name when `None`.
    pub label: Option<String>,

    /// Suggested widget shape. `None` lets the synthesizer pick based on
    /// command shape (zero-parameter → Button, boolean parameter → Toggle,
    /// etc).
    pub widget_hint: Option<WidgetHint>,

    /// Named group for clustering related commands in the synthesized panel.
    /// Seeds Phase 2 G3's named layout slots.
    pub group: Option<String>,

    /// Tooltip / help text. Falls back to the command's top-level
    /// `description` field when `None`.
    pub description: Option<String>,
}

/// Validated inline UI declaration for a parameter (bd-jcb4x G1).
///
/// Companion to [`CommandUiHint`] for the `[parameters.<name>.ui]` table.
/// Phase 2 G2 synthesis picks a widget from `widget_hint` if present, or
/// falls back to the parameter's `dtype` and range.
#[derive(Debug, Clone)]
pub struct ParameterUiHint {
    /// Human-readable label. Falls back to the parameter name when `None`.
    pub label: Option<String>,

    /// Suggested widget shape. `None` lets the synthesizer pick based on
    /// dtype + range (e.g. float with min/max → Slider, bool → Toggle,
    /// enum with enum_values → EnumSelect).
    pub widget_hint: Option<WidgetHint>,

    /// Named group for clustering related parameters in the synthesized
    /// panel.
    pub group: Option<String>,

    /// Optional slider / numeric-input step size. Only meaningful for
    /// numeric widgets.
    pub step: Option<f64>,

    /// Discrete allowed values for `EnumSelect`. Ignored for other widget
    /// shapes.
    pub enum_values: Vec<String>,
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

/// A validated format-string response parser.
///
/// Holds one or more variants, each pre-parsed into segments. Parsing tries
/// each variant in declaration order; the first one that matches the input
/// wins. A manifest that declares `format = "..."` (single variant) and one
/// that declares `variants = [...]` (multi) both land here — downstream
/// code sees a uniform structure.
#[derive(Debug)]
pub struct ValidatedFormat {
    /// One or more format variants. Invariant: at least one, enforced at
    /// construction time.
    pub variants: Vec<FormatVariant>,
}

impl ValidatedFormat {
    /// First variant's pre-parsed segments. Used by the emulator's inverse
    /// path, which synthesizes one representative response per command.
    pub fn first_segments(&self) -> &[FormatSegment] {
        &self.variants[0].segments
    }

    /// First variant's source string (for diagnostics).
    pub fn first_source(&self) -> &str {
        &self.variants[0].source
    }
}

/// A single format-string variant: source text + pre-parsed segments.
#[derive(Debug)]
pub struct FormatVariant {
    /// The original format string as written in the manifest.
    pub source: String,
    /// Pre-parsed segments, ready to feed `format_parser::parse_response`.
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
        assert_eq!(
            ScpiResponseType::parse("float"),
            Some(ScpiResponseType::Float)
        );
        assert_eq!(
            ScpiResponseType::parse("FLOAT"),
            Some(ScpiResponseType::Float)
        );
    }

    #[test]
    fn test_scpi_response_type_parse_integer() {
        assert_eq!(
            ScpiResponseType::parse("integer"),
            Some(ScpiResponseType::Integer)
        );
        assert_eq!(
            ScpiResponseType::parse("int"),
            Some(ScpiResponseType::Integer)
        );
        assert_eq!(
            ScpiResponseType::parse("INT"),
            Some(ScpiResponseType::Integer)
        );
    }

    #[test]
    fn test_scpi_response_type_parse_string() {
        assert_eq!(
            ScpiResponseType::parse("string"),
            Some(ScpiResponseType::String)
        );
        assert_eq!(
            ScpiResponseType::parse("str"),
            Some(ScpiResponseType::String)
        );
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

    // ---- bd-jcb4x G1: WidgetHint ----

    #[test]
    fn widget_hint_parses_canonical_labels() {
        // Every variant must round-trip through label() + parse().
        for hint in [
            WidgetHint::Slider,
            WidgetHint::Toggle,
            WidgetHint::EnumSelect,
            WidgetHint::NumericInput,
            WidgetHint::TextInput,
            WidgetHint::Button,
        ] {
            assert_eq!(WidgetHint::parse(hint.label()), Some(hint));
        }
    }

    #[test]
    fn widget_hint_parses_aliases() {
        // Documented aliases must resolve to the right variant.
        assert_eq!(WidgetHint::parse("checkbox"), Some(WidgetHint::Toggle));
        assert_eq!(WidgetHint::parse("enum"), Some(WidgetHint::EnumSelect));
        assert_eq!(WidgetHint::parse("select"), Some(WidgetHint::EnumSelect));
        assert_eq!(WidgetHint::parse("dropdown"), Some(WidgetHint::EnumSelect));
        assert_eq!(WidgetHint::parse("numeric"), Some(WidgetHint::NumericInput));
        assert_eq!(WidgetHint::parse("number"), Some(WidgetHint::NumericInput));
        assert_eq!(WidgetHint::parse("text"), Some(WidgetHint::TextInput));
        assert_eq!(WidgetHint::parse("string"), Some(WidgetHint::TextInput));
        assert_eq!(WidgetHint::parse("action"), Some(WidgetHint::Button));
    }

    #[test]
    fn widget_hint_parse_is_case_insensitive_and_trimmed() {
        assert_eq!(WidgetHint::parse("  SLIDER  "), Some(WidgetHint::Slider));
        assert_eq!(WidgetHint::parse("Toggle"), Some(WidgetHint::Toggle));
        assert_eq!(
            WidgetHint::parse("Enum_Select"),
            Some(WidgetHint::EnumSelect)
        );
    }

    #[test]
    fn widget_hint_parse_rejects_unknown() {
        assert_eq!(WidgetHint::parse(""), None);
        assert_eq!(WidgetHint::parse("gyroscope"), None);
        assert_eq!(WidgetHint::parse("float"), None);
    }

    #[test]
    fn widget_hint_all_labels_parse_back() {
        // The ALL_LABELS constant is surfaced in error messages and editor
        // documentation; every entry must actually parse.
        for label in WidgetHint::ALL_LABELS {
            assert!(
                WidgetHint::parse(label).is_some(),
                "ALL_LABELS entry {label:?} must parse back"
            );
        }
    }
}
