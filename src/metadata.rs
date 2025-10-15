//! Experimental metadata structures and handling.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Captures comprehensive metadata for an experiment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    /// The name of the experiment.
    pub experiment_name: String,
    /// A detailed description of the experiment.
    pub description: String,
    /// Configuration of the instruments used.
    pub instrument_config: HashMap<String, String>,
    /// User-defined experimental parameters.
    pub parameters: HashMap<String, serde_json::Value>,
    /// User annotations or notes.
    pub annotations: String,
    /// Environmental conditions (e.g., temperature, humidity).
    pub environment: HashMap<String, f64>,
    /// Version of the data acquisition software.
    pub software_version: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            experiment_name: "Default Experiment".to_string(),
            description: "".to_string(),
            instrument_config: HashMap::new(),
            parameters: HashMap::new(),
            annotations: "".to_string(),
            environment: HashMap::new(),
            software_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// A builder for constructing `Metadata` instances.
#[derive(Default)]
pub struct MetadataBuilder {
    inner: Metadata,
}

impl MetadataBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn experiment_name(mut self, name: &str) -> Self {
        self.inner.experiment_name = name.to_string();
        self
    }

    pub fn description(mut self, description: &str) -> Self {
        self.inner.description = description.to_string();
        self
    }

    pub fn instrument_config(mut self, key: &str, value: &str) -> Self {
        self.inner
            .instrument_config
            .insert(key.to_string(), value.to_string());
        self
    }

    pub fn parameter(mut self, key: &str, value: serde_json::Value) -> Self {
        self.inner.parameters.insert(key.to_string(), value);
        self
    }

    pub fn annotations(mut self, annotations: &str) -> Self {
        self.inner.annotations = annotations.to_string();
        self
    }

    pub fn environment(mut self, key: &str, value: f64) -> Self {
        self.inner.environment.insert(key.to_string(), value);
        self
    }

    pub fn build(self) -> Metadata {
        self.inner
    }
}

impl Metadata {
    /// Validates the metadata.
    pub fn validate(&self) -> Result<(), String> {
        if self.experiment_name.is_empty() {
            return Err("Experiment name cannot be empty.".to_string());
        }
        Ok(())
    }
}
