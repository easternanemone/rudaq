//! Stage 1: Permissive serde deserialization types.
//!
//! These types accept loose TOML input with minimal validation. The real
//! validation happens in Stage 2 (`parse.rs` -> `validated.rs`).

use serde::Deserialize;
use std::collections::HashMap;

/// The top-level raw manifest as read from a TOML file.
#[derive(Debug, Deserialize)]
pub struct RawManifest {
    /// Must be 3 for this schema version.
    pub schema_version: u32,

    /// Device metadata.
    pub device: RawDeviceConfig,

    /// Connection configuration.
    pub connection: RawConnectionConfig,

    /// Named command definitions.
    #[serde(default)]
    pub commands: HashMap<String, RawCommandConfig>,

    /// Named response format definitions.
    #[serde(default)]
    pub responses: HashMap<String, RawResponseConfig>,

    /// Named conversion formulas.
    #[serde(default)]
    pub conversions: HashMap<String, RawConversionConfig>,

    /// Capability-to-command mappings.
    #[serde(default)]
    pub capabilities: RawCapabilityConfig,
}

/// Device metadata section.
#[derive(Debug, Deserialize)]
pub struct RawDeviceConfig {
    /// Human-readable device name.
    pub name: String,

    /// List of capabilities this device supports.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Connection configuration, tagged by type.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RawConnectionConfig {
    Serial {
        baud_rate: u32,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
        /// Optional line terminator (e.g. "\n", "\r\n").
        #[serde(default)]
        terminator: Option<String>,
    },
    Tcp {
        host: String,
        port: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
    },
    Udp {
        host: String,
        port: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
    },
}

fn default_timeout_ms() -> u32 {
    1000
}

/// A single command definition.
#[derive(Debug, Deserialize)]
pub struct RawCommandConfig {
    /// MiniJinja template string for command construction.
    pub template: String,

    /// Optional parameter type declarations (name -> type string).
    #[serde(default)]
    pub parameters: HashMap<String, String>,

    /// Optional reference to a response format for parsing the reply.
    #[serde(default)]
    pub response: Option<String>,

    /// Optional SCPI auto-parse type (alternative to explicit response).
    #[serde(default)]
    pub response_type: Option<String>,

    /// Whether this command expects a response at all. Default true.
    #[serde(default = "default_true")]
    pub expects_response: bool,
}

fn default_true() -> bool {
    true
}

/// A response format definition with tiered parsing options.
#[derive(Debug, Deserialize)]
pub struct RawResponseConfig {
    /// Tier 1: Format string for structured parsing.
    #[serde(default)]
    pub format: Option<String>,

    /// Tier 2: Transform pipeline (list of shorthand operations).
    #[serde(default)]
    pub transform: Option<Vec<String>>,

    /// Tier 3: Regex with named capture groups.
    #[serde(default)]
    pub regex: Option<String>,
}

/// A conversion formula definition.
#[derive(Debug, Deserialize)]
pub struct RawConversionConfig {
    /// An evalexpr formula string.
    pub formula: String,
}

/// Capability-to-command mappings.
#[derive(Debug, Default, Deserialize)]
pub struct RawCapabilityConfig {
    #[serde(default)]
    pub movable: Option<RawMovableMapping>,

    #[serde(default)]
    pub readable: Option<RawReadableMapping>,

    #[serde(default)]
    pub settable: Option<RawSettableMapping>,

    #[serde(default)]
    pub shutter_control: Option<RawShutterControlMapping>,

    #[serde(default)]
    pub wavelength_tunable: Option<RawWavelengthTunableMapping>,

    #[serde(default)]
    pub emission_control: Option<RawEmissionControlMapping>,

    /// Catch-all for future/custom capabilities.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// Method mappings for the Movable capability.
#[derive(Debug, Deserialize)]
pub struct RawMovableMapping {
    pub move_abs: Option<RawMethodMapping>,
    pub position: Option<RawMethodMapping>,
    pub stop: Option<RawMethodMapping>,
    pub wait_settled: Option<RawWaitSettledMapping>,
}

/// Method mappings for the Readable capability.
#[derive(Debug, Deserialize)]
pub struct RawReadableMapping {
    pub read: Option<RawMethodMapping>,
}

/// Method mappings for the Settable capability.
#[derive(Debug, Deserialize)]
pub struct RawSettableMapping {
    pub set: Option<RawMethodMapping>,
}

/// Method mappings for the ShutterControl capability.
#[derive(Debug, Deserialize)]
pub struct RawShutterControlMapping {
    pub open: Option<RawMethodMapping>,
    pub close: Option<RawMethodMapping>,
    pub is_open: Option<RawMethodMapping>,
}

/// Method mappings for the WavelengthTunable capability.
#[derive(Debug, Deserialize)]
pub struct RawWavelengthTunableMapping {
    pub set_wavelength: Option<RawMethodMapping>,
    pub get_wavelength: Option<RawMethodMapping>,
}

/// Method mappings for the EmissionControl capability.
#[derive(Debug, Deserialize)]
pub struct RawEmissionControlMapping {
    pub enable: Option<RawMethodMapping>,
    pub disable: Option<RawMethodMapping>,
    pub is_enabled: Option<RawMethodMapping>,
}

/// A mapping from a trait method to a command + optional conversions.
#[derive(Debug, Deserialize)]
pub struct RawMethodMapping {
    /// Reference to a command name in the `[commands]` table.
    pub command: String,

    /// Optional conversion to apply to input before sending.
    #[serde(default)]
    pub input_conversion: Option<String>,

    /// Name of the template parameter to fill with converted input.
    #[serde(default)]
    pub input_param: Option<String>,

    /// Name of the template parameter to extract input from.
    #[serde(default)]
    pub from_param: Option<String>,

    /// Optional conversion to apply to output after receiving.
    #[serde(default)]
    pub output_conversion: Option<String>,

    /// Field name to extract from parsed response.
    #[serde(default)]
    pub output_field: Option<String>,
}

/// Configuration for wait/poll-based settling.
#[derive(Debug, Deserialize)]
pub struct RawWaitSettledMapping {
    /// Command to poll for status.
    pub poll_command: String,

    /// Condition expression that indicates settled (e.g., "code == 0").
    pub success_condition: String,

    /// Interval between polls in milliseconds.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u32,

    /// Total timeout in milliseconds.
    #[serde(default = "default_settle_timeout")]
    pub timeout_ms: u32,
}

fn default_poll_interval() -> u32 {
    50
}

fn default_settle_timeout() -> u32 {
    10000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_ell14_config() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Thorlabs ELL14"
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000

[commands.move_absolute]
template = "{{ address }}ma{{ position_pulses | hex(8) }}"
parameters = { position_pulses = "int32" }
response = "position"

[commands.get_position]
template = "{{ address }}gp"
response = "position"

[commands.get_status]
template = "{{ address }}gs"
response = "status"

[commands.stop]
template = "{{ address }}st"
expects_response = false

[responses.position]
format = "{addr:1}PO{pulses:hex8}"

[responses.status]
format = "{addr:1}GS{code:hex2}"

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[capabilities.movable]
move_abs = { command = "move_absolute", input_conversion = "degrees_to_pulses", input_param = "position_pulses", from_param = "position" }
position = { command = "get_position", output_conversion = "pulses_to_degrees", output_field = "pulses" }
stop = { command = "stop" }

[capabilities.movable.wait_settled]
poll_command = "get_status"
success_condition = "code == 0"
poll_interval_ms = 50
timeout_ms = 10000
"#;

        let raw: RawManifest = toml::from_str(toml_str).expect("should parse ELL14 config");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Thorlabs ELL14");
        assert_eq!(raw.commands.len(), 4);
        assert_eq!(raw.responses.len(), 2);
        assert_eq!(raw.conversions.len(), 2);
        assert!(raw.capabilities.movable.is_some());
    }

    #[test]
    fn deserialize_scpi_tcp_config() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Keithley 2400"
capabilities = ["Readable", "Settable"]

[connection]
type = "tcp"
host = "192.168.1.50"
port = 5025
timeout_ms = 2000

[commands.measure_voltage]
template = ":MEAS:VOLT?"
response_type = "float"

[commands.set_voltage]
template = ":SOUR:VOLT {{ value }}"

[capabilities.readable]
read = { command = "measure_voltage" }

[capabilities.settable]
set = { command = "set_voltage", from_param = "value" }
"#;

        let raw: RawManifest = toml::from_str(toml_str).expect("should parse SCPI TCP config");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Keithley 2400");
        assert!(matches!(
            raw.connection,
            RawConnectionConfig::Tcp { port: 5025, .. }
        ));
        assert!(raw.capabilities.readable.is_some());
        assert!(raw.capabilities.settable.is_some());
    }

    #[test]
    fn deserialize_serial_defaults() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "serial"
baud_rate = 9600
"#;

        let raw: RawManifest = toml::from_str(toml_str).expect("should parse minimal config");
        match &raw.connection {
            RawConnectionConfig::Serial { timeout_ms, .. } => {
                assert_eq!(*timeout_ms, 1000, "default timeout should be 1000ms");
            }
            _ => panic!("expected serial connection"),
        }
    }
}
