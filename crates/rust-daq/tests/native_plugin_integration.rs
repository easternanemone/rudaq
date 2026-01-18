//! Integration tests for native (abi_stable) plugin loading.
//!
//! These tests verify the dynamic plugin loading system using the daq-plugin-example
//! as a test fixture.

#![cfg(all(not(target_arch = "wasm32"), feature = "native_plugins"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use anyhow::Result;
use rust_daq::hardware::registry::DeviceRegistry;
use rust_daq::modules::ModuleRegistry;
use rust_daq::plugins::PluginManager;
use std::path::PathBuf;
use std::sync::Arc;

/// Get the path to the built plugin library.
fn plugin_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates
    path.pop(); // rust_daq
    path.push("target");
    path.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });

    #[cfg(target_os = "macos")]
    path.push("libdaq_plugin_example.dylib");
    #[cfg(target_os = "linux")]
    path.push("libdaq_plugin_example.so");
    #[cfg(target_os = "windows")]
    path.push("daq_plugin_example.dll");

    path
}

/// Get the plugin directory (parent of the plugin file).
fn plugin_dir() -> PathBuf {
    let mut path = plugin_path();
    path.pop();
    path
}

#[test]
fn test_plugin_manager_creation() {
    let manager = PluginManager::new();
    assert!(manager.search_paths().is_empty());
}

#[test]
fn test_add_search_path() {
    let mut manager = PluginManager::new();
    manager.add_search_path("./plugins");
    assert_eq!(manager.search_paths().len(), 1);
}

#[test]
fn test_load_example_plugin() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!(
            "Skipping test: plugin not found at {:?}. Run `cargo build -p daq-plugin-example` first.",
            path
        );
        return Ok(());
    }

    let mut manager = PluginManager::new();
    let plugin_id = manager.load_plugin(&path)?;

    assert_eq!(plugin_id, "example-plugin");

    // Verify plugin is accessible
    let plugin = manager.get_plugin(&plugin_id).expect("Plugin should exist");
    assert_eq!(plugin.metadata.plugin_id.as_str(), "example-plugin");
    assert_eq!(plugin.metadata.name.as_str(), "Example Plugin");

    Ok(())
}

#[test]
fn test_list_module_types() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut manager = PluginManager::new();
    manager.load_plugin(&path)?;

    let module_types = manager.list_module_types();
    assert_eq!(module_types.len(), 1);

    let (plugin_id, type_info) = &module_types[0];
    assert_eq!(plugin_id, "example-plugin");
    assert_eq!(type_info.type_id.as_str(), "echo_module");
    assert_eq!(type_info.display_name.as_str(), "Echo Module");

    Ok(())
}

#[test]
fn test_create_module_instance() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut manager = PluginManager::new();
    manager.load_plugin(&path)?;

    // Create module via plugin manager
    let module = manager
        .create_module("echo_module")
        .map_err(|e| anyhow::anyhow!("Failed to create module: {}", e))?;
    assert_eq!(module.type_id().as_str(), "echo_module");

    Ok(())
}

#[test]
fn test_find_plugin_for_type() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut manager = PluginManager::new();
    manager.load_plugin(&path)?;

    let plugin_id = manager.find_plugin_for_type("echo_module");
    assert_eq!(plugin_id, Some("example-plugin"));

    let unknown = manager.find_plugin_for_type("unknown_type");
    assert_eq!(unknown, None);

    Ok(())
}

#[test]
fn test_discover_plugins() -> Result<()> {
    let dir = plugin_dir();
    if !plugin_path().exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut manager = PluginManager::new();
    manager.add_search_path(&dir);
    let loaded = manager.discover_plugins()?;

    assert!(
        loaded.contains(&"example-plugin".to_string()),
        "Should discover example-plugin"
    );

    Ok(())
}

#[test]
fn test_register_plugin_types_in_registry() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugin(&path)?;

    let device_registry = Arc::new(DeviceRegistry::new());
    let mut module_registry = ModuleRegistry::new(device_registry);

    // Register plugin types
    let count = module_registry.register_plugin_types(&plugin_manager);
    assert_eq!(count, 1, "Should register one module type from plugin");

    // Verify type is available
    let types = module_registry.list_types();
    let has_echo = types.iter().any(|t| t.type_id == "echo_module");
    assert!(has_echo, "echo_module should be registered");

    Ok(())
}

#[test]
fn test_create_plugin_module_via_registry() -> Result<()> {
    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugin(&path)?;

    let device_registry = Arc::new(DeviceRegistry::new());
    let mut module_registry = ModuleRegistry::new(device_registry);
    module_registry.register_plugin_types(&plugin_manager);

    // Create module instance through registry
    let module_id =
        module_registry.create_plugin_module("echo_module", "Test Echo", &plugin_manager)?;

    // Verify instance was created
    let instance = module_registry
        .get_module(&module_id)
        .expect("Should exist");
    assert_eq!(instance.name, "Test Echo");
    assert_eq!(instance.type_id(), "echo_module");

    Ok(())
}

#[tokio::test]
async fn test_plugin_module_lifecycle() -> Result<()> {
    use daq_core::modules::ModuleState;
    use std::collections::HashMap;

    let path = plugin_path();
    if !path.exists() {
        eprintln!("Skipping test: plugin not found");
        return Ok(());
    }

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugin(&path)?;

    let device_registry = Arc::new(DeviceRegistry::new());
    let mut module_registry = ModuleRegistry::new(device_registry);
    module_registry.register_plugin_types(&plugin_manager);

    let module_id =
        module_registry.create_plugin_module("echo_module", "Lifecycle Test", &plugin_manager)?;

    // Verify initial state
    {
        let instance = module_registry.get_module(&module_id).unwrap();
        assert_eq!(instance.state(), ModuleState::Created);
    }

    // Configure
    let mut config = HashMap::new();
    config.insert("message".to_string(), "Hello Test!".to_string());
    config.insert("echo_count".to_string(), "5".to_string());
    let warnings = module_registry.configure_module(&module_id, config)?;
    assert!(warnings.is_empty(), "No config warnings expected");

    // Verify configured state
    {
        let instance = module_registry.get_module(&module_id).unwrap();
        assert_eq!(instance.state(), ModuleState::Configured);
    }

    // Verify config was applied
    {
        let instance = module_registry.get_module(&module_id).unwrap();
        let config = instance.get_config();
        assert_eq!(config.get("message"), Some(&"Hello Test!".to_string()));
        assert_eq!(config.get("echo_count"), Some(&"5".to_string()));
    }

    Ok(())
}
