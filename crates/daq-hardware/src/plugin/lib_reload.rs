//! Library hot-reload support for plugin development.
//!
//! Enables hot-reloading of compiled Rust plugin libraries without restarting
//! the application. Built on top of [`hot-lib-reloader`](https://docs.rs/hot-lib-reloader).
//!
//! # Feature Gate
//!
//! This module is only available with the `plugins_hot_reload` feature:
//! ```bash
//! cargo build --features plugins_hot_reload
//! ```
//!
//! # Architecture
//!
//! This module provides:
//! - [`ReloadableState`] - Trait for serializable plugin state
//! - [`StatePreserver`] - Helper for preserving state across reloads
//! - [`LibReloadConfig`] - Configuration for the reload watcher
//!
//! The actual hot-reloading uses `hot-lib-reloader` macros in your plugin code.
//!
//! # Usage
//!
//! ## 1. Plugin Library Setup
//!
//! Your plugin library needs dylib output in `Cargo.toml`:
//!
//! ```toml
//! [lib]
//! crate-type = ["rlib", "dylib"]
//! ```
//!
//! Mark reloadable functions with `#[unsafe(no_mangle)]`:
//!
//! ```rust,ignore
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! pub struct PluginState {
//!     pub position: f64,
//!     pub connected: bool,
//! }
//!
//! #[unsafe(no_mangle)]
//! pub fn process_command(state: &mut PluginState, cmd: &str) -> String {
//!     // Your plugin logic here
//!     format!("Processed: {}", cmd)
//! }
//!
//! #[unsafe(no_mangle)]
//! pub fn get_state_json(state: &PluginState) -> String {
//!     serde_json::to_string(state).unwrap_or_default()
//! }
//!
//! #[unsafe(no_mangle)]
//! pub fn restore_state_json(json: &str) -> Option<PluginState> {
//!     serde_json::from_str(json).ok()
//! }
//! ```
//!
//! ## 2. Host Application Setup
//!
//! In your host application, use the `hot_module` macro:
//!
//! ```rust,ignore
//! #[hot_lib_reloader::hot_module(dylib = "my_plugin")]
//! mod hot_plugin {
//!     hot_functions_from_file!("crates/my-plugin/src/lib.rs");
//!     pub use my_plugin::PluginState;
//! }
//!
//! fn main() {
//!     let mut state = hot_plugin::PluginState {
//!         position: 0.0,
//!         connected: false,
//!     };
//!
//!     // Use StatePreserver for automatic state persistence
//!     let preserver = StatePreserver::new("my_plugin");
//!
//!     loop {
//!         // Check for reload and preserve state
//!         if hot_plugin::was_updated() {
//!             let json = hot_plugin::get_state_json(&state);
//!             if let Some(restored) = hot_plugin::restore_state_json(&json) {
//!                 state = restored;
//!             }
//!         }
//!
//!         let result = hot_plugin::process_command(&mut state, "MOVE 10");
//!         println!("{}", result);
//!         std::thread::sleep(std::time::Duration::from_secs(1));
//!     }
//! }
//! ```
//!
//! ## 3. Development Workflow
//!
//! In one terminal, watch and rebuild the library:
//! ```bash
//! cargo watch -w crates/my-plugin -x 'build -p my-plugin'
//! ```
//!
//! In another terminal, run your application:
//! ```bash
//! cargo run --features plugins_hot_reload
//! ```
//!
//! Modify your plugin code and watch it reload automatically!
//!
//! # Limitations
//!
//! **IMPORTANT**: Hot-reloading has significant constraints. Read carefully.
//!
//! ## Function Signatures Cannot Change
//!
//! Between reloads, function signatures must remain identical:
//! - Same parameter types
//! - Same return type
//! - Same calling convention
//!
//! Changing signatures causes undefined behavior and likely crashes.
//!
//! ## Tracing Crate Conflicts
//!
//! The `tracing` crate has known conflicts with hot-lib-reloader:
//! - "DefaultCallsite already exists" errors
//! - Subscriber registration failures
//!
//! **Workaround**: Either:
//! 1. Disable tracing in hot-reloaded libraries
//! 2. Use `println!` for debug output during hot-reload development
//! 3. Keep tracing only in the host application, not in dylib plugins
//!
//! ## Global State
//!
//! Global state (statics, thread-locals) is NOT preserved across reloads.
//! Always pass state explicitly to reloadable functions.
//!
//! ## TypeId Changes
//!
//! Types get new TypeIds after reload. This breaks:
//! - ECS systems (bevy, specs, legion)
//! - Any TypeId-based registration
//! - downcast_ref operations
//!
//! ## macOS Code Signing
//!
//! On macOS, reloaded libraries need code signing. hot-lib-reloader
//! handles this automatically if Xcode command line tools are installed.
//!
//! ## Development Only
//!
//! Hot-reloading is strictly for development. Production builds should
//! use static linking:
//!
//! ```toml
//! [features]
//! default = []  # No hot-reload in release
//! dev = ["plugins_hot_reload"]
//! ```
//!
//! # State Preservation
//!
//! The safest pattern for state preservation uses JSON serialization:
//!
//! ```rust,ignore
//! // Before reload
//! let state_json = serde_json::to_string(&state)?;
//!
//! // After reload
//! let state: PluginState = serde_json::from_str(&state_json)?;
//! ```
//!
//! This allows some flexibility in struct field changes (adding optional fields,
//! removing fields) while maintaining compatibility.

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::PathBuf;

/// Trait for plugin state that can be preserved across hot-reloads.
///
/// Implementing this trait allows automatic state serialization before
/// reload and restoration after reload.
pub trait ReloadableState: Serialize + DeserializeOwned + Default {
    /// Returns a unique identifier for this state type.
    ///
    /// Used to namespace state files when multiple plugins are loaded.
    fn state_id() -> &'static str;
}

/// Helper for preserving plugin state across hot-reloads.
///
/// Stores state as JSON in a temporary location, allowing recovery
/// after library reload.
///
/// # Example
///
/// ```rust,ignore
/// let preserver = StatePreserver::new("my_plugin");
///
/// // Save before reload
/// preserver.save(&state)?;
///
/// // After reload
/// if let Ok(restored) = preserver.load::<PluginState>() {
///     state = restored;
/// }
/// ```
pub struct StatePreserver {
    /// Base directory for state files
    state_dir: PathBuf,
    /// Plugin identifier
    plugin_id: String,
}

impl StatePreserver {
    /// Creates a new StatePreserver for the given plugin.
    ///
    /// State is stored in a temp directory under the plugin ID.
    pub fn new(plugin_id: &str) -> Self {
        let state_dir = std::env::temp_dir().join("daq_hot_reload");
        Self {
            state_dir,
            plugin_id: plugin_id.to_string(),
        }
    }

    /// Creates a StatePreserver with a custom state directory.
    pub fn with_dir(plugin_id: &str, state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            plugin_id: plugin_id.to_string(),
        }
    }

    /// Returns the path to the state file for this plugin.
    pub fn state_path(&self) -> PathBuf {
        self.state_dir.join(format!("{}.json", self.plugin_id))
    }

    /// Saves state to disk as JSON.
    ///
    /// # Errors
    ///
    /// Returns error if serialization or file write fails.
    pub fn save<T: Serialize>(&self, state: &T) -> anyhow::Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        let json = serde_json::to_string_pretty(state)?;
        fs::write(self.state_path(), json)?;
        tracing::debug!("Saved hot-reload state: {}", self.state_path().display());
        Ok(())
    }

    /// Loads state from disk.
    ///
    /// # Errors
    ///
    /// Returns error if file doesn't exist, read fails, or deserialization fails.
    pub fn load<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        let json = fs::read_to_string(self.state_path())?;
        let state = serde_json::from_str(&json)?;
        tracing::debug!("Loaded hot-reload state: {}", self.state_path().display());
        Ok(state)
    }

    /// Attempts to load state, returning default if not found.
    ///
    /// Useful for first-time initialization.
    pub fn load_or_default<T: DeserializeOwned + Default>(&self) -> T {
        self.load().unwrap_or_default()
    }

    /// Clears saved state.
    pub fn clear(&self) -> anyhow::Result<()> {
        let path = self.state_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Configuration for library hot-reload behavior.
#[derive(Debug, Clone)]
pub struct LibReloadConfig {
    /// Debounce time for file changes in milliseconds.
    ///
    /// Prevents multiple reloads for rapid consecutive saves.
    /// Default: 100ms
    pub debounce_ms: u64,

    /// Whether to automatically save state before reload.
    ///
    /// If true, the StatePreserver will be used to persist state
    /// when a reload is detected.
    pub auto_save_state: bool,

    /// Whether to automatically restore state after reload.
    ///
    /// If true, the StatePreserver will attempt to restore state
    /// after the library reloads.
    pub auto_restore_state: bool,
}

impl Default for LibReloadConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 100,
            auto_save_state: true,
            auto_restore_state: true,
        }
    }
}

/// Generates the hot_module wrapper for a plugin library.
///
/// This is a convenience macro that sets up the hot-lib-reloader
/// with common DAQ plugin patterns.
///
/// # Usage
///
/// ```rust,ignore
/// daq_hot_plugin!(my_plugin, "crates/my-plugin/src/lib.rs");
/// ```
///
/// Expands to:
///
/// ```rust,ignore
/// #[hot_lib_reloader::hot_module(dylib = "my_plugin", file_watch_debounce = 100)]
/// mod hot_my_plugin {
///     hot_functions_from_file!("crates/my-plugin/src/lib.rs");
///
///     #[lib_change_subscription]
///     pub fn subscribe() -> hot_lib_reloader::LibReloadObserver {}
///
///     #[lib_updated]
///     pub fn was_updated() -> bool {}
/// }
/// ```
#[macro_export]
macro_rules! daq_hot_plugin {
    ($name:ident, $path:literal) => {
        #[hot_lib_reloader::hot_module(dylib = stringify!($name), file_watch_debounce = 100)]
        mod $name {
            hot_functions_from_file!($path);

            #[lib_change_subscription]
            pub fn subscribe() -> hot_lib_reloader::LibReloadObserver {}

            #[lib_updated]
            pub fn was_updated() -> bool {}
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
    struct TestState {
        counter: u32,
        name: String,
    }

    #[test]
    fn test_state_preserver_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let preserver = StatePreserver::with_dir("test_plugin", temp_dir.path().to_path_buf());

        let state = TestState {
            counter: 42,
            name: "test".to_string(),
        };

        preserver.save(&state).unwrap();
        let loaded: TestState = preserver.load().unwrap();

        assert_eq!(state, loaded);
    }

    #[test]
    fn test_state_preserver_load_or_default() {
        let temp_dir = TempDir::new().unwrap();
        let preserver = StatePreserver::with_dir("nonexistent", temp_dir.path().to_path_buf());

        let state: TestState = preserver.load_or_default();
        assert_eq!(state, TestState::default());
    }

    #[test]
    fn test_state_preserver_clear() {
        let temp_dir = TempDir::new().unwrap();
        let preserver = StatePreserver::with_dir("test_plugin", temp_dir.path().to_path_buf());

        let state = TestState {
            counter: 1,
            name: "x".to_string(),
        };
        preserver.save(&state).unwrap();
        assert!(preserver.state_path().exists());

        preserver.clear().unwrap();
        assert!(!preserver.state_path().exists());
    }

    #[test]
    fn test_lib_reload_config_default() {
        let config = LibReloadConfig::default();
        assert_eq!(config.debounce_ms, 100);
        assert!(config.auto_save_state);
        assert!(config.auto_restore_state);
    }
}
