//! V4 Configuration System
//!
//! This module provides configuration management for the V4 architecture using Figment.
//!
//! # Configuration Sources
//!
//! Configuration is loaded from (in order of precedence):
//! 1. Environment variables prefixed with `RUSTDAQ_`
//! 2. TOML configuration file (default: `config/config.v4.toml`)
//!
//! # Example
//!
//! ```no_run
//! use v4_daq::config::V4Config;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Load from default location
//!     let config = V4Config::load()?;
//!
//!     // Or load from custom location
//!     let config = V4Config::load_from("custom/path.toml")?;
//!
//!     println!("App name: {}", config.application.name);
//!     println!("Log level: {}", config.application.log_level);
//!     println!("Enabled instruments: {}", config.enabled_instruments().len());
//!
//!     Ok(())
//! }
//! ```
//!
//! # Environment Variables
//!
//! Any configuration value can be overridden via environment variables with the
//! `RUSTDAQ_` prefix and key path separated by underscores:
//!
//! ```text
//! # Set application name
//! RUSTDAQ_APPLICATION_NAME="My DAQ System"
//!
//! # Set log level
//! RUSTDAQ_APPLICATION_LOG_LEVEL=debug
//!
//! # Set storage compression
//! RUSTDAQ_STORAGE_COMPRESSION_LEVEL=9
//! ```

pub mod v4_config;

pub use v4_config::{
    V4Config, ConfigError, ApplicationConfig, ActorConfig, StorageConfig, InstrumentDefinition,
    InstrumentSpecificConfig, ScpiConfig, ESP300Config, PVCAMConfig, NewportConfig, MaiTaiConfig,
};
