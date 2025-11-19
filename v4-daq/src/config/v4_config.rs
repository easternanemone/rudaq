//! V4 Configuration System using Figment
//!
//! This module provides strongly-typed configuration loading for the V4 architecture.
//! Configuration is loaded from:
//! 1. config.v4.toml file (base configuration)
//! 2. Environment variables (prefixed with RUSTDAQ_)
//!
//! # Environment Variable Overrides
//!
//! Environment variables with the `RUSTDAQ_` prefix can override configuration values:
//!
//! ```text
//! RUSTDAQ_APPLICATION_LOG_LEVEL=debug
//! RUSTDAQ_APPLICATION_NAME="My DAQ"
//! RUSTDAQ_STORAGE_COMPRESSION_LEVEL=9
//! ```
//!
//! # Example
//!
//! ```no_run
//! use v4_daq::config::V4Config;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = V4Config::load()?;
//!     println!("Application: {}", config.application.name);
//!     println!("Log level: {}", config.application.log_level);
//!     println!("Instruments: {}", config.instruments.len());
//!     Ok(())
//! }
//! ```

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Configuration error types
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Configuration load error: {0}")]
    LoadError(#[from] figment::Error),
    #[error("Configuration validation error: {0}")]
    ValidationError(String),
}

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
    /// Optional data directory for persistent storage
    #[serde(default)]
    pub data_dir: Option<PathBuf>,
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
    /// Instrument type (ScpiInstrument, ESP300, PVCAMInstrument, Newport1830C, MaiTai)
    pub r#type: String,
    /// Whether this instrument is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Instrument-specific configuration
    #[serde(default)]
    pub config: InstrumentSpecificConfig,
}

/// Instrument-specific configuration, supporting all 5 V4 actor types
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstrumentSpecificConfig {
    /// SCPI-specific configuration
    #[serde(default)]
    pub scpi: Option<ScpiConfig>,
    /// ESP300 motion control configuration
    #[serde(default)]
    pub esp300: Option<ESP300Config>,
    /// PVCAM camera configuration
    #[serde(default)]
    pub pvcam: Option<PVCAMConfig>,
    /// Newport 1830-C power meter configuration
    #[serde(default)]
    pub newport: Option<NewportConfig>,
    /// MaiTai laser configuration
    #[serde(default)]
    pub maitai: Option<MaiTaiConfig>,
}

/// SCPI instrument configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpiConfig {
    /// VISA resource string (e.g., "TCPIP0::192.168.1.100::INSTR")
    pub resource: String,
    /// Query timeout in milliseconds
    #[serde(default = "default_scpi_timeout")]
    pub timeout_ms: u64,
    /// Enable caching of repeated queries
    #[serde(default)]
    pub enable_caching: bool,
}

/// ESP300 motion controller configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ESP300Config {
    /// Serial port (e.g., "/dev/ttyUSB1")
    pub serial_port: String,
    /// Number of controlled axes
    #[serde(default = "default_esp300_axes")]
    pub axes: u32,
    /// Baud rate
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
}

/// PVCAM camera configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PVCAMConfig {
    /// Camera name (e.g., "PrimeBSI")
    pub camera_name: String,
    /// Frame width in pixels
    #[serde(default)]
    pub frame_width: Option<u32>,
    /// Frame height in pixels
    #[serde(default)]
    pub frame_height: Option<u32>,
    /// Exposure time in milliseconds
    #[serde(default)]
    pub exposure_ms: Option<f64>,
}

/// Newport 1830-C power meter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewportConfig {
    /// VISA resource string (e.g., "ASRL2::INSTR")
    pub resource: String,
    /// Query timeout in milliseconds
    #[serde(default = "default_newport_timeout")]
    pub timeout_ms: u64,
    /// Wavelength in nanometers for power correction
    #[serde(default)]
    pub wavelength_nm: Option<f64>,
}

/// MaiTai laser configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaiTaiConfig {
    /// Serial port (e.g., "/dev/ttyUSB2")
    pub serial_port: String,
    /// Baud rate
    #[serde(default = "default_maitai_baud_rate")]
    pub baud_rate: u32,
    /// Query timeout in milliseconds
    #[serde(default = "default_maitai_timeout")]
    pub timeout_ms: u64,
    /// Enable automatic wavelength control
    #[serde(default = "default_maitai_auto_control")]
    pub auto_control: bool,
}

// ============================================================================
// Default value functions
// ============================================================================

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

fn default_scpi_timeout() -> u64 {
    5000
}

fn default_esp300_axes() -> u32 {
    3
}

fn default_baud_rate() -> u32 {
    9600
}

fn default_newport_timeout() -> u64 {
    5000
}

fn default_maitai_baud_rate() -> u32 {
    19200
}

fn default_maitai_timeout() -> u64 {
    5000
}

fn default_maitai_auto_control() -> bool {
    false
}

// ============================================================================
// Configuration Loading and Validation
// ============================================================================

impl V4Config {
    /// Load configuration from config.v4.toml and environment variables
    ///
    /// Configuration is loaded in this order of precedence (highest to lowest):
    /// 1. Environment variables (RUSTDAQ_ prefix)
    /// 2. config.v4.toml file
    ///
    /// After loading, configuration is validated.
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if:
    /// - The config file cannot be loaded
    /// - Configuration validation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// use v4_daq::config::V4Config;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = V4Config::load()?;
    ///     config.validate()?;
    ///     Ok(())
    /// }
    /// ```
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from("config/config.v4.toml")
    }

    /// Load configuration from a specific file path
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the configuration file (relative or absolute)
    ///
    /// # Errors
    ///
    /// Returns a ConfigError if the file cannot be loaded or is invalid.
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let config: Self = Figment::new()
            .merge(Toml::file(path.as_ref()))
            .merge(Env::prefixed("RUSTDAQ_").split("_"))
            .extract()
            .map_err(ConfigError::LoadError)?;

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration after loading
    ///
    /// Checks:
    /// - Log level is valid (trace, debug, info, warn, error)
    /// - Storage backend is valid (arrow, hdf5, both)
    /// - Compression level is 0-9
    /// - Instrument IDs are unique
    /// - Each instrument has required fields for its type
    ///
    /// # Errors
    ///
    /// Returns a ConfigError with a descriptive message for any validation failure.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate log level
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.application.log_level.as_str()) {
            return Err(ConfigError::ValidationError(format!(
                "Invalid log_level '{}'. Must be one of: {}",
                self.application.log_level,
                valid_levels.join(", ")
            )));
        }

        // Validate storage backend
        let valid_backends = ["arrow", "hdf5", "both"];
        if !valid_backends.contains(&self.storage.default_backend.as_str()) {
            return Err(ConfigError::ValidationError(format!(
                "Invalid storage backend '{}'. Must be one of: {}",
                self.storage.default_backend,
                valid_backends.join(", ")
            )));
        }

        // Validate compression level
        if self.storage.compression_level > 9 {
            return Err(ConfigError::ValidationError(format!(
                "Invalid compression_level {}. Must be 0-9",
                self.storage.compression_level
            )));
        }

        // Validate instrument IDs are unique
        let mut ids = std::collections::HashSet::new();
        for instrument in &self.instruments {
            if !ids.insert(&instrument.id) {
                return Err(ConfigError::ValidationError(format!(
                    "Duplicate instrument ID: '{}'",
                    instrument.id
                )));
            }

            // Validate instrument type is one of the 5 V4 actors
            let valid_types = [
                "ScpiInstrument",
                "ESP300",
                "PVCAMInstrument",
                "Newport1830C",
                "MaiTai",
            ];
            if !valid_types.contains(&instrument.r#type.as_str()) {
                return Err(ConfigError::ValidationError(format!(
                    "Invalid instrument type '{}' for instrument '{}'. Must be one of: {}",
                    instrument.r#type,
                    instrument.id,
                    valid_types.join(", ")
                )));
            }

            // Validate instrument-specific configuration
            self.validate_instrument(&instrument)?;
        }

        Ok(())
    }

    /// Validate a specific instrument's configuration
    fn validate_instrument(&self, instrument: &InstrumentDefinition) -> Result<(), ConfigError> {
        match instrument.r#type.as_str() {
            "ScpiInstrument" => {
                if instrument.config.scpi.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "SCPI instrument '{}' missing 'scpi' configuration block",
                        instrument.id
                    )));
                }
                if let Some(scpi) = &instrument.config.scpi {
                    if scpi.resource.is_empty() {
                        return Err(ConfigError::ValidationError(format!(
                            "SCPI instrument '{}': 'resource' cannot be empty",
                            instrument.id
                        )));
                    }
                }
            }
            "ESP300" => {
                if instrument.config.esp300.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "ESP300 instrument '{}' missing 'esp300' configuration block",
                        instrument.id
                    )));
                }
                if let Some(esp300) = &instrument.config.esp300 {
                    if esp300.serial_port.is_empty() {
                        return Err(ConfigError::ValidationError(format!(
                            "ESP300 instrument '{}': 'serial_port' cannot be empty",
                            instrument.id
                        )));
                    }
                    if esp300.axes == 0 {
                        return Err(ConfigError::ValidationError(format!(
                            "ESP300 instrument '{}': 'axes' must be > 0",
                            instrument.id
                        )));
                    }
                }
            }
            "PVCAMInstrument" => {
                if instrument.config.pvcam.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "PVCAM instrument '{}' missing 'pvcam' configuration block",
                        instrument.id
                    )));
                }
                if let Some(pvcam) = &instrument.config.pvcam {
                    if pvcam.camera_name.is_empty() {
                        return Err(ConfigError::ValidationError(format!(
                            "PVCAM instrument '{}': 'camera_name' cannot be empty",
                            instrument.id
                        )));
                    }
                }
            }
            "Newport1830C" => {
                if instrument.config.newport.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "Newport1830C instrument '{}' missing 'newport' configuration block",
                        instrument.id
                    )));
                }
                if let Some(newport) = &instrument.config.newport {
                    if newport.resource.is_empty() {
                        return Err(ConfigError::ValidationError(format!(
                            "Newport1830C instrument '{}': 'resource' cannot be empty",
                            instrument.id
                        )));
                    }
                }
            }
            "MaiTai" => {
                if instrument.config.maitai.is_none() {
                    return Err(ConfigError::ValidationError(format!(
                        "MaiTai instrument '{}' missing 'maitai' configuration block",
                        instrument.id
                    )));
                }
                if let Some(maitai) = &instrument.config.maitai {
                    if maitai.serial_port.is_empty() {
                        return Err(ConfigError::ValidationError(format!(
                            "MaiTai instrument '{}': 'serial_port' cannot be empty",
                            instrument.id
                        )));
                    }
                }
            }
            _ => {
                return Err(ConfigError::ValidationError(format!(
                    "Unknown instrument type: '{}'",
                    instrument.r#type
                )))
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

    /// Get instruments of a specific type
    pub fn instruments_by_type(&self, instrument_type: &str) -> Vec<&InstrumentDefinition> {
        self.instruments
            .iter()
            .filter(|inst| inst.r#type == instrument_type && inst.enabled)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation_valid() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test DAQ".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("/tmp/daq"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_log_level() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "invalid".to_string(),
                data_dir: None,
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

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid log_level"));
    }

    #[test]
    fn test_invalid_storage_backend() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "invalid".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 6,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid storage backend"));
    }

    #[test]
    fn test_invalid_compression_level() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
            },
            actors: ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 5000,
                shutdown_timeout_ms: 5000,
            },
            storage: StorageConfig {
                default_backend: "hdf5".to_string(),
                output_dir: PathBuf::from("data"),
                compression_level: 10,
                auto_flush_interval_secs: 30,
            },
            instruments: vec![],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid compression_level"));
    }

    #[test]
    fn test_duplicate_instrument_ids() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
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
                    r#type: "ESP300".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        esp300: Some(ESP300Config {
                            serial_port: "/dev/ttyUSB1".to_string(),
                            axes: 3,
                            baud_rate: 9600,
                        }),
                        ..Default::default()
                    },
                },
                InstrumentDefinition {
                    id: "test1".to_string(),
                    r#type: "MaiTai".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        maitai: Some(MaiTaiConfig {
                            serial_port: "/dev/ttyUSB2".to_string(),
                            baud_rate: 19200,
                            timeout_ms: 5000,
                            auto_control: false,
                        }),
                        ..Default::default()
                    },
                },
            ],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate instrument ID"));
    }

    #[test]
    fn test_invalid_instrument_type() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
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
            instruments: vec![InstrumentDefinition {
                id: "test1".to_string(),
                r#type: "InvalidType".to_string(),
                enabled: true,
                config: InstrumentSpecificConfig::default(),
            }],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid instrument type"));
    }

    #[test]
    fn test_scpi_missing_resource() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
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
            instruments: vec![InstrumentDefinition {
                id: "scpi_meter".to_string(),
                r#type: "ScpiInstrument".to_string(),
                enabled: true,
                config: InstrumentSpecificConfig {
                    scpi: Some(ScpiConfig {
                        resource: String::new(),
                        timeout_ms: 5000,
                        enable_caching: false,
                    }),
                    ..Default::default()
                },
            }],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("resource' cannot be empty"));
    }

    #[test]
    fn test_enabled_instruments_filter() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
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
                    id: "instrument1".to_string(),
                    r#type: "ESP300".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        esp300: Some(ESP300Config {
                            serial_port: "/dev/ttyUSB1".to_string(),
                            axes: 3,
                            baud_rate: 9600,
                        }),
                        ..Default::default()
                    },
                },
                InstrumentDefinition {
                    id: "instrument2".to_string(),
                    r#type: "MaiTai".to_string(),
                    enabled: false,
                    config: InstrumentSpecificConfig {
                        maitai: Some(MaiTaiConfig {
                            serial_port: "/dev/ttyUSB2".to_string(),
                            baud_rate: 19200,
                            timeout_ms: 5000,
                            auto_control: false,
                        }),
                        ..Default::default()
                    },
                },
            ],
        };

        let enabled = config.enabled_instruments();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "instrument1");
    }

    #[test]
    fn test_instruments_by_type() {
        let config = V4Config {
            application: ApplicationConfig {
                name: "Test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
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
                    id: "scpi_meter1".to_string(),
                    r#type: "ScpiInstrument".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        scpi: Some(ScpiConfig {
                            resource: "TCPIP0::192.168.1.100::INSTR".to_string(),
                            timeout_ms: 5000,
                            enable_caching: false,
                        }),
                        ..Default::default()
                    },
                },
                InstrumentDefinition {
                    id: "scpi_meter2".to_string(),
                    r#type: "ScpiInstrument".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        scpi: Some(ScpiConfig {
                            resource: "TCPIP0::192.168.1.101::INSTR".to_string(),
                            timeout_ms: 5000,
                            enable_caching: false,
                        }),
                        ..Default::default()
                    },
                },
                InstrumentDefinition {
                    id: "esp300_stage".to_string(),
                    r#type: "ESP300".to_string(),
                    enabled: true,
                    config: InstrumentSpecificConfig {
                        esp300: Some(ESP300Config {
                            serial_port: "/dev/ttyUSB1".to_string(),
                            axes: 3,
                            baud_rate: 9600,
                        }),
                        ..Default::default()
                    },
                },
            ],
        };

        let scpi_instruments = config.instruments_by_type("ScpiInstrument");
        assert_eq!(scpi_instruments.len(), 2);
        assert_eq!(scpi_instruments[0].id, "scpi_meter1");
        assert_eq!(scpi_instruments[1].id, "scpi_meter2");

        let esp300_instruments = config.instruments_by_type("ESP300");
        assert_eq!(esp300_instruments.len(), 1);
        assert_eq!(esp300_instruments[0].id, "esp300_stage");
    }
}
