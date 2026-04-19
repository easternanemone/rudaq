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

    /// Device parameters (type, default, range, etc.).
    #[serde(default)]
    pub parameters: HashMap<String, toml::Value>,

    /// Error code definitions.
    #[serde(default)]
    pub error_codes: HashMap<String, toml::Value>,

    /// Initialization sequence (commands to run on connect).
    #[serde(default)]
    pub init_sequence: Vec<toml::Value>,

    /// Default retry configuration.
    #[serde(default)]
    pub default_retry: Option<toml::Value>,

    /// UI configuration for control panels.
    #[serde(default)]
    pub ui: Option<toml::Value>,

    /// Inherits from another config file.
    #[serde(default)]
    pub extends: Option<String>,

    /// Validation rules for parameters.
    #[serde(default)]
    pub validation: Option<toml::Value>,

    // LEGACY: v1 config fields below are accepted but ignored in schema v3.
    // Remove after all manifests in config/devices/ use v3 syntax only.
    // See docs/reference/deprecation-plan.md Section 3.2.
    /// Rhai scripts for complex operations (v1 legacy, ignored in v3).
    #[serde(default)]
    pub scripts: Option<toml::Value>,

    /// Binary command definitions (Modbus, etc.).
    #[serde(default)]
    pub binary_commands: Option<toml::Value>,

    /// Binary response definitions (Modbus, etc.).
    #[serde(default)]
    pub binary_responses: Option<toml::Value>,

    /// v1 trait mappings (accepted but ignored in v3; use `capabilities`).
    #[serde(default)]
    pub trait_mapping: Option<toml::Value>,
}

/// Device metadata section.
#[derive(Debug, Deserialize)]
pub struct RawDeviceConfig {
    /// Human-readable device name.
    pub name: String,

    /// List of capabilities this device supports.
    #[serde(default)]
    pub capabilities: Vec<String>,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,

    /// Device manufacturer.
    #[serde(default)]
    pub manufacturer: Option<String>,

    /// Device model number.
    #[serde(default)]
    pub model: Option<String>,

    /// Protocol identifier.
    #[serde(default)]
    pub protocol: Option<String>,

    /// Device category (stage, sensor, source, etc.).
    #[serde(default)]
    pub category: Option<String>,
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
        /// Data bits (5-8).
        #[serde(default)]
        data_bits: Option<u8>,
        /// Parity: none, odd, even.
        #[serde(default)]
        parity: Option<String>,
        /// Stop bits: 1 or 2.
        #[serde(default)]
        stop_bits: Option<u8>,
        /// Flow control: none, software, hardware.
        #[serde(default)]
        flow_control: Option<String>,
        /// Separate TX terminator.
        #[serde(default)]
        terminator_tx: Option<String>,
        /// Separate RX terminator.
        #[serde(default)]
        terminator_rx: Option<String>,
        /// RS-485 bus configuration.
        #[serde(default)]
        bus: Option<toml::Value>,
    },
    Tcp {
        host: String,
        port: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
        /// Optional line terminator (e.g. "\r\n").
        #[serde(default)]
        terminator: Option<String>,
    },
    Udp {
        host: String,
        port: u16,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
    },
    /// USB TMC (Test & Measurement Class) connection.
    ///
    /// On Linux, USB TMC devices appear as `/dev/usbtmcN` character devices
    /// via the kernel `usbtmc` module. The device path is specified in the
    /// instance config (not the manifest), since it varies per host.
    Usbtmc {
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u32,
        /// TX terminator appended to commands (e.g. "\n" for SCPI).
        #[serde(default)]
        terminator_tx: Option<String>,
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

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,

    /// Per-command timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u32>,

    /// Per-command retry configuration.
    #[serde(default)]
    pub retry: Option<toml::Value>,

    /// Delay after command execution (ms).
    #[serde(default)]
    pub delay_ms: Option<u32>,

    // LEGACY: v2 query flag, accepted but ignored in v3. See deprecation-plan.md 3.2.
    /// Whether this is a query command (v2 legacy, ignored in v3).
    #[serde(default)]
    pub query: Option<bool>,
}

fn default_true() -> bool {
    true
}

/// A response format definition with tiered parsing options.
#[derive(Debug, Deserialize)]
pub struct RawResponseConfig {
    /// Single-variant format string (equivalent to `variants = ["..."]`).
    #[serde(default)]
    pub format: Option<String>,

    /// Multi-variant format strings, tried in order. First match wins.
    /// Use this for devices that emit different response shapes (firmware
    /// variation, optional fields). Covers cases that historically required
    /// regex alternation.
    #[serde(default)]
    pub variants: Option<Vec<String>>,

    /// Transform pipeline (list of shorthand operations).
    #[serde(default)]
    pub transform: Option<Vec<String>>,

    /// Regex with named capture groups.
    ///
    /// DEPRECATED at top level. Prefer `variants` for multi-shape responses.
    /// If regex is genuinely required, move it into `[responses.X.advanced]`
    /// to acknowledge the escape hatch and suppress the load-time warning.
    #[serde(default)]
    pub regex: Option<String>,

    /// Escape-hatch options. Regex placed here silences the deprecation
    /// warning emitted by a top-level `regex = "..."`.
    #[serde(default)]
    pub advanced: Option<RawAdvancedResponse>,

    // LEGACY: v1 regex alias. See deprecation-plan.md 3.2.
    /// v1 regex pattern (alias for `regex`).
    #[serde(default)]
    pub pattern: Option<String>,

    // LEGACY: v1 regex capture type declarations. See deprecation-plan.md 3.2.
    /// Field type declarations for regex captures (v1 legacy, ignored in v3).
    #[serde(default)]
    pub fields: Option<HashMap<String, toml::Value>>,
}

/// `[responses.X.advanced]` — escape-hatch parsing options.
///
/// Authors who reach for these features implicitly opt into the "I know what
/// I'm doing" tier; the validator stops nagging about them.
#[derive(Debug, Deserialize)]
pub struct RawAdvancedResponse {
    /// Regex with named capture groups (canonical location for regex).
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

    #[serde(default)]
    pub spectrum_readable: Option<RawSpectrumReadableMapping>,

    /// Catch-all for future/custom capabilities.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// Method mappings for the Movable capability.
#[derive(Debug, Deserialize)]
pub struct RawMovableMapping {
    pub move_abs: Option<RawMethodMapping>,
    pub move_rel: Option<RawMethodMapping>,
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

/// Method mappings for the SpectrumReadable capability (1D detectors).
#[derive(Debug, Deserialize)]
pub struct RawSpectrumReadableMapping {
    /// Command that reads the 1D spectrum/waveform data.
    pub read_spectrum: Option<RawMethodMapping>,
    /// Number of channels/pixels (static or from a query).
    #[serde(default)]
    pub spectrum_length: Option<usize>,
    /// Units for the value axis (e.g., "counts", "W").
    #[serde(default)]
    pub value_units: Option<String>,
    /// Units for the wavelength/channel axis (e.g., "nm", "eV").
    #[serde(default)]
    pub axis_units: Option<String>,
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

    #[test]
    fn deserialize_usbtmc_config() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Thorlabs PM400"
capabilities = ["Readable"]

[connection]
type = "usbtmc"
timeout_ms = 5000
terminator_tx = "\n"

[commands.read_power]
template = "MEASure:SCALar:POWer?"
response_type = "float"

[capabilities.readable]
read = { command = "read_power" }
"#;

        let raw: RawManifest = toml::from_str(toml_str).expect("should parse USB TMC config");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Thorlabs PM400");
        match &raw.connection {
            RawConnectionConfig::Usbtmc {
                timeout_ms,
                terminator_tx,
            } => {
                assert_eq!(*timeout_ms, 5000);
                assert_eq!(terminator_tx.as_deref(), Some("\n"));
            }
            _ => panic!("expected USB TMC connection"),
        }
    }

    #[test]
    fn deserialize_usbtmc_defaults() {
        let toml_str = r#"
schema_version = 3

[device]
name = "Test"
capabilities = []

[connection]
type = "usbtmc"
"#;

        let raw: RawManifest =
            toml::from_str(toml_str).expect("should parse minimal USB TMC config");
        match &raw.connection {
            RawConnectionConfig::Usbtmc {
                timeout_ms,
                terminator_tx,
            } => {
                assert_eq!(*timeout_ms, 1000, "default timeout should be 1000ms");
                assert!(terminator_tx.is_none());
            }
            _ => panic!("expected USB TMC connection"),
        }
    }

    /// Returns the path to the config/devices directory relative to the crate root.
    fn config_devices_dir() -> std::path::PathBuf {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest_dir.join("../../config/devices")
    }

    #[test]
    fn load_all_config_files_as_raw_manifest() {
        let dir = config_devices_dir();
        assert!(dir.exists(), "config/devices dir should exist at {dir:?}");

        let mut count = 0;
        let mut failures: Vec<String> = Vec::new();

        for entry in std::fs::read_dir(&dir).expect("should read config/devices") {
            let entry = entry.expect("should read dir entry");
            let path = entry.path();

            // Skip directories and non-TOML files
            if path.is_dir() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }

            let contents = std::fs::read_to_string(&path).expect("should read TOML file");

            match toml::from_str::<RawManifest>(&contents) {
                Ok(manifest) => {
                    assert_eq!(
                        manifest.schema_version,
                        3,
                        "{}: expected schema_version=3, got {}",
                        path.display(),
                        manifest.schema_version
                    );
                    count += 1;
                }
                Err(e) => {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "Config files failed to parse as RawManifest:\n{}",
            failures.join("\n")
        );

        // We expect at least 10 config files
        assert!(
            count >= 10,
            "expected at least 10 config files, found {count}"
        );
    }

    #[test]
    fn load_ell14_toml_from_disk() {
        let path = config_devices_dir().join("ell14.toml");
        let contents = std::fs::read_to_string(&path).expect("should read ell14.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse ell14.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Thorlabs ELL14");
        let movable = raw
            .capabilities
            .movable
            .as_ref()
            .expect("ELL14 should define movable capability mappings");
        assert!(
            movable.move_rel.is_some(),
            "ELL14 should define capabilities.movable.move_rel mapping"
        );
        assert!(!raw.commands.is_empty());
        assert!(!raw.responses.is_empty());
        assert!(!raw.conversions.is_empty());
    }

    #[test]
    fn load_maitai_toml_from_disk() {
        let path = config_devices_dir().join("maitai.toml");
        let contents = std::fs::read_to_string(&path).expect("should read maitai.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse maitai.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Spectra-Physics MaiTai");
        assert!(raw.capabilities.readable.is_some());
        assert!(raw.capabilities.wavelength_tunable.is_some());
        assert!(raw.capabilities.shutter_control.is_some());
    }

    #[test]
    fn load_esp300_toml_from_disk() {
        let path = config_devices_dir().join("esp300.toml");
        let contents = std::fs::read_to_string(&path).expect("should read esp300.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse esp300.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Newport ESP300");
        assert!(raw.capabilities.movable.is_some());
    }

    #[test]
    fn load_newport_1830c_toml_from_disk() {
        let path = config_devices_dir().join("newport_1830c.toml");
        let contents = std::fs::read_to_string(&path).expect("should read newport_1830c.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse newport_1830c.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Newport 1830-C");
        assert!(raw.capabilities.readable.is_some());
        assert!(raw.capabilities.wavelength_tunable.is_some());
    }

    #[test]
    fn load_ipg_laser_toml_from_disk() {
        let path = config_devices_dir().join("ipg_laser.toml");
        let contents = std::fs::read_to_string(&path).expect("should read ipg_laser.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse ipg_laser.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "IPG YLPP-200-1-50-R");
        assert!(raw.capabilities.readable.is_some());
    }

    #[test]
    fn load_thorlabs_pm400_toml_from_disk() {
        let path = config_devices_dir().join("thorlabs_pm400.toml");
        let contents = std::fs::read_to_string(&path).expect("should read thorlabs_pm400.toml");
        let raw: RawManifest = toml::from_str(&contents).expect("should parse thorlabs_pm400.toml");
        assert_eq!(raw.schema_version, 3);
        assert_eq!(raw.device.name, "Thorlabs PM400");
        assert!(matches!(raw.connection, RawConnectionConfig::Usbtmc { .. }));
        assert!(raw.capabilities.readable.is_some());
        assert!(raw.capabilities.wavelength_tunable.is_some());
    }
}
