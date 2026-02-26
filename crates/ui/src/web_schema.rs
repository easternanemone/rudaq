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
