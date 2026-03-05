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
//! - `pvcam`, `comedi`, `andor` - Native SDK hardware drivers
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

// --- Runtime modules (use tokio::fs, tokio_util, etc. — native-only) ---
#[cfg(not(target_arch = "wasm32"))]
pub mod drivers;
#[cfg(not(target_arch = "wasm32"))]
pub mod manifest_driver;
/// Backward-compat alias: `plugin` → `manifest_driver`
#[cfg(not(target_arch = "wasm32"))]
pub use manifest_driver as plugin;
#[cfg(not(target_arch = "wasm32"))]
pub mod port_resolver;
#[cfg(not(target_arch = "wasm32"))]
pub mod registry;
#[cfg(not(target_arch = "wasm32"))]
pub mod resource_pool;
#[cfg(not(target_arch = "wasm32"))]
pub mod supervisor;

pub use capabilities::*;
#[cfg(not(target_arch = "wasm32"))]
pub use registry::{
    populate_registry_from_config, register_mock_factories, DeviceConfig, DeviceInfo,
    DeviceRegistry, DriverConfig,
};
