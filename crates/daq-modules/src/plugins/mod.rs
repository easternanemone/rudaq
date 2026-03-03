//! Plugin system for rust-daq modules.
//!
//! This module provides infrastructure for loading modules from various sources:
//!
//! - **Script plugins** (this module): Rhai and Python scripts that implement modules
//!
//! # Architecture
//!
//! ```text
//! ModuleRegistry (rust-daq/src/modules/)
//! ├── Built-in modules (PowerMonitor, etc.)
//! └── Script plugins (this module) [requires scripting feature]
//!     ├── ScriptPluginLoader - Discovery and loading
//!     └── ScriptModule - Script-based Module implementation
//! ```

// Script plugins - requires scripting feature (depends on scripting)
#[cfg(feature = "scripting")]
pub mod loader;
#[cfg(feature = "scripting")]
pub mod script_module;

#[cfg(feature = "scripting")]
pub use loader::{ScriptLanguage, ScriptModuleInfo, ScriptPluginLoader};
#[cfg(feature = "scripting")]
pub use script_module::ScriptModule;
