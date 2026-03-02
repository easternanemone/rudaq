//! Plugin manifest types for native plugin discovery.
//!
//! This module defines the Rust types that correspond to the plugin.toml
//! manifest format for native, script, and WASM plugins.
//!
//! # Example plugin.toml
//!
//! ```toml
//! [plugin]
//! name = "power-logger"
//! version = "1.0.0"
//! description = "Logs power meter readings to file"
//! type = "native"
//!
//! [plugin.requires]
//! rust_daq = ">=0.5.0"
//! api_version = "0.1"
//!
//! [plugin.entry]
//! library = "power_logger"
//!
//! [module]
//! type_id = "power_logger"
//! display_name = "Power Logger"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level plugin manifest structure (plugin.toml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin metadata and configuration.
    pub plugin: PluginConfig,

    /// Module definition for the plugin.
    #[serde(default)]
    pub module: Option<ModuleConfig>,

    /// Dependencies on other plugins.
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

/// Plugin metadata and configuration section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Unique plugin name (lowercase, no spaces).
    pub name: String,

    /// Plugin version (semver format).
    pub version: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: String,

    /// Plugin author(s).
    #[serde(default)]
    pub author: Option<String>,

    /// License identifier (e.g., "MIT", "Apache-2.0").
    #[serde(default)]
    pub license: Option<String>,

    /// Repository URL.
    #[serde(default)]
    pub repository: Option<String>,

    /// Plugin categories for discovery.
    #[serde(default)]
    pub categories: Vec<String>,

    /// Keywords for search.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Plugin type: native, script, or wasm.
    #[serde(rename = "type", default)]
    pub plugin_type: PluginType,

    /// Host requirements.
    #[serde(default)]
    pub requires: PluginRequirements,

    /// Entry point configuration.
    #[serde(default)]
    pub entry: PluginEntry,

    /// Activation triggers.
    #[serde(default)]
    pub activation: PluginActivation,

    /// Lifecycle hooks.
    #[serde(default)]
    pub hooks: PluginHooks,
}

/// Plugin type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    /// Native Rust plugin (shared library).
    #[default]
    Native,
    /// Script-based plugin (Rhai, etc.).
    Script,
    /// WebAssembly plugin.
    Wasm,
}

impl std::fmt::Display for PluginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginType::Native => write!(f, "native"),
            PluginType::Script => write!(f, "script"),
            PluginType::Wasm => write!(f, "wasm"),
        }
    }
}

/// Host and API version requirements.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginRequirements {
    /// Required rust_daq version (semver requirement).
    #[serde(default)]
    pub rust_daq: Option<String>,

    /// Required plugin API version.
    #[serde(default)]
    pub api_version: Option<String>,
}

/// Plugin entry point configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Library base name for native plugins (without extension).
    #[serde(default)]
    pub library: Option<String>,

    /// Symbol name for native plugins (default: ROOT_MODULE_LOADER_NAME).
    #[serde(default)]
    pub symbol: Option<String>,

    /// Script engine for script plugins.
    #[serde(default)]
    pub engine: Option<String>,

    /// Script file path for script plugins.
    #[serde(default)]
    pub script: Option<String>,

    /// WASM module path for WASM plugins.
    #[serde(default)]
    pub wasm_module: Option<String>,
}

/// Plugin activation configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginActivation {
    /// When to load the plugin.
    #[serde(default)]
    pub trigger: ActivationTrigger,

    /// Device types that trigger loading (for on_device_connect).
    #[serde(default)]
    pub device_types: Vec<String>,
}

/// Activation trigger types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationTrigger {
    /// Load when explicitly requested.
    #[default]
    OnDemand,
    /// Load at application startup.
    OnStartup,
    /// Load when a matching device connects.
    OnDeviceConnect,
}

/// Plugin lifecycle hooks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginHooks {
    /// Commands to run when plugin loads.
    #[serde(default)]
    pub on_load: Vec<String>,

    /// Commands to run when plugin unloads.
    #[serde(default)]
    pub on_unload: Vec<String>,
}

/// Module definition within a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleConfig {
    /// Unique module type identifier.
    pub type_id: String,

    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,

    /// Module description.
    #[serde(default)]
    pub description: String,

    /// Module category.
    #[serde(default)]
    pub category: Option<String>,

    /// Role requirements.
    #[serde(default)]
    pub roles: Vec<ModuleRole>,

    /// Module parameters.
    #[serde(default)]
    pub parameters: Vec<ModuleParameter>,

    /// Event types this module emits.
    #[serde(default)]
    pub events: ModuleEvents,

    /// Data types this module produces.
    #[serde(default)]
    pub data: ModuleData,

    /// Error patterns for this module.
    #[serde(default)]
    pub errors: ModuleErrors,
}

/// Role requirement for a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleRole {
    /// Role identifier.
    pub role_id: String,

    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,

    /// Role description.
    #[serde(default)]
    pub description: String,

    /// Required capability (e.g., "readable", "movable").
    #[serde(default)]
    pub capability: String,

    /// Whether this role is required.
    #[serde(default)]
    pub required: bool,

    /// Whether multiple devices can fulfill this role.
    #[serde(default)]
    pub allows_multiple: bool,
}

/// Module parameter definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleParameter {
    /// Parameter identifier.
    pub param_id: String,

    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,

    /// Parameter description.
    #[serde(default)]
    pub description: String,

    /// Parameter type.
    #[serde(default)]
    pub param_type: ParameterType,

    /// Default value (as string).
    #[serde(default)]
    pub default_value: Option<String>,

    /// Minimum value (for numeric types).
    #[serde(default)]
    pub min_value: Option<String>,

    /// Maximum value (for numeric types).
    #[serde(default)]
    pub max_value: Option<String>,

    /// Unit of measurement.
    #[serde(default)]
    pub units: Option<String>,

    /// Whether the parameter is required.
    #[serde(default)]
    pub required: bool,

    /// Enum values (for enum type).
    #[serde(default)]
    pub enum_values: Vec<String>,

    /// Mock data configuration.
    #[serde(default)]
    pub mock: Option<MockConfig>,
}

/// Parameter type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    #[default]
    String,
    Float,
    Int,
    Bool,
    Enum,
}

impl std::fmt::Display for ParameterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterType::String => write!(f, "string"),
            ParameterType::Float => write!(f, "float"),
            ParameterType::Int => write!(f, "int"),
            ParameterType::Bool => write!(f, "bool"),
            ParameterType::Enum => write!(f, "enum"),
        }
    }
}

/// Mock data configuration for testing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockConfig {
    /// Default mock value.
    #[serde(default)]
    pub default: f64,

    /// Random jitter to add.
    #[serde(default)]
    pub jitter: f64,
}

/// Event types configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModuleEvents {
    /// List of event type names.
    #[serde(default)]
    pub types: Vec<String>,
}

/// Data types configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModuleData {
    /// List of data type names.
    #[serde(default)]
    pub types: Vec<String>,
}

/// Error patterns configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModuleErrors {
    /// List of error pattern strings.
    #[serde(default)]
    pub patterns: Vec<String>,
}

// =============================================================================
// Parsing and Loading
// =============================================================================

impl PluginManifest {
    /// Parses a plugin manifest from TOML string content.
    ///
    /// # Errors
    /// Returns error if the TOML is invalid or required fields are missing.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Loads a plugin manifest from a file path.
    ///
    /// # Errors
    /// Returns error if the file cannot be read or parsed.
    pub fn from_file(path: &std::path::Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        Self::from_toml(&content).map_err(|e| ManifestError::Parse {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Returns the library path for a native plugin, resolving the platform extension.
    ///
    /// # Arguments
    /// * `plugin_dir` - Directory containing the plugin
    ///
    /// # Returns
    /// Full path to the library file with platform-appropriate extension.
    pub fn library_path(&self, plugin_dir: &std::path::Path) -> Option<PathBuf> {
        let library_name = self.plugin.entry.library.as_ref()?;

        let extension = if cfg!(target_os = "windows") {
            "dll"
        } else if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };

        let lib_prefix = if cfg!(target_os = "windows") {
            ""
        } else {
            "lib"
        };

        Some(plugin_dir.join(format!("{}{}.{}", lib_prefix, library_name, extension)))
    }
}

/// Errors that can occur when loading a plugin manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Failed to read manifest file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse manifest {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Validation failed for {}: {message}", path.display())]
    Validation { path: PathBuf, message: String },
}

// =============================================================================
// Validation
// =============================================================================

/// A specific validation error within a plugin manifest.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Path to the invalid field (e.g., "plugin.name").
    pub path: String,
    /// Human-readable error message.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl PluginManifest {
    /// Validates the manifest and returns any errors found.
    ///
    /// Validation checks:
    /// 1. Plugin name is valid (lowercase, no spaces)
    /// 2. Plugin version is valid semver
    /// 3. Required fields based on plugin type
    /// 4. Module type_id uniqueness requirements
    /// 5. Parameter constraints are valid
    /// 6. Enum parameters have values defined
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Validate plugin name
        if self.plugin.name.is_empty() {
            errors.push(ValidationError {
                path: "plugin.name".to_string(),
                message: "Plugin name cannot be empty".to_string(),
            });
        } else if self.plugin.name.contains(' ') {
            errors.push(ValidationError {
                path: "plugin.name".to_string(),
                message: "Plugin name cannot contain spaces".to_string(),
            });
        } else if self.plugin.name != self.plugin.name.to_lowercase() {
            errors.push(ValidationError {
                path: "plugin.name".to_string(),
                message: "Plugin name must be lowercase".to_string(),
            });
        }

        // Validate version
        if self.plugin.version.is_empty() {
            errors.push(ValidationError {
                path: "plugin.version".to_string(),
                message: "Plugin version cannot be empty".to_string(),
            });
        } else if !is_valid_semver(&self.plugin.version) {
            errors.push(ValidationError {
                path: "plugin.version".to_string(),
                message: format!("Invalid semver version: {}", self.plugin.version),
            });
        }

        // Validate entry based on plugin type
        match self.plugin.plugin_type {
            PluginType::Native => {
                if self.plugin.entry.library.is_none() {
                    errors.push(ValidationError {
                        path: "plugin.entry.library".to_string(),
                        message: "Native plugins must specify a library name".to_string(),
                    });
                }
            }
            PluginType::Script => {
                if self.plugin.entry.script.is_none() {
                    errors.push(ValidationError {
                        path: "plugin.entry.script".to_string(),
                        message: "Script plugins must specify a script file".to_string(),
                    });
                }
                if self.plugin.entry.engine.is_none() {
                    errors.push(ValidationError {
                        path: "plugin.entry.engine".to_string(),
                        message: "Script plugins must specify a script engine".to_string(),
                    });
                }
            }
            PluginType::Wasm => {
                if self.plugin.entry.wasm_module.is_none() {
                    errors.push(ValidationError {
                        path: "plugin.entry.wasm_module".to_string(),
                        message: "WASM plugins must specify a WASM module path".to_string(),
                    });
                }
            }
        }

        // Validate module if present
        if let Some(ref module) = self.module {
            if module.type_id.is_empty() {
                errors.push(ValidationError {
                    path: "module.type_id".to_string(),
                    message: "Module type_id cannot be empty".to_string(),
                });
            }

            // Validate roles
            for (i, role) in module.roles.iter().enumerate() {
                if role.role_id.is_empty() {
                    errors.push(ValidationError {
                        path: format!("module.roles[{}].role_id", i),
                        message: "Role role_id cannot be empty".to_string(),
                    });
                }
            }

            // Validate parameters
            for (i, param) in module.parameters.iter().enumerate() {
                if param.param_id.is_empty() {
                    errors.push(ValidationError {
                        path: format!("module.parameters[{}].param_id", i),
                        message: "Parameter param_id cannot be empty".to_string(),
                    });
                }

                // Enum parameters must have values
                if param.param_type == ParameterType::Enum && param.enum_values.is_empty() {
                    errors.push(ValidationError {
                        path: format!("module.parameters[{}].enum_values", i),
                        message: "Enum parameters must have enum_values defined".to_string(),
                    });
                }

                // Validate numeric constraints
                if let (Some(min_str), Some(max_str)) =
                    (param.min_value.as_ref(), param.max_value.as_ref())
                {
                    if let (Ok(min), Ok(max)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                        if min >= max {
                            errors.push(ValidationError {
                                path: format!("module.parameters[{}]", i),
                                message: format!(
                                    "min_value ({}) must be less than max_value ({})",
                                    min, max
                                ),
                            });
                        }
                    }
                }
            }
        }

        errors
    }

    /// Validates the manifest and returns an error if validation fails.
    pub fn validate_or_err(&self, path: &std::path::Path) -> Result<(), ManifestError> {
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            Err(ManifestError::Validation {
                path: path.to_path_buf(),
                message: messages.join("; "),
            })
        }
    }
}

/// Checks if a string is a valid semver version.
fn is_valid_semver(s: &str) -> bool {
    let (version_part, _prerelease) = if let Some(idx) = s.find('-') {
        (&s[..idx], Some(&s[idx + 1..]))
    } else {
        (s, None)
    };

    let parts: Vec<&str> = version_part.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }

    parts.iter().all(|p| p.parse::<u64>().is_ok())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_MANIFEST: &str = r#"
[plugin]
name = "test-plugin"
version = "1.0.0"

[plugin.entry]
library = "test_plugin"
"#;

    const FULL_MANIFEST: &str = r#"
[plugin]
name = "power-logger"
version = "1.0.0"
description = "Logs power meter readings to file"
author = "Lab User <user@lab.edu>"
license = "MIT"
repository = "https://github.com/user/power-logger"
categories = ["data_acquisition", "logging"]
keywords = ["power", "meter", "csv", "export"]
type = "native"

[plugin.requires]
rust_daq = ">=0.5.0"
api_version = "0.1"

[plugin.entry]
library = "power_logger"
symbol = "get_root_module"

[plugin.activation]
trigger = "on_demand"

[plugin.hooks]
on_load = []
on_unload = []

[module]
type_id = "power_logger"
display_name = "Power Logger"
description = "Continuously logs power readings to CSV file"
category = "data_acquisition"

[[module.roles]]
role_id = "power_meter"
display_name = "Power Meter"
description = "Device to read power values from"
capability = "readable"
required = true
allows_multiple = false

[[module.parameters]]
param_id = "log_path"
display_name = "Log File Path"
description = "Output CSV file path"
param_type = "string"
default_value = "/tmp/power.csv"
required = true

[[module.parameters]]
param_id = "sample_rate_hz"
display_name = "Sample Rate"
description = "How often to read the power meter"
param_type = "float"
default_value = "10.0"
min_value = "0.1"
max_value = "1000.0"
units = "Hz"
required = false

[module.events]
types = ["started", "stopped", "file_rotated", "error"]

[module.data]
types = ["power_reading", "statistics"]

[module.errors]
patterns = ["IO_ERROR", "DEVICE_DISCONNECTED"]

[dependencies]
# storage-parquet = ">=1.0.0"
"#;

    #[test]
    fn test_parse_minimal_manifest() {
        let manifest = PluginManifest::from_toml(MINIMAL_MANIFEST).unwrap();
        assert_eq!(manifest.plugin.name, "test-plugin");
        assert_eq!(manifest.plugin.version, "1.0.0");
        assert_eq!(manifest.plugin.plugin_type, PluginType::Native);
        assert_eq!(
            manifest.plugin.entry.library,
            Some("test_plugin".to_string())
        );
    }

    #[test]
    fn test_parse_full_manifest() {
        let manifest = PluginManifest::from_toml(FULL_MANIFEST).unwrap();
        assert_eq!(manifest.plugin.name, "power-logger");
        assert_eq!(manifest.plugin.version, "1.0.0");
        assert_eq!(
            manifest.plugin.description,
            "Logs power meter readings to file"
        );
        assert_eq!(manifest.plugin.categories.len(), 2);
        assert_eq!(manifest.plugin.keywords.len(), 4);

        // Check requires
        assert_eq!(
            manifest.plugin.requires.rust_daq,
            Some(">=0.5.0".to_string())
        );
        assert_eq!(
            manifest.plugin.requires.api_version,
            Some("0.1".to_string())
        );

        // Check module
        let module = manifest.module.unwrap();
        assert_eq!(module.type_id, "power_logger");
        assert_eq!(module.roles.len(), 1);
        assert_eq!(module.parameters.len(), 2);
        assert_eq!(module.events.types.len(), 4);
    }

    #[test]
    fn test_plugin_type_serialization() {
        let manifest: PluginManifest = toml::from_str(
            r#"[plugin]
name = "test"
version = "1.0.0"
type = "script"

[plugin.entry]
engine = "rhai"
script = "test.rhai"
"#,
        )
        .unwrap();

        assert_eq!(manifest.plugin.plugin_type, PluginType::Script);
    }

    #[test]
    fn test_library_path_generation() {
        let manifest = PluginManifest::from_toml(MINIMAL_MANIFEST).unwrap();
        let plugin_dir = std::path::Path::new("/plugins/test-plugin");
        let lib_path = manifest.library_path(plugin_dir).unwrap();

        #[cfg(target_os = "windows")]
        assert!(lib_path.to_string_lossy().ends_with("test_plugin.dll"));

        #[cfg(target_os = "macos")]
        assert!(lib_path.to_string_lossy().ends_with("libtest_plugin.dylib"));

        #[cfg(target_os = "linux")]
        assert!(lib_path.to_string_lossy().ends_with("libtest_plugin.so"));
    }

    #[test]
    fn test_parameter_types() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test"
version = "1.0.0"

[module]
type_id = "test"

[[module.parameters]]
param_id = "string_param"
param_type = "string"

[[module.parameters]]
param_id = "float_param"
param_type = "float"

[[module.parameters]]
param_id = "int_param"
param_type = "int"

[[module.parameters]]
param_id = "bool_param"
param_type = "bool"

[[module.parameters]]
param_id = "enum_param"
param_type = "enum"
enum_values = ["A", "B", "C"]
"#,
        )
        .unwrap();

        let module = manifest.module.unwrap();
        assert_eq!(module.parameters.len(), 5);
        assert_eq!(module.parameters[0].param_type, ParameterType::String);
        assert_eq!(module.parameters[1].param_type, ParameterType::Float);
        assert_eq!(module.parameters[2].param_type, ParameterType::Int);
        assert_eq!(module.parameters[3].param_type, ParameterType::Bool);
        assert_eq!(module.parameters[4].param_type, ParameterType::Enum);
        assert_eq!(module.parameters[4].enum_values, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_validation_valid_manifest() {
        let manifest = PluginManifest::from_toml(MINIMAL_MANIFEST).unwrap();
        let errors = manifest.validate();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validation_empty_name() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = ""
version = "1.0.0"

[plugin.entry]
library = "test"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors.iter().any(|e| e.path == "plugin.name"));
    }

    #[test]
    fn test_validation_name_with_spaces() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test plugin"
version = "1.0.0"

[plugin.entry]
library = "test"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors
            .iter()
            .any(|e| e.path == "plugin.name" && e.message.contains("spaces")));
    }

    #[test]
    fn test_validation_name_uppercase() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "TestPlugin"
version = "1.0.0"

[plugin.entry]
library = "test"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors
            .iter()
            .any(|e| e.path == "plugin.name" && e.message.contains("lowercase")));
    }

    #[test]
    fn test_validation_invalid_version() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test"
version = "not-a-version"

[plugin.entry]
library = "test"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors.iter().any(|e| e.path == "plugin.version"));
    }

    #[test]
    fn test_validation_script_missing_engine() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test"
version = "1.0.0"
type = "script"

[plugin.entry]
script = "test.rhai"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors.iter().any(|e| e.path == "plugin.entry.engine"));
    }

    #[test]
    fn test_validation_enum_without_values() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test"
version = "1.0.0"

[plugin.entry]
library = "test"

[module]
type_id = "test"

[[module.parameters]]
param_id = "enum_param"
param_type = "enum"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors.iter().any(|e| e.message.contains("enum_values")));
    }

    #[test]
    fn test_validation_min_max_invalid() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
name = "test"
version = "1.0.0"

[plugin.entry]
library = "test"

[module]
type_id = "test"

[[module.parameters]]
param_id = "value"
param_type = "float"
min_value = "100"
max_value = "10"
"#,
        )
        .unwrap();
        let errors = manifest.validate();
        assert!(errors.iter().any(|e| e.message.contains("min_value")));
    }
}
