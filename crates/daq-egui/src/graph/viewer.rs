//! SnarlViewer implementation for ExperimentNode.

use std::collections::HashMap;

use egui_snarl::ui::{PinInfo, SnarlViewer};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use super::nodes::ExperimentNode;
use super::validation::{output_pin_type, validate_connection, PinType};

/// Viewer for rendering experiment nodes in the graph editor.
#[derive(Default)]
pub struct ExperimentViewer {
    /// Last validation error (shown as toast/status)
    pub last_error: Option<String>,
    /// Per-node validation errors
    pub node_errors: HashMap<NodeId, String>,
}

impl ExperimentViewer {
    pub fn new() -> Self {
        Self {
            last_error: None,
            node_errors: HashMap::new(),
        }
    }

    /// Clears the last error if any.
    #[allow(dead_code)]
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Set a validation error for a specific node.
    pub fn set_node_error(&mut self, node_id: NodeId, error: String) {
        self.node_errors.insert(node_id, error);
    }

    /// Clear the validation error for a specific node.
    pub fn clear_node_error(&mut self, node_id: NodeId) {
        self.node_errors.remove(&node_id);
    }

    /// Clear all validation errors.
    pub fn clear_all_errors(&mut self) {
        self.node_errors.clear();
        self.last_error = None;
    }

    /// Get the number of nodes with errors.
    pub fn error_count(&self) -> usize {
        self.node_errors.len()
    }

    /// Check if there are any validation errors.
    pub fn has_errors(&self) -> bool {
        !self.node_errors.is_empty()
    }
}


impl SnarlViewer<ExperimentNode> for ExperimentViewer {
    fn title(&mut self, node: &ExperimentNode) -> String {
        node.node_name().to_string()
    }

    fn inputs(&mut self, node: &ExperimentNode) -> usize {
        match node {
            ExperimentNode::Scan { .. } => 0, // Entry point, no inputs
            ExperimentNode::Loop { .. } => 1, // Has body input
            _ => 1,                           // Sequential flow input
        }
    }

    fn outputs(&mut self, node: &ExperimentNode) -> usize {
        match node {
            ExperimentNode::Loop { .. } => 2, // Next + loop body outputs
            _ => 1,                           // Sequential flow output
        }
    }

    #[allow(refining_impl_trait)]
    fn show_input(
        &mut self,
        _pin: &InPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<ExperimentNode>,
    ) -> PinInfo {
        ui.label(">"); // Flow input indicator
        PinInfo::default()
    }

    #[allow(refining_impl_trait)]
    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<ExperimentNode>,
    ) -> PinInfo {
        // Show appropriate label based on pin type
        if let Some(node) = snarl.get_node(pin.id.node) {
            let pin_type = output_pin_type(node, pin.id.output);
            match pin_type {
                PinType::Flow => ui.label(">"),
                PinType::LoopBody => ui.label("L"), // Loop body indicator
            };
        } else {
            ui.label(">"); // Default flow indicator
        }
        PinInfo::default()
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ExperimentNode>) {
        // Get node data for validation
        let from_node = snarl.get_node(from.id.node).cloned();
        let to_node = snarl.get_node(to.id.node).cloned();

        if let (Some(from_node), Some(to_node)) = (from_node, to_node) {
            match validate_connection(&from_node, from.id.output, &to_node, to.id.input) {
                Ok(()) => {
                    // Valid connection, create it
                    snarl.connect(from.id, to.id);
                    self.last_error = None;
                }
                Err(msg) => {
                    // Invalid, store error for display
                    self.last_error = Some(msg);
                }
            }
        }
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ExperimentNode>) {
        snarl.disconnect(from.id, to.id);
    }

    // show_body will be expanded in Plan 02-03 for property editing
}
