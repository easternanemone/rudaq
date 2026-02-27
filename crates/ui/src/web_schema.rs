//! Lightweight schema types for the WASM web build.
//!
//! These mirror the UI configuration structs from `hardware::config::schema`
//! but without the `hardware` crate's native-only dependencies (serial, FFI
//! bindings, `garde`/`schemars` validators). The serde attributes are identical
//! so that JSON produced by the daemon deserializes correctly in the browser.
//!
//! Note: `deny_unknown_fields` is intentionally omitted so the web build
//! gracefully ignores any new fields added to the server-side schema.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Top-level UI config ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub control_panel: Option<ControlPanelConfig>,
    #[serde(default)]
    pub status_display: Option<StatusDisplayConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlPanelConfig {
    #[serde(default)]
    pub layout: PanelLayout,
    #[serde(default)]
    pub sections: Vec<ControlSection>,
    #[serde(default)]
    pub width: u16,
    #[serde(default = "default_true")]
    pub show_header: bool,
    #[serde(default)]
    pub collapsible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayout {
    #[default]
    Vertical,
    Horizontal,
    Grid,
}

// ── Control sections (tagged enum) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlSection {
    Motion(MotionSectionConfig),
    PresetButtons(PresetButtonsSectionConfig),
    CustomAction(CustomActionSectionConfig),
    Camera(CameraSectionConfig),
    Shutter(ShutterSectionConfig),
    Wavelength(WavelengthSectionConfig),
    Parameter(ParameterSectionConfig),
    StatusDisplay(StatusDisplaySectionConfig),
    Sensor(SensorSectionConfig),
    Separator(SeparatorConfig),
    Custom(CustomSectionConfig),
}

// ── Section configs ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MotionSectionConfig {
    #[serde(default = "default_motion_label")]
    pub label: String,
    #[serde(default = "default_true")]
    pub show_jog: bool,
    #[serde(default = "default_jog_steps")]
    pub jog_steps: Vec<f64>,
    #[serde(default)]
    pub show_home: bool,
    #[serde(default = "default_true")]
    pub show_stop: bool,
    #[serde(default = "default_precision")]
    pub precision: u8,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetButtonsSectionConfig {
    #[serde(default = "default_presets_label")]
    pub label: String,
    #[serde(default)]
    pub presets: Vec<PresetValue>,
    #[serde(default)]
    pub vertical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PresetValue {
    Number(f64),
    Labeled { label: String, value: f64 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomActionSectionConfig {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub style: ButtonStyle,
    #[serde(default)]
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
    Secondary,
    Danger,
    Success,
    Warning,
    Info,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CameraSectionConfig {
    #[serde(default = "default_camera_label")]
    pub label: String,
    #[serde(default = "default_true")]
    pub show_exposure: bool,
    #[serde(default)]
    pub show_gain: bool,
    #[serde(default)]
    pub show_binning: bool,
    #[serde(default)]
    pub show_roi: bool,
    #[serde(default)]
    pub show_histogram: bool,
    #[serde(default = "default_true")]
    pub show_stats: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShutterSectionConfig {
    #[serde(default = "default_shutter_label")]
    pub label: String,
    #[serde(default)]
    pub toggle_style: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WavelengthSectionConfig {
    #[serde(default = "default_wavelength_label")]
    pub label: String,
    #[serde(default = "default_true")]
    pub show_slider: bool,
    #[serde(default)]
    pub presets: Vec<f64>,
    #[serde(default = "default_true")]
    pub show_color: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterSectionConfig {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub parameter: String,
    #[serde(default)]
    pub widget: ParameterWidget,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub read_command: Option<String>,
    #[serde(default)]
    pub write_command: Option<String>,
    #[serde(default)]
    pub write_param: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterWidget {
    #[default]
    Auto,
    TextInput,
    Slider,
    Spinner,
    Toggle,
    Dropdown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusDisplaySectionConfig {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub parameters: Vec<String>,
    #[serde(default)]
    pub compact: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SensorSectionConfig {
    #[serde(default = "default_sensor_label")]
    pub label: String,
    #[serde(default = "default_precision")]
    pub precision: u8,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub show_trend: bool,
    #[serde(default)]
    pub refresh_ms: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeparatorConfig {
    #[serde(default)]
    pub height: u8,
    #[serde(default = "default_true")]
    pub visible: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomSectionConfig {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub widget: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusDisplayConfig {
    #[serde(default)]
    pub summary_params: Vec<String>,
    #[serde(default)]
    pub summary_format: Option<String>,
    #[serde(default = "default_true")]
    pub show_connection: bool,
}

// ── Default helpers ─────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}
fn default_precision() -> u8 {
    3
}
fn default_jog_steps() -> Vec<f64> {
    vec![0.1, 1.0, 10.0]
}
fn default_motion_label() -> String {
    "Position".to_string()
}
fn default_presets_label() -> String {
    "Presets".to_string()
}
fn default_camera_label() -> String {
    "Camera".to_string()
}
fn default_shutter_label() -> String {
    "Shutter".to_string()
}
fn default_wavelength_label() -> String {
    "Wavelength".to_string()
}
fn default_sensor_label() -> String {
    "Reading".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // JSON produced by serde_json::to_string() on the server-side hardware types.
    // If deserialization breaks here it means a serde attribute diverged.
    const IPG_LASER_UI_JSON: &str = r##"{
        "icon": "laser",
        "color": "#FF6B00",
        "control_panel": {
            "layout": "vertical",
            "show_header": true,
            "sections": [
                { "type": "sensor", "label": "Output Power", "precision": 2, "unit": "W", "refresh_ms": 500 },
                { "type": "separator", "visible": true },
                { "type": "custom_action", "label": "Emission ON", "command": "emission_on", "style": "success" },
                { "type": "custom_action", "label": "Emission OFF", "command": "emission_off", "style": "danger" },
                { "type": "separator", "visible": true },
                { "type": "parameter", "label": "Repetition Rate", "parameter": "rep_rate", "widget": "spinner", "read_only": false, "read_command": "read_rep_rate", "write_command": "set_rep_rate" },
                { "type": "status_display", "label": "Laser Status", "parameters": ["status", "error"], "compact": true }
            ]
        },
        "status_display": {
            "summary_params": ["power"],
            "summary_format": "{{ power }} W",
            "show_connection": true
        }
    }"##;

    #[test]
    fn test_ipg_laser_ui_config_deserializes() {
        let config: UiConfig = serde_json::from_str(IPG_LASER_UI_JSON).unwrap();
        assert_eq!(config.icon.as_deref(), Some("laser"));
        assert_eq!(config.color.as_deref(), Some("#FF6B00"));

        let panel = config.control_panel.as_ref().unwrap();
        assert_eq!(panel.layout, PanelLayout::Vertical);
        assert!(panel.show_header);
        assert_eq!(panel.sections.len(), 7);
    }

    #[test]
    fn test_section_types_deserialize_correctly() {
        let config: UiConfig = serde_json::from_str(IPG_LASER_UI_JSON).unwrap();
        let sections = &config.control_panel.unwrap().sections;

        assert!(matches!(&sections[0], ControlSection::Sensor(s) if s.label == "Output Power"));
        assert!(matches!(&sections[1], ControlSection::Separator(_)));
        assert!(
            matches!(&sections[2], ControlSection::CustomAction(a) if a.command == "emission_on")
        );
        assert!(
            matches!(&sections[3], ControlSection::CustomAction(a) if a.style == ButtonStyle::Danger)
        );
        assert!(
            matches!(&sections[5], ControlSection::Parameter(p) if p.read_command.as_deref() == Some("read_rep_rate"))
        );
        assert!(
            matches!(&sections[6], ControlSection::StatusDisplay(s) if s.parameters == ["status", "error"])
        );
    }

    #[test]
    fn test_sensor_defaults() {
        let json = r#"{ "type": "sensor", "label": "Power" }"#;
        let section: ControlSection = serde_json::from_str(json).unwrap();
        match section {
            ControlSection::Sensor(s) => {
                assert_eq!(s.label, "Power");
                assert_eq!(s.precision, 3); // default
                assert!(s.unit.is_none());
                assert!(!s.show_trend); // default false
            }
            _ => panic!("expected Sensor"),
        }
    }

    #[test]
    fn test_unknown_fields_are_ignored() {
        // The web schema must tolerate extra fields the server may add in future
        let json = r#"{
            "icon": "test",
            "future_field_unknown": "should be ignored",
            "control_panel": null
        }"#;
        let config: UiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.icon.as_deref(), Some("test"));
    }

    #[test]
    fn test_minimal_config_uses_defaults() {
        let json = r#"{}"#;
        let config: UiConfig = serde_json::from_str(json).unwrap();
        assert!(config.icon.is_none());
        assert!(config.control_panel.is_none());
    }
}
