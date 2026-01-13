//! Plugin loading infrastructure using abi_stable.
//!
//! This module provides the `PluginManager` for discovering and loading native plugins.

use crate::metadata::{AbiVersion, PluginMetadata};
use crate::module_ffi::{FfiModuleTypeInfo, ModuleFfiBox};
use crate::plugin::{PluginLoadError, PluginMod_Ref};
use abi_stable::library::lib_header_from_path;
use abi_stable::std_types::{RResult, RString, RVec};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A loaded plugin instance
pub struct LoadedPlugin {
    /// Plugin metadata
    pub metadata: PluginMetadata,
    /// The plugin's root module reference
    plugin_ref: PluginMod_Ref,
    /// Path to the loaded library
    pub path: PathBuf,
}

impl LoadedPlugin {
    /// Get the available module types from this plugin
    pub fn module_types(&self) -> RVec<FfiModuleTypeInfo> {
        self.plugin_ref.list_module_types()()
    }

    /// Create a module instance by type ID
    pub fn create_module(&self, type_id: &str) -> Result<ModuleFfiBox, String> {
        let result = self.plugin_ref.create_module()(RString::from(type_id));
        match result {
            RResult::ROk(module) => Ok(module),
            RResult::RErr(err) => Err(err.to_string()),
        }
    }

    /// Check if this plugin provides a given module type
    pub fn has_module_type(&self, type_id: &str) -> bool {
        self.module_types()
            .iter()
            .any(|info| info.type_id.as_str() == type_id)
    }
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("metadata", &self.metadata)
            .field("path", &self.path)
            .finish()
    }
}

/// Manager for discovering and loading native plugins.
///
/// The `PluginManager` handles:
/// - Plugin discovery from configured directories
/// - Loading and ABI verification of plugins
/// - Caching of loaded plugins
/// - Module type registry across all plugins
///
/// # Example
///
/// ```rust,ignore
/// let mut manager = PluginManager::new();
/// manager.add_search_path("./plugins");
/// manager.discover_plugins()?;
///
/// // Create a module from a plugin
/// let module = manager.create_module("my_plugin.custom_module")?;
/// ```
#[derive(Debug)]
pub struct PluginManager {
    /// Directories to search for plugins
    search_paths: Vec<PathBuf>,
    /// Loaded plugins by plugin ID
    plugins: HashMap<String, LoadedPlugin>,
    /// Module type -> plugin ID mapping for fast lookup
    module_type_index: HashMap<String, String>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    /// Create a new plugin manager
    pub fn new() -> Self {
        Self {
            search_paths: Vec::new(),
            plugins: HashMap::new(),
            module_type_index: HashMap::new(),
        }
    }

    /// Add a directory to search for plugins
    pub fn add_search_path<P: AsRef<Path>>(&mut self, path: P) {
        self.search_paths.push(path.as_ref().to_path_buf());
    }

    /// Get all search paths
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Discover and load all plugins from search paths
    ///
    /// This scans all configured search paths for dynamic libraries matching
    /// the platform's naming convention (lib*.so, lib*.dylib, *.dll).
    pub fn discover_plugins(&mut self) -> Result<Vec<String>, PluginLoadError> {
        let mut loaded = Vec::new();

        for search_path in &self.search_paths.clone() {
            if !search_path.exists() {
                continue;
            }

            let entries = std::fs::read_dir(search_path).map_err(|e| {
                PluginLoadError::LoadFailed(format!(
                    "Failed to read directory {}: {}",
                    search_path.display(),
                    e
                ))
            })?;

            for entry in entries.flatten() {
                let path = entry.path();
                if Self::is_plugin_library(&path) {
                    match self.load_plugin(&path) {
                        Ok(plugin_id) => loaded.push(plugin_id),
                        Err(e) => {
                            // Log but continue with other plugins
                            tracing::warn!("Failed to load plugin {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        Ok(loaded)
    }

    /// Check if a path looks like a plugin library
    fn is_plugin_library(path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        #[cfg(target_os = "macos")]
        {
            extension == "dylib"
        }
        #[cfg(target_os = "linux")]
        {
            extension == "so"
        }
        #[cfg(target_os = "windows")]
        {
            extension == "dll"
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            false
        }
    }

    /// Load a plugin from a specific path
    pub fn load_plugin<P: AsRef<Path>>(&mut self, path: P) -> Result<String, PluginLoadError> {
        let path = path.as_ref();

        // Load the library and get the root module
        let header = lib_header_from_path(path).map_err(|e| {
            PluginLoadError::LoadFailed(format!("Failed to load library header: {}", e))
        })?;

        // Check if this is a valid daq_plugin library
        let plugin_ref = header.init_root_module::<PluginMod_Ref>().map_err(|e| {
            PluginLoadError::LoadFailed(format!("Failed to init root module: {}", e))
        })?;

        // Check ABI compatibility
        let plugin_abi = plugin_ref.abi_version()();
        if !plugin_abi.is_compatible_with(&AbiVersion::CURRENT) {
            return Err(PluginLoadError::IncompatibleAbi {
                plugin_version: plugin_abi,
                host_version: AbiVersion::CURRENT,
            });
        }

        // Get metadata
        let metadata = plugin_ref.get_metadata()();
        let plugin_id = metadata.plugin_id.to_string();

        // Index module types
        for type_info in plugin_ref.list_module_types()().iter() {
            self.module_type_index
                .insert(type_info.type_id.to_string(), plugin_id.clone());
        }

        // Store the loaded plugin
        let loaded = LoadedPlugin {
            metadata,
            plugin_ref,
            path: path.to_path_buf(),
        };

        self.plugins.insert(plugin_id.clone(), loaded);

        tracing::info!("Loaded plugin: {} from {:?}", plugin_id, path);

        Ok(plugin_id)
    }

    /// Get a loaded plugin by ID
    pub fn get_plugin(&self, plugin_id: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin_id)
    }

    /// List all loaded plugins
    pub fn list_plugins(&self) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins.values()
    }

    /// List all available module types across all plugins
    pub fn list_module_types(&self) -> Vec<(String, FfiModuleTypeInfo)> {
        let mut types = Vec::new();
        for (plugin_id, plugin) in &self.plugins {
            for type_info in plugin.module_types().iter() {
                types.push((plugin_id.clone(), type_info.clone()));
            }
        }
        types
    }

    /// Find which plugin provides a given module type
    pub fn find_plugin_for_type(&self, type_id: &str) -> Option<&str> {
        self.module_type_index.get(type_id).map(|s| s.as_str())
    }

    /// Create a module instance by type ID
    ///
    /// This looks up which plugin provides the type and delegates to it.
    pub fn create_module(&self, type_id: &str) -> Result<ModuleFfiBox, String> {
        let plugin_id = self
            .find_plugin_for_type(type_id)
            .ok_or_else(|| format!("No plugin provides module type: {}", type_id))?;

        let plugin = self
            .get_plugin(plugin_id)
            .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?;

        plugin.create_module(type_id)
    }

    /// Unload a plugin by ID
    ///
    /// Note: Due to limitations in abi_stable, plugins cannot be fully unloaded
    /// as they are leaked to prevent use-after-free. This removes the plugin
    /// from the registry but the library remains in memory.
    pub fn unload_plugin(&mut self, plugin_id: &str) -> bool {
        if let Some(plugin) = self.plugins.remove(plugin_id) {
            // Remove module type index entries
            let types_to_remove: Vec<_> = self
                .module_type_index
                .iter()
                .filter(|(_, pid)| *pid == plugin_id)
                .map(|(tid, _)| tid.clone())
                .collect();

            for tid in types_to_remove {
                self.module_type_index.remove(&tid);
            }

            tracing::info!(
                "Unloaded plugin: {} (library remains in memory)",
                plugin.metadata.plugin_id
            );
            true
        } else {
            false
        }
    }
}
