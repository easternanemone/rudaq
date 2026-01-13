//! Plugin root module and entry point definitions.
//!
//! Plugins export a root module via `get_root_module()` that provides
//! metadata and factory functions for creating modules.

#![allow(non_camel_case_types)] // abi_stable generates `*_Ref` types

use crate::metadata::{AbiVersion, PluginMetadata};
use crate::module_ffi::{FfiModuleTypeInfo, ModuleFfiBox};
use abi_stable::library::RootModule;
use abi_stable::package_version_strings;
use abi_stable::sabi_types::VersionStrings;
use abi_stable::std_types::{RResult, RString, RVec};
use abi_stable::{declare_root_module_statics, StableAbi};

/// The root module that plugins export.
///
/// This is the entry point for the plugin loader. It provides:
/// - Plugin metadata for identification and compatibility checking
/// - Factory function to create module instances
/// - List of available module types
///
/// # Example
///
/// ```rust,ignore
/// use daq_plugin_api::prelude::*;
///
/// #[export_root_module]
/// fn get_root_module() -> PluginMod_Ref {
///     PluginMod {
///         abi_version,
///         get_metadata,
///         list_module_types,
///         create_module,
///     }
///     .leak_into_prefix()
/// }
///
/// #[sabi_extern_fn]
/// fn abi_version() -> AbiVersion {
///     AbiVersion::CURRENT
/// }
///
/// #[sabi_extern_fn]
/// fn get_metadata() -> PluginMetadata {
///     PluginMetadata::new("my-plugin", "My Plugin", "1.0.0")
/// }
/// ```
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = PluginMod_Ref)))]
#[sabi(missing_field(panic))]
pub struct PluginMod {
    /// Get the ABI version this plugin was compiled with
    pub abi_version: extern "C" fn() -> AbiVersion,

    /// Get plugin metadata
    pub get_metadata: extern "C" fn() -> PluginMetadata,

    /// List available module types provided by this plugin
    #[sabi(last_prefix_field)]
    pub list_module_types: extern "C" fn() -> RVec<FfiModuleTypeInfo>,

    /// Create a module instance by type ID
    ///
    /// Returns the module boxed in an FFI-safe container, or an error message.
    pub create_module: extern "C" fn(type_id: RString) -> RResult<ModuleFfiBox, RString>,
}

impl RootModule for PluginMod_Ref {
    declare_root_module_statics! {PluginMod_Ref}

    const BASE_NAME: &'static str = "daq_plugin";
    const NAME: &'static str = "daq_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}

impl PluginMod_Ref {
    /// Check if this plugin's ABI is compatible with the host
    pub fn is_compatible(&self) -> bool {
        let plugin_version = self.abi_version()();
        plugin_version.is_compatible_with(&AbiVersion::CURRENT)
    }
}

/// Type alias for the plugin reference type
pub type PluginRef = PluginMod_Ref;

/// Error type for plugin loading
#[derive(Debug, Clone)]
pub enum PluginLoadError {
    /// The library file could not be loaded
    LoadFailed(String),
    /// The plugin's ABI version is incompatible
    IncompatibleAbi {
        plugin_version: AbiVersion,
        host_version: AbiVersion,
    },
    /// The root module could not be found
    NoRootModule,
    /// Plugin initialization failed
    InitFailed(String),
}

impl std::fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadFailed(msg) => write!(f, "Failed to load plugin library: {}", msg),
            Self::IncompatibleAbi {
                plugin_version,
                host_version,
            } => write!(
                f,
                "Plugin ABI version {} is incompatible with host version {}",
                plugin_version, host_version
            ),
            Self::NoRootModule => write!(f, "Plugin does not export a root module"),
            Self::InitFailed(msg) => write!(f, "Plugin initialization failed: {}", msg),
        }
    }
}

impl std::error::Error for PluginLoadError {}
