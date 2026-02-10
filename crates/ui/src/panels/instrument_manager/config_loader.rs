// Internal consumer of the deprecated DeviceConfig schema for UI rendering.
#![allow(deprecated)]
//! Device configuration loader for UI rendering
//!
//! Loads device TOML configurations and provides access to UiConfig

use anyhow::{Context, Result};
use hardware::config::schema::DeviceConfig;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Cache of loaded device configurations
pub struct DeviceConfigCache {
    /// Map from protocol name (e.g., "elliptec", "maitai") to DeviceConfig
    configs: HashMap<String, DeviceConfig>,
    /// Directory where device configs are stored
    config_dir: PathBuf,
    /// Whether load_all has been attempted (prevents per-frame retry)
    load_attempted: bool,
}

impl Default for DeviceConfigCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceConfigCache {
    /// Create a new config cache
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            config_dir: PathBuf::from("config/devices"),
            load_attempted: false,
        }
    }

    /// Whether a load has been attempted (successful or not)
    pub fn load_attempted(&self) -> bool {
        self.load_attempted
    }

    /// Set the configuration directory
    #[allow(dead_code)]
    pub fn with_config_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config_dir = dir.into();
        self
    }

    /// Load all device configs from the config directory
    pub fn load_all(&mut self) -> Result<()> {
        self.load_attempted = true;

        if !self.config_dir.exists() {
            // Config directory doesn't exist - return early (not an error in GUI)
            return Ok(());
        }

        // Read all .toml files in the directory
        let entries = fs::read_dir(&self.config_dir)
            .with_context(|| format!("Failed to read config dir: {:?}", self.config_dir))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip non-TOML files
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }

            // Try to load the config, distinguishing read errors from parse errors
            match self.load_config(&path) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("{:#}", e);
                    if msg.contains("Failed to read") {
                        tracing::warn!("Cannot read device config {:?}: {}", path, e);
                    } else {
                        tracing::debug!(
                            "Skipping device config {:?} (schema mismatch): {}",
                            path,
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single device config file
    fn load_config(&mut self, path: &Path) -> Result<()> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let config: DeviceConfig = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        // Use protocol name as key
        let protocol = config.device.protocol.clone();
        self.configs.insert(protocol, config);

        Ok(())
    }

    /// Get a device config by protocol name
    #[allow(dead_code)]
    pub fn get_by_protocol(&self, protocol: &str) -> Option<&DeviceConfig> {
        self.configs.get(protocol)
    }

    /// Get a device config by driver type (fuzzy match)
    ///
    /// Tries to match the driver_type string against protocol names.
    /// For example, "ell14_driver" would match protocol "elliptec".
    pub fn get_by_driver_type(&self, driver_type: &str) -> Option<&DeviceConfig> {
        let driver_lower = driver_type.to_lowercase();

        // Try exact protocol match first
        if let Some(config) = self.configs.get(&driver_lower) {
            return Some(config);
        }

        // Try fuzzy match - check if driver_type contains protocol name
        for (protocol, config) in &self.configs {
            if driver_lower.contains(protocol) || protocol.contains(&driver_lower) {
                return Some(config);
            }
        }

        None
    }

    /// Get all loaded protocol names
    pub fn protocols(&self) -> impl Iterator<Item = &str> {
        self.configs.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_creation() {
        let cache = DeviceConfigCache::new();
        assert_eq!(cache.config_dir, PathBuf::from("config/devices"));
    }

    #[test]
    fn test_default_matches_new() {
        let from_new = DeviceConfigCache::new();
        let from_default = DeviceConfigCache::default();
        assert_eq!(from_new.config_dir, from_default.config_dir);
        assert_eq!(from_new.load_attempted, from_default.load_attempted);
    }

    #[test]
    fn test_with_config_dir() {
        let cache = DeviceConfigCache::new().with_config_dir("/custom/path");
        assert_eq!(cache.config_dir, PathBuf::from("/custom/path"));
    }
}
