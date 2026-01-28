//! Feature flag infrastructure for runtime feature toggles.
//!
//! This module provides a simple, file-based feature flag system for controlling
//! feature rollouts and experimental functionality without code changes.
//!
//! # Configuration
//!
//! Feature flags are loaded from `feature_flags.toml` in the config directory:
//!
//! ```toml
//! [flags]
//! experimental_streaming = true
//! new_scan_algorithm = false
//! debug_frame_timing = true
//!
//! [flags.rollout]
//! percentage = 50  # Only enabled for 50% of sessions
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use common::feature_flags::FeatureFlags;
//!
//! let flags = FeatureFlags::load("config/feature_flags.toml")?;
//!
//! if flags.is_enabled("experimental_streaming") {
//!     // Use new streaming implementation
//! } else {
//!     // Use stable implementation
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Feature flag configuration loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureFlagsConfig {
    /// Map of flag names to their enabled state.
    #[serde(default)]
    pub flags: HashMap<String, FlagValue>,

    /// Default value for undefined flags.
    #[serde(default)]
    pub default_enabled: bool,
}

/// A feature flag value - can be a simple bool or a complex rollout config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FlagValue {
    /// Simple boolean flag.
    Bool(bool),
    /// Percentage-based rollout (0-100).
    Rollout { percentage: u8 },
    /// Enabled for specific environments.
    Environment { environments: Vec<String> },
}

impl FlagValue {
    /// Check if the flag is enabled.
    pub fn is_enabled(&self, session_id: Option<u64>, environment: Option<&str>) -> bool {
        match self {
            FlagValue::Bool(enabled) => *enabled,
            FlagValue::Rollout { percentage } => {
                let id = session_id.unwrap_or_else(|| rand_session_id());
                (id % 100) < u64::from(*percentage)
            }
            FlagValue::Environment { environments } => environment
                .map(|env| environments.iter().any(|e| e == env))
                .unwrap_or(false),
        }
    }
}

/// Runtime feature flag manager.
#[derive(Debug)]
pub struct FeatureFlags {
    config: RwLock<FeatureFlagsConfig>,
    session_id: u64,
    environment: Option<String>,
}

impl FeatureFlags {
    /// Create a new feature flags instance with default config.
    pub fn new() -> Self {
        Self {
            config: RwLock::new(FeatureFlagsConfig::default()),
            session_id: rand_session_id(),
            environment: std::env::var("DAQ_ENVIRONMENT").ok(),
        }
    }

    /// Load feature flags from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read feature flags from {}", path.display()))?;

        let config: FeatureFlagsConfig =
            toml::from_str(&content).context("Failed to parse feature flags TOML")?;

        Ok(Self {
            config: RwLock::new(config),
            session_id: rand_session_id(),
            environment: std::env::var("DAQ_ENVIRONMENT").ok(),
        })
    }

    /// Load feature flags, returning defaults if file doesn't exist.
    pub fn load_or_default(path: impl AsRef<Path>) -> Self {
        Self::load(path).unwrap_or_else(|_| Self::new())
    }

    /// Check if a feature flag is enabled.
    pub fn is_enabled(&self, flag_name: &str) -> bool {
        let config = self.config.read().expect("lock poisoned");

        match config.flags.get(flag_name) {
            Some(value) => value.is_enabled(Some(self.session_id), self.environment.as_deref()),
            None => config.default_enabled,
        }
    }

    /// Set a flag value at runtime (for testing or dynamic updates).
    pub fn set_flag(&self, flag_name: &str, enabled: bool) {
        let mut config = self.config.write().expect("lock poisoned");
        config
            .flags
            .insert(flag_name.to_string(), FlagValue::Bool(enabled));
    }

    /// Get all flag names.
    pub fn flag_names(&self) -> Vec<String> {
        let config = self.config.read().expect("lock poisoned");
        config.flags.keys().cloned().collect()
    }

    /// Reload configuration from file.
    pub fn reload(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read feature flags from {}", path.display()))?;

        let new_config: FeatureFlagsConfig =
            toml::from_str(&content).context("Failed to parse feature flags TOML")?;

        let mut config = self.config.write().expect("lock poisoned");
        *config = new_config;
        Ok(())
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self::new()
    }
}

fn rand_session_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ std::process::id() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_flags() {
        let flags = FeatureFlags::new();
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_load_flags() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[flags]
test_feature = true
disabled_feature = false
"#
        )
        .unwrap();

        let flags = FeatureFlags::load(file.path()).unwrap();
        assert!(flags.is_enabled("test_feature"));
        assert!(!flags.is_enabled("disabled_feature"));
    }

    #[test]
    fn test_set_flag() {
        let flags = FeatureFlags::new();
        flags.set_flag("dynamic_flag", true);
        assert!(flags.is_enabled("dynamic_flag"));
        flags.set_flag("dynamic_flag", false);
        assert!(!flags.is_enabled("dynamic_flag"));
    }

    #[test]
    fn test_rollout_flag() {
        let value = FlagValue::Rollout { percentage: 100 };
        assert!(value.is_enabled(Some(50), None));

        let value = FlagValue::Rollout { percentage: 0 };
        assert!(!value.is_enabled(Some(50), None));
    }

    #[test]
    fn test_environment_flag() {
        let value = FlagValue::Environment {
            environments: vec!["production".to_string(), "staging".to_string()],
        };
        assert!(value.is_enabled(None, Some("production")));
        assert!(!value.is_enabled(None, Some("development")));
    }
}
