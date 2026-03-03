//! Runtime parameter editor widget for modifying device parameters during paused execution.
//!
//! This module provides widgets for editing device parameters at runtime while an experiment
//! is paused. Unlike the `parameter_editor` module which handles device parameter introspection,
//! this module provides lightweight editing for parameters extracted from graph nodes.

use egui::Ui;

/// A parameter that can be edited at runtime during paused execution
#[derive(Clone, Debug)]
pub struct EditableParameter {
    /// Device ID this parameter belongs to
    pub device_id: String,
    /// Parameter name (e.g., "exposure_ms", "position")
    pub name: String,
    /// Display label
    pub label: String,
    /// Current value (as string for flexibility)
    pub value: String,
    /// Parameter type hint for appropriate editor
    pub param_type: ParameterType,
    /// Optional range for numeric parameters
    pub range: Option<(f64, f64)>,
}

/// Type of parameter for appropriate editor selection
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ParameterType {
    Float,
    Integer,
    String,
    Boolean,
}

/// Result of parameter editing
pub enum RuntimeParameterEditResult {
    /// No change
    NoChange,
    /// Value was modified, needs to be sent to device
    Modified {
        device_id: String,
        param_name: String,
        new_value: String,
    },
}

/// Widget for editing parameters while paused
pub struct RuntimeParameterEditor;

impl RuntimeParameterEditor {
    /// Show parameter editor UI
    ///
    /// Returns `Modified` if user changed a value, `NoChange` otherwise.
    #[allow(clippy::cast_possible_truncation)]
    pub fn show(
        ui: &mut Ui,
        param: &mut EditableParameter,
        enabled: bool,
    ) -> RuntimeParameterEditResult {
        ui.horizontal(|ui| {
            ui.label(&param.label);
            ui.label(":");

            let response = match param.param_type {
                ParameterType::Float => {
                    let mut value: f64 = param.value.parse().unwrap_or(0.0);
                    let response = if let Some((min, max)) = param.range {
                        ui.add_enabled(enabled, egui::Slider::new(&mut value, min..=max).text(""))
                    } else {
                        ui.add_enabled(enabled, egui::DragValue::new(&mut value).speed(0.1))
                    };
                    if response.changed() {
                        param.value = value.to_string();
                    }
                    response
                }
                ParameterType::Integer => {
                    let mut value: i64 = param.value.parse().unwrap_or(0);
                    let response = if let Some((min, max)) = param.range {
                        ui.add_enabled(
                            enabled,
                            egui::Slider::new(&mut value, min as i64..=max as i64).text(""),
                        )
                    } else {
                        ui.add_enabled(enabled, egui::DragValue::new(&mut value))
                    };
                    if response.changed() {
                        param.value = value.to_string();
                    }
                    response
                }
                ParameterType::String => {
                    let response = ui.add_enabled(
                        enabled,
                        egui::TextEdit::singleline(&mut param.value).desired_width(100.0),
                    );
                    response
                }
                ParameterType::Boolean => {
                    let mut value: bool = param.value.parse().unwrap_or(false);
                    let response =
                        ui.add_enabled(enabled, egui::Checkbox::without_text(&mut value));
                    if response.changed() {
                        param.value = value.to_string();
                    }
                    response
                }
            };

            if response.changed() {
                return RuntimeParameterEditResult::Modified {
                    device_id: param.device_id.clone(),
                    param_name: param.name.clone(),
                    new_value: param.value.clone(),
                };
            }

            RuntimeParameterEditResult::NoChange
        })
        .inner
    }

    /// Show a group of parameters with a header
    pub fn show_group(
        ui: &mut Ui,
        title: &str,
        params: &mut [EditableParameter],
        enabled: bool,
    ) -> Vec<RuntimeParameterEditResult> {
        let mut results = Vec::new();

        if params.is_empty() {
            ui.label("No editable parameters");
            return results;
        }

        ui.group(|ui| {
            ui.heading(title);
            ui.separator();

            for param in params.iter_mut() {
                let result = Self::show(ui, param, enabled);
                results.push(result);
            }
        });

        results
    }
}

/// Helper constructors for common parameter types
impl EditableParameter {
    /// Create a float parameter
    pub fn float(device_id: &str, name: &str, label: &str, value: f64) -> Self {
        Self {
            device_id: device_id.to_string(),
            name: name.to_string(),
            label: label.to_string(),
            value: value.to_string(),
            param_type: ParameterType::Float,
            range: None,
        }
    }

    /// Create a float parameter with range constraints
    pub fn float_ranged(
        device_id: &str,
        name: &str,
        label: &str,
        value: f64,
        min: f64,
        max: f64,
    ) -> Self {
        Self {
            device_id: device_id.to_string(),
            name: name.to_string(),
            label: label.to_string(),
            value: value.to_string(),
            param_type: ParameterType::Float,
            range: Some((min, max)),
        }
    }

    /// Create an integer parameter
    #[allow(dead_code)]
    pub fn integer(device_id: &str, name: &str, label: &str, value: i64) -> Self {
        Self {
            device_id: device_id.to_string(),
            name: name.to_string(),
            label: label.to_string(),
            value: value.to_string(),
            param_type: ParameterType::Integer,
            range: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_creation() {
        let param = EditableParameter::float("cam1", "exposure_ms", "Exposure (ms)", 100.0);
        assert_eq!(param.device_id, "cam1");
        assert_eq!(param.name, "exposure_ms");
        assert_eq!(param.value, "100");
        assert_eq!(param.param_type, ParameterType::Float);
    }

    #[test]
    fn test_float_ranged_parameter() {
        let param =
            EditableParameter::float_ranged("stage1", "position", "Position", 50.0, 0.0, 100.0);
        assert_eq!(param.range, Some((0.0, 100.0)));
    }

    #[test]
    fn test_integer_parameter() {
        let param = EditableParameter::integer("cam1", "binning", "Binning", 2);
        assert_eq!(param.value, "2");
        assert_eq!(param.param_type, ParameterType::Integer);
    }
}
