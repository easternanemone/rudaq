//! Plugin system for rust-daq modules.
//!
//! This module provides infrastructure for loading modules from various sources:
//!
//! - **Native plugins** (via daq-plugin-api): Compiled Rust plugins using abi_stable
//! - **Script plugins** (this module): Rhai and Python scripts that implement modules
//!
//! # Architecture
//!
//! ```text
//! ModuleRegistry (rust-daq/src/modules/)
//! ├── Built-in modules (PowerMonitor, etc.)
//! ├── Native plugins (daq-plugin-api)
//! └── Script plugins (this module)
//!     ├── ScriptPluginLoader - Discovery and loading
//!     └── ScriptModule - Script-based Module implementation
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use rust_daq::plugins::{ScriptPluginLoader, ScriptModule};
//!
//! // Discover script modules from a directory
//! let mut loader = ScriptPluginLoader::new();
//! loader.add_search_path("./scripts");
//! loader.discover().await?;
//!
//! // Create a module instance
//! let module = loader.create_module("my_script_module").await?;
//!
//! // Or load directly from a file
//! let module = ScriptModule::from_file("./scripts/my_module.rhai").await?;
//! ```
//!
//! # Script Module Contract
//!
//! Scripts must define `module_type_info()` returning module metadata:
//!
//! ```rhai
//! fn module_type_info() {
//!     #{
//!         type_id: "my_module",
//!         display_name: "My Module",
//!         description: "Does something useful",
//!         version: "1.0.0",
//!         parameters: [...],
//!         required_roles: [...],
//!         optional_roles: [...],
//!         event_types: [...],
//!         data_types: [...]
//!     }
//! }
//!
//! fn start(ctx) {
//!     // Main module logic
//! }
//! ```

pub mod loader;
pub mod script_module;

pub use loader::{ScriptLanguage, ScriptModuleInfo, ScriptPluginLoader};
pub use script_module::ScriptModule;
