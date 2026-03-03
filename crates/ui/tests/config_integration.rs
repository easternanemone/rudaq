//! Integration tests for gui.toml config loading.
//!
//! Exercises the full round-trip: write TOML -> read from disk -> parse -> verify fields.
//! Uses the same `GuiConfig` structs and `toml` serde that `load_gui_config()` uses
//! internally, with a controlled temp directory instead of fixed discovery paths.
//!
//! Run with: cargo test -p ui --test config_integration

use std::io::Write;
use tempfile::TempDir;
use ui::gui_config::{DaemonPreset, GuiConfig, GuiDefaults, SshConfig};

// ---------------------------------------------------------------------------
// Helper: write a TOML string to `<tmpdir>/gui.toml` and parse it back
// ---------------------------------------------------------------------------

/// Write `toml_content` to a temp file, read it back, and parse as `GuiConfig`.
///
/// This mirrors what `load_gui_config()` does internally:
///   `fs::read_to_string` -> `toml::from_str`
fn write_and_load(toml_content: &str) -> Result<GuiConfig, String> {
    let dir = TempDir::new().expect("failed to create temp dir");
    let path = dir.path().join("gui.toml");

    let mut file =
        std::fs::File::create(&path).map_err(|e| format!("Failed to create temp file: {e}"))?;
    file.write_all(toml_content.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {e}"))?;
    drop(file);

    // Read back from disk — same as load_gui_config's read step
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let config: GuiConfig =
        toml::from_str(&contents).map_err(|e| format!("Failed to parse TOML: {e}"))?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Full round-trip: struct -> TOML string -> disk -> read -> parse -> verify
// ---------------------------------------------------------------------------

#[test]
fn full_config_round_trip() {
    let original = GuiConfig {
        config_version: 1,
        defaults: GuiDefaults {
            theme: Some("Light".to_string()),
            font_scale: Some(1.25),
            auto_reconnect: Some(false),
        },
        presets: vec![
            DaemonPreset {
                name: "Local Dev".to_string(),
                grpc_url: "http://127.0.0.1:50051".to_string(),
                description: "Local mock daemon".to_string(),
                default: true,
                ssh: None,
            },
            DaemonPreset {
                name: "Remote Lab".to_string(),
                grpc_url: "http://10.0.0.42:50051".to_string(),
                description: "Lab hardware via SSH tunnel".to_string(),
                default: false,
                ssh: Some(SshConfig {
                    user: "labuser".to_string(),
                    host: "10.0.0.42".to_string(),
                    identity_file: Some("/home/labuser/.ssh/id_ed25519".to_string()),
                    remote_dir: "/opt/rust-daq".to_string(),
                    env_file: ".env.lab".to_string(),
                    daemon_port: 50051,
                    hardware_config: "config/hardware.toml".to_string(),
                    connect_timeout_secs: 30,
                }),
            },
        ],
    };

    // Serialize to TOML string
    let toml_str =
        toml::to_string(&original).expect("serialization should succeed for valid config");

    // Write to disk, read back, and parse
    let loaded = write_and_load(&toml_str).expect("round-trip load should succeed");

    // Verify top-level
    assert_eq!(loaded.config_version, 1);

    // Verify defaults
    assert_eq!(loaded.defaults.theme.as_deref(), Some("Light"));
    assert_eq!(loaded.defaults.font_scale, Some(1.25));
    assert_eq!(loaded.defaults.auto_reconnect, Some(false));

    // Verify preset count and ordering
    assert_eq!(loaded.presets.len(), 2);

    // First preset: local, no SSH
    let p0 = &loaded.presets[0];
    assert_eq!(p0.name, "Local Dev");
    assert_eq!(p0.grpc_url, "http://127.0.0.1:50051");
    assert_eq!(p0.description, "Local mock daemon");
    assert!(p0.default);
    assert!(p0.ssh.is_none());

    // Second preset: remote with full SSH config
    let p1 = &loaded.presets[1];
    assert_eq!(p1.name, "Remote Lab");
    assert_eq!(p1.grpc_url, "http://10.0.0.42:50051");
    assert!(!p1.default);
    let ssh = p1.ssh.as_ref().expect("preset should have SSH config");
    assert_eq!(ssh.user, "labuser");
    assert_eq!(ssh.host, "10.0.0.42");
    assert_eq!(
        ssh.identity_file.as_deref(),
        Some("/home/labuser/.ssh/id_ed25519")
    );
    assert_eq!(ssh.remote_dir, "/opt/rust-daq");
    assert_eq!(ssh.env_file, ".env.lab");
    assert_eq!(ssh.daemon_port, 50051);
    assert_eq!(ssh.hardware_config, "config/hardware.toml");
    assert_eq!(ssh.connect_timeout_secs, 30);
}

// ---------------------------------------------------------------------------
// Verify hand-written TOML (mimics user-authored gui.toml)
// ---------------------------------------------------------------------------

#[test]
fn hand_written_toml_loads_all_fields() {
    let toml_content = r#"
config_version = 1

[defaults]
theme = "Dark"
font_scale = 1.5
auto_reconnect = true

[[presets]]
name = "Localhost"
grpc_url = "http://localhost:50051"
description = "Default local daemon"
default = true

[[presets]]
name = "Maitai"
grpc_url = "http://100.117.5.12:50051"
description = "Lab PVCAM hardware"
default = false

[presets.ssh]
user = "maitai"
host = "100.117.5.12"
identity_file = "~/.ssh/maitai_ed25519"
remote_dir = "/home/maitai/rust-daq"
env_file = ".env"
daemon_port = 50051
hardware_config = "config/hardware-maitai.toml"
connect_timeout_secs = 15
"#;

    let config = write_and_load(toml_content).expect("hand-written TOML should parse");

    assert_eq!(config.config_version, 1);
    assert_eq!(config.defaults.theme.as_deref(), Some("Dark"));
    assert_eq!(config.defaults.font_scale, Some(1.5));
    assert_eq!(config.defaults.auto_reconnect, Some(true));

    assert_eq!(config.presets.len(), 2);

    // Verify first preset fields
    assert_eq!(config.presets[0].name, "Localhost");
    assert!(config.presets[0].default);
    assert!(config.presets[0].ssh.is_none());

    // Verify second preset with SSH
    let maitai = &config.presets[1];
    assert_eq!(maitai.name, "Maitai");
    assert!(!maitai.default);
    let ssh = maitai.ssh.as_ref().expect("Maitai preset should have SSH");
    assert_eq!(ssh.user, "maitai");
    assert_eq!(ssh.host, "100.117.5.12");
    assert_eq!(ssh.identity_file.as_deref(), Some("~/.ssh/maitai_ed25519"));
    assert_eq!(ssh.remote_dir, "/home/maitai/rust-daq");
    assert_eq!(ssh.connect_timeout_secs, 15);
}

// ---------------------------------------------------------------------------
// Preset configs load with correct types (Option vs required, bool, u16, etc.)
// ---------------------------------------------------------------------------

#[test]
fn preset_types_are_correct() {
    let toml_content = r#"
config_version = 1

[[presets]]
name = "TypeCheck"
grpc_url = "http://[::1]:50051"
description = "IPv6 loopback"
default = false

[presets.ssh]
user = "test-user"
host = "node-42.lab"
remote_dir = "/data"
env_file = ".env"
daemon_port = 60000
hardware_config = "hw.toml"
connect_timeout_secs = 120
"#;

    let config = write_and_load(toml_content).expect("type-check TOML should parse");

    // Defaults should all be None when omitted
    assert!(config.defaults.theme.is_none());
    assert!(config.defaults.font_scale.is_none());
    assert!(config.defaults.auto_reconnect.is_none());

    let preset = &config.presets[0];
    // String fields
    assert_eq!(preset.name, "TypeCheck");
    assert_eq!(preset.grpc_url, "http://[::1]:50051");
    assert_eq!(preset.description, "IPv6 loopback");
    // Bool field
    assert!(!preset.default);

    let ssh = preset.ssh.as_ref().expect("preset should have SSH");
    // u16 field — verify high port number round-trips
    assert_eq!(ssh.daemon_port, 60000_u16);
    // u64 field
    assert_eq!(ssh.connect_timeout_secs, 120_u64);
    // Optional field omitted -> None
    assert!(ssh.identity_file.is_none());
    // String fields with special characters (hyphens, dots)
    assert_eq!(ssh.user, "test-user");
    assert_eq!(ssh.host, "node-42.lab");
}

// ---------------------------------------------------------------------------
// Minimal config: only required field (config_version)
// ---------------------------------------------------------------------------

#[test]
fn minimal_config_round_trip() {
    let toml_content = "config_version = 1\n";
    let config = write_and_load(toml_content).expect("minimal config should parse");

    assert_eq!(config.config_version, 1);
    assert!(config.presets.is_empty());
    assert!(config.defaults.theme.is_none());
    assert!(config.defaults.font_scale.is_none());
    assert!(config.defaults.auto_reconnect.is_none());

    // Round-trip: serialize back and verify it parses identically
    let re_serialized = toml::to_string(&config).expect("re-serialization should succeed");
    let re_loaded: GuiConfig =
        toml::from_str(&re_serialized).expect("re-serialized TOML should parse");
    assert_eq!(re_loaded.config_version, config.config_version);
    assert!(re_loaded.presets.is_empty());
}

// ---------------------------------------------------------------------------
// Default struct round-trip (GuiConfig::default)
// ---------------------------------------------------------------------------

#[test]
fn default_struct_round_trips_through_file() {
    let config = GuiConfig::default();
    let toml_str = toml::to_string(&config).expect("default config serialization");
    let loaded = write_and_load(&toml_str).expect("default config file round-trip");

    assert_eq!(loaded.config_version, config.config_version);
    assert!(loaded.presets.is_empty());
    assert!(loaded.defaults.theme.is_none());
    assert!(loaded.defaults.font_scale.is_none());
    assert!(loaded.defaults.auto_reconnect.is_none());
}

// ---------------------------------------------------------------------------
// SSH validation runs correctly on loaded config
// ---------------------------------------------------------------------------

#[test]
fn loaded_ssh_config_validates() {
    let toml_content = r#"
config_version = 1

[[presets]]
name = "Valid SSH"
grpc_url = "http://10.0.0.1:50051"
description = "Should pass validation"

[presets.ssh]
user = "valid_user"
host = "valid-host.example"
remote_dir = "/home/user"
env_file = ".env"
daemon_port = 50051
hardware_config = "config/hw.toml"
"#;

    let config = write_and_load(toml_content).expect("valid SSH config should parse");
    let ssh = config.presets[0]
        .ssh
        .as_ref()
        .expect("preset should have SSH");
    assert!(
        ssh.validate().is_ok(),
        "valid SSH config should pass validation"
    );
}

#[test]
fn loaded_ssh_config_rejects_injection() {
    // This tests the same validation that load_gui_config() performs
    let ssh = SshConfig {
        user: "user$(whoami)".to_string(),
        host: "localhost".to_string(),
        ..Default::default()
    };
    assert!(
        ssh.validate().is_err(),
        "SSH user with shell injection should fail validation"
    );

    let ssh = SshConfig {
        user: "gooduser".to_string(),
        host: "host;evil".to_string(),
        ..Default::default()
    };
    assert!(
        ssh.validate().is_err(),
        "SSH host with semicolon should fail validation"
    );
}

// ---------------------------------------------------------------------------
// Reject unknown fields (deny_unknown_fields enforcement)
// ---------------------------------------------------------------------------

#[test]
fn rejects_unknown_top_level_field() {
    let toml_content = r#"
config_version = 1
unknown_field = "should fail"
"#;
    let result = write_and_load(toml_content);
    assert!(
        result.is_err(),
        "unknown top-level field should be rejected"
    );
    let err = result.expect_err("should have error message");
    assert!(
        err.contains("unknown field"),
        "error should mention unknown field, got: {err}"
    );
}

#[test]
fn rejects_unknown_preset_field() {
    let toml_content = r#"
config_version = 1

[[presets]]
name = "Bad"
grpc_url = "http://localhost:50051"
exposure_ms = 100
"#;
    let result = write_and_load(toml_content);
    assert!(
        result.is_err(),
        "unknown preset field should be rejected by deny_unknown_fields"
    );
}

// ---------------------------------------------------------------------------
// Multiple presets with mixed SSH / no-SSH
// ---------------------------------------------------------------------------

#[test]
fn multiple_presets_mixed_ssh() {
    let toml_content = r#"
config_version = 1

[[presets]]
name = "Local"
grpc_url = "http://127.0.0.1:50051"
description = "No SSH"

[[presets]]
name = "Remote-A"
grpc_url = "http://10.0.0.1:50051"
description = "With SSH"
default = true

[presets.ssh]
user = "alice"
host = "10.0.0.1"
remote_dir = "/data"
env_file = ".env"
daemon_port = 50051
hardware_config = "hw.toml"

[[presets]]
name = "Remote-B"
grpc_url = "http://10.0.0.2:50051"
description = "Different host"

[presets.ssh]
user = "bob"
host = "10.0.0.2"
remote_dir = "/opt/daq"
env_file = ".env.prod"
daemon_port = 50052
hardware_config = "config/hardware.toml"
connect_timeout_secs = 60
"#;

    let config = write_and_load(toml_content).expect("mixed presets should parse");
    assert_eq!(config.presets.len(), 3);

    // First has no SSH
    assert!(config.presets[0].ssh.is_none());
    assert!(!config.presets[0].default);

    // Second has SSH and is default
    assert!(config.presets[1].ssh.is_some());
    assert!(config.presets[1].default);
    assert_eq!(config.presets[1].ssh.as_ref().expect("ssh").user, "alice");

    // Third has SSH with different port and timeout
    let ssh_b = config.presets[2].ssh.as_ref().expect("ssh for Remote-B");
    assert_eq!(ssh_b.user, "bob");
    assert_eq!(ssh_b.daemon_port, 50052);
    assert_eq!(ssh_b.connect_timeout_secs, 60);
}
