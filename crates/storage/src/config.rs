//! Storage Configuration
//!
//! Provides configurable data paths for storage writers, replacing hardcoded
//! paths with a layered configuration approach.
//!
//! # Configuration Precedence
//!
//! Path resolution follows this priority (highest to lowest):
//!
//! 1. Explicit value passed to [`StorageConfig::new`]
//! 2. `DAQ_DATA_PATH` environment variable
//! 3. Platform-appropriate default:
//!    - macOS: `~/Library/Application Support/daq/data`
//!    - Linux: `~/.local/share/daq/data`
//!    - Windows: `{FOLDERID_LocalAppData}/daq/data`
//!    - Fallback: `./data`
//!
//! # Example
//!
//! ```rust
//! use storage::config::StorageConfig;
//!
//! // Use environment variable or platform default
//! let config = StorageConfig::from_env();
//! let data_path = config.data_path();
//!
//! // Use an explicit path
//! let config = StorageConfig::new("/custom/data/path");
//! assert_eq!(config.data_path().to_str(), Some("/custom/data/path"));
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Environment variable name for overriding the data path.
pub const DAQ_DATA_PATH_ENV: &str = "DAQ_DATA_PATH";

/// Storage configuration with configurable data paths.
///
/// Controls where storage writers place output files (HDF5, Arrow, Parquet, etc.).
/// Supports configuration via explicit paths, environment variables, or
/// platform-appropriate defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Root directory for data output files.
    data_path: PathBuf,
}

impl StorageConfig {
    /// Create a `StorageConfig` with an explicit data path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use storage::config::StorageConfig;
    ///
    /// let config = StorageConfig::new("/my/data/dir");
    /// assert_eq!(config.data_path().to_str(), Some("/my/data/dir"));
    /// ```
    pub fn new(data_path: impl Into<PathBuf>) -> Self {
        Self {
            data_path: data_path.into(),
        }
    }

    /// Create a `StorageConfig` from environment or platform defaults.
    ///
    /// Checks `DAQ_DATA_PATH` environment variable first, then falls back
    /// to a platform-appropriate default directory.
    ///
    /// # Example
    ///
    /// ```rust
    /// use storage::config::StorageConfig;
    ///
    /// let config = StorageConfig::from_env();
    /// // Uses DAQ_DATA_PATH env var if set, otherwise platform default
    /// let path = config.data_path();
    /// ```
    pub fn from_env() -> Self {
        let data_path = std::env::var(DAQ_DATA_PATH_ENV)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(platform_default_data_path);

        Self { data_path }
    }

    /// Returns the configured data path.
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    /// Returns a subdirectory within the data path for a specific run or category.
    ///
    /// # Example
    ///
    /// ```rust
    /// use storage::config::StorageConfig;
    ///
    /// let config = StorageConfig::new("/data");
    /// let runs_path = config.data_subdir("runs");
    /// assert_eq!(runs_path.to_str(), Some("/data/runs"));
    /// ```
    pub fn data_subdir(&self, subdir: &str) -> PathBuf {
        self.data_path.join(subdir)
    }
}

impl Default for StorageConfig {
    /// Returns a `StorageConfig` using environment or platform defaults.
    ///
    /// Equivalent to [`StorageConfig::from_env()`].
    fn default() -> Self {
        Self::from_env()
    }
}

/// Returns the platform-appropriate default data directory.
///
/// - macOS: `~/Library/Application Support/daq/data`
/// - Linux: `~/.local/share/daq/data`
/// - Windows: `{FOLDERID_LocalAppData}/daq/data`
/// - Fallback: `./data`
fn platform_default_data_path() -> PathBuf {
    // Use dirs crate if available at runtime via home_dir detection,
    // otherwise fall back to ./data.
    //
    // We intentionally avoid adding `dirs` as a dependency to keep the
    // storage crate lightweight. Instead we use the standard approach:
    // data_local_dir equivalent via env vars and known platform paths.

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = home_dir() {
            return home
                .join("Library")
                .join("Application Support")
                .join("daq")
                .join("data");
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Respect XDG_DATA_HOME if set
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("daq").join("data");
        }
        if let Some(home) = home_dir() {
            return home.join(".local").join("share").join("daq").join("data");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            if !local_app_data.is_empty() {
                return PathBuf::from(local_app_data).join("daq").join("data");
            }
        }
    }

    // Fallback for all platforms
    PathBuf::from("./data")
}

/// Returns the user's home directory, if detectable.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
#[allow(unsafe_code)] // edition 2024: env::set_var/remove_var require unsafe
mod tests {
    use super::*;

    /// Serializes access to `DAQ_DATA_PATH` so parallel tests don't race on
    /// `std::env::set_var` / `remove_var`.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that captures the original value of `DAQ_DATA_PATH` and
    /// restores it on drop -- even if the test panics.
    struct EnvGuard {
        original: Option<String>,
    }

    impl EnvGuard {
        /// Acquire the env lock and return both the `MutexGuard` (to keep the
        /// lock held) and the `EnvGuard` (to restore on drop).
        fn lock() -> (std::sync::MutexGuard<'static, ()>, Self) {
            let mutex_guard = ENV_LOCK.lock().expect("ENV_LOCK poisoned");
            let env_guard = Self {
                original: std::env::var(DAQ_DATA_PATH_ENV).ok(),
            };
            (mutex_guard, env_guard)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: Tests using EnvGuard hold ENV_LOCK, ensuring single-threaded
            // access to environment variables within this test module.
            unsafe {
                match &self.original {
                    Some(val) => std::env::set_var(DAQ_DATA_PATH_ENV, val),
                    None => std::env::remove_var(DAQ_DATA_PATH_ENV),
                }
            }
        }
    }

    #[test]
    fn test_explicit_path() {
        let config = StorageConfig::new("/custom/path");
        assert_eq!(config.data_path(), Path::new("/custom/path"));
    }

    #[test]
    fn test_data_subdir() {
        let config = StorageConfig::new("/data");
        assert_eq!(config.data_subdir("runs"), PathBuf::from("/data/runs"));
        assert_eq!(
            config.data_subdir("archive"),
            PathBuf::from("/data/archive")
        );
    }

    #[test]
    fn test_from_env_with_var() {
        let (_lock, _env) = EnvGuard::lock();

        // SAFETY: Test holds ENV_LOCK via EnvGuard, ensuring exclusive access.
        unsafe { std::env::set_var(DAQ_DATA_PATH_ENV, "/env/data/path") };
        let config = StorageConfig::from_env();
        assert_eq!(config.data_path(), Path::new("/env/data/path"));
    }

    #[test]
    fn test_from_env_empty_var_uses_default() {
        let (_lock, _env) = EnvGuard::lock();

        // SAFETY: Test holds ENV_LOCK via EnvGuard, ensuring exclusive access.
        unsafe { std::env::set_var(DAQ_DATA_PATH_ENV, "") };
        let config = StorageConfig::from_env();
        // Should not be empty -- should fall back to platform default
        assert!(!config.data_path().as_os_str().is_empty());
    }

    #[test]
    fn test_default_is_from_env() {
        let (_lock, _env) = EnvGuard::lock();
        // SAFETY: Test holds ENV_LOCK via EnvGuard, ensuring exclusive access.
        unsafe { std::env::remove_var(DAQ_DATA_PATH_ENV) };

        let default_config = StorageConfig::default();
        let env_config = StorageConfig::from_env();
        assert_eq!(default_config.data_path(), env_config.data_path());
    }

    #[test]
    fn test_platform_default_is_not_empty() {
        let path = platform_default_data_path();
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let config = StorageConfig::new("/serde/test");
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: StorageConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.data_path(), Path::new("/serde/test"));
    }
}
