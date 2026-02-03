use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstrumentConfig {
    /// Optional base template to inherit from
    #[serde(default)]
    pub extends: Option<String>,
    pub device: DeviceConfig,
    #[serde(default)]
    pub connection: ConnectionConfig,
    #[serde(default)]
    pub parameters: HashMap<String, ParameterDef>,
    #[serde(default)]
    pub commands: HashMap<String, CommandConfig>,
    #[serde(default)]
    pub responses: HashMap<String, ResponseConfig>,
    #[serde(default)]
    pub conversions: HashMap<String, ConversionConfig>,
    #[serde(default)]
    pub trait_mapping: TraitMappingConfig,
    #[serde(default)]
    pub error_codes: HashMap<String, ErrorCodeConfig>,
    #[serde(default)]
    pub default_retry: Option<RetryConfig>,
    /// Expanded parameters (shorthand → full ParameterConfig).
    /// Populated by calling `expand_shorthand()`.
    #[serde(skip)]
    pub expanded_parameters: Option<HashMap<String, ParameterConfig>>,
}

impl InstrumentConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, response) in &self.responses {
            Regex::new(&response.pattern)
                .map_err(|e| format!("Invalid regex in response '{}': {}", name, e))?;
        }
        Ok(())
    }

    /// Expand all shorthand parameter definitions to full ParameterConfig form.
    ///
    /// This converts all `ParameterDef` variants (Simple, GetSet, Full) into
    /// their expanded `ParameterConfig` representation and stores them in
    /// `expanded_parameters`.
    ///
    /// This should be called after loading a config file and before using it
    /// to construct a driver.
    ///
    /// # Example
    ///
    /// ```
    /// use plugin_api::config::InstrumentConfig;
    ///
    /// let toml_str = r#"
    /// [device]
    /// name = "Test Device"
    /// protocol = "test"
    ///
    /// [connection]
    /// baud_rate = 9600
    ///
    /// [parameters]
    /// voltage = "VOLT?"
    ///
    /// [commands]
    /// [responses]
    /// [trait_mapping]
    /// "#;
    ///
    /// let mut config: InstrumentConfig = toml::from_str(toml_str).unwrap();
    /// config.expand_shorthand();
    ///
    /// assert!(config.expanded_parameters.is_some());
    /// let expanded = config.expanded_parameters.as_ref().unwrap();
    /// assert!(expanded.contains_key("voltage"));
    /// ```
    pub fn expand_shorthand(&mut self) {
        let expanded: HashMap<String, ParameterConfig> = self
            .parameters
            .iter()
            .map(|(name, def)| (name.clone(), def.expand(name)))
            .collect();
        self.expanded_parameters = Some(expanded);
    }

    /// Get the expanded parameter config for a given parameter name.
    ///
    /// Returns None if expand_shorthand() hasn't been called yet or if the
    /// parameter doesn't exist.
    pub fn get_expanded_parameter(&self, name: &str) -> Option<&ParameterConfig> {
        self.expanded_parameters
            .as_ref()
            .and_then(|params| params.get(name))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceConfig {
    pub name: String,
    pub protocol: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub description: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionConfig {
    #[serde(default)]
    pub r#type: ConnectionType,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default)]
    pub parity: Parity,
    #[serde(default)]
    pub flow_control: FlowControl,
    #[serde(default)]
    pub terminator_tx: String,
    #[serde(default = "default_terminator_rx")]
    pub terminator_rx: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub bus: Option<BusConfig>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            r#type: ConnectionType::Serial,
            baud_rate: default_baud_rate(),
            data_bits: default_data_bits(),
            stop_bits: default_stop_bits(),
            parity: Parity::None,
            flow_control: FlowControl::None,
            terminator_tx: String::new(),
            terminator_rx: default_terminator_rx(),
            timeout_ms: default_timeout_ms(),
            bus: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    #[default]
    Serial,
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FlowControl {
    #[default]
    None,
    Software,
    Hardware,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BusConfig {
    pub r#type: String,
    #[serde(default = "default_address_format")]
    pub address_format: AddressFormat,
    #[serde(default = "default_bus_address")]
    pub default_address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AddressFormat {
    #[default]
    HexChar,
    Decimal,
    HexByte,
}

/// Parameter definition supporting shorthand and verbose syntax.
///
/// # Variant Order (CRITICAL)
/// Serde tries variants in order. Primitives MUST come before structs:
/// 1. Simple (String) - primitive
/// 2. GetSet (struct with get/set) - specific struct shape
/// 3. Full (ParameterConfig) - any struct
///
/// # Examples
/// ```toml
/// # Simple: read-only float
/// voltage = "MEAS:VOLT?"
///
/// # GetSet: read-write float
/// current = { get = "MEAS:CURR?", set = "CURR {}" }
///
/// # Full: verbose form (existing)
/// [parameters.power]
/// type = "float"
/// default = 0.0
/// unit = "W"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterDef {
    /// Simple read-only parameter: "COMMAND?" → infers RO float
    Simple(String),

    /// Get/Set pair: { get = "X?", set = "X {}" } → infers RW float
    GetSet { get: String, set: String },

    /// Full verbose form (existing ParameterConfig)
    Full(ParameterConfig),
}

impl ParameterDef {
    /// Expand shorthand to full ParameterConfig.
    ///
    /// # Simple Form
    /// Infers: type=Float, read-only (no set command)
    ///
    /// # GetSet Form
    /// Infers: type=Float, read-write
    ///
    /// # Full Form
    /// Returns as-is
    pub fn expand(&self, name: &str) -> ParameterConfig {
        match self {
            ParameterDef::Simple(_cmd) => ParameterConfig {
                r#type: ParameterType::Float,
                default: serde_json::Value::Null,
                range: None,
                unit: None,
                description: Some(format!("Parameter: {}", name)),
            },
            ParameterDef::GetSet { get: _, set: _ } => ParameterConfig {
                r#type: ParameterType::Float,
                default: serde_json::Value::Null,
                range: None,
                unit: None,
                description: Some(format!("Parameter: {}", name)),
            },
            ParameterDef::Full(config) => config.clone(),
        }
    }

    /// Get the command string for simple form (if applicable).
    pub fn get_command(&self) -> Option<&str> {
        match self {
            ParameterDef::Simple(cmd) => Some(cmd.as_str()),
            ParameterDef::GetSet { get, set: _ } => Some(get.as_str()),
            ParameterDef::Full(_) => None,
        }
    }

    /// Get the set command string (if applicable).
    pub fn set_command(&self) -> Option<&str> {
        match self {
            ParameterDef::Simple(_) => None,
            ParameterDef::GetSet { get: _, set } => Some(set.as_str()),
            ParameterDef::Full(_) => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterConfig {
    pub r#type: ParameterType,
    #[serde(default)]
    pub default: serde_json::Value,
    pub range: Option<(serde_json::Value, serde_json::Value)>,
    pub unit: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    String,
    Int,
    Float,
    Bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandConfig {
    pub template: String,
    pub description: Option<String>,
    #[serde(default = "default_expects_response")]
    pub expects_response: bool,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    pub response: Option<String>,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseConfig {
    pub pattern: String,
    #[serde(default)]
    pub fields: HashMap<String, ResponseFieldConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFieldConfig {
    pub r#type: ResponseFieldType,
    #[serde(default)]
    pub signed: bool,
    /// Optional regex pattern to extract a substring before applying transforms.
    /// Uses the first capture group (group 1) by default.
    #[serde(default)]
    pub extract_regex: Option<String>,
    /// Optional declarative transform pipeline (string shorthand).
    /// Applied after extract_regex (if present) and before type parsing.
    /// Example: `["trim", "remove_suffix('C')", "to_float"]`
    #[serde(default)]
    pub transform: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFieldType {
    HexU32,
    HexI32,
    Int,
    Float,
    String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversionConfig {
    pub formula: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TraitMappingConfig {
    #[serde(rename = "Movable")]
    pub movable: Option<HashMap<String, TraitCommandMapping>>,
    #[serde(flatten)]
    pub traits: HashMap<String, HashMap<String, TraitCommandMapping>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TraitCommandMapping {
    pub command: Option<String>,
    pub poll_command: Option<String>,
    pub input_conversion: Option<String>,
    pub input_param: Option<String>,
    pub from_param: Option<String>,
    pub output_conversion: Option<String>,
    pub output_field: Option<String>,
    pub success_condition: Option<String>,
    pub poll_interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorCodeConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub severity: ErrorSeverity,
    #[serde(default = "default_recoverable")]
    pub recoverable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    Warning,
    #[default]
    Error,
    Critical,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RetryConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,
    #[serde(default = "default_initial_delay_ms")]
    pub initial_delay_ms: u32,
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u32,
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
    #[serde(default)]
    pub retry_on_errors: Vec<String>,
    #[serde(default)]
    pub no_retry_on_errors: Vec<String>,
}

/// Default baud rate for serial connections (9600).
pub fn default_baud_rate() -> u32 {
    9600
}
/// Default data bits for serial connections (8).
pub fn default_data_bits() -> u8 {
    8
}
/// Default stop bits for serial connections (1).
pub fn default_stop_bits() -> u8 {
    1
}
/// Default receive terminator ("\r\n").
pub fn default_terminator_rx() -> String {
    "\r\n".to_string()
}
/// Default timeout in milliseconds (1000).
pub fn default_timeout_ms() -> u64 {
    1000
}
fn default_expects_response() -> bool {
    true
}
fn default_address_format() -> AddressFormat {
    AddressFormat::HexChar
}
fn default_bus_address() -> String {
    "0".to_string()
}
fn default_recoverable() -> bool {
    true
}
fn default_max_retries() -> u8 {
    3
}
fn default_initial_delay_ms() -> u32 {
    100
}
fn default_max_delay_ms() -> u32 {
    1000
}
fn default_backoff_multiplier() -> f64 {
    2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_def_simple_roundtrip() {
        let toml_str = r#"voltage = "MEAS:VOLT?""#;
        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();

        let voltage = parsed.get("voltage").unwrap();
        assert!(matches!(voltage, ParameterDef::Simple(_)));

        if let ParameterDef::Simple(cmd) = voltage {
            assert_eq!(cmd, "MEAS:VOLT?");
        }

        // Round-trip
        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: HashMap<String, ParameterDef> = toml::from_str(&serialized).unwrap();
        assert!(matches!(
            reparsed.get("voltage").unwrap(),
            ParameterDef::Simple(_)
        ));
    }

    #[test]
    fn test_parameter_def_getset_roundtrip() {
        let toml_str = r#"current = { get = "MEAS:CURR?", set = "CURR {}" }"#;
        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();

        let current = parsed.get("current").unwrap();
        assert!(matches!(current, ParameterDef::GetSet { .. }));

        if let ParameterDef::GetSet { get, set } = current {
            assert_eq!(get, "MEAS:CURR?");
            assert_eq!(set, "CURR {}");
        }

        // Round-trip
        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: HashMap<String, ParameterDef> = toml::from_str(&serialized).unwrap();
        assert!(matches!(
            reparsed.get("current").unwrap(),
            ParameterDef::GetSet { .. }
        ));
    }

    #[test]
    fn test_parameter_def_full_roundtrip() {
        let toml_str = r#"
[power]
type = "float"
default = 0.0
unit = "W"
description = "Output power"
"#;
        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();

        let power = parsed.get("power").unwrap();
        assert!(matches!(power, ParameterDef::Full(_)));

        if let ParameterDef::Full(config) = power {
            assert_eq!(config.r#type, ParameterType::Float);
            assert_eq!(config.unit, Some("W".to_string()));
            assert_eq!(config.description, Some("Output power".to_string()));
        }

        // Round-trip
        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: HashMap<String, ParameterDef> = toml::from_str(&serialized).unwrap();
        assert!(matches!(
            reparsed.get("power").unwrap(),
            ParameterDef::Full(_)
        ));
    }

    #[test]
    fn test_parameter_def_expand_simple() {
        let def = ParameterDef::Simple("MEAS:VOLT?".to_string());
        let config = def.expand("voltage");

        assert_eq!(config.r#type, ParameterType::Float);
        assert_eq!(config.default, serde_json::Value::Null);
        assert_eq!(config.range, None);
        assert_eq!(config.unit, None);
        assert_eq!(config.description, Some("Parameter: voltage".to_string()));
    }

    #[test]
    fn test_parameter_def_expand_getset() {
        let def = ParameterDef::GetSet {
            get: "MEAS:CURR?".to_string(),
            set: "CURR {}".to_string(),
        };
        let config = def.expand("current");

        assert_eq!(config.r#type, ParameterType::Float);
        assert_eq!(config.default, serde_json::Value::Null);
    }

    #[test]
    fn test_parameter_def_expand_full() {
        let original = ParameterConfig {
            r#type: ParameterType::Int,
            default: serde_json::json!(42),
            range: Some((serde_json::json!(0), serde_json::json!(100))),
            unit: Some("mA".to_string()),
            description: Some("Current setpoint".to_string()),
        };

        let def = ParameterDef::Full(original.clone());
        let expanded = def.expand("current");

        assert_eq!(expanded.r#type, original.r#type);
        assert_eq!(expanded.default, original.default);
        assert_eq!(expanded.range, original.range);
        assert_eq!(expanded.unit, original.unit);
        assert_eq!(expanded.description, original.description);
    }

    #[test]
    fn test_parameter_def_get_command() {
        let simple = ParameterDef::Simple("VOLT?".to_string());
        assert_eq!(simple.get_command(), Some("VOLT?"));

        let getset = ParameterDef::GetSet {
            get: "CURR?".to_string(),
            set: "CURR {}".to_string(),
        };
        assert_eq!(getset.get_command(), Some("CURR?"));

        let full = ParameterDef::Full(ParameterConfig {
            r#type: ParameterType::Float,
            default: serde_json::Value::Null,
            range: None,
            unit: None,
            description: None,
        });
        assert_eq!(full.get_command(), None);
    }

    #[test]
    fn test_parameter_def_set_command() {
        let simple = ParameterDef::Simple("VOLT?".to_string());
        assert_eq!(simple.set_command(), None);

        let getset = ParameterDef::GetSet {
            get: "CURR?".to_string(),
            set: "CURR {}".to_string(),
        };
        assert_eq!(getset.set_command(), Some("CURR {}"));

        let full = ParameterDef::Full(ParameterConfig {
            r#type: ParameterType::Float,
            default: serde_json::Value::Null,
            range: None,
            unit: None,
            description: None,
        });
        assert_eq!(full.set_command(), None);
    }

    #[test]
    fn test_mixed_parameter_definitions() {
        let toml_str = r#"
# Simple form
voltage = "MEAS:VOLT?"

# GetSet form
current = { get = "MEAS:CURR?", set = "CURR {}" }

# Full form
[power]
type = "float"
default = 1.0
unit = "W"
"#;

        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();

        assert_eq!(parsed.len(), 3);
        assert!(matches!(
            parsed.get("voltage").unwrap(),
            ParameterDef::Simple(_)
        ));
        assert!(matches!(
            parsed.get("current").unwrap(),
            ParameterDef::GetSet { .. }
        ));
        assert!(matches!(
            parsed.get("power").unwrap(),
            ParameterDef::Full(_)
        ));
    }

    #[test]
    fn test_existing_verbose_configs_still_parse() {
        // This ensures backward compatibility with existing config files
        let toml_str = r#"
[wavelength_nm]
type = "float"
default = 800.0
range = [690.0, 1040.0]
unit = "nm"
description = "Tunable laser wavelength"
"#;

        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();
        let wavelength = parsed.get("wavelength_nm").unwrap();

        assert!(matches!(wavelength, ParameterDef::Full(_)));

        if let ParameterDef::Full(config) = wavelength {
            assert_eq!(config.r#type, ParameterType::Float);
            assert_eq!(config.unit, Some("nm".to_string()));
            assert!(config.range.is_some());
        }
    }

    #[test]
    fn test_real_maitai_config_snippet() {
        // Real snippet from config/devices/maitai.toml to ensure backward compat
        let toml_str = r#"
[wavelength_nm]
type = "float"
default = 800.0
range = [690.0, 1040.0]
unit = "nm"
description = "Tunable laser wavelength"

[shutter_open]
type = "bool"
default = false
description = "Shutter state (true=open, false=closed)"

[emission_on]
type = "bool"
default = false
description = "Emission state (true=on, false=off)"
"#;

        let parsed: HashMap<String, ParameterDef> = toml::from_str(toml_str).unwrap();

        // All should parse as Full variants
        assert_eq!(parsed.len(), 3);
        assert!(matches!(
            parsed.get("wavelength_nm").unwrap(),
            ParameterDef::Full(_)
        ));
        assert!(matches!(
            parsed.get("shutter_open").unwrap(),
            ParameterDef::Full(_)
        ));
        assert!(matches!(
            parsed.get("emission_on").unwrap(),
            ParameterDef::Full(_)
        ));

        // Verify wavelength details
        if let ParameterDef::Full(config) = parsed.get("wavelength_nm").unwrap() {
            assert_eq!(config.r#type, ParameterType::Float);
            assert_eq!(config.default, serde_json::json!(800.0));
            assert_eq!(config.unit, Some("nm".to_string()));
        }

        // Verify bool types
        if let ParameterDef::Full(config) = parsed.get("shutter_open").unwrap() {
            assert_eq!(config.r#type, ParameterType::Bool);
            assert_eq!(config.default, serde_json::json!(false));
        }
    }
}
