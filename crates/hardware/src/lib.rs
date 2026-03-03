//! # daq-hardware
//!
//! Hardware abstraction layer for rust-daq with device registry and driver management.
//!
//! This crate provides the central hardware driver system:
//!
//! - **[`DeviceRegistry`]** - Thread-safe device registration and discovery
//! - **Capability Traits** - [`Movable`], [`Readable`], [`FrameProducer`], etc.
//! - **Manifest Drivers** - TOML/YAML-driven instrument definitions
//! - **Serial Port Management** - Stable by-id paths and multidrop bus support
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use daq_hardware::{DeviceRegistry, register_mock_factories};
//! // For concrete hardware drivers, use driver_registry::register_all_factories
//!
//! let registry = DeviceRegistry::new();
//! register_mock_factories(&registry);
//!
//! // Register a device via factory
//! registry.register_from_toml(
//!     "stage", "Mock Stage", "mock_stage",
//!     toml::Value::Table(Default::default()),
//! ).await?;
//!
//! // Access by capability
//! if let Some(device) = registry.get_movable("stage") {
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
//! [`Movable`]: capabilities::Movable
//! [`Readable`]: capabilities::Readable
//! [`FrameProducer`]: capabilities::FrameProducer

// TODO: Fix doc comment generic types (e.g., `Arc<Mutex>`) to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub use common::capabilities;
pub mod config;
pub mod drivers;
pub mod manifest_driver;
/// Backward-compat alias: `plugin` → `manifest_driver`
pub use manifest_driver as plugin;
pub mod port_resolver;
pub mod registry;
pub mod resource_pool;
pub mod supervisor;

pub use capabilities::*;
pub use registry::{
    populate_registry_from_config, register_mock_factories, DeviceConfig, DeviceInfo,
    DeviceRegistry, DriverConfig,
};
