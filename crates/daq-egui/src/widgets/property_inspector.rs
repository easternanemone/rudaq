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
                ExperimentNode::Acquire {
                    detector,
                    duration_ms,
                } => {
                    changed |= Self::text_field(ui, "Detector", detector);
                    changed |= Self::float_field(ui, "Duration (ms)", duration_ms);
                }
                ExperimentNode::Move { device, position } => {
                    changed |= Self::text_field(ui, "Device", device);
                    changed |= Self::float_field(ui, "Position", position);
                }
                ExperimentNode::Wait { duration_ms } => {
                    changed |= Self::float_field(ui, "Duration (ms)", duration_ms);
                }
                ExperimentNode::Loop { iterations } => {
                    changed |= Self::u32_field(ui, "Iterations", iterations);
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

    /// Show placeholder when no node is selected.
    pub fn show_empty(ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label("Select a node to edit its properties");
        });
    }
}
