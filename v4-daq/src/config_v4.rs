//! V4 Configuration System using Figment
//!
//! This module provides strongly-typed configuration loading for the V4 architecture.
//! Configuration is loaded from:
//! 1. config.v4.toml file (base configuration)
//! 2. Environment variables (prefixed with RUST_DAQ_)
//!
//! # Example
//! ```no_run
//! use rust_daq::config_v4::V4Config;
//!
//! let config = V4Config::load()?;
//! println!("Application: {}", config.application.name);
//! ```

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

/// Top-level V4 configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V4Config {
    /// Application settings
    pub application: ApplicationConfig,
    /// Kameo actor system settings
    pub actors: ActorConfig,
    /// Storage backend settings
    pub storage: StorageConfig,
    /// Instrument definitions
    pub instruments: Vec<InstrumentDefinition>,
}

/// Application-level configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationConfig {
    /// Application name
    pub name: String,
    /// Logging level (trace, debug, info, warn, error)
    pub log_level: String,
}

/// Kameo actor system configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorConfig {
    /// Default mailbox capacity for actor message queues
    #[serde(default = "default_mailbox_capacity")]
    pub default_mailbox_capacity: usize,
    /// Actor spawn timeout in milliseconds
    #[serde(default = "default_spawn_timeout")]
    pub spawn_timeout_ms: u64,
    /// Actor shutdown timeout in milliseconds
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_ms: u64,
}

/// Storage backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Default storage backend (arrow, hdf5, or both)
    pub default_backend: String,
    /// Output directory for data files
    pub output_dir: PathBuf,
    /// Compression level (0-9)
    #[serde(default = "default_compression")]
    pub compression_level: u8,
    /// Auto-flush interval in seconds (0 = manual only)
    #[serde(default)]
    pub auto_flush_interval_secs: u64,
}

/// Instrument definition in configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentDefinition {
    /// Unique instrument identifier
    pub id: String,
    /// Instrument type (e.g., "MockPowerMeter", "Newport1830C")
    pub r#type: String,
    /// Whether this instrument is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Instrument-specific configuration (dynamic)
    pub config: TomlValue,
}

// Default value functions
fn default_mailbox_capacity() -> usize {
    100
}

fn default_spawn_timeout() -> u64 {
    5000
}

fn default_shutdown_timeout() -> u64 {
    5000
}

fn default_compression() -> u8 {
    6
}

fn default_enabled() -> bool {
    true
}

impl V4Config {
    /// Load configuration from config.v4.toml and environment variables
    ///
    /// Environment variables can override configuration with prefix RUST_DAQ_
    /// Example: RUST_DAQ_APPLICATION_LOG_LEVEL=debug
    pub fn load() -> Result<Self, figment::Error> {
        Self::load_from("config/config.v4.toml")
    }

    /// Load configuration from a specific file path
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Toml::file(path.as_ref()))
            .merge(Env::prefixed("RUST_DAQ_").split("_"))
            .extract()
    }

    /// Validate configuration after loading
    pub fn validate(&self) -> Result<(), String> {
        // Validate log level
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.application.log_level.as_str()) {
            return Err(format!(
                "Invalid log_level '{}'. Must be one of: {}",
                self.application.log_level,
                valid_levels.join(", ")
            ));
        }

        // Validate storage backend
        let valid_backends = ["arrow", "hdf5", "both"];
        if !valid_backends.contains(&self.storage.default_backend.as_str()) {
            return Err(format!(
                "Invalid storage backend '{}'. Must be one of: {}",
                self.storage.default_backend,
                valid_backends.join(", ")
            ));
        }

        // Validate compression level
        if self.storage.compression_level > 9 {
            return Err(format!(
                "Invalid compression_level {}. Must be 0-9",
                self.storage.compression_level
            ));
        }

        // Validate instrument IDs are unique
        let mut ids = std::collections::HashSet::new();
        for instrument in &self.instruments {
            if !ids.insert(&instrument.id) {
                return Err(format!("Duplicate instrument ID: {}", instrument.id));
            }
        }

        Ok(())
    }

    /// Get all enabled instruments
    pub fn enabled_instruments(&self) -> Vec<&InstrumentDefinition> {
        self.instruments
            .iter()
            .filter(|inst| inst.enabled)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config() {
        // This test requires config.v4.toml to exist
        let result = V4Config::load();

        // In CI or environments without the config file, this test will be skipped
        // But if the file exists, we must validate it properly
        if result.is_err() {
            let err_msg = result.unwrap_err().to_string();
            // Only accept "file not found" errors - all other errors should fail
            assert!(
                err_msg.contains("No such file") || err_msg.contains("cannot find"),
                "Config load failed with unexpected error: {}",
                err_msg
            );
            eprintln!("Config file not found (expected in CI) - skipping validation");
            return;
        }

        // If we successfully loaded, validate the config
        let config = result.unwrap();
        assert_eq!(config.application.name, "Rust DAQ V4");
        assert!(config.validate().is_ok(), "Config validation failed");

        // Verify required fields are present
        assert!(!config.application.log_level.is_empty());
        assert!(config.actors.default_mailbox_capacity > 0);
        assert!(!config.storage.default_backend.is_empty());
    }

    #[test]
    fn test_config_validation() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_log_level() {
        let mut config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "invalid".to_string(),
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_duplicate_instrument_ids() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![
                InstrumentDefinition {
                    id: "test1".to_string(),
                    r#type: "Mock".to_string(),
                    enabled: true,
                    config: toml::Value::Table(toml::map::Map::new()),
                },
                InstrumentDefinition {
                    id: "test1".to_string(),
                    r#type: "Mock".to_string(),
                    enabled: true,
                    config: toml::Value::Table(toml::map::Map::new()),
                },
            ],
        };

        assert!(config.validate().is_err());
    }
}
