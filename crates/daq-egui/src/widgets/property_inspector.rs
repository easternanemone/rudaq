//! Property inspector widget for editing node properties.

use egui::Ui;

use crate::graph::ExperimentNode;
use crate::widgets::DeviceSelector;

/// Property inspector for editing selected node properties.
///
/// Shows editable fields for the selected node type and returns
/// the modified node if any changes were made.
pub struct PropertyInspector;

impl PropertyInspector {
    /// Show properties for a node. Returns `Some(modified_node)` if user made changes.
    ///
    /// # Arguments
    /// - `ui` - The egui UI context
    /// - `node` - The node to inspect/edit
    /// - `device_ids` - List of available device IDs for autocomplete (empty for fallback to text field)
    pub fn show(ui: &mut Ui, node: &ExperimentNode, device_ids: &[String]) -> Option<ExperimentNode> {
        let mut modified = node.clone();
        let mut changed = false;

        ui.vertical(|ui| {
            ui.heading(node.node_name());
            ui.separator();

            match &mut modified {
                ExperimentNode::Scan {
                    actuator,
                    start,
                    stop,
                    points,
                } => {
                    changed |= Self::show_scan_inspector(ui, actuator, start, stop, points, device_ids);
                }
                ExperimentNode::Acquire(config) => {
                    changed |= Self::show_acquire_inspector(ui, config, device_ids);
                }
                ExperimentNode::Move(config) => {
                    changed |= Self::show_move_inspector(ui, config, device_ids);
                }
                ExperimentNode::Wait { condition } => {
                    changed |= Self::show_wait_inspector(ui, condition, device_ids);
                }
                ExperimentNode::Loop(config) => {
                    changed |= Self::show_loop_inspector(ui, config, device_ids);
                }
            }
        });

        if changed {
            Some(modified)
        } else {
            None
        }
    }

    /// Show Move node inspector with device selector and all configuration options.
    fn show_move_inspector(
        ui: &mut Ui,
        config: &mut crate::graph::nodes::MoveConfig,
        device_ids: &[String],
    ) -> bool {
        let mut changed = false;

        // Device selection with autocomplete
        ui.horizontal(|ui| {
            ui.label("Device:");
            if device_ids.is_empty() {
                // Fallback to text field when no devices available
                changed |= ui.text_edit_singleline(&mut config.device).changed();
            } else {
                let mut selector = DeviceSelector::new(device_ids);
                selector.set_selected(&config.device);
                if selector.show(ui, "Select actuator...") {
                    config.device = selector.selected().to_string();
                    changed = true;
                }
            }
        });

        // Mode toggle (Absolute / Relative)
        ui.horizontal(|ui| {
            ui.label("Mode:");
            use crate::graph::nodes::MoveMode;
            changed |= ui.radio_value(&mut config.mode, MoveMode::Absolute, "Absolute").changed();
            changed |= ui.radio_value(&mut config.mode, MoveMode::Relative, "Relative").changed();
        });

        // Position field with label that changes based on mode
        ui.horizontal(|ui| {
            let label = match config.mode {
                crate::graph::nodes::MoveMode::Absolute => "Position:",
                crate::graph::nodes::MoveMode::Relative => "Distance:",
            };
            ui.label(label);
            changed |= ui.add(egui::DragValue::new(&mut config.position).speed(0.1)).changed();
        });

        // Wait for settle checkbox
        changed |= ui.checkbox(&mut config.wait_settled, "Wait for motion to settle").changed();

        changed
    }

    /// Show Scan node inspector.
    fn show_scan_inspector(
        ui: &mut Ui,
        actuator: &mut String,
        start: &mut f64,
        stop: &mut f64,
        points: &mut u32,
        device_ids: &[String],
    ) -> bool {
        let mut changed = false;

        // Device selection with autocomplete
        ui.horizontal(|ui| {
            ui.label("Actuator:");
            if device_ids.is_empty() {
                changed |= ui.text_edit_singleline(actuator).changed();
            } else {
                let mut selector = DeviceSelector::new(device_ids);
                selector.set_selected(actuator);
                if selector.show(ui, "Select actuator...") {
                    *actuator = selector.selected().to_string();
                    changed = true;
                }
            }
        });

        changed |= Self::float_field(ui, "Start", start);
        changed |= Self::float_field(ui, "Stop", stop);
        changed |= Self::u32_field(ui, "Points", points);

        changed
    }

    /// Show Acquire node inspector.
    fn show_acquire_inspector(
        ui: &mut Ui,
        config: &mut crate::graph::nodes::AcquireConfig,
        device_ids: &[String],
    ) -> bool {
        let mut changed = false;

        // Device selection with autocomplete
        ui.horizontal(|ui| {
            ui.label("Detector:");
            if device_ids.is_empty() {
                changed |= ui.text_edit_singleline(&mut config.detector).changed();
            } else {
                let mut selector = DeviceSelector::new(device_ids);
                selector.set_selected(&config.detector);
                if selector.show(ui, "Select detector...") {
                    config.detector = selector.selected().to_string();
                    changed = true;
                }
            }
        });

        // Exposure control with optional override
        ui.horizontal(|ui| {
            ui.label("Exposure (ms):");
            let mut has_override = config.exposure_ms.is_some();
            if ui.checkbox(&mut has_override, "Override").changed() {
                config.exposure_ms = if has_override { Some(100.0) } else { None };
                changed = true;
            }
            if let Some(ref mut exp) = config.exposure_ms {
                changed |= ui.add(egui::DragValue::new(exp).speed(0.1)).changed();
            } else {
                ui.label("(use device default)");
            }
        });

        // Frame count with range limit
        ui.horizontal(|ui| {
            ui.label("Frame Count:");
            let mut v = config.frame_count as i32;
            let resp = ui.add(egui::DragValue::new(&mut v).speed(1).range(1..=1000));
            if resp.changed() {
                config.frame_count = v.max(1) as u32;
                changed = true;
            }
        });

        changed
    }

    /// Show Wait node inspector.
    fn show_wait_inspector(
        ui: &mut Ui,
        condition: &mut crate::graph::nodes::WaitCondition,
        device_ids: &[String],
    ) -> bool {
        use crate::graph::nodes::{ThresholdOp, WaitCondition};
        let mut changed = false;

        // Condition type selector
        ui.horizontal(|ui| {
            ui.label("Wait Type:");

            let current_type = match condition {
                WaitCondition::Duration { .. } => 0,
                WaitCondition::Threshold { .. } => 1,
                WaitCondition::Stability { .. } => 2,
            };

            let mut new_type = current_type;
            egui::ComboBox::from_id_salt("wait_type")
                .selected_text(match current_type {
                    0 => "Duration",
                    1 => "Threshold",
                    2 => "Stability",
                    _ => unreachable!(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_type, 0, "Duration");
                    ui.selectable_value(&mut new_type, 1, "Threshold");
                    ui.selectable_value(&mut new_type, 2, "Stability");
                });

            // If type changed, create new default variant
            if new_type != current_type {
                *condition = match new_type {
                    0 => WaitCondition::Duration { milliseconds: 1000.0 },
                    1 => WaitCondition::Threshold {
                        device_id: String::new(),
                        operator: ThresholdOp::GreaterThan,
                        value: 0.0,
                        timeout_ms: 30000.0,
                    },
                    2 => WaitCondition::Stability {
                        device_id: String::new(),
                        tolerance: 0.01,
                        duration_ms: 1000.0,
                        timeout_ms: 30000.0,
                    },
                    _ => unreachable!(),
                };
                changed = true;
            }
        });

        ui.separator();

        // Show fields based on condition type
        match condition {
            WaitCondition::Duration { milliseconds } => {
                changed |= Self::float_field(ui, "Duration (ms)", milliseconds);
            }
            WaitCondition::Threshold {
                device_id,
                operator,
                value,
                timeout_ms,
            } => {
                // Device selector
                ui.horizontal(|ui| {
                    ui.label("Device:");
                    if device_ids.is_empty() {
                        changed |= ui.text_edit_singleline(device_id).changed();
                    } else {
                        let mut selector = DeviceSelector::new(device_ids);
                        selector.set_selected(device_id);
                        if selector.show(ui, "Select device...") {
                            *device_id = selector.selected().to_string();
                            changed = true;
                        }
                    }
                });

                // Operator selector
                ui.horizontal(|ui| {
                    ui.label("Operator:");
                    egui::ComboBox::from_id_salt("threshold_op")
                        .selected_text(match operator {
                            ThresholdOp::LessThan => "Less Than",
                            ThresholdOp::GreaterThan => "Greater Than",
                            ThresholdOp::EqualWithin { .. } => "Equal Within",
                        })
                        .show_ui(ui, |ui| {
                            let before = operator.clone();
                            if ui.selectable_label(matches!(operator, ThresholdOp::LessThan), "Less Than").clicked() {
                                *operator = ThresholdOp::LessThan;
                            }
                            if ui.selectable_label(matches!(operator, ThresholdOp::GreaterThan), "Greater Than").clicked() {
                                *operator = ThresholdOp::GreaterThan;
                            }
                            if ui.selectable_label(matches!(operator, ThresholdOp::EqualWithin { .. }), "Equal Within").clicked() {
                                *operator = ThresholdOp::EqualWithin { tolerance: 0.01 };
                            }
                            if *operator != before {
                                changed = true;
                            }
                        });
                });

                // Value field
                changed |= Self::float_field(ui, "Value", value);

                // Tolerance if EqualWithin
                if let ThresholdOp::EqualWithin { tolerance } = operator {
                    changed |= Self::float_field(ui, "Tolerance", tolerance);
                }

                changed |= Self::float_field(ui, "Timeout (ms)", timeout_ms);
            }
            WaitCondition::Stability {
                device_id,
                tolerance,
                duration_ms,
                timeout_ms,
            } => {
                // Device selector
                ui.horizontal(|ui| {
                    ui.label("Device:");
                    if device_ids.is_empty() {
                        changed |= ui.text_edit_singleline(device_id).changed();
                    } else {
                        let mut selector = DeviceSelector::new(device_ids);
                        selector.set_selected(device_id);
                        if selector.show(ui, "Select device...") {
                            *device_id = selector.selected().to_string();
                            changed = true;
                        }
                    }
                });

                changed |= Self::float_field(ui, "Tolerance", tolerance);
                changed |= Self::float_field(ui, "Duration (ms)", duration_ms);
                changed |= Self::float_field(ui, "Timeout (ms)", timeout_ms);
            }
        }

        changed
    }

    /// Show Loop node inspector.
    fn show_loop_inspector(
        ui: &mut Ui,
        config: &mut crate::graph::nodes::LoopConfig,
        device_ids: &[String],
    ) -> bool {
        use crate::graph::nodes::{LoopTermination, ThresholdOp};
        let mut changed = false;

        // Termination type selector
        ui.horizontal(|ui| {
            ui.label("Loop Type:");

            let current_type = match &config.termination {
                LoopTermination::Count { .. } => 0,
                LoopTermination::Condition { .. } => 1,
                LoopTermination::Infinite { .. } => 2,
            };

            let mut new_type = current_type;
            egui::ComboBox::from_id_salt("loop_type")
                .selected_text(match current_type {
                    0 => "Count",
                    1 => "Condition",
                    2 => "Infinite",
                    _ => unreachable!(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_type, 0, "Count");
                    ui.selectable_value(&mut new_type, 1, "Condition");
                    ui.selectable_value(&mut new_type, 2, "Infinite");
                });

            // If type changed, create new default variant
            if new_type != current_type {
                config.termination = match new_type {
                    0 => LoopTermination::Count { iterations: 10 },
                    1 => LoopTermination::Condition {
                        device_id: String::new(),
                        operator: ThresholdOp::GreaterThan,
                        value: 0.0,
                        max_iterations: 10000,
                    },
                    2 => LoopTermination::Infinite { max_iterations: 10000 },
                    _ => unreachable!(),
                };
                changed = true;
            }
        });

        ui.separator();

        // Show fields based on termination type
        match &mut config.termination {
            LoopTermination::Count { iterations } => {
                ui.horizontal(|ui| {
                    ui.label("Iterations:");
                    let mut v = *iterations as i32;
                    let resp = ui.add(egui::DragValue::new(&mut v).speed(1).range(1..=10000));
                    if resp.changed() {
                        *iterations = v.max(1) as u32;
                        changed = true;
                    }
                });
            }
            LoopTermination::Condition {
                device_id,
                operator,
                value,
                max_iterations,
            } => {
                // Device selector
                ui.horizontal(|ui| {
                    ui.label("Device:");
                    if device_ids.is_empty() {
                        changed |= ui.text_edit_singleline(device_id).changed();
                    } else {
                        let mut selector = DeviceSelector::new(device_ids);
                        selector.set_selected(device_id);
                        if selector.show(ui, "Select device...") {
                            *device_id = selector.selected().to_string();
                            changed = true;
                        }
                    }
                });

                // Operator selector
                ui.horizontal(|ui| {
                    ui.label("Operator:");
                    egui::ComboBox::from_id_salt("loop_condition_op")
                        .selected_text(match operator {
                            ThresholdOp::LessThan => "Less Than",
                            ThresholdOp::GreaterThan => "Greater Than",
                            ThresholdOp::EqualWithin { .. } => "Equal Within",
                        })
                        .show_ui(ui, |ui| {
                            let before = operator.clone();
                            if ui.selectable_label(matches!(operator, ThresholdOp::LessThan), "Less Than").clicked() {
                                *operator = ThresholdOp::LessThan;
                            }
                            if ui.selectable_label(matches!(operator, ThresholdOp::GreaterThan), "Greater Than").clicked() {
                                *operator = ThresholdOp::GreaterThan;
                            }
                            if ui.selectable_label(matches!(operator, ThresholdOp::EqualWithin { .. }), "Equal Within").clicked() {
                                *operator = ThresholdOp::EqualWithin { tolerance: 0.01 };
                            }
                            if *operator != before {
                                changed = true;
                            }
                        });
                });

                // Value field
                changed |= Self::float_field(ui, "Value", value);

                // Tolerance if EqualWithin
                if let ThresholdOp::EqualWithin { tolerance } = operator {
                    changed |= Self::float_field(ui, "Tolerance", tolerance);
                }

                // Max iterations (safety limit)
                ui.horizontal(|ui| {
                    ui.label("Max Iterations:");
                    let mut v = *max_iterations as i32;
                    let resp = ui.add(egui::DragValue::new(&mut v).speed(1).range(1..=100000));
                    if resp.changed() {
                        *max_iterations = v.max(1) as u32;
                        changed = true;
                    }
                });
            }
            LoopTermination::Infinite { max_iterations } => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ Requires manual abort"
                );

                // Max iterations (safety limit)
                ui.horizontal(|ui| {
                    ui.label("Max Iterations:");
                    let mut v = *max_iterations as i32;
                    let resp = ui.add(egui::DragValue::new(&mut v).speed(1).range(1..=100000));
                    if resp.changed() {
                        *max_iterations = v.max(1) as u32;
                        changed = true;
                    }
                });
                ui.label("(safety limit)");
            }
        }

        changed
    }

    fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.text_edit_singleline(value).changed()
        })
        .inner
    }

    fn float_field(ui: &mut Ui, label: &str, value: &mut f64) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            // Use DragValue for numeric input with drag support
            ui.add(egui::DragValue::new(value).speed(0.1)).changed()
        })
        .inner
    }

    fn u32_field(ui: &mut Ui, label: &str, value: &mut u32) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            let mut v = *value as i32;
            let changed = ui
                .add(egui::DragValue::new(&mut v).speed(1).range(1..=10000))
                .changed();
            if changed {
                *value = v.max(1) as u32;
            }
            changed
        })
        .inner
    }

    fn checkbox_field(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.checkbox(value, "").changed()
        })
        .inner
    }

    /// Show placeholder when no node is selected.
    pub fn show_empty(ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label("Select a node to edit its properties");
        });
    }
}
