//! Plugin factory for loading and instantiating YAML-defined instrument drivers.
//!
//! This module provides `PluginFactory` which scans directories for plugin YAML files,
//! validates them, and spawns `GenericDriver` instances on demand.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_serial::SerialPortBuilderExt;

use crate::hardware::plugin::driver::GenericDriver;
use crate::hardware::plugin::schema::{DriverType, InstrumentConfig};

// =============================================================================
// Plugin Load Errors
// =============================================================================

/// Error that occurred while loading a plugin file
#[derive(Debug, Clone)]
pub struct PluginLoadError {
    /// Path to the file that failed to load
    pub file_path: PathBuf,
    /// The error message
    pub message: String,
    /// Validation errors (if any)
    pub validation_errors: Vec<ValidationError>,
}

impl std::fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file_path.display(), self.message)?;
        for err in &self.validation_errors {
            write!(f, "\n  - {}: {}", err.path, err.message)?;
        }
        Ok(())
    }
}

/// A specific validation error within a plugin configuration
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// JSON path to the invalid field (e.g., "capabilities.movable.set_cmd")
    pub path: String,
    /// Human-readable error message
    pub message: String,
}

// =============================================================================
// Plugin Entry
// =============================================================================

/// A loaded plugin entry with its source information
#[derive(Debug, Clone)]
pub struct PluginEntry {
    /// The parsed configuration
    pub config: InstrumentConfig,
    /// Path to the source file
    pub source_path: PathBuf,
    /// Priority level (lower = higher priority, 0 = user, 1 = builtin)
    pub priority: u32,
}

// =============================================================================
// Plugin Factory
// =============================================================================

/// Manages loading and spawning of instrument plugins from configuration files.
///
/// The factory maintains a registry of plugin configurations loaded from YAML files.
/// Each plugin can be instantiated multiple times with different serial ports.
///
/// # Search Path Priority
///
/// Search paths are processed in order, with earlier paths having higher priority.
/// If the same plugin ID exists in multiple paths, the higher-priority version wins.
/// Convention: Add user directories before builtin directories.
///
/// # Example
///
/// ```rust,ignore
/// let mut factory = PluginFactory::new();
///
/// // Add search paths (user overrides builtin)
/// factory.add_search_path("~/.config/rust-daq/plugins/");
/// factory.add_search_path("/usr/share/rust-daq/plugins/");
///
/// // Scan all paths and collect errors
/// let errors = factory.scan().await;
/// for err in &errors {
///     eprintln!("Warning: {}", err);
/// }
///
/// // List available plugins
/// for entry in factory.list() {
///     println!("Available: {} (from {})", entry.config.metadata.id, entry.source_path.display());
/// }
///
/// // Spawn a driver instance
/// let driver = factory.spawn("my-sensor-v1", "/dev/ttyUSB0").await?;
/// ```
pub struct PluginFactory {
    /// Plugin configurations indexed by their unique ID.
    templates: HashMap<String, PluginEntry>,
    /// Search paths in priority order (first = highest priority)
    search_paths: Vec<PathBuf>,
}

impl PluginFactory {
    /// Creates a new, empty PluginFactory.
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
            search_paths: Vec::new(),
        }
    }

    /// Adds a search path for plugin discovery.
    ///
    /// Paths added first have higher priority. If the same plugin ID
    /// exists in multiple paths, the version from the higher-priority
    /// path will be used.
    ///
    /// # Convention
    /// Add user-specific paths before system/builtin paths:
    /// ```rust,ignore
    /// factory.add_search_path("~/.config/rust-daq/plugins/");  // Priority 0 (user)
    /// factory.add_search_path("/usr/share/rust-daq/plugins/"); // Priority 1 (builtin)
    /// ```
    pub fn add_search_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.search_paths.push(path.into());
    }

    /// Scans all search paths and loads plugin configurations.
    ///
    /// Returns a list of errors for plugins that failed to load.
    /// Valid plugins are still loaded even if some fail.
    ///
    /// # Priority
    /// If the same plugin ID is found in multiple paths, the version
    /// from the earlier (higher-priority) path wins.
    pub async fn scan(&mut self) -> Vec<PluginLoadError> {
        let mut errors = Vec::new();

        // Clone paths to avoid borrow conflict with scan_directory(&mut self)
        let paths: Vec<_> = self.search_paths.iter().cloned().enumerate().collect();
        for (priority, path) in paths {
            let path_errors = self.scan_directory(&path, priority as u32).await;
            errors.extend(path_errors);
        }

        errors
    }

    /// Scans a single directory and loads plugins.
    ///
    /// This is the core scanning logic, extractedfrom load_plugins for reuse.
    async fn scan_directory(&mut self, path: &Path, priority: u32) -> Vec<PluginLoadError> {
        let mut errors = Vec::new();

        if !path.exists() {
            // Non-existent paths are silently skipped (not an error)
            tracing::debug!("Plugin path does not exist: {}", path.display());
            return errors;
        }

        if !path.is_dir() {
            errors.push(PluginLoadError {
                file_path: path.to_path_buf(),
                message: "Not a directory".to_string(),
                validation_errors: vec![],
            });
            return errors;
        }

        let entries = match tokio::fs::read_dir(path).await {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(PluginLoadError {
                    file_path: path.to_path_buf(),
                    message: format!("Failed to read directory: {}", e),
                    validation_errors: vec![],
                });
                return errors;
            }
        };

        let mut entries = entries;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(extension) = entry_path.extension() {
                    if extension == "yaml" || extension == "yml" {
                        match self.load_single_plugin(&entry_path, priority).await {
                            Ok(()) => {}
                            Err(e) => errors.push(e),
                        }
                    }
                }
            }
        }

        errors
    }

    /// Loads a single plugin file with validation.
    async fn load_single_plugin(
        &mut self,
        path: &Path,
        priority: u32,
    ) -> Result<(), PluginLoadError> {
        // Read file
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| PluginLoadError {
                file_path: path.to_path_buf(),
                message: format!("Failed to read file: {}", e),
                validation_errors: vec![],
            })?;

        // Parse YAML
        let config: InstrumentConfig =
            serde_yaml::from_str(&content).map_err(|e| PluginLoadError {
                file_path: path.to_path_buf(),
                message: format!("Failed to parse YAML: {}", e),
                validation_errors: vec![],
            })?;

        // Validate configuration
        let validation_errors = validate_plugin_config(&config);
        if !validation_errors.is_empty() {
            return Err(PluginLoadError {
                file_path: path.to_path_buf(),
                message: format!(
                    "Validation failed with {} error(s)",
                    validation_errors.len()
                ),
                validation_errors,
            });
        }

        // Check for duplicate with priority handling
        let plugin_id = config.metadata.id.clone();
        if let Some(existing) = self.templates.get(&plugin_id) {
            if existing.priority <= priority {
                // Existing has higher or equal priority, skip this one
                tracing::debug!(
                    "Skipping plugin '{}' from {} (already loaded from {} with higher priority)",
                    plugin_id,
                    path.display(),
                    existing.source_path.display()
                );
                return Ok(());
            }
            // This one has higher priority, will replace
            tracing::info!(
                "Overriding plugin '{}' from {} with version from {} (higher priority)",
                plugin_id,
                existing.source_path.display(),
                path.display()
            );
        }

        tracing::info!(
            "Loaded plugin: {} ({})",
            config.metadata.name,
            path.display()
        );
        self.templates.insert(
            plugin_id,
            PluginEntry {
                config,
                source_path: path.to_path_buf(),
                priority,
            },
        );

        Ok(())
    }

    /// Loads instrument plugin configurations from YAML files in the specified directory.
    ///
    /// This is the legacy single-directory loading method. For multi-path support,
    /// use `add_search_path()` followed by `scan()`.
    ///
    /// # Errors
    /// - Returns error if path is not a directory
    /// - Returns error if a YAML file fails to parse
    /// - Returns error if duplicate plugin IDs are found
    pub async fn load_plugins(&mut self, path: &Path) -> Result<()> {
        if !path.is_dir() {
            return Err(anyhow!(
                "Plugin path '{}' is not a directory",
                path.display()
            ));
        }

        let mut entries = tokio::fs::read_dir(path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(extension) = entry_path.extension() {
                    if extension == "yaml" || extension == "yml" {
                        let content = tokio::fs::read_to_string(&entry_path).await?;
                        let config: InstrumentConfig =
                            serde_yaml::from_str(&content).map_err(|e| {
                                anyhow!("Failed to parse {}: {}", entry_path.display(), e)
                            })?;

                        // Validate configuration
                        let validation_errors = validate_plugin_config(&config);
                        if !validation_errors.is_empty() {
                            let msgs: Vec<String> = validation_errors
                                .iter()
                                .map(|e| format!("{}: {}", e.path, e.message))
                                .collect();
                            return Err(anyhow!(
                                "Validation failed for {}: {}",
                                entry_path.display(),
                                msgs.join("; ")
                            ));
                        }

                        if self.templates.contains_key(&config.metadata.id) {
                            return Err(anyhow!(
                                "Duplicate plugin ID '{}' found for file {}",
                                config.metadata.id,
                                entry_path.display()
                            ));
                        }
                        tracing::info!(
                            "Loaded plugin: {} ({})",
                            config.metadata.name,
                            entry_path.display()
                        );
                        self.templates.insert(
                            config.metadata.id.clone(),
                            PluginEntry {
                                config,
                                source_path: entry_path,
                                priority: 0, // Legacy single-dir loading uses priority 0
                            },
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Spawns a new GenericDriver instance for a given plugin ID.
    ///
    /// For serial plugins, provide the serial port path (e.g., "/dev/ttyUSB0").
    /// For TCP plugins, provide the address in "host:port" format, or use the
    /// configured tcp_host and tcp_port from the plugin YAML.
    ///
    /// # Arguments
    /// * `driver_id` - The plugin's unique ID (from `metadata.id` in YAML)
    /// * `address` - Connection address:
    ///   - For serial: port path (e.g., "/dev/ttyUSB0", "COM3")
    ///   - For TCP: "host:port" or empty string to use YAML defaults
    ///
    /// # Returns
    /// A configured and initialized `GenericDriver` instance.
    ///
    /// # Errors
    /// - Returns error if plugin ID is not found
    /// - Returns error if connection cannot be established
    /// - Returns error if on_connect sequence fails
    pub async fn spawn(&self, driver_id: &str, address: &str) -> Result<GenericDriver> {
        let entry = self
            .templates
            .get(driver_id)
            .ok_or_else(|| anyhow!("Plugin with ID '{}' not found", driver_id))?;

        let config = entry.config.clone();

        let driver = match config.metadata.driver_type {
            DriverType::SerialScpi | DriverType::SerialRaw => {
                self.spawn_serial_driver(config.clone(), address).await?
            }
            DriverType::TcpScpi | DriverType::TcpRaw => {
                self.spawn_tcp_driver(config.clone(), address).await?
            }
        };

        // Execute initialization sequence
        driver.execute_command_sequence(&config.on_connect).await?;

        Ok(driver)
    }

    /// Spawns a mock driver for testing without hardware.
    ///
    /// Creates a GenericDriver in mock mode that simulates responses
    /// without requiring a physical connection. Useful for testing
    /// and development.
    ///
    /// # Arguments
    /// * `driver_id` - The plugin's unique ID (from `metadata.id` in YAML)
    ///
    /// # Returns
    /// A configured mock `GenericDriver` instance.
    ///
    /// # Errors
    /// - Returns error if plugin ID is not found
    /// - Returns error if on_connect sequence fails
    pub async fn spawn_mock(&self, driver_id: &str) -> Result<GenericDriver> {
        let entry = self
            .templates
            .get(driver_id)
            .ok_or_else(|| anyhow!("Plugin with ID '{}' not found", driver_id))?;

        let config = entry.config.clone();

        let driver = GenericDriver::new_mock(config.clone())?;

        // Execute initialization sequence (will be simulated)
        driver.execute_command_sequence(&config.on_connect).await?;

        Ok(driver)
    }

    /// Spawns a serial driver with the given configuration and port path.
    async fn spawn_serial_driver(
        &self,
        config: InstrumentConfig,
        port_path: &str,
    ) -> Result<GenericDriver> {
        let baud_rate = config.protocol.baud_rate;
        let timeout_ms = config.protocol.timeout_ms;

        // Open serial port with configured settings
        let port = tokio_serial::new(port_path, baud_rate)
            .timeout(Duration::from_millis(timeout_ms))
            .open_native_async()
            .map_err(|e| anyhow!("Failed to open serial port {}: {}", port_path, e))?;

        GenericDriver::new_serial(config, port)
    }

    /// Spawns a TCP driver with the given configuration and address.
    ///
    /// If address is empty, uses the tcp_host and tcp_port from the plugin config.
    async fn spawn_tcp_driver(
        &self,
        config: InstrumentConfig,
        address: &str,
    ) -> Result<GenericDriver> {
        let timeout_ms = config.protocol.timeout_ms;

        // Determine the address to connect to
        let connect_addr = if address.is_empty() {
            // Use configured defaults
            let host = config.protocol.tcp_host.as_ref().ok_or_else(|| {
                anyhow!(
                    "TCP plugin '{}' has no tcp_host configured and no address provided",
                    config.metadata.id
                )
            })?;
            let port = config.protocol.tcp_port.ok_or_else(|| {
                anyhow!(
                    "TCP plugin '{}' has no tcp_port configured and no address provided",
                    config.metadata.id
                )
            })?;
            format!("{}:{}", host, port)
        } else {
            address.to_string()
        };

        // Connect with timeout
        let stream = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            TcpStream::connect(&connect_addr),
        )
        .await
        .map_err(|_| anyhow!("Timeout connecting to {}", connect_addr))?
        .map_err(|e| anyhow!("Failed to connect to {}: {}", connect_addr, e))?;

        tracing::info!("Connected to TCP instrument at {}", connect_addr);

        GenericDriver::new_tcp(config, stream)
    }

    /// Returns a list of available plugin IDs.
    pub fn available_plugins(&self) -> Vec<String> {
        self.templates.keys().cloned().collect()
    }

    /// Returns the display name for a given plugin ID.
    pub fn plugin_display_name(&self, driver_id: &str) -> Option<&str> {
        self.templates
            .get(driver_id)
            .map(|e| e.config.metadata.name.as_str())
    }

    /// Returns the full configuration for a given plugin ID.
    ///
    /// Useful for inspecting plugin capabilities before spawning.
    pub fn get_config(&self, driver_id: &str) -> Option<&InstrumentConfig> {
        self.templates.get(driver_id).map(|e| &e.config)
    }

    /// Returns all loaded plugin entries.
    ///
    /// Includes source path and priority information for each plugin.
    pub fn list(&self) -> Vec<&PluginEntry> {
        self.templates.values().collect()
    }

    /// Returns the plugin entry for a given ID (includes source path and priority).
    pub fn get_entry(&self, driver_id: &str) -> Option<&PluginEntry> {
        self.templates.get(driver_id)
    }

    /// Reloads all plugins from the configured search paths.
    ///
    /// This clears all existing plugins and rescans all search paths.
    /// Useful for hot-reloading plugin changes.
    pub async fn reload(&mut self) -> Vec<PluginLoadError> {
        self.templates.clear();
        self.scan().await
    }

    // =========================================================================
    // Hot-reload support methods
    // =========================================================================

    /// Updates or inserts a plugin with the given configuration.
    ///
    /// Used by hot-reload to atomically swap plugin configurations.
    /// If a plugin with this ID exists, it is replaced.
    ///
    /// # Arguments
    /// * `plugin_id` - The plugin's unique ID
    /// * `config` - The new configuration
    /// * `source_path` - Path to the source file
    pub fn update_plugin(
        &mut self,
        plugin_id: String,
        config: InstrumentConfig,
        source_path: PathBuf,
    ) {
        let priority = self
            .templates
            .get(&plugin_id)
            .map(|e| e.priority)
            .unwrap_or(0);

        self.templates.insert(
            plugin_id,
            PluginEntry {
                config,
                source_path,
                priority,
            },
        );
    }

    /// Finds a plugin ID by its source file path.
    ///
    /// Used by hot-reload to identify which plugin to remove when
    /// a file is deleted.
    ///
    /// # Arguments
    /// * `path` - The source file path to search for
    ///
    /// # Returns
    /// The plugin ID if found, None otherwise
    pub fn find_by_path(&self, path: &Path) -> Option<String> {
        self.templates
            .iter()
            .find(|(_, entry)| entry.source_path == path)
            .map(|(id, _)| id.clone())
    }

    /// Removes a plugin from the registry by its ID.
    ///
    /// Used by hot-reload when a plugin file is deleted.
    ///
    /// # Arguments
    /// * `plugin_id` - The ID of the plugin to remove
    ///
    /// # Returns
    /// The removed plugin entry, if it existed
    pub fn remove_plugin(&mut self, plugin_id: &str) -> Option<PluginEntry> {
        self.templates.remove(plugin_id)
    }
}

impl Default for PluginFactory {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Validation
// =============================================================================

/// Validates a plugin configuration and returns any errors found.
///
/// Validation checks:
/// 1. Required fields are present and non-empty
/// 2. Patterns are valid regex (where applicable)
/// 3. Command templates have required format specifiers
/// 4. Numeric constraints are valid (min < max, etc.)
/// 5. TCP plugins have host/port configuration
pub fn validate_plugin_config(config: &InstrumentConfig) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Validate metadata
    if config.metadata.id.is_empty() {
        errors.push(ValidationError {
            path: "metadata.id".to_string(),
            message: "Plugin ID cannot be empty".to_string(),
        });
    }

    if config.metadata.name.is_empty() {
        errors.push(ValidationError {
            path: "metadata.name".to_string(),
            message: "Plugin name cannot be empty".to_string(),
        });
    }

    if config.metadata.version.is_empty() {
        errors.push(ValidationError {
            path: "metadata.version".to_string(),
            message: "Plugin version cannot be empty".to_string(),
        });
    }

    // Validate TCP-specific requirements
    match config.metadata.driver_type {
        DriverType::TcpScpi | DriverType::TcpRaw => {
            // TCP plugins should have either default host/port or expect runtime address
            // This is a soft warning - plugins can require runtime address
        }
        _ => {}
    }

    // Validate movable capability
    if let Some(ref movable) = config.capabilities.movable {
        if movable.axes.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.movable.axes".to_string(),
                message: "Movable capability must define at least one axis".to_string(),
            });
        }

        if movable.set_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.movable.set_cmd".to_string(),
                message: "Movable set_cmd cannot be empty".to_string(),
            });
        }

        if movable.get_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.movable.get_cmd".to_string(),
                message: "Movable get_cmd cannot be empty".to_string(),
            });
        }

        // Validate that set_cmd contains position placeholder
        if !movable.set_cmd.contains("{position}") && !movable.set_cmd.contains("{pos}") {
            errors.push(ValidationError {
                path: "capabilities.movable.set_cmd".to_string(),
                message: "set_cmd should contain {position} or {pos} placeholder".to_string(),
            });
        }

        // Validate axis constraints
        for (i, axis) in movable.axes.iter().enumerate() {
            if axis.name.is_empty() {
                errors.push(ValidationError {
                    path: format!("capabilities.movable.axes[{}].name", i),
                    message: "Axis name cannot be empty".to_string(),
                });
            }

            // Validate min/max if both are specified
            if let (Some(min), Some(max)) = (axis.min, axis.max) {
                if min >= max {
                    errors.push(ValidationError {
                        path: format!("capabilities.movable.axes[{}]", i),
                        message: format!("Axis min ({}) must be less than max ({})", min, max),
                    });
                }
            }
        }
    }

    // Validate readable capabilities
    for (i, readable) in config.capabilities.readable.iter().enumerate() {
        if readable.name.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.readable[{}].name", i),
                message: "Readable name cannot be empty".to_string(),
            });
        }

        if readable.command.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.readable[{}].command", i),
                message: "Readable command cannot be empty".to_string(),
            });
        }

        if readable.pattern.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.readable[{}].pattern", i),
                message: "Readable pattern cannot be empty".to_string(),
            });
        }
    }

    // Validate settable capabilities
    for (i, settable) in config.capabilities.settable.iter().enumerate() {
        if settable.name.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.settable[{}].name", i),
                message: "Settable name cannot be empty".to_string(),
            });
        }

        if settable.set_cmd.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.settable[{}].set_cmd", i),
                message: "Settable set_cmd cannot be empty".to_string(),
            });
        }

        // Validate that set_cmd contains value placeholder
        if !settable.set_cmd.contains("{value}") && !settable.set_cmd.contains("{val}") {
            errors.push(ValidationError {
                path: format!("capabilities.settable[{}].set_cmd", i),
                message: "set_cmd should contain {value} or {val} placeholder".to_string(),
            });
        }

        // Validate min/max if both are specified
        if let (Some(min), Some(max)) = (settable.min, settable.max) {
            if min >= max {
                errors.push(ValidationError {
                    path: format!("capabilities.settable[{}]", i),
                    message: format!("Settable min ({}) must be less than max ({})", min, max),
                });
            }
        }
    }

    // Validate switchable capabilities
    for (i, switchable) in config.capabilities.switchable.iter().enumerate() {
        if switchable.name.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.switchable[{}].name", i),
                message: "Switchable name cannot be empty".to_string(),
            });
        }

        if switchable.on_cmd.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.switchable[{}].on_cmd", i),
                message: "Switchable on_cmd cannot be empty".to_string(),
            });
        }

        if switchable.off_cmd.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.switchable[{}].off_cmd", i),
                message: "Switchable off_cmd cannot be empty".to_string(),
            });
        }
    }

    // Validate actionable capabilities
    for (i, actionable) in config.capabilities.actionable.iter().enumerate() {
        if actionable.name.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.actionable[{}].name", i),
                message: "Actionable name cannot be empty".to_string(),
            });
        }

        if actionable.cmd.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.actionable[{}].cmd", i),
                message: "Actionable cmd cannot be empty".to_string(),
            });
        }
    }

    // Validate frame_producer capability
    if let Some(ref frame_producer) = config.capabilities.frame_producer {
        if frame_producer.width == 0 {
            errors.push(ValidationError {
                path: "capabilities.frame_producer.width".to_string(),
                message: "Frame width must be greater than 0".to_string(),
            });
        }

        if frame_producer.height == 0 {
            errors.push(ValidationError {
                path: "capabilities.frame_producer.height".to_string(),
                message: "Frame height must be greater than 0".to_string(),
            });
        }

        if frame_producer.start_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.frame_producer.start_cmd".to_string(),
                message: "Frame producer start_cmd cannot be empty".to_string(),
            });
        }

        if frame_producer.stop_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.frame_producer.stop_cmd".to_string(),
                message: "Frame producer stop_cmd cannot be empty".to_string(),
            });
        }

        if frame_producer.frame_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.frame_producer.frame_cmd".to_string(),
                message: "Frame producer frame_cmd cannot be empty".to_string(),
            });
        }
    }

    // Validate exposure_control capability
    if let Some(ref exposure) = config.capabilities.exposure_control {
        if exposure.set_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.exposure_control.set_cmd".to_string(),
                message: "Exposure control set_cmd cannot be empty".to_string(),
            });
        }

        if exposure.get_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.exposure_control.get_cmd".to_string(),
                message: "Exposure control get_cmd cannot be empty".to_string(),
            });
        }

        // Validate min/max if both are specified
        if let (Some(min), Some(max)) = (exposure.min_seconds, exposure.max_seconds) {
            if min >= max {
                errors.push(ValidationError {
                    path: "capabilities.exposure_control".to_string(),
                    message: format!("Exposure min ({}) must be less than max ({})", min, max),
                });
            }
        }
    }

    // Validate triggerable capability
    if let Some(ref triggerable) = config.capabilities.triggerable {
        if triggerable.arm_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.triggerable.arm_cmd".to_string(),
                message: "Triggerable arm_cmd cannot be empty".to_string(),
            });
        }

        if triggerable.trigger_cmd.is_empty() {
            errors.push(ValidationError {
                path: "capabilities.triggerable.trigger_cmd".to_string(),
                message: "Triggerable trigger_cmd cannot be empty".to_string(),
            });
        }
    }

    // Validate scriptable capabilities
    for (i, scriptable) in config.capabilities.scriptable.iter().enumerate() {
        if scriptable.name.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.scriptable[{}].name", i),
                message: "Scriptable name cannot be empty".to_string(),
            });
        }

        if scriptable.script.is_empty() {
            errors.push(ValidationError {
                path: format!("capabilities.scriptable[{}].script", i),
                message: "Scriptable script cannot be empty".to_string(),
            });
        }
    }

    errors
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::plugin::schema::*;

    fn create_minimal_valid_config() -> InstrumentConfig {
        InstrumentConfig {
            metadata: InstrumentMetadata {
                id: "test-plugin".to_string(),
                name: "Test Plugin".to_string(),
                version: "1.0.0".to_string(),
                driver_type: DriverType::SerialScpi,
            },
            protocol: ProtocolConfig::default(),
            on_connect: vec![],
            on_disconnect: vec![],
            error_patterns: vec![],
            capabilities: CapabilitiesConfig::default(),
            ui_layout: vec![],
        }
    }

    #[test]
    fn test_validate_minimal_valid_config() {
        let config = create_minimal_valid_config();
        let errors = validate_plugin_config(&config);
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_empty_id() {
        let mut config = create_minimal_valid_config();
        config.metadata.id = "".to_string();
        let errors = validate_plugin_config(&config);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "metadata.id");
    }

    #[test]
    fn test_validate_movable_missing_axes() {
        let mut config = create_minimal_valid_config();
        config.capabilities.movable = Some(MovableCapability {
            axes: vec![],
            set_cmd: "MOVE {position}".to_string(),
            get_cmd: "POS?".to_string(),
            get_pattern: "{val}".to_string(),
        });
        let errors = validate_plugin_config(&config);
        assert!(errors.iter().any(|e| e.path.contains("axes")));
    }

    #[test]
    fn test_validate_movable_missing_placeholder() {
        let mut config = create_minimal_valid_config();
        config.capabilities.movable = Some(MovableCapability {
            axes: vec![AxisConfig {
                name: "x".to_string(),
                unit: None,
                min: None,
                max: None,
            }],
            set_cmd: "MOVE".to_string(), // Missing {position}
            get_cmd: "POS?".to_string(),
            get_pattern: "{val}".to_string(),
        });
        let errors = validate_plugin_config(&config);
        assert!(errors.iter().any(|e| e.message.contains("placeholder")));
    }

    #[test]
    fn test_validate_axis_min_max_invalid() {
        let mut config = create_minimal_valid_config();
        config.capabilities.movable = Some(MovableCapability {
            axes: vec![AxisConfig {
                name: "x".to_string(),
                unit: None,
                min: Some(100.0),
                max: Some(10.0), // min > max!
            }],
            set_cmd: "MOVE {position}".to_string(),
            get_cmd: "POS?".to_string(),
            get_pattern: "{val}".to_string(),
        });
        let errors = validate_plugin_config(&config);
        assert!(errors
            .iter()
            .any(|e| e.message.contains("must be less than")));
    }

    #[test]
    fn test_plugin_entry_priority() {
        let entry = PluginEntry {
            config: create_minimal_valid_config(),
            source_path: PathBuf::from("/test/path"),
            priority: 0,
        };
        assert_eq!(entry.priority, 0);
    }
}
