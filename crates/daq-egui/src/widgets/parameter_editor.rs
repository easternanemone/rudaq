//! Parameter editor widgets for the DAQ GUI (bd-cdh5.1).
//!
//! Provides type-specific widgets for editing device parameters:
//! - BoolEditor: checkbox for boolean values
//! - NumericEditor: DragValue for numeric values (int/float)
//! - StringEditor: TextEdit for string values
//! - EnumEditor: ComboBox for enum values
//! - JsonFallback: read-only display for complex types

use eframe::egui;

/// Cached parameter value with editing state
#[derive(Clone)]
pub struct ParameterCache {
    pub descriptor: daq_proto::daq::ParameterDescriptor,
    pub current_value: String,
    pub edit_buffer: String,
    pub is_editing: bool,
    pub last_error: Option<String>,
}

impl ParameterCache {
    pub fn new(descriptor: daq_proto::daq::ParameterDescriptor, value: String) -> Self {
        Self {
            descriptor,
            current_value: value.clone(),
            edit_buffer: value,
            is_editing: false,
            last_error: None,
        }
    }

    /// Update the current value (called after successful server response)
    pub fn update_value(&mut self, value: String) {
        self.current_value = value.clone();
        if !self.is_editing {
            self.edit_buffer = value;
        }
        self.last_error = None;
    }

    /// Set error state
    #[allow(dead_code)]
    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
    }
}

/// Result of rendering a parameter editor
#[allow(dead_code)]
pub enum ParameterEditResult {
    /// No change
    None,
    /// Value changed, request server update
    Changed(String),
}

/// Render the appropriate editor widget for a parameter
#[allow(dead_code)]
pub fn render_parameter_editor(
    ui: &mut egui::Ui,
    param: &mut ParameterCache,
) -> ParameterEditResult {
    let desc = &param.descriptor;

    // Show read-only indicator if not writable
    if !desc.writable {
        ui.horizontal(|ui| {
            ui.label(&desc.name);
            ui.label(format!(": {}", param.current_value));
            if !desc.units.is_empty() {
                ui.weak(&desc.units);
            }
        });
        return ParameterEditResult::None;
    }

    // Check for enum values first (takes precedence over dtype)
    if !desc.enum_values.is_empty() {
        return render_enum_editor(ui, param);
    }

    // Render based on dtype
    match desc.dtype.as_str() {
        "bool" => render_bool_editor(ui, param),
        "int" => render_int_editor(ui, param),
        "float" => render_float_editor(ui, param),
        "string" => render_string_editor(ui, param),
        _ => render_json_fallback(ui, param),
    }
}

/// Render a checkbox for boolean parameters
#[allow(dead_code)]
fn render_bool_editor(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    let mut value = param.current_value.parse::<bool>().unwrap_or(false);
    let old_value = value;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut value, &param.descriptor.name).changed() && value != old_value {
            param.edit_buffer = value.to_string();
        }
        if let Some(err) = &param.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }
    });

    if value != old_value {
        ParameterEditResult::Changed(value.to_string())
    } else {
        ParameterEditResult::None
    }
}

/// Render a Slider or DragValue for integer parameters
/// Uses Slider when both min/max are available (bd-cdh5.2), DragValue otherwise.
#[allow(dead_code)]
fn render_int_editor(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    let mut value = param.edit_buffer.parse::<i64>().unwrap_or(0);
    let original = param.current_value.parse::<i64>().unwrap_or(0);
    let mut result = ParameterEditResult::None;

    ui.horizontal(|ui| {
        ui.label(&param.descriptor.name);

        // Use Slider when both bounds are available (Phase 2: bd-cdh5.2)
        let response = if let (Some(min), Some(max)) =
            (param.descriptor.min_value, param.descriptor.max_value)
        {
            let min_i = min as i64;
            let max_i = max as i64;
            ui.add(egui::Slider::new(&mut value, min_i..=max_i))
        } else {
            // Fallback to DragValue for unbounded parameters
            let mut drag = egui::DragValue::new(&mut value).speed(1);
            if let Some(min) = param.descriptor.min_value {
                drag = drag.range(min as i64..=i64::MAX);
            }
            if let Some(max) = param.descriptor.max_value {
                drag = drag.range(i64::MIN..=max as i64);
            }
            ui.add(drag)
        };

        if !param.descriptor.units.is_empty() {
            ui.weak(&param.descriptor.units);
        }

        if let Some(err) = &param.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }

        // Commit on focus lost or change (slider changes immediately)
        if (response.lost_focus() || response.changed()) && value != original {
            param.edit_buffer = value.to_string();
            result = ParameterEditResult::Changed(value.to_string());
        }
    });

    result
}

/// Render a Slider or DragValue for float parameters
/// Uses Slider when both min/max are available (bd-cdh5.2), DragValue otherwise.
#[allow(dead_code)]
fn render_float_editor(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    let mut value = param.edit_buffer.parse::<f64>().unwrap_or(0.0);
    let original = param.current_value.parse::<f64>().unwrap_or(0.0);

    let mut result = ParameterEditResult::None;

    ui.horizontal(|ui| {
        ui.label(&param.descriptor.name);

        // Use Slider when both bounds are available (Phase 2: bd-cdh5.2)
        let response = if let (Some(min), Some(max)) =
            (param.descriptor.min_value, param.descriptor.max_value)
        {
            ui.add(egui::Slider::new(&mut value, min..=max))
        } else {
            // Fallback to DragValue for unbounded parameters
            let mut drag = egui::DragValue::new(&mut value).speed(0.01);
            if let Some(min) = param.descriptor.min_value {
                drag = drag.range(min..=f64::MAX);
            }
            if let Some(max) = param.descriptor.max_value {
                drag = drag.range(f64::MIN..=max);
            }
            ui.add(drag)
        };

        if !param.descriptor.units.is_empty() {
            ui.weak(&param.descriptor.units);
        }

        if let Some(err) = &param.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }

        // Commit on focus lost or change (slider changes immediately)
        if (response.lost_focus() || response.changed()) && (value - original).abs() > f64::EPSILON
        {
            param.edit_buffer = value.to_string();
            result = ParameterEditResult::Changed(value.to_string());
        }
    });

    result
}

/// Render a TextEdit for string parameters
#[allow(dead_code)]
fn render_string_editor(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    let original = param.current_value.clone();
    let mut result = ParameterEditResult::None;

    ui.horizontal(|ui| {
        ui.label(&param.descriptor.name);
        let response = ui.text_edit_singleline(&mut param.edit_buffer);

        if let Some(err) = &param.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }

        // Commit on Enter or focus lost
        if response.lost_focus() && param.edit_buffer != original {
            result = ParameterEditResult::Changed(format!("\"{}\"", param.edit_buffer));
        }
    });

    result
}

/// Render a ComboBox for enum parameters
#[allow(dead_code)]
fn render_enum_editor(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    let mut selected = param.current_value.trim_matches('"').to_string();
    let original = selected.clone();
    let mut result = ParameterEditResult::None;

    ui.horizontal(|ui| {
        ui.label(&param.descriptor.name);

        let combo_id = egui::Id::new(&param.descriptor.name).with("combo");
        egui::ComboBox::from_id_salt(combo_id)
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                for option in &param.descriptor.enum_values {
                    if ui.selectable_value(&mut selected, option.clone(), option).clicked() {
                        param.edit_buffer = format!("\"{}\"", selected);
                    }
                }
            });

        if let Some(err) = &param.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }
    });

    if selected != original {
        result = ParameterEditResult::Changed(format!("\"{}\"", selected));
    }

    result
}

/// Render a read-only display for complex/unknown types
#[allow(dead_code)]
fn render_json_fallback(ui: &mut egui::Ui, param: &mut ParameterCache) -> ParameterEditResult {
    ui.horizontal(|ui| {
        ui.label(&param.descriptor.name);
        ui.label(format!(": {}", param.current_value));
        if !param.descriptor.units.is_empty() {
            ui.weak(&param.descriptor.units);
        }
        if !param.descriptor.writable {
            ui.weak("(read-only)");
        }
    });

    ParameterEditResult::None
}

/// Group parameters by prefix (e.g., "thermal.temperature" -> "thermal")
pub fn group_parameters_by_prefix(params: &[ParameterCache]) -> Vec<(String, Vec<&ParameterCache>)> {
    use std::collections::BTreeMap;

    let mut groups: BTreeMap<String, Vec<&ParameterCache>> = BTreeMap::new();

    for param in params {
        let group = if let Some(dot_pos) = param.descriptor.name.find('.') {
            param.descriptor.name[..dot_pos].to_string()
        } else {
            "general".to_string()
        };

        groups.entry(group).or_default().push(param);
    }

    groups.into_iter().collect()
}

/// Filter parameters by search query
pub fn filter_parameters<'a>(params: &'a [ParameterCache], query: &str) -> Vec<&'a ParameterCache> {
    if query.is_empty() {
        return params.iter().collect();
    }

    let query_lower = query.to_lowercase();
    params
        .iter()
        .filter(|p| {
            p.descriptor.name.to_lowercase().contains(&query_lower)
                || p.descriptor.description.to_lowercase().contains(&query_lower)
        })
        .collect()
}
