//! Data-driven instrument plugin system.
//!
//! This module provides YAML-defined instrument drivers that can be loaded
//! and instantiated at runtime without recompilation.
//!
//! # Architecture
//!
//! - `schema` - YAML schema types for plugin configuration
//! - `driver` - Generic driver that interprets YAML configs
//! - `registry` - Plugin factory for loading and spawning drivers
//! - `handles` - Capability handle types implementing standard traits
//!
//! # Example
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

pub mod driver;
pub mod handles;
pub mod registry;
pub mod schema;

#[cfg(feature = "plugins_hot_reload")]
pub mod hot_reload;
