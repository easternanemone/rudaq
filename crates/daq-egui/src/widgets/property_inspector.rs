//! Property inspector widget for editing node properties.

use egui::Ui;

use crate::graph::ExperimentNode;

/// Property inspector for editing selected node properties.
///
/// Shows editable fields for the selected node type and returns
/// the modified node if any changes were made.
pub struct PropertyInspector;

impl PropertyInspector {
    /// Show properties for a node. Returns `Some(modified_node)` if user made changes.
    pub fn show(ui: &mut Ui, node: &ExperimentNode) -> Option<ExperimentNode> {
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
                    changed |= Self::text_field(ui, "Actuator", actuator);
                    changed |= Self::float_field(ui, "Start", start);
                    changed |= Self::float_field(ui, "Stop", stop);
                    changed |= Self::u32_field(ui, "Points", points);
                }
                ExperimentNode::Acquire(config) => {
                    changed |= Self::text_field(ui, "Detector", &mut config.detector);

                    // Exposure control with optional override
                    ui.horizontal(|ui| {
                        ui.label("Exposure (ms)");
                        let mut has_override = config.exposure_ms.is_some();
                        if ui.checkbox(&mut has_override, "Override").changed() {
                            config.exposure_ms = if has_override { Some(100.0) } else { None };
                            changed = true;
                        }
                        if let Some(ref mut exp) = config.exposure_ms {
                            changed |= ui.add(egui::DragValue::new(exp).speed(0.1)).changed();
                        }
                    });

                    changed |= Self::u32_field(ui, "Frame Count", &mut config.frame_count);
                }
                ExperimentNode::Move(config) => {
                    changed |= Self::text_field(ui, "Device", &mut config.device);
                    changed |= Self::float_field(ui, "Position", &mut config.position);

                    // Mode selection
                    ui.horizontal(|ui| {
                        ui.label("Mode");
                        use crate::graph::nodes::MoveMode;
                        let before = config.mode.clone();
                        ui.radio_value(&mut config.mode, MoveMode::Absolute, "Absolute");
                        ui.radio_value(&mut config.mode, MoveMode::Relative, "Relative");
                        if config.mode != before {
                            changed = true;
                        }
                    });

                    changed |= Self::checkbox_field(ui, "Wait Settled", &mut config.wait_settled);
                }
                ExperimentNode::Wait { condition } => {
                    use crate::graph::nodes::WaitCondition;
                    match condition {
                        WaitCondition::Duration { milliseconds } => {
                            changed |= Self::float_field(ui, "Duration (ms)", milliseconds);
                        }
                        WaitCondition::Threshold { .. } => {
                            ui.label("⚠ Threshold waits: UI coming in Plan 02");
                        }
                        WaitCondition::Stability { .. } => {
                            ui.label("⚠ Stability waits: UI coming in Plan 02");
                        }
                    }
                }
                ExperimentNode::Loop(config) => {
                    use crate::graph::nodes::LoopTermination;
                    match &mut config.termination {
                        LoopTermination::Count { iterations } => {
                            changed |= Self::u32_field(ui, "Iterations", iterations);
                        }
                        LoopTermination::Condition { .. } => {
                            ui.label("⚠ Condition loops: UI coming in Plan 02");
                        }
                        LoopTermination::Infinite { .. } => {
                            ui.label("⚠ Infinite loops: UI coming in Plan 02");
                        }
                    }
                }
            }
        });

        if changed {
            Some(modified)
        } else {
            None
        }
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
