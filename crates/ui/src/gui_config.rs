//! GUI configuration types and loader.
//!
//! Loads `gui.toml` with connection presets and default settings.
//! Discovery order (first found wins):
//!   1. `./config/gui.toml`
//!   2. `<exe_dir>/../config/gui.toml`
//!   3. `~/.config/rust-daq/gui.toml`

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level GUI configuration from gui.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuiConfig {
    /// Schema version for forward-compatible parsing.
    pub config_version: u32,
    /// Default settings applied when no user preference exists.
    pub defaults: GuiDefaults,
    /// Named connection presets for the Daemon menu.
    pub presets: Vec<DaemonPreset>,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            config_version: 1,
            defaults: GuiDefaults::default(),
            presets: Vec::new(),
        }
    }
}

/// Default GUI settings from config file.
///
/// All fields are `Option<T>` so the config file can omit any of them;
/// omitted values keep the application's built-in defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuiDefaults {
    /// Theme preference: "Light", "Dark", or "System".
    pub theme: Option<String>,
    /// Font size multiplier (1.0 = default).
    pub font_scale: Option<f32>,
    /// Enable automatic reconnection on disconnect.
    pub auto_reconnect: Option<bool>,
}

/// A named daemon connection preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonPreset {
    /// Display name shown in the Daemon menu.
    pub name: String,
    /// gRPC endpoint URL.
    pub grpc_url: String,
    /// Tooltip description.
    pub description: String,
    /// Whether this is the default preset (at most one should be true).
    pub default: bool,
    /// Optional SSH tunnel configuration for remote daemons.
    pub ssh: Option<SshConfig>,
}

impl Default for DaemonPreset {
    fn default() -> Self {
        Self {
            name: String::new(),
            grpc_url: "http://127.0.0.1:50051".to_string(),
            description: String::new(),
            default: false,
            ssh: None,
        }
    }
}

/// SSH tunnel configuration for remote daemon presets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshConfig {
    /// SSH username (validated: alphanumeric, dot, underscore, hyphen).
    pub user: String,
    /// SSH hostname or IP (validated: alphanumeric, dot, underscore, hyphen).
    pub host: String,
    /// Optional path to SSH identity file.
    pub identity_file: Option<String>,
    /// Remote working directory on the target host.
    pub remote_dir: String,
    /// Environment file to source before starting daemon.
    pub env_file: String,
    /// Daemon gRPC port on the remote host.
    pub daemon_port: u16,
    /// Path to hardware config on the remote host (relative to remote_dir).
    pub hardware_config: String,
    /// SSH connection timeout in seconds.
    pub connect_timeout_secs: u64,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            user: String::new(),
            host: String::new(),
            identity_file: None,
            remote_dir: String::new(),
            env_file: String::new(),
            daemon_port: 50051,
            hardware_config: String::new(),
            connect_timeout_secs: 10,
        }
    }
}

impl SshConfig {
    /// Validate SSH config fields for safe characters.
    ///
    /// User and host must match `^[a-zA-Z0-9._-]+$` to prevent injection.
    pub fn validate(&self) -> Result<(), String> {
        let safe_pattern = |s: &str| -> bool {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        };

        if !safe_pattern(&self.user) {
            return Err(format!(
                "SSH user '{}' contains invalid characters (allowed: a-zA-Z, 0-9, ., _, -)",
                self.user
            ));
        }
        if !safe_pattern(&self.host) {
            return Err(format!(
                "SSH host '{}' contains invalid characters (allowed: a-zA-Z, 0-9, ., _, -)",
                self.host
            ));
        }
        Ok(())
    }
}

/// Maximum supported gui.toml config version.
const MAX_GUI_CONFIG_VERSION: u32 = 1;

/// Discover and load gui.toml from standard locations.
///
/// Returns `Ok(None)` if no config file is found (normal for development).
/// Returns `Err` if a config file exists but cannot be read or parsed.
/// Validates SSH configs and version bounds on load.
pub fn load_gui_config() -> Result<Option<GuiConfig>, String> {
    let Some(path) = discover_config_path() else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let config: GuiConfig = toml::from_str(&contents)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    // Validate config version
    if config.config_version < 1 || config.config_version > MAX_GUI_CONFIG_VERSION {
        return Err(format!(
            "Unsupported gui.toml version {} in {} (supported: 1..={})",
            config.config_version,
            path.display(),
            MAX_GUI_CONFIG_VERSION
        ));
    }
    // Validate SSH configs in all presets
    for preset in &config.presets {
        if let Some(ref ssh) = preset.ssh
            && let Err(e) = ssh.validate()
        {
            return Err(format!(
                "Invalid SSH config in preset '{}': {}",
                preset.name, e
            ));
        }
    }
    tracing::info!("Loaded GUI config from {}", path.display());
    Ok(Some(config))
}

/// Apply config defaults to AppSettings (only fills in fields the user hasn't set).
pub fn apply_defaults(settings: &mut crate::settings::AppSettings, defaults: &GuiDefaults) {
    if let Some(ref theme_str) = defaults.theme {
        // Only apply if settings still have the built-in default
        let builtin = crate::settings::AppearanceSettings::default();
        if settings.appearance.theme == builtin.theme {
            let pref = match theme_str.as_str() {
                "Light" => Some(crate::theme::ThemePreference::Light),
                "Dark" => Some(crate::theme::ThemePreference::Dark),
                "System" => Some(crate::theme::ThemePreference::System),
                _ => None,
            };
            if let Some(pref) = pref {
                settings.appearance.theme = pref;
            }
        }
    }
    if let Some(scale) = defaults.font_scale {
        let builtin = crate::settings::AppearanceSettings::default();
        if (settings.appearance.font_scale - builtin.font_scale).abs() < f32::EPSILON {
            settings.appearance.font_scale = scale;
        }
    }
    if let Some(auto) = defaults.auto_reconnect {
        let builtin = crate::settings::ConnectionSettings::default();
        if settings.connection.auto_reconnect == builtin.auto_reconnect {
            settings.connection.auto_reconnect = auto;
        }
    }
}

/// Search standard locations for gui.toml.
fn discover_config_path() -> Option<PathBuf> {
    // 1. ./config/gui.toml (project root / CWD)
    let cwd = PathBuf::from("./config/gui.toml");
    if cwd.is_file() {
        return Some(cwd);
    }

    // 2. <exe_dir>/../config/gui.toml (relative to binary)
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        let relative = exe_dir.join("../config/gui.toml");
        if relative.is_file() {
            return Some(relative);
        }
    }

    // 3. ~/.config/rust-daq/gui.toml (XDG-style user config)
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(config_dir) = dirs::config_dir() {
        let user_config = config_dir.join("rust-daq/gui.toml");
        if user_config.is_file() {
            return Some(user_config);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gui_config() {
        let toml_str = r#"
config_version = 1

[defaults]
theme = "Dark"
font_scale = 1.0
auto_reconnect = true

[[presets]]
name = "Local Mock"
grpc_url = "http://127.0.0.1:50051"
description = "Mock devices"

[[presets]]
name = "Remote Lab"
grpc_url = "http://10.0.0.1:50051"
description = "Lab hardware"
default = true

[presets.ssh]
user = "labuser"
host = "10.0.0.1"
remote_dir = "/home/labuser/daq"
env_file = ".env"
daemon_port = 50051
hardware_config = "config/hardware.toml"
"#;
        let config: GuiConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.config_version, 1);
        assert_eq!(config.defaults.theme.as_deref(), Some("Dark"));
        assert_eq!(config.presets.len(), 2);
        assert_eq!(config.presets[0].name, "Local Mock");
        assert!(!config.presets[0].default);
        assert!(config.presets[0].ssh.is_none());
        assert_eq!(config.presets[1].name, "Remote Lab");
        assert!(config.presets[1].default);
        assert!(config.presets[1].ssh.is_some());
        let ssh = config.presets[1].ssh.as_ref().unwrap();
        assert_eq!(ssh.user, "labuser");
        assert_eq!(ssh.host, "10.0.0.1");
        assert_eq!(ssh.daemon_port, 50051);
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = "config_version = 1\n";
        let config: GuiConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.config_version, 1);
        assert!(config.presets.is_empty());
        assert!(config.defaults.theme.is_none());
    }

    #[test]
    fn test_ssh_validate_ok() {
        let ssh = SshConfig {
            user: "maitai".to_string(),
            host: "100.117.5.12".to_string(),
            ..Default::default()
        };
        assert!(ssh.validate().is_ok());
    }

    #[test]
    fn test_ssh_validate_bad_user() {
        let ssh = SshConfig {
            user: "user; rm -rf /".to_string(),
            host: "localhost".to_string(),
            ..Default::default()
        };
        assert!(ssh.validate().is_err());
    }

    #[test]
    fn test_ssh_validate_bad_host() {
        let ssh = SshConfig {
            user: "user".to_string(),
            host: "host$(evil)".to_string(),
            ..Default::default()
        };
        assert!(ssh.validate().is_err());
    }

    #[test]
    fn test_ssh_validate_empty_user() {
        let ssh = SshConfig {
            user: String::new(),
            host: "localhost".to_string(),
            ..Default::default()
        };
        assert!(ssh.validate().is_err());
    }

    #[test]
    fn test_default_config_roundtrip() {
        let config = GuiConfig::default();
        let serialized = toml::to_string(&config).unwrap();
        let deserialized: GuiConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.config_version, config.config_version);
    }

    #[test]
    fn test_reject_hardware_fields_at_top_level() {
        let toml_str = r"
config_version = 1
stage_limits = { x_min = -50.0, x_max = 50.0 }
";
        let result = toml::from_str::<GuiConfig>(toml_str);
        assert!(result.is_err(), "gui.toml must reject hardware parameters");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown field"),
            "Error should mention unknown field: {err}"
        );
    }

    #[test]
    fn test_reject_hardware_fields_in_defaults() {
        let toml_str = r#"
config_version = 1
[defaults]
theme = "Dark"
pid_constants = { kp = 1.0, ki = 0.1, kd = 0.01 }
"#;
        let result = toml::from_str::<GuiConfig>(toml_str);
        assert!(result.is_err(), "defaults must reject hardware parameters");
    }

    #[test]
    fn test_reject_hardware_fields_in_preset() {
        let toml_str = r#"
config_version = 1
[[presets]]
name = "Lab"
grpc_url = "http://localhost:50051"
exposure_ms = 100
"#;
        let result = toml::from_str::<GuiConfig>(toml_str);
        assert!(result.is_err(), "presets must reject hardware parameters");
    }

    #[test]
    fn test_reject_devices_section() {
        let toml_str = r#"
config_version = 1
[[devices]]
id = "camera"
name = "Test Camera"
"#;
        let result = toml::from_str::<GuiConfig>(toml_str);
        assert!(result.is_err(), "gui.toml must reject [[devices]] sections");
    }
}
