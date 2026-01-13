//! FFI-stable plugin API for rust-daq modules.
//!
//! This crate provides the ABI-stable interface for native plugins using `abi_stable`.
//! Plugins implement the `ModuleFfi` trait and export a root module via `get_root_module()`.
//!
//! # Architecture
//!
//! ```text
//! PluginManager
//! ├── NativeLoader (abi_stable) ← This crate
//! ├── ScriptLoader (daq-scripting)
//! └── WasmLoader (future)
//! ```
//!
//! # Creating a Plugin
//!
//! ```rust,ignore
//! use daq_plugin_api::prelude::*;
//!
//! #[export_root_module]
//! fn get_root_module() -> PluginMod_Ref {
//!     PluginMod { ... }.leak_into_prefix()
//! }
//! ```

pub mod loader;
pub mod metadata;
pub mod module_ffi;
pub mod plugin;

pub use loader::*;
pub use metadata::*;
pub use module_ffi::*;
pub use plugin::*;

/// Prelude for plugin authors
pub mod prelude {
    pub use crate::loader::{LoadedPlugin, PluginManager};
    pub use crate::metadata::{AbiVersion, PluginMetadata};
    pub use crate::module_ffi::{
        FfiModuleConfig, FfiModuleContext, FfiModuleDataPoint, FfiModuleEvent, FfiModuleParameter,
        FfiModuleResult, FfiModuleRole, FfiModuleState, FfiModuleTypeInfo, ModuleFfi, ModuleFfiBox,
        ModuleFfi_TO,
    };
    pub use crate::plugin::{PluginLoadError, PluginMod, PluginMod_Ref, PluginRef};
    pub use abi_stable::export_root_module;
    pub use abi_stable::library::RootModule;
    pub use abi_stable::prefix_type::PrefixTypeTrait;
    pub use abi_stable::sabi_extern_fn;
    pub use abi_stable::sabi_trait;
    pub use abi_stable::std_types::{RHashMap, ROption, RResult, RStr, RString, RVec};
    pub use abi_stable::StableAbi;
}

/// Re-export abi_stable for plugin convenience
pub use abi_stable;
