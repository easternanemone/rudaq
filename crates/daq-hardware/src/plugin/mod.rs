//! Data-driven instrument plugin system.
//!
//! This module provides both YAML-defined instrument drivers and native plugin
//! discovery that can be loaded and instantiated at runtime without recompilation.
//!
//! # Architecture
//!
//! ## YAML Instrument Plugins
//! - `schema` - YAML schema types for plugin configuration
//! - `driver` - Generic driver that interprets YAML configs
//! - `registry` - Plugin factory for loading and spawning drivers
//! - `handles` - Capability handle types implementing standard traits
//!
//! ## Native Plugin Discovery
//! - `manifest` - Plugin.toml manifest types for native/script/WASM plugins
//! - `discovery` - Directory scanner and plugin registry with versioning
//!
//! # Example (YAML Instrument Plugin)
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use rust_daq::hardware::plugin::registry::PluginFactory;
//! use rust_daq::hardware::capabilities::Movable;
//!
//! let mut factory = PluginFactory::new();
//! factory.load_plugins(Path::new("plugins/")).await?;
//!
//! // Spawn driver and wrap in Arc for handle creation
//! let driver = Arc::new(factory.spawn("my-stage", "/dev/ttyUSB0").await?);
//!
//! // Create axis handle that implements Movable trait
//! let x_axis = driver.axis_handle("x", false);
//! x_axis.move_abs(10.0).await?;
//! ```
//!
//! # Example (Native Plugin Discovery)
//!
//! ```rust,ignore
//! use rust_daq::hardware::plugin::discovery::PluginRegistry;
//!
//! let mut registry = PluginRegistry::new();
//! registry.add_search_path("~/.config/rust-daq/plugins/");
//! registry.add_search_path("/usr/share/rust-daq/plugins/");
//!
//! let errors = registry.scan();
//! for info in registry.list() {
//!     println!("{} v{}", info.name(), info.version);
//! }
//! ```

// Native plugin discovery (always available)
pub mod discovery;
pub mod manifest;

// YAML instrument plugins (requires serial feature)
#[cfg(feature = "serial")]
pub mod driver;
#[cfg(feature = "serial")]
pub mod handles;
#[cfg(feature = "plugins_hot_reload")]
pub mod hot_reload;
#[cfg(feature = "plugins_hot_reload")]
pub mod lib_reload;
#[cfg(feature = "serial")]
pub mod registry;
pub mod schema;
