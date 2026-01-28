//! # daq-hardware
//!
//! Hardware abstraction layer for rust-daq with device registry and driver management.
//!
//! This crate provides the central hardware driver system:
//!
//! - **[`DeviceRegistry`]** - Thread-safe device registration and discovery
//! - **[`DriverFactory`]** - Plugin architecture for dynamic driver loading
//! - **Capability Traits** - [`Movable`], [`Readable`], [`FrameProducer`], etc.
//! - **Config-Driven Drivers** - TOML-based generic serial drivers
//! - **Serial Port Management** - Stable by-id paths and multidrop bus support
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use daq_hardware::{DeviceRegistry, DeviceConfig, DriverType};
//!
//! let registry = DeviceRegistry::new();
//!
//! // Register a device
//! registry.register(DeviceConfig {
//!     id: "rotator".into(),
//!     name: "ELL14 Rotator".into(),
//!     driver: DriverType::Ell14 { port: "/dev/ttyUSB0".into(), address: "2".into() },
//! }).await?;
//!
//! // Access by capability
//! if let Some(device) = registry.get_movable("rotator") {
//!     device.move_abs(45.0).await?;
//! }
//! ```
//!
//! ## Feature Flags
//!
//! - `serial` - Serial communication via tokio-serial
//! - `thorlabs`, `newport`, `spectra_physics` - Hardware-specific drivers
//! - `pvcam` - Photometrics camera support
//! - `comedi` - NI DAQ card support
//!
//! [`DeviceRegistry`]: registry::DeviceRegistry
//! [`DriverFactory`]: factory::DriverFactory
//! [`Movable`]: capabilities::Movable
//! [`Readable`]: capabilities::Readable
//! [`FrameProducer`]: capabilities::FrameProducer

// TODO: Fix doc comment generic types (e.g., `Arc<Mutex>`) to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub use common::capabilities;
pub mod config;
pub mod drivers;
pub mod factory;
pub mod plugin;
pub mod port_resolver;
pub mod registry;
pub mod resource_pool;

pub use capabilities::*;
pub use registry::{
    register_all_factories, register_mock_factories, DeviceConfig, DeviceInfo, DeviceRegistry,
    DriverType,
};

// Re-export declarative config types under a distinct name to avoid confusion
// with registry::DeviceConfig (which is for device registration)
pub use config::DeviceConfig as DeclarativeDeviceConfig;

// Re-export factory types for config-driven driver creation
pub use factory::{
    load_all_factories, ConfiguredBus, ConfiguredDriver, DriverFactory, GenericSerialDriverFactory,
    GenericSerialInstanceConfig,
};
