//! Cross-platform settings persistence via the `preferences` crate (native only).
//!
//! On native targets this module uses [`preferences::Preferences`] for robust,
//! OS-appropriate settings storage (XDG on Linux, Library/Preferences on macOS,
//! AppData on Windows). On WASM the module is not compiled — eframe's built-in
//! storage handles persistence in the browser.
//!
//! This is *additive*: existing eframe storage is untouched. `AppPreferences`
//! captures a small subset of settings that benefit from cross-platform file
//! persistence independent of the eframe state directory.

use preferences::AppInfo;
use serde::{Deserialize, Serialize};

/// Application metadata used by the `preferences` crate to locate the settings
/// file on each platform.
const APP_INFO: AppInfo = AppInfo {
    name: "rust-daq-gui",
    author: "TheFermiSea",
};

/// Key under which `AppPreferences` is stored.
const PREFS_KEY: &str = "app-preferences";

/// Minimal set of user preferences persisted via the `preferences` crate.
///
/// Keep this struct small and backwards-compatible — new fields should always
/// have `#[serde(default)]` so that loading older files succeeds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppPreferences {
    /// Last-used daemon address (e.g. `localhost:50051`).
    #[serde(default = "default_daemon_url")]
    pub daemon_url: String,

    /// Theme name persisted across sessions (`"Dark"`, `"Light"`, or `"System"`).
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_daemon_url() -> String {
    "localhost:50051".to_string()
}

fn default_theme() -> String {
    "Dark".to_string()
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            daemon_url: default_daemon_url(),
            theme: default_theme(),
        }
    }
}

impl AppPreferences {
    /// Load preferences from the OS-appropriate location.
    ///
    /// Returns [`Default`] values if the file does not exist or cannot be parsed
    /// (e.g. first launch, corrupt file, schema change).
    pub fn load_or_default() -> Self {
        use preferences::Preferences;
        match <Self as Preferences>::load(&APP_INFO, PREFS_KEY) {
            Ok(prefs) => {
                tracing::debug!(
                    "Loaded native preferences (daemon_url={})",
                    prefs.daemon_url
                );
                prefs
            }
            Err(e) => {
                tracing::debug!("No saved native preferences ({}), using defaults", e);
                Self::default()
            }
        }
    }

    /// Persist preferences to the OS-appropriate location.
    ///
    /// Errors are logged but not propagated — settings persistence should never
    /// crash the application.
    pub fn persist(&self) {
        use preferences::Preferences;
        if let Err(e) = <Self as Preferences>::save(self, &APP_INFO, PREFS_KEY) {
            tracing::warn!("Failed to save native preferences: {}", e);
        } else {
            tracing::debug!("Saved native preferences (daemon_url={})", self.daemon_url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_preferences() {
        let prefs = AppPreferences::default();
        assert_eq!(prefs.daemon_url, "localhost:50051");
        assert_eq!(prefs.theme, "Dark");
    }

    #[test]
    fn test_serde_roundtrip() {
        let prefs = AppPreferences {
            daemon_url: "http://10.0.0.1:50051".to_string(),
            theme: "Light".to_string(),
        };
        let json = serde_json::to_string(&prefs).expect("serialize");
        let loaded: AppPreferences = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(prefs, loaded);
    }

    #[test]
    fn test_serde_missing_fields_use_defaults() {
        // Simulates loading an older file that lacks the `theme` field.
        let json = r#"{"daemon_url":"myhost:9999"}"#;
        let loaded: AppPreferences = serde_json::from_str(json).expect("deserialize");
        assert_eq!(loaded.daemon_url, "myhost:9999");
        assert_eq!(loaded.theme, "Dark"); // default
    }
}
