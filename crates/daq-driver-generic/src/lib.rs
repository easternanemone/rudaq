//! Generic config-driven serial driver for rust-daq.
//!
//! This crate provides a [`GenericSerialDriver`] that interprets TOML device
//! configurations at runtime, enabling new devices to be added without code changes.
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_driver_generic::{GenericSerialDriver, SharedPort};
//! use daq_plugin_api::config::InstrumentConfig;
//!
//! // Load config from TOML
//! let config: InstrumentConfig = toml::from_str(config_str)?;
//!
//! // Create driver with shared port
//! let driver = GenericSerialDriver::new(config, shared_port, "2")?;
//!
//! // Use via capability traits
//! use common::capabilities::Movable;
//! driver.move_abs(45.0).await?;
//! ```

pub mod driver;
pub mod factory;

#[cfg(feature = "scripting")]
pub mod script_engine;

// Re-export main driver types
pub use driver::{
    CommandResult, DeviceError, DynSerial, GenericSerialDriver, ParsedResponse, ResponseValue,
    SerialPortIO, SharedPort,
};
pub use factory::{load_all_factories, GenericSerialDriverFactory, GenericSerialInstanceConfig};
