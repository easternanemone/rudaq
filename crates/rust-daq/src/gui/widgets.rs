//! Parameter control widgets for dynamic UI generation.
//!
//! Auto-generates appropriate egui widgets based on parameter type and metadata.

use eframe::egui::{self, Response, Ui};

use super::types::{ParameterDescriptor, ParameterType};

/// State for tracking parameter value edits in immediate-mode UI.
#[derive(Clone, Default)]
pub struct ParameterEditState {
    /// Temporary string buffer for text input
    pub text_buffer: String,
    /// Temporary float value for sliders/drags
    pub float_value: f64,
    /// Temporary int value
    pub int_value: i64,
    /// Temporary bool value
    pub bool_value: bool,
    /// Currently selected enum index
    pub enum_index: usize,
    /// Whether the state has been initialized from the parameter
    pub initialized: bool,
}

impl ParameterEditState {
    /// Initialize state from a parameter descriptor's current value.
    pub fn init_from_param(&mut self, param: &ParameterDescriptor) {
        if self.initialized {
            return;
        }

        let value = param.current_value.as_deref().unwrap_or("");

        match param.dtype {
            ParameterType::Float => {
                self.float_value = value.parse().unwrap_or(param.min_value.unwrap_or(0.0));
            }
            ParameterType::Int => {
                self.int_value = value.parse().unwrap_or(0);
            }
            ParameterType::Bool => {
                self.bool_value = value.parse().unwrap_or(false)
                    || value.eq_ignore_ascii_case("true")
                    || value == "1";
            }
            ParameterType::String => {
                self.text_buffer = value.to_string();
            }
            ParameterType::Enum => {
                self.enum_index = param
                    .enum_values
                    .iter()
                    .position(|v| v == value)
                    .unwrap_or(0);
            }
        }

        self.initialized = true;
    }

    /// Reset initialization flag (call when parameter value changes externally).
    pub fn reset(&mut self) {
        self.initialized = false;
    }
}

/// Result of rendering a parameter widget.
pub enum WidgetResult {
    /// No change
    NoChange,
    /// Value changed but not committed (e.g., dragging slider)
    Changed(String),
    /// Value committed (e.g., released slider, pressed enter)
    Committed(String),
}

/// Render a parameter control widget based on its type.
///
/// Returns `Some(new_value)` if the user committed a change.
pub fn parameter_widget(
    ui: &mut Ui,
    param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    // Initialize state if needed
    state.init_from_param(param);

    // Show read-only values differently
    if !param.writable {
        return render_readonly(ui, param);
    }

    match param.dtype {
        ParameterType::Float => render_float(ui, param, state),
        ParameterType::Int => render_int(ui, param, state),
        ParameterType::Bool => render_bool(ui, param, state),
        ParameterType::String => render_string(ui, param, state),
        ParameterType::Enum => render_enum(ui, param, state),
    }
}

/// Render a read-only parameter value.
fn render_readonly(ui: &mut Ui, param: &ParameterDescriptor) -> WidgetResult {
    let value = param.current_value.as_deref().unwrap_or("-");
    let text = if param.units.is_empty() {
        value.to_string()
    } else {
        format!("{} {}", value, param.units)
    };
    ui.label(text);
    WidgetResult::NoChange
}

/// Render a float parameter with slider or drag value.
fn render_float(
    ui: &mut Ui,
    param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    let min = param.min_value.unwrap_or(f64::MIN);
    let max = param.max_value.unwrap_or(f64::MAX);

    let response: Response;

    // Use slider if we have a reasonable range
    if param.min_value.is_some() && param.max_value.is_some() {
        response = ui.add(
            egui::Slider::new(&mut state.float_value, min..=max)
                .text(&param.units)
                .clamp_to_range(true),
        );
    } else {
        // Use drag value for unbounded ranges
        response = ui.add(
            egui::DragValue::new(&mut state.float_value)
                .speed(0.1)
                .suffix(format!(" {}", param.units)),
        );
    }

    if response.drag_stopped() || response.lost_focus() {
        WidgetResult::Committed(state.float_value.to_string())
    } else if response.changed() {
        WidgetResult::Changed(state.float_value.to_string())
    } else {
        WidgetResult::NoChange
    }
}

/// Render an integer parameter.
fn render_int(
    ui: &mut Ui,
    param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    let response = ui.add(
        egui::DragValue::new(&mut state.int_value)
            .speed(1.0)
            .suffix(format!(" {}", param.units)),
    );

    if response.drag_stopped() || response.lost_focus() {
        WidgetResult::Committed(state.int_value.to_string())
    } else if response.changed() {
        WidgetResult::Changed(state.int_value.to_string())
    } else {
        WidgetResult::NoChange
    }
}

/// Render a boolean parameter as checkbox.
fn render_bool(
    ui: &mut Ui,
    _param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    let response = ui.checkbox(&mut state.bool_value, "");

    if response.changed() {
        WidgetResult::Committed(state.bool_value.to_string())
    } else {
        WidgetResult::NoChange
    }
}

/// Render a string parameter as text edit.
fn render_string(
    ui: &mut Ui,
    _param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    let response = ui.text_edit_singleline(&mut state.text_buffer);

    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        WidgetResult::Committed(state.text_buffer.clone())
    } else if response.changed() {
        WidgetResult::Changed(state.text_buffer.clone())
    } else {
        WidgetResult::NoChange
    }
}

/// Render an enum parameter as combo box.
fn render_enum(
    ui: &mut Ui,
    param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    if param.enum_values.is_empty() {
        ui.label("(no options)");
        return WidgetResult::NoChange;
    }

    let current = param
        .enum_values
        .get(state.enum_index)
        .map(|s| s.as_str())
        .unwrap_or("-");

    let mut changed = false;

    egui::ComboBox::from_id_salt(&param.name)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (idx, value) in param.enum_values.iter().enumerate() {
                if ui
                    .selectable_value(&mut state.enum_index, idx, value)
                    .changed()
                {
                    changed = true;
                }
            }
        });

    if changed {
        let new_value = param
            .enum_values
            .get(state.enum_index)
            .cloned()
            .unwrap_or_default();
        WidgetResult::Committed(new_value)
    } else {
        WidgetResult::NoChange
    }
}

/// Render a collapsible parameter group.
pub fn parameter_group(
    ui: &mut Ui,
    group_name: &str,
    params: &[ParameterDescriptor],
    states: &mut std::collections::HashMap<String, ParameterEditState>,
    mut on_change: impl FnMut(&str, &str, String),
) {
    egui::CollapsingHeader::new(group_name)
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new(format!("param_grid_{}", group_name))
                .striped(true)
                .num_columns(3)
                .show(ui, |ui| {
                    for param in params {
                        // Parameter name with tooltip
                        let name_label = ui.label(&param.name);
                        if !param.description.is_empty() {
                            name_label.on_hover_text(&param.description);
                        }

                        // Get or create edit state for this parameter
                        let state = states
                            .entry(format!("{}:{}", param.device_id, param.name))
                            .or_default();

                        // Render the appropriate widget
                        match parameter_widget(ui, param, state) {
                            WidgetResult::Committed(value) => {
                                on_change(&param.device_id, &param.name, value);
                                state.reset(); // Allow re-init from server value
                            }
                            WidgetResult::Changed(_) => {
                                // Could show pending indicator
                            }
                            WidgetResult::NoChange => {}
                        }

                        // Read-only indicator
                        if !param.writable {
                            ui.label("🔒");
                        } else {
                            ui.label("");
                        }

                        ui.end_row();
                    }
                });
        });
}
