//! Procedure Configuration System
//!
//! Hierarchical configuration with composition and overrides, inspired by Hydra.
//!
//! # Features
//!
//! - TOML-based configuration files
//! - Config groups for reusable parameter sets
//! - Defaults with override support
//! - Role-based device assignment
//! - Runtime parameter overrides
//!
//! # Configuration Structure
//!
//! ```toml
//! [procedure]
//! type = "rotator_calibration"
//! name = "My Calibration"
//!
//! [defaults]
//! # Include config groups (optional)
//! motion = "standard"  # Loads config/procedures/groups/motion/standard.toml
//!
//! [params]
//! num_cycles = 3
//! tolerance = 0.1
//!
//! [roles.rotator]
//! device_id = "rotator_2"
//! ```
//!
//! # Config Groups
//!
//! Config groups allow sharing common parameter sets:
//!
//! ```text
//! config/procedures/
//! ├── groups/
//! │   ├── motion/
//! │   │   ├── standard.toml   # Default motion parameters
//! │   │   ├── fast.toml       # Fast motion parameters
//! │   │   └── precise.toml    # High-precision parameters
//! │   └── acquisition/
//! │       ├── standard.toml
//! │       └── high_speed.toml
//! └── rotator_calibration.toml
//! ```

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// =============================================================================
// ProcedureConfig
// =============================================================================

/// Complete configuration for a procedure instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureConfig {
    /// Procedure type identifier
    #[serde(rename = "type")]
    pub procedure_type: String,

    /// Instance name (human-readable)
    #[serde(default)]
    pub name: String,

    /// Description (optional)
    #[serde(default)]
    pub description: Option<String>,

    /// Config group defaults to include
    #[serde(default)]
    pub defaults: HashMap<String, String>,

    /// Parameters (merged from defaults and overrides)
    #[serde(default)]
    pub params: HashMap<String, ConfigValue>,

    /// Device role assignments
    #[serde(default)]
    pub roles: HashMap<String, RoleAssignment>,

    /// Runtime overrides (applied last)
    #[serde(skip)]
    pub overrides: Vec<ConfigOverride>,
}

/// A configuration value that can be various types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    /// Boolean value
    Bool(bool),
    /// Integer value
    Integer(i64),
    /// Float value
    Float(f64),
    /// String value
    String(String),
    /// Array of values
    Array(Vec<ConfigValue>),
    /// Nested table
    Table(HashMap<String, ConfigValue>),
}

impl ConfigValue {
    /// Get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as integer
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConfigValue::Integer(v) => Some(*v),
            ConfigValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// Get as float
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConfigValue::Float(v) => Some(*v),
            ConfigValue::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Get as string
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConfigValue::String(v) => Some(v.as_str()),
            _ => None,
        }
    }

    /// Get as array
    pub fn as_array(&self) -> Option<&Vec<ConfigValue>> {
        match self {
            ConfigValue::Array(v) => Some(v),
            _ => None,
        }
    }
}

impl From<bool> for ConfigValue {
    fn from(v: bool) -> Self {
        ConfigValue::Bool(v)
    }
}

impl From<i64> for ConfigValue {
    fn from(v: i64) -> Self {
        ConfigValue::Integer(v)
    }
}

impl From<i32> for ConfigValue {
    fn from(v: i32) -> Self {
        ConfigValue::Integer(v as i64)
    }
}

impl From<f64> for ConfigValue {
    fn from(v: f64) -> Self {
        ConfigValue::Float(v)
    }
}

impl From<String> for ConfigValue {
    fn from(v: String) -> Self {
        ConfigValue::String(v)
    }
}

impl From<&str> for ConfigValue {
    fn from(v: &str) -> Self {
        ConfigValue::String(v.to_string())
    }
}

/// Device role assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleAssignment {
    /// Device ID to assign
    pub device_id: String,

    /// Optional capability override (for validation)
    #[serde(default)]
    pub capability: Option<String>,
}

/// Runtime configuration override
#[derive(Debug, Clone)]
pub struct ConfigOverride {
    /// Dot-separated path (e.g., "params.num_cycles")
    pub path: String,
    /// New value
    pub value: ConfigValue,
}

// =============================================================================
// TOML Configuration File
// =============================================================================

/// Raw TOML file structure (before processing)
#[derive(Debug, Deserialize)]
struct RawProcedureConfig {
    procedure: ProcedureSection,
    #[serde(default)]
    defaults: HashMap<String, String>,
    #[serde(default)]
    params: HashMap<String, toml::Value>,
    #[serde(default)]
    roles: HashMap<String, RoleAssignment>,
}

#[derive(Debug, Deserialize)]
struct ProcedureSection {
    #[serde(rename = "type")]
    procedure_type: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
}

impl ProcedureConfig {
    /// Create a new empty configuration
    pub fn new(procedure_type: impl Into<String>) -> Self {
        Self {
            procedure_type: procedure_type.into(),
            name: String::new(),
            description: None,
            defaults: HashMap::new(),
            params: HashMap::new(),
            roles: HashMap::new(),
            overrides: Vec::new(),
        }
    }

    /// Load configuration from a TOML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read config file: {}", e))?;

        Self::from_toml_str(&content, Some(path))
    }

    /// Parse configuration from TOML string
    pub fn from_toml_str(content: &str, base_path: Option<&Path>) -> Result<Self> {
        let raw: RawProcedureConfig =
            toml::from_str(content).map_err(|e| anyhow!("Failed to parse config: {}", e))?;

        let mut config = Self {
            procedure_type: raw.procedure.procedure_type,
            name: raw.procedure.name,
            description: raw.procedure.description,
            defaults: raw.defaults.clone(),
            params: HashMap::new(),
            roles: raw.roles,
            overrides: Vec::new(),
        };

        // Load and merge defaults (config groups)
        if let Some(base) = base_path {
            config.load_defaults(base)?;
        }

        // Merge explicit params (override defaults)
        for (key, value) in raw.params {
            config.params.insert(key, toml_to_config_value(value));
        }

        Ok(config)
    }

    /// Load config group defaults
    fn load_defaults(&mut self, config_path: &Path) -> Result<()> {
        let groups_dir = config_path
            .parent()
            .map(|p| p.join("groups"))
            .unwrap_or_else(|| std::path::PathBuf::from("config/procedures/groups"));

        for (group_name, variant) in &self.defaults {
            let group_file = groups_dir
                .join(group_name)
                .join(format!("{}.toml", variant));

            if group_file.exists() {
                let content = std::fs::read_to_string(&group_file).map_err(|e| {
                    anyhow!(
                        "Failed to read config group {}/{}: {}",
                        group_name,
                        variant,
                        e
                    )
                })?;

                let group_params: HashMap<String, toml::Value> = toml::from_str(&content)
                    .map_err(|e| anyhow!("Failed to parse config group: {}", e))?;

                // Merge group params (can be overridden by explicit params)
                for (key, value) in group_params {
                    if !self.params.contains_key(&key) {
                        self.params.insert(key, toml_to_config_value(value));
                    }
                }
            }
        }

        Ok(())
    }

    /// Set a parameter value
    pub fn set_param(&mut self, name: impl Into<String>, value: impl Into<ConfigValue>) {
        self.params.insert(name.into(), value.into());
    }

    /// Get a parameter value
    pub fn get_param(&self, name: &str) -> Option<&ConfigValue> {
        self.params.get(name)
    }

    /// Get a parameter as f64
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.params.get(name).and_then(|v| v.as_f64())
    }

    /// Get a parameter as i64
    pub fn get_i64(&self, name: &str) -> Option<i64> {
        self.params.get(name).and_then(|v| v.as_i64())
    }

    /// Get a parameter as bool
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.params.get(name).and_then(|v| v.as_bool())
    }

    /// Get a parameter as string
    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(|v| v.as_str())
    }

    /// Assign a device to a role
    pub fn assign_role(&mut self, role_id: impl Into<String>, device_id: impl Into<String>) {
        self.roles.insert(
            role_id.into(),
            RoleAssignment {
                device_id: device_id.into(),
                capability: None,
            },
        );
    }

    /// Get device ID for a role
    pub fn get_role_device(&self, role_id: &str) -> Option<&str> {
        self.roles.get(role_id).map(|r| r.device_id.as_str())
    }

    /// Apply a runtime override
    pub fn apply_override(&mut self, path: &str, value: impl Into<ConfigValue>) {
        let config_value = value.into();

        // Handle dotted paths
        let parts: Vec<&str> = path.split('.').collect();

        if parts.len() == 1 {
            // Simple param
            self.params.insert(path.to_string(), config_value.clone());
        } else if parts[0] == "params" && parts.len() == 2 {
            // params.name -> just store as name
            self.params
                .insert(parts[1].to_string(), config_value.clone());
        } else if parts[0] == "roles" && parts.len() >= 2 {
            // roles.role_name.device_id
            if let ConfigValue::String(device_id) = &config_value {
                self.assign_role(parts[1], device_id.clone());
            }
        }

        self.overrides.push(ConfigOverride {
            path: path.to_string(),
            value: config_value,
        });
    }

    /// Get all role assignments as a HashMap
    pub fn get_assignments(&self) -> HashMap<String, String> {
        self.roles
            .iter()
            .map(|(k, v)| (k.clone(), v.device_id.clone()))
            .collect()
    }

    /// Validate the configuration against a procedure type
    pub fn validate(&self, type_info: &super::ProcedureTypeInfo) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Check required roles
        for role in &type_info.roles {
            if !role.optional && !self.roles.contains_key(&role.role_id) {
                return Err(anyhow!(
                    "Missing required role assignment: {}",
                    role.role_id
                ));
            }
        }

        // Check unknown roles (warning)
        for role_id in self.roles.keys() {
            if !type_info.roles.iter().any(|r| &r.role_id == role_id) {
                warnings.push(format!("Unknown role '{}' - will be ignored", role_id));
            }
        }

        // Validate parameters
        for param_def in &type_info.parameters {
            if let Some(value) = self.params.get(&param_def.name) {
                // Validate constraints if present
                if let Some(constraints) = &param_def.constraints {
                    if let Some(v) = value.as_f64() {
                        if let Some(min) = constraints.min {
                            if v < min {
                                return Err(anyhow!(
                                    "Parameter '{}' value {} is below minimum {}",
                                    param_def.name,
                                    v,
                                    min
                                ));
                            }
                        }
                        if let Some(max) = constraints.max {
                            if v > max {
                                return Err(anyhow!(
                                    "Parameter '{}' value {} is above maximum {}",
                                    param_def.name,
                                    v,
                                    max
                                ));
                            }
                        }
                    }
                }
            } else if param_def.default.is_none() {
                // Required parameter without value
                return Err(anyhow!("Missing required parameter: {}", param_def.name));
            }
        }

        Ok(warnings)
    }
}

/// Convert TOML value to ConfigValue
fn toml_to_config_value(value: toml::Value) -> ConfigValue {
    match value {
        toml::Value::Boolean(v) => ConfigValue::Bool(v),
        toml::Value::Integer(v) => ConfigValue::Integer(v),
        toml::Value::Float(v) => ConfigValue::Float(v),
        toml::Value::String(v) => ConfigValue::String(v),
        toml::Value::Array(arr) => {
            ConfigValue::Array(arr.into_iter().map(toml_to_config_value).collect())
        }
        toml::Value::Table(table) => ConfigValue::Table(
            table
                .into_iter()
                .map(|(k, v)| (k, toml_to_config_value(v)))
                .collect(),
        ),
        toml::Value::Datetime(dt) => ConfigValue::String(dt.to_string()),
    }
}

// =============================================================================
// Builder Pattern
// =============================================================================

impl ProcedureConfig {
    /// Set the instance name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set a parameter
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<ConfigValue>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }

    /// Assign a device to a role
    pub fn with_role(mut self, role_id: impl Into<String>, device_id: impl Into<String>) -> Self {
        self.assign_role(role_id, device_id);
        self
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = ProcedureConfig::new("test_procedure")
            .with_name("Test")
            .with_param("cycles", 5i32)
            .with_param("tolerance", 0.1f64)
            .with_role("stage", "mock_stage");

        assert_eq!(config.procedure_type, "test_procedure");
        assert_eq!(config.name, "Test");
        assert_eq!(config.get_i64("cycles"), Some(5));
        assert_eq!(config.get_f64("tolerance"), Some(0.1));
        assert_eq!(config.get_role_device("stage"), Some("mock_stage"));
    }

    #[test]
    fn test_config_from_toml() {
        let toml = r#"
            [procedure]
            type = "rotator_calibration"
            name = "My Calibration"

            [params]
            num_cycles = 3
            tolerance = 0.05

            [roles.rotator]
            device_id = "rotator_2"
        "#;

        let config = ProcedureConfig::from_toml_str(toml, None).unwrap();

        assert_eq!(config.procedure_type, "rotator_calibration");
        assert_eq!(config.name, "My Calibration");
        assert_eq!(config.get_i64("num_cycles"), Some(3));
        assert_eq!(config.get_f64("tolerance"), Some(0.05));
        assert_eq!(config.get_role_device("rotator"), Some("rotator_2"));
    }

    #[test]
    fn test_config_override() {
        let mut config = ProcedureConfig::new("test").with_param("value", 10i32);

        config.apply_override("value", 20i32);
        assert_eq!(config.get_i64("value"), Some(20));

        config.apply_override("params.new_param", 3.14f64);
        assert_eq!(config.get_f64("new_param"), Some(3.14));
    }

    #[test]
    fn test_config_value_conversions() {
        let bool_val: ConfigValue = true.into();
        assert_eq!(bool_val.as_bool(), Some(true));

        let int_val: ConfigValue = 42i32.into();
        assert_eq!(int_val.as_i64(), Some(42));
        assert_eq!(int_val.as_f64(), Some(42.0));

        let float_val: ConfigValue = 3.14f64.into();
        assert_eq!(float_val.as_f64(), Some(3.14));
        assert_eq!(float_val.as_i64(), Some(3));

        let str_val: ConfigValue = "hello".into();
        assert_eq!(str_val.as_str(), Some("hello"));
    }
}
