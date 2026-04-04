use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// State of an experiment module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleState {
    Unknown = 0,
    Created = 1,
    Configured = 2,
    Staged = 3,
    Running = 4,
    Paused = 5,
    Stopped = 6,
    Error = 7,
}

/// Severity level for module events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleEventSeverity {
    Unknown = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
    Critical = 4,
}

impl From<i32> for ModuleEventSeverity {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Info,
            2 => Self::Warning,
            3 => Self::Error,
            4 => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

/// An event emitted by a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleEvent {
    pub module_id: String,
    pub event_type: String,
    pub timestamp_ns: u64,
    pub severity: ModuleEventSeverity,
    pub message: String,
    pub data: HashMap<String, String>,
}

/// A data point emitted by a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDataPoint {
    pub module_id: String,
    pub data_type: String,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,
    pub metadata: HashMap<String, String>,
}

/// Generic role requirement for a module (e.g. "needs a power meter")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleRole {
    pub role_id: String,
    pub description: String,
    pub display_name: String,
    pub required_capability: String,
    pub allows_multiple: bool,
}

/// Parameter definition for a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleParameter {
    pub param_id: String,
    pub display_name: String,
    pub description: String,
    pub param_type: String,
    pub default_value: String,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub enum_values: Vec<String>,
    pub units: String,
    pub required: bool,
}

/// Static information about a module type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleTypeInfo {
    pub type_id: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub parameters: Vec<ModuleParameter>,
    pub event_types: Vec<String>,
    pub data_types: Vec<String>,
    pub required_roles: Vec<ModuleRole>,
    pub optional_roles: Vec<ModuleRole>,
}
