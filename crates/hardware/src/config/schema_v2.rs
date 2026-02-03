use garde::{Error, Path, Report, Validate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct InstrumentManifest {
    #[garde(range(min = 2, max = 2))]
    pub schema_version: u32,
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub version: String,
    #[garde(dive)]
    pub connection: ConnectionConfig,
    #[serde(default)]
    #[garde(dive)]
    pub settings: SettingsConfig,
    #[garde(dive)]
    pub commands: HashMap<String, CommandProfile>,
    #[serde(default)]
    #[garde(dive)]
    pub capabilities: CapabilityMapping,
    #[serde(default)]
    #[garde(dive)]
    pub instances: Vec<ScpiInstanceConfig>,
}

impl InstrumentManifest {
    pub fn validate_strict(&self) -> Result<(), Report> {
        let mut report = Report::new();
        if let Err(existing) = self.validate(&()) {
            for (path, error) in existing.iter() {
                report.append(path.clone(), error.clone());
            }
        }

        validate_capability_mappings(self, &mut report);
        validate_instances(self, &mut report);

        if report.is_empty() {
            Ok(())
        } else {
            Err(report)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default)]
#[serde(deny_unknown_fields)]
pub struct SettingsConfig {
    #[serde(default)]
    #[garde(dive)]
    pub parameters: HashMap<String, ParameterConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default)]
#[serde(deny_unknown_fields)]
pub struct ParameterConfig {
    #[garde(skip)]
    pub default: Option<String>,
    #[garde(skip)]
    pub min: Option<f64>,
    #[garde(skip)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionConfig {
    Serial {
        #[garde(range(min = 1200, max = 921_600))]
        baud_rate: u32,
        #[garde(length(min = 1))]
        terminator: String,
        #[garde(range(min = 1))]
        timeout_ms: u64,
    },
    Tcp {
        #[garde(length(min = 1))]
        host: String,
        #[garde(range(min = 1, max = 65_535))]
        port: u16,
        #[garde(length(min = 1))]
        terminator: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct CommandProfile {
    #[garde(length(min = 1))]
    pub template: String,
    #[serde(default)]
    #[garde(skip)]
    pub response_type: ResponseType,
    #[serde(default)]
    #[garde(skip)]
    pub query: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseType {
    #[default]
    None,
    String,
    Float,
    Integer,
    Boolean,
    ArrayFloat,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMapping {
    #[serde(default)]
    #[garde(dive)]
    pub movable: Option<MovableMapping>,
    #[serde(default)]
    #[garde(dive)]
    pub readable: Option<ReadableMapping>,
    #[serde(default)]
    #[garde(dive)]
    pub wavelength_tunable: Option<WavelengthTunableMapping>,
    #[serde(default)]
    #[garde(dive)]
    pub shutter_control: Option<ShutterControlMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct MovableMapping {
    #[garde(length(min = 1))]
    pub move_abs: String,
    #[garde(length(min = 1))]
    pub move_rel: String,
    #[garde(length(min = 1))]
    pub position: String,
    #[garde(length(min = 1))]
    pub stop: String,
    #[serde(default)]
    #[garde(skip)]
    pub wait_settled: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ReadableMapping {
    #[garde(length(min = 1))]
    pub read: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct WavelengthTunableMapping {
    #[garde(length(min = 1))]
    pub set_wavelength: String,
    #[garde(length(min = 1))]
    pub get_wavelength: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ShutterControlMapping {
    #[garde(length(min = 1))]
    pub open_shutter: String,
    #[garde(length(min = 1))]
    pub close_shutter: String,
    #[garde(length(min = 1))]
    pub is_shutter_open: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ScpiInstanceConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub port: String,
    #[serde(default = "default_scpi_address")]
    #[garde(length(min = 1))]
    pub address: String,
    #[serde(default)]
    #[garde(skip)]
    pub baud_rate: Option<u32>,
    #[serde(default)]
    #[garde(skip)]
    pub driver_type: Option<String>,
    #[serde(default)]
    #[garde(skip)]
    pub disable: bool,
}

fn validate_capability_mappings(manifest: &InstrumentManifest, report: &mut Report) {
    if let Some(movable) = &manifest.capabilities.movable {
        validate_command_ref(
            manifest,
            &movable.move_abs,
            path_for(&["capabilities", "movable", "move_abs"]),
            report,
        );
        validate_command_ref(
            manifest,
            &movable.move_rel,
            path_for(&["capabilities", "movable", "move_rel"]),
            report,
        );
        validate_command_ref(
            manifest,
            &movable.stop,
            path_for(&["capabilities", "movable", "stop"]),
            report,
        );
        validate_command_ref(
            manifest,
            &movable.position,
            path_for(&["capabilities", "movable", "position"]),
            report,
        );
        validate_response_type(
            manifest,
            &movable.position,
            &[ResponseType::Float, ResponseType::Integer],
            path_for(&["capabilities", "movable", "position"]),
            report,
        );
        if let Some(wait_cmd) = &movable.wait_settled {
            validate_command_ref(
                manifest,
                wait_cmd,
                path_for(&["capabilities", "movable", "wait_settled"]),
                report,
            );
        }
    }

    if let Some(readable) = &manifest.capabilities.readable {
        validate_command_ref(
            manifest,
            &readable.read,
            path_for(&["capabilities", "readable", "read"]),
            report,
        );
        validate_response_type(
            manifest,
            &readable.read,
            &[
                ResponseType::Float,
                ResponseType::Integer,
                ResponseType::String,
                ResponseType::Boolean,
            ],
            path_for(&["capabilities", "readable", "read"]),
            report,
        );
    }

    if let Some(wavelength) = &manifest.capabilities.wavelength_tunable {
        validate_command_ref(
            manifest,
            &wavelength.set_wavelength,
            path_for(&["capabilities", "wavelength_tunable", "set_wavelength"]),
            report,
        );
        validate_command_ref(
            manifest,
            &wavelength.get_wavelength,
            path_for(&["capabilities", "wavelength_tunable", "get_wavelength"]),
            report,
        );
        validate_response_type(
            manifest,
            &wavelength.get_wavelength,
            &[ResponseType::Float],
            path_for(&["capabilities", "wavelength_tunable", "get_wavelength"]),
            report,
        );
    }

    if let Some(shutter) = &manifest.capabilities.shutter_control {
        validate_command_ref(
            manifest,
            &shutter.open_shutter,
            path_for(&["capabilities", "shutter_control", "open_shutter"]),
            report,
        );
        validate_command_ref(
            manifest,
            &shutter.close_shutter,
            path_for(&["capabilities", "shutter_control", "close_shutter"]),
            report,
        );
        validate_command_ref(
            manifest,
            &shutter.is_shutter_open,
            path_for(&["capabilities", "shutter_control", "is_shutter_open"]),
            report,
        );
        validate_response_type(
            manifest,
            &shutter.is_shutter_open,
            &[ResponseType::Boolean, ResponseType::Integer],
            path_for(&["capabilities", "shutter_control", "is_shutter_open"]),
            report,
        );
    }
}

fn validate_instances(manifest: &InstrumentManifest, report: &mut Report) {
    for (index, instance) in manifest.instances.iter().enumerate() {
        let path = Path::new("instances").join(index);
        let Some(driver_type) = instance.driver_type.as_deref() else {
            continue;
        };
        if driver_type.trim().is_empty() {
            report.append(
                path.join("driver_type"),
                Error::new("driver_type cannot be empty when provided."),
            );
            continue;
        }
        if !matches!(driver_type, "generic_scpi" | "declarative_scpi") {
            report.append(
                path.join("driver_type"),
                Error::new(format!(
                    "Unsupported driver_type '{}'. Expected generic_scpi or declarative_scpi.",
                    driver_type
                )),
            );
        }
    }
}

fn validate_command_ref(
    manifest: &InstrumentManifest,
    command_name: &str,
    path: Path,
    report: &mut Report,
) {
    if !manifest.commands.contains_key(command_name) {
        report.append(
            path,
            Error::new(format!("Unknown command '{}'.", command_name)),
        );
    }
}

fn validate_response_type(
    manifest: &InstrumentManifest,
    command_name: &str,
    expected: &[ResponseType],
    path: Path,
    report: &mut Report,
) {
    let Some(command) = manifest.commands.get(command_name) else {
        return;
    };
    if !command.query {
        report.append(
            path.clone(),
            Error::new("Command must be marked as query to satisfy this capability."),
        );
        return;
    }
    if !expected.contains(&command.response_type) {
        report.append(
            path,
            Error::new(format!(
                "Command response_type {:?} is incompatible with this capability.",
                command.response_type
            )),
        );
    }
}

fn path_for(segments: &[&str]) -> Path {
    let mut iter = segments.iter();
    let Some(first) = iter.next() else {
        return Path::empty();
    };
    let mut path = Path::new(*first);
    for segment in iter {
        path = path.join(*segment);
    }
    path
}

fn default_scpi_address() -> String {
    "0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> InstrumentManifest {
        let mut commands = HashMap::new();
        commands.insert(
            "read_value".to_string(),
            CommandProfile {
                template: "READ?".to_string(),
                response_type: ResponseType::Float,
                query: true,
            },
        );

        InstrumentManifest {
            schema_version: 2,
            name: "Example".to_string(),
            version: "1.0.0".to_string(),
            connection: ConnectionConfig::Serial {
                baud_rate: 9600,
                terminator: "\r\n".to_string(),
                timeout_ms: 1000,
            },
            settings: SettingsConfig::default(),
            commands,
            capabilities: CapabilityMapping {
                readable: Some(ReadableMapping {
                    read: "read_value".to_string(),
                }),
                ..CapabilityMapping::default()
            },
            instances: Vec::new(),
        }
    }

    #[test]
    fn validate_strict_accepts_valid_manifest() {
        let manifest = base_manifest();
        assert!(manifest.validate_strict().is_ok());
    }

    #[test]
    fn validate_strict_flags_unknown_command() {
        let mut manifest = base_manifest();
        manifest.capabilities.readable = Some(ReadableMapping {
            read: "missing_command".to_string(),
        });
        let report = manifest
            .validate_strict()
            .expect_err("expected validation error");
        assert!(!report.is_empty());
    }
}
