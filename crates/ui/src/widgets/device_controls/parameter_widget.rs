#![allow(dead_code)]
//! Generic parameter widget for device control panels.
//!
//! `ParameterWidget` encapsulates the common pattern of:
//! display current value -> render type-appropriate editor -> return action for caller.
//!
//! The widget is a pure rendering component: it does **not** own async state or gRPC
//! clients. The calling panel is responsible for dispatching [`ParameterAction`] results
//! (spawning gRPC calls, updating cached values, etc.).

use egui::Ui;
use protocol::daq::ParameterDescriptor;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Describes what kind of editing widget to render for a parameter.
#[derive(Debug, Clone)]
pub enum ParameterType {
    /// Numeric with optional min/max/step and unit label.
    Numeric {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        unit: Option<String>,
    },
    /// String enumeration with a fixed set of choices.
    Enumeration { choices: Vec<String> },
    /// Free-form text input.
    Text,
    /// Boolean on/off toggle.
    Boolean,
    /// Type could not be determined -- render as read-only label.
    Unknown,
}

/// An action the calling panel should perform after the widget is shown.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterAction {
    /// User committed a new value (always serialised as `String` for gRPC).
    SetValue(String),
    /// User requested an explicit refresh of the current value.
    Refresh,
    /// No user interaction requiring action.
    None,
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// A self-contained, type-aware widget for a single device parameter.
///
/// Renders a label + value editor appropriate to [`ParameterType`] and returns a
/// [`ParameterAction`] indicating what the caller should do. The widget does not
/// perform any I/O itself.
pub struct ParameterWidget<'a> {
    /// Human-readable label (usually the parameter name).
    label: &'a str,
    /// Current value from the last fetch, rendered as display text and used as the
    /// starting point for edits.
    current_value: Option<&'a str>,
    /// Determines which editor widget is shown.
    param_type: &'a ParameterType,
    /// When `true`, the widget shows a spinner and disables editing.
    is_busy: bool,
    /// Stable egui ID salt to disambiguate multiple widgets in the same panel.
    id_salt: &'a str,
}

impl<'a> ParameterWidget<'a> {
    /// Create a new `ParameterWidget`.
    ///
    /// * `label`         -- display label (typically the parameter name).
    /// * `current_value` -- last-known value string, or `None` if not yet fetched.
    /// * `param_type`    -- determines the editor variant.
    /// * `is_busy`       -- set `true` while a set/get is in-flight.
    /// * `id_salt`       -- unique string for egui ID generation (e.g. `"device::param"`).
    #[must_use]
    pub fn new(
        label: &'a str,
        current_value: Option<&'a str>,
        param_type: &'a ParameterType,
        is_busy: bool,
        id_salt: &'a str,
    ) -> Self {
        Self {
            label,
            current_value,
            param_type,
            is_busy,
            id_salt,
        }
    }

    /// Render the widget and return any resulting action.
    pub fn show(&self, ui: &mut Ui) -> ParameterAction {
        let mut action = ParameterAction::None;

        ui.horizontal(|ui| {
            // -- label --
            ui.label(self.label);

            if self.is_busy {
                ui.spinner();
                return;
            }

            match self.param_type {
                ParameterType::Numeric {
                    min,
                    max,
                    step,
                    unit,
                } => {
                    action = self.show_numeric(ui, *min, *max, *step, unit.as_deref());
                }
                ParameterType::Enumeration { choices } => {
                    action = self.show_enum(ui, choices);
                }
                ParameterType::Boolean => {
                    action = self.show_boolean(ui);
                }
                ParameterType::Text => {
                    action = self.show_text(ui);
                }
                ParameterType::Unknown => {
                    // Read-only display
                    let display = self.current_value.unwrap_or("-");
                    ui.label(display);
                }
            }

            // Refresh button (small, unobtrusive)
            if ui
                .small_button("\u{21bb}")
                .on_hover_text("Refresh")
                .clicked()
            {
                action = ParameterAction::Refresh;
            }
        });

        action
    }

    // -- private editor variants --

    fn show_numeric(
        &self,
        ui: &mut Ui,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        unit: Option<&str>,
    ) -> ParameterAction {
        let current = self
            .current_value
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        // We need mutable state for DragValue; use egui's temp storage keyed by id_salt.
        let temp_id = egui::Id::new(self.id_salt).with("_numeric_tmp");
        let mut value: f64 = ui
            .ctx()
            .data_mut(|d| d.get_temp(temp_id))
            .unwrap_or(current);

        let mut drag = egui::DragValue::new(&mut value).speed(step.unwrap_or(0.1));
        if let (Some(lo), Some(hi)) = (min, max) {
            drag = drag.range(lo..=hi);
        }
        if let Some(u) = unit {
            drag = drag.suffix(format!(" {u}"));
        }

        let response = ui.add(drag);

        // Persist the temp value so it survives frames while dragging.
        ui.ctx().data_mut(|d| d.insert_temp(temp_id, value));

        // Commit on drag release or lost focus (Enter key)
        if response.drag_stopped() || (response.lost_focus() && !response.dragged()) {
            // Only emit SetValue if it actually changed from the current display value
            #[allow(clippy::float_cmp)]
            if value != current {
                return ParameterAction::SetValue(value.to_string());
            }
        }

        ParameterAction::None
    }

    fn show_enum(&self, ui: &mut Ui, choices: &[String]) -> ParameterAction {
        let current = self.current_value.unwrap_or("");

        // Use temp storage for the selected index
        let temp_id = egui::Id::new(self.id_salt).with("_enum_idx");
        let initial_idx = choices
            .iter()
            .position(|c| c.eq_ignore_ascii_case(current))
            .unwrap_or(0);
        let mut selected: usize = ui
            .ctx()
            .data_mut(|d| d.get_temp(temp_id))
            .unwrap_or(initial_idx);

        // Clamp in case choices changed
        if selected >= choices.len() {
            selected = 0;
        }

        let combo_id = egui::Id::new(self.id_salt).with("_enum_combo");
        let mut changed = false;
        egui::ComboBox::from_id_salt(combo_id)
            .selected_text(choices.get(selected).map_or("-", String::as_str))
            .width(120.0)
            .show_ui(ui, |ui| {
                for (i, choice) in choices.iter().enumerate() {
                    if ui.selectable_value(&mut selected, i, choice).changed() {
                        changed = true;
                    }
                }
            });

        ui.ctx().data_mut(|d| d.insert_temp(temp_id, selected));

        if changed && let Some(choice) = choices.get(selected) {
            return ParameterAction::SetValue(choice.clone());
        }

        ParameterAction::None
    }

    fn show_boolean(&self, ui: &mut Ui) -> ParameterAction {
        let current = self.current_value.map(parse_bool_value).unwrap_or(false);

        let temp_id = egui::Id::new(self.id_salt).with("_bool_tmp");
        let mut value: bool = ui
            .ctx()
            .data_mut(|d| d.get_temp(temp_id))
            .unwrap_or(current);

        if ui.checkbox(&mut value, "").changed() {
            ui.ctx().data_mut(|d| d.insert_temp(temp_id, value));
            return ParameterAction::SetValue(value.to_string());
        }

        ui.ctx().data_mut(|d| d.insert_temp(temp_id, value));
        ParameterAction::None
    }

    fn show_text(&self, ui: &mut Ui) -> ParameterAction {
        let current = self.current_value.unwrap_or("");

        let temp_id = egui::Id::new(self.id_salt).with("_text_buf");
        let mut buf: String = ui
            .ctx()
            .data_mut(|d| d.get_temp(temp_id))
            .unwrap_or_else(|| current.to_string());

        let response = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(140.0)
                .hint_text("value"),
        );

        ui.ctx().data_mut(|d| d.insert_temp(temp_id, buf.clone()));

        // Commit on Enter / lost focus
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && buf != current
        {
            return ParameterAction::SetValue(buf);
        }

        ParameterAction::None
    }
}

// ---------------------------------------------------------------------------
// Inference helpers
// ---------------------------------------------------------------------------

/// Infer the [`ParameterType`] from a [`ParameterDescriptor`].
///
/// Resolution order:
/// 1. If enum values are present (descriptor or metadata), use `Enumeration`.
/// 2. If `dtype` contains "bool", use `Boolean`.
/// 3. If `dtype` contains "float", "int", or "double", or numeric range is set, use `Numeric`.
/// 4. If `dtype` contains "string" or "str", use `Text`.
/// 5. Otherwise `Unknown`.
#[must_use]
pub fn infer_parameter_type(descriptor: &ParameterDescriptor) -> ParameterType {
    let enum_values = super::parameter_enum_values(descriptor);
    if !enum_values.is_empty() {
        return ParameterType::Enumeration {
            choices: enum_values,
        };
    }

    // Merge dtype from top-level and metadata
    let dtype = if descriptor.dtype.is_empty() {
        descriptor
            .metadata
            .as_ref()
            .map(|m| m.dtype.as_str())
            .unwrap_or("")
    } else {
        descriptor.dtype.as_str()
    };
    let dtype_lower = dtype.to_ascii_lowercase();

    if dtype_lower.contains("bool") {
        return ParameterType::Boolean;
    }

    if dtype_lower.contains("float")
        || dtype_lower.contains("double")
        || dtype_lower.contains("int")
        || dtype_lower.contains("number")
    {
        return build_numeric_type(descriptor);
    }

    // Even without a dtype hint, if numeric bounds exist, treat as numeric.
    if super::parameter_numeric_range(descriptor).is_some() {
        return build_numeric_type(descriptor);
    }

    if dtype_lower.contains("str") {
        return ParameterType::Text;
    }

    ParameterType::Unknown
}

/// Infer [`ParameterType`] from a raw value string when no descriptor is available.
///
/// Tries numeric parse first, then bool, then falls back to `Text`.
#[must_use]
pub fn infer_parameter_type_from_value(value: &str) -> ParameterType {
    if parse_numeric(value).is_some() {
        return ParameterType::Numeric {
            min: None,
            max: None,
            step: None,
            unit: None,
        };
    }

    let lower = value.to_ascii_lowercase();
    if lower == "true" || lower == "false" {
        return ParameterType::Boolean;
    }

    ParameterType::Text
}

/// Try parsing a string as `f64`.
#[must_use]
pub fn parse_numeric(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_numeric_type(descriptor: &ParameterDescriptor) -> ParameterType {
    let range = super::parameter_numeric_range(descriptor);
    let step = descriptor.metadata.as_ref().and_then(|m| m.step);

    let unit_str = if !descriptor.units.is_empty() {
        Some(descriptor.units.clone())
    } else {
        descriptor
            .metadata
            .as_ref()
            .filter(|m| !m.units.is_empty())
            .map(|m| m.units.clone())
    };

    ParameterType::Numeric {
        min: range.map(|(lo, _)| lo),
        max: range.map(|(_, hi)| hi),
        step,
        unit: unit_str,
    }
}

/// Loosely parse a boolean from a value string.
fn parse_bool_value(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "true" | "1" | "on" | "yes" | "enabled")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::daq::{ParameterDescriptor, ParameterMetadata};

    fn empty_descriptor() -> ParameterDescriptor {
        ParameterDescriptor {
            device_id: String::new(),
            name: String::new(),
            description: String::new(),
            dtype: String::new(),
            units: String::new(),
            readable: true,
            writable: true,
            min_value: None,
            max_value: None,
            enum_values: Vec::new(),
            metadata: None,
            group_name: None,
        }
    }

    #[test]
    fn test_infer_enum_from_descriptor() {
        let mut desc = empty_descriptor();
        desc.enum_values = vec!["Internal".into(), "External".into()];
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Enumeration { ref choices } if choices.len() == 2),
            "expected Enumeration, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_bool_from_dtype() {
        let mut desc = empty_descriptor();
        desc.dtype = "bool".into();
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Boolean),
            "expected Boolean, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_numeric_from_dtype() {
        let mut desc = empty_descriptor();
        desc.dtype = "float".into();
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Numeric { .. }),
            "expected Numeric, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_numeric_from_range() {
        let mut desc = empty_descriptor();
        desc.min_value = Some(0.0);
        desc.max_value = Some(100.0);
        let ty = infer_parameter_type(&desc);
        match &ty {
            ParameterType::Numeric { min, max, .. } => {
                assert_eq!(*min, Some(0.0));
                assert_eq!(*max, Some(100.0));
            }
            other => panic!("expected Numeric, got {other:?}"),
        }
    }

    #[test]
    fn test_infer_text_from_dtype() {
        let mut desc = empty_descriptor();
        desc.dtype = "string".into();
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Text),
            "expected Text, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_unknown_empty_descriptor() {
        let desc = empty_descriptor();
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Unknown),
            "expected Unknown, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_numeric_with_metadata() {
        let mut desc = empty_descriptor();
        desc.metadata = Some(ParameterMetadata {
            min_value: Some(1.0),
            max_value: Some(50.0),
            step: Some(0.5),
            units: "ms".into(),
            dtype: "float".into(),
            ..Default::default()
        });
        let ty = infer_parameter_type(&desc);
        match &ty {
            ParameterType::Numeric {
                min,
                max,
                step,
                unit,
            } => {
                assert_eq!(*min, Some(1.0));
                assert_eq!(*max, Some(50.0));
                assert_eq!(*step, Some(0.5));
                assert_eq!(unit.as_deref(), Some("ms"));
            }
            other => panic!("expected Numeric, got {other:?}"),
        }
    }

    #[test]
    fn test_infer_enum_from_metadata() {
        let mut desc = empty_descriptor();
        desc.metadata = Some(ParameterMetadata {
            enum_values: vec!["A".into(), "B".into(), "C".into()],
            ..Default::default()
        });
        let ty = infer_parameter_type(&desc);
        assert!(
            matches!(ty, ParameterType::Enumeration { ref choices } if choices.len() == 3),
            "expected Enumeration with 3 choices, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_from_value_numeric() {
        let ty = infer_parameter_type_from_value("3.14");
        assert!(
            matches!(ty, ParameterType::Numeric { .. }),
            "expected Numeric, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_from_value_bool() {
        let ty = infer_parameter_type_from_value("true");
        assert!(
            matches!(ty, ParameterType::Boolean),
            "expected Boolean, got {ty:?}"
        );
    }

    #[test]
    fn test_infer_from_value_text() {
        let ty = infer_parameter_type_from_value("hello world");
        assert!(
            matches!(ty, ParameterType::Text),
            "expected Text, got {ty:?}"
        );
    }

    #[test]
    fn test_parse_numeric() {
        assert_eq!(parse_numeric("42"), Some(42.0));
        assert_eq!(parse_numeric("  -3.5  "), Some(-3.5));
        assert_eq!(parse_numeric("not a number"), None);
        assert_eq!(parse_numeric(""), None);
    }

    #[test]
    fn test_parse_bool_value() {
        assert!(parse_bool_value("true"));
        assert!(parse_bool_value("True"));
        assert!(parse_bool_value("1"));
        assert!(parse_bool_value("on"));
        assert!(parse_bool_value("yes"));
        assert!(parse_bool_value("enabled"));
        assert!(!parse_bool_value("false"));
        assert!(!parse_bool_value("0"));
        assert!(!parse_bool_value("off"));
        assert!(!parse_bool_value("garbage"));
    }

    #[test]
    fn test_parameter_action_equality() {
        assert_eq!(ParameterAction::None, ParameterAction::None);
        assert_eq!(ParameterAction::Refresh, ParameterAction::Refresh);
        assert_eq!(
            ParameterAction::SetValue("42".into()),
            ParameterAction::SetValue("42".into())
        );
        assert_ne!(ParameterAction::None, ParameterAction::Refresh);
    }
}
