//! Generic config-driven serial driver for rust-daq.
//!
//! This crate provides a config-driven approach to hardware drivers, enabling
//! new devices to be added without code changes by defining them in TOML files.
//!
//! # Architecture
//!
//! The [`GenericSerialDriver`] interprets TOML device configurations at runtime:
//! - Command templating with parameter interpolation
//! - Response parsing using regex patterns
//! - Unit conversions via evalexpr formulas
//! - Trait implementations based on config mappings
//! - Optional Rhai scripting for complex operations
//!
//! # Usage
//!
//! Add to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! daq-driver-generic = { path = "../daq-driver-generic" }
//! ```
//!
//! Create a driver from a TOML config:
//!
//! ```rust,ignore
//! use daq_driver_generic::{DriverFactory, SharedPort};
//! use std::path::Path;
//!
//! // Create driver from config file
//! let driver = DriverFactory::create_from_file_async(
//!     Path::new("config/devices/ell14.toml"),
//!     shared_port,
//!     "2"
//! ).await?;
//!
//! // Use via capability traits
//! use daq_core::capabilities::Movable;
//! driver.move_abs(45.0).await?;
//! ```
//!
//! Or use the factory with a device registry:
//!
//! ```rust,ignore
//! use daq_driver_generic::GenericSerialDriverFactory;
//!
//! let factory = GenericSerialDriverFactory::from_file(
//!     Path::new("config/devices/ell14.toml")
//! )?;
//! registry.register_factory(Box::new(factory));
//! ```

// Re-export core driver types from daq-hardware
pub use daq_hardware::drivers::generic_serial::{
    CommandResult, DeviceError, DynSerial, GenericSerialDriver, ParsedResponse, ResponseValue,
    SerialPortIO, SharedPort,
};

// Re-export factory types
pub use daq_hardware::factory::{
    load_all_factories, ConfiguredBus, ConfiguredDriver, DriverFactory,
    GenericSerialDriverFactory, GenericSerialInstanceConfig,
};

// Re-export config types for defining devices
pub use daq_hardware::config::{
    load_all_devices, load_device_config, load_device_config_from_str, DeviceConfig,
};

// Re-export capability traits for convenience
pub use daq_core::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};

/// Force the linker to include this crate.
///
/// Call this function from main() to ensure the driver factories are
/// linked into the final binary and not stripped by the linker.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<GenericSerialDriverFactory>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_does_not_panic() {
        link();
    }

    #[test]
    fn test_config_loads() {
        // Verify we can load a config from string
        let config_str = r#"
[device]
name = "Test Device"
protocol = "test"

[connection]
type = "serial"
timeout_ms = 500
"#;
        let config = load_device_config_from_str(config_str).unwrap();
        assert_eq!(config.device.name, "Test Device");
        assert_eq!(config.device.protocol, "test");
    }
}
