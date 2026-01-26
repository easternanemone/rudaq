//! SnarlViewer implementation for ExperimentNode.

use std::collections::HashMap;

use egui_snarl::ui::{PinInfo, SnarlViewer};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use super::execution_state::{ExecutionState, NodeExecutionState};
use super::nodes::{ExperimentNode, LoopTermination, MoveMode, WaitCondition};
use super::validation::{output_pin_type, validate_connection, PinType};

/// Viewer for rendering experiment nodes in the graph editor.
#[derive(Default)]
pub struct ExperimentViewer {
    /// Last validation error (shown as toast/status)
    pub last_error: Option<String>,
    /// Per-node validation errors
    pub node_errors: HashMap<NodeId, String>,
    /// Execution state for visual highlighting
    pub execution_state: Option<ExecutionState>,
    /// Available device IDs for dropdown selectors (updated from DaqClient)
    pub device_ids: Vec<String>,
}

impl ExperimentViewer {
    pub fn new() -> Self {
        Self {
            last_error: None,
            node_errors: HashMap::new(),
            execution_state: None,
            device_ids: Vec::new(),
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn has_errors(&self) -> bool {
        !self.node_errors.is_empty()
    }

    /// Get the header color for a node based on validation and execution state.
    #[allow(dead_code)]
    fn header_color(&self, node_id: NodeId) -> egui::Color32 {
        // Check for validation errors first (highest priority)
        if self.node_errors.contains_key(&node_id) {
            return egui::Color32::from_rgb(200, 80, 80); // Red for errors
        }

        // Check execution state
        if let Some(exec_state) = &self.execution_state {
            match exec_state.node_state(node_id) {
                NodeExecutionState::Running => {
                    return egui::Color32::from_rgb(100, 200, 100); // Green for running
                }
                NodeExecutionState::Completed => {
                    return egui::Color32::from_rgb(100, 150, 200); // Blue for completed
                }
                _ => {}
            }
        }

        // Default color
        egui::Color32::from_rgb(60, 60, 60)
    }

    // ========== Inline Node Body Editors ==========

    /// Show device dropdown selector.
    fn device_dropdown(&self, ui: &mut egui::Ui, id: &str, device: &mut String, label: &str) {
        ui.horizontal(|ui| {
            ui.label(label);
            let selected_text = if device.is_empty() {
                "Select...".to_string()
            } else {
                device.clone()
            };
            egui::ComboBox::from_id_salt(id)
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for dev_id in &self.device_ids {
                        ui.selectable_value(device, dev_id.clone(), dev_id);
                    }
                    // Always allow manual text entry at the end
                    if self.device_ids.is_empty() {
                        ui.label("(No devices available)");
                    }
                });
        });
    }

    /// Show Scan node body with inline editors.
    fn show_scan_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        actuator: &mut String,
        start: &mut f64,
        stop: &mut f64,
        points: &mut u32,
    ) {
        // Show validation error if any
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        self.device_dropdown(ui, "scan_actuator", actuator, "Actuator:");

        ui.horizontal(|ui| {
            ui.label("Start:");
            ui.add(egui::DragValue::new(start).speed(0.1));
            ui.label("Stop:");
            ui.add(egui::DragValue::new(stop).speed(0.1));
        });

        ui.horizontal(|ui| {
            ui.label("Points:");
            let mut pts = *points as i32;
            if ui
                .add(egui::DragValue::new(&mut pts).speed(1).range(1..=10000))
                .changed()
            {
                *points = pts.max(1) as u32;
            }
        });
    }

    /// Show Acquire node body with inline editors.
    fn show_acquire_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        config: &mut super::nodes::AcquireConfig,
    ) {
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        self.device_dropdown(ui, "acquire_detector", &mut config.detector, "Detector:");

        ui.horizontal(|ui| {
            ui.label("Exposure:");
            let mut has_override = config.exposure_ms.is_some();
            if ui.checkbox(&mut has_override, "").changed() {
                config.exposure_ms = if has_override { Some(100.0) } else { None };
            }
            if let Some(ref mut exp) = config.exposure_ms {
                ui.add(egui::DragValue::new(exp).speed(0.1).suffix(" ms"));
            } else {
                ui.label("(default)");
            }
        });

        ui.horizontal(|ui| {
            ui.label("Frames:");
            let mut v = config.frame_count as i32;
            if ui
                .add(egui::DragValue::new(&mut v).speed(1).range(1..=1000))
                .changed()
            {
                config.frame_count = v.max(1) as u32;
            }
        });
    }

    /// Show Move node body with inline editors.
    fn show_move_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        config: &mut super::nodes::MoveConfig,
    ) {
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        self.device_dropdown(ui, "move_device", &mut config.device, "Device:");

        ui.horizontal(|ui| {
            ui.radio_value(&mut config.mode, MoveMode::Absolute, "Abs");
            ui.radio_value(&mut config.mode, MoveMode::Relative, "Rel");
        });

        ui.horizontal(|ui| {
            let label = match config.mode {
                MoveMode::Absolute => "Position:",
                MoveMode::Relative => "Distance:",
            };
            ui.label(label);
            ui.add(egui::DragValue::new(&mut config.position).speed(0.1));
        });

        ui.checkbox(&mut config.wait_settled, "Wait for settle");
    }

    /// Show Wait node body with inline editors.
    fn show_wait_body(&self, ui: &mut egui::Ui, node_id: NodeId, condition: &mut WaitCondition) {
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        // Type selector
        let mut condition_type = match condition {
            WaitCondition::Duration { .. } => 0,
            WaitCondition::Threshold { .. } => 1,
            WaitCondition::Stability { .. } => 2,
        };

        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("wait_type")
                .selected_text(match condition_type {
                    0 => "Duration",
                    1 => "Threshold",
                    _ => "Stability",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut condition_type, 0, "Duration");
                    ui.selectable_value(&mut condition_type, 1, "Threshold");
                    ui.selectable_value(&mut condition_type, 2, "Stability");
                });
        });

        // Convert condition type if changed
        match (condition_type, &*condition) {
            (0, WaitCondition::Duration { .. }) => {}
            (0, _) => {
                *condition = WaitCondition::Duration {
                    milliseconds: 1000.0,
                };
            }
            (1, WaitCondition::Threshold { .. }) => {}
            (1, _) => {
                *condition = WaitCondition::Threshold {
                    device_id: String::new(),
                    operator: Default::default(),
                    value: 0.0,
                    timeout_ms: 5000.0,
                };
            }
            (2, WaitCondition::Stability { .. }) => {}
            (2, _) => {
                *condition = WaitCondition::Stability {
                    device_id: String::new(),
                    tolerance: 0.01,
                    duration_ms: 1000.0,
                    timeout_ms: 10000.0,
                };
            }
            _ => {}
        }

        // Show fields based on condition type
        match condition {
            WaitCondition::Duration { milliseconds } => {
                ui.horizontal(|ui| {
                    ui.label("Duration:");
                    ui.add(egui::DragValue::new(milliseconds).speed(10.0).suffix(" ms"));
                });
            }
            WaitCondition::Threshold {
                device_id,
                value,
                timeout_ms,
                ..
            } => {
                self.device_dropdown(ui, "wait_device", device_id, "Device:");
                ui.horizontal(|ui| {
                    ui.label("Value:");
                    ui.add(egui::DragValue::new(value).speed(0.1));
                });
                ui.horizontal(|ui| {
                    ui.label("Timeout:");
                    ui.add(egui::DragValue::new(timeout_ms).speed(100.0).suffix(" ms"));
                });
            }
            WaitCondition::Stability {
                device_id,
                tolerance,
                duration_ms,
                timeout_ms,
            } => {
                self.device_dropdown(ui, "wait_device", device_id, "Device:");
                ui.horizontal(|ui| {
                    ui.label("Tolerance:");
                    ui.add(egui::DragValue::new(tolerance).speed(0.001));
                });
                ui.horizontal(|ui| {
                    ui.label("Hold:");
                    ui.add(egui::DragValue::new(duration_ms).speed(10.0).suffix(" ms"));
                });
                ui.horizontal(|ui| {
                    ui.label("Timeout:");
                    ui.add(egui::DragValue::new(timeout_ms).speed(100.0).suffix(" ms"));
                });
            }
        }
    }

    /// Show Loop node body with inline editors.
    fn show_loop_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        config: &mut super::nodes::LoopConfig,
    ) {
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        // Type selector
        let mut term_type = match &config.termination {
            LoopTermination::Count { .. } => 0,
            LoopTermination::Condition { .. } => 1,
            LoopTermination::Infinite { .. } => 2,
        };

        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("loop_type")
                .selected_text(match term_type {
                    0 => "Count",
                    1 => "Condition",
                    _ => "Infinite",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut term_type, 0, "Count");
                    ui.selectable_value(&mut term_type, 1, "Condition");
                    ui.selectable_value(&mut term_type, 2, "Infinite");
                });
        });

        // Convert type if changed
        match (term_type, &config.termination) {
            (0, LoopTermination::Count { .. }) => {}
            (0, _) => {
                config.termination = LoopTermination::Count { iterations: 10 };
            }
            (1, LoopTermination::Condition { .. }) => {}
            (1, _) => {
                config.termination = LoopTermination::Condition {
                    device_id: String::new(),
                    operator: Default::default(),
                    value: 0.0,
                    max_iterations: 1000,
                };
            }
            (2, LoopTermination::Infinite { .. }) => {}
            (2, _) => {
                config.termination = LoopTermination::Infinite {
                    max_iterations: 1000,
                };
            }
            _ => {}
        }

        // Show fields
        match &mut config.termination {
            LoopTermination::Count { iterations } => {
                ui.horizontal(|ui| {
                    ui.label("Iterations:");
                    let mut v = *iterations as i32;
                    if ui
                        .add(egui::DragValue::new(&mut v).speed(1).range(1..=100000))
                        .changed()
                    {
                        *iterations = v.max(1) as u32;
                    }
                });
            }
            LoopTermination::Condition {
                device_id,
                value,
                max_iterations,
                ..
            } => {
                self.device_dropdown(ui, "loop_device", device_id, "Device:");
                ui.horizontal(|ui| {
                    ui.label("Target:");
                    ui.add(egui::DragValue::new(value).speed(0.1));
                });
                ui.horizontal(|ui| {
                    ui.label("Max iter:");
                    let mut v = *max_iterations as i32;
                    if ui
                        .add(egui::DragValue::new(&mut v).speed(1).range(1..=100000))
                        .changed()
                    {
                        *max_iterations = v.max(1) as u32;
                    }
                });
            }
            LoopTermination::Infinite { max_iterations } => {
                ui.horizontal(|ui| {
                    ui.label("Safety limit:");
                    let mut v = *max_iterations as i32;
                    if ui
                        .add(egui::DragValue::new(&mut v).speed(1).range(1..=100000))
                        .changed()
                    {
                        *max_iterations = v.max(1) as u32;
                    }
                });
            }
        }
    }

    /// Show NestedScan node body with inline editors.
    fn show_nested_scan_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        config: &mut super::nodes::NestedScanConfig,
    ) {
        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        ui.collapsing("Outer", |ui| {
            self.device_dropdown(
                ui,
                "nested_outer_dev",
                &mut config.outer.actuator,
                "Actuator:",
            );
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut config.outer.dimension_name);
            });
            ui.horizontal(|ui| {
                ui.label("Start:");
                ui.add(egui::DragValue::new(&mut config.outer.start).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Stop:");
                ui.add(egui::DragValue::new(&mut config.outer.stop).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Points:");
                let mut v = config.outer.points as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).speed(1).range(1..=10000))
                    .changed()
                {
                    config.outer.points = v.max(1) as u32;
                }
            });
        });

        ui.collapsing("Inner", |ui| {
            self.device_dropdown(
                ui,
                "nested_inner_dev",
                &mut config.inner.actuator,
                "Actuator:",
            );
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut config.inner.dimension_name);
            });
            ui.horizontal(|ui| {
                ui.label("Start:");
                ui.add(egui::DragValue::new(&mut config.inner.start).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Stop:");
                ui.add(egui::DragValue::new(&mut config.inner.stop).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Points:");
                let mut v = config.inner.points as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).speed(1).range(1..=10000))
                    .changed()
                {
                    config.inner.points = v.max(1) as u32;
                }
            });
        });
    }

    /// Show AdaptiveScan node body with inline editors.
    fn show_adaptive_scan_body(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        config: &mut super::nodes::AdaptiveScanConfig,
    ) {
        use super::nodes::{AdaptiveAction, TriggerCondition, TriggerLogic};

        if let Some(error) = self.node_errors.get(&node_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
        }

        // Base scan configuration
        ui.collapsing("Scan", |ui| {
            self.device_dropdown(
                ui,
                "adaptive_scan_dev",
                &mut config.scan.actuator,
                "Actuator:",
            );
            ui.horizontal(|ui| {
                ui.label("Start:");
                ui.add(egui::DragValue::new(&mut config.scan.start).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Stop:");
                ui.add(egui::DragValue::new(&mut config.scan.stop).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("Points:");
                let mut v = config.scan.points as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).speed(1).range(1..=10000))
                    .changed()
                {
                    config.scan.points = v.max(1) as u32;
                }
            });
        });

        // Trigger configuration
        ui.collapsing("Triggers", |ui| {
            // Logic selector
            ui.horizontal(|ui| {
                ui.label("Logic:");
                egui::ComboBox::from_id_salt("trigger_logic")
                    .selected_text(match config.trigger_logic {
                        TriggerLogic::Any => "Any (OR)",
                        TriggerLogic::All => "All (AND)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut config.trigger_logic,
                            TriggerLogic::Any,
                            "Any (OR)",
                        );
                        ui.selectable_value(
                            &mut config.trigger_logic,
                            TriggerLogic::All,
                            "All (AND)",
                        );
                    });
            });

            // Show each trigger with remove button
            let mut remove_idx = None;
            let trigger_count = config.triggers.len();
            for (idx, trigger) in config.triggers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", idx + 1));
                    // Trigger type selector
                    let is_threshold = matches!(trigger, TriggerCondition::Threshold { .. });
                    let mut trigger_type = if is_threshold { 0 } else { 1 };
                    egui::ComboBox::from_id_salt(format!("trigger_type_{}", idx))
                        .width(80.0)
                        .selected_text(if is_threshold { "Threshold" } else { "Peak" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut trigger_type, 0, "Threshold");
                            ui.selectable_value(&mut trigger_type, 1, "Peak");
                        });
                    // Convert if type changed
                    if trigger_type == 0 && !is_threshold {
                        *trigger = TriggerCondition::default();
                    } else if trigger_type == 1 && is_threshold {
                        *trigger = TriggerCondition::PeakDetection {
                            device_id: String::new(),
                            min_prominence: 1.0,
                            min_height: None,
                        };
                    }
                    // Remove button (disable if only one trigger)
                    if trigger_count > 1 && ui.button("x").clicked() {
                        remove_idx = Some(idx);
                    }
                });

                // Trigger-specific fields
                match trigger {
                    TriggerCondition::Threshold {
                        device_id,
                        operator,
                        value,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("  Device:");
                            ui.text_edit_singleline(device_id);
                        });
                        ui.horizontal(|ui| {
                            ui.label("  Op:");
                            egui::ComboBox::from_id_salt(format!("threshold_op_{}", idx))
                                .width(60.0)
                                .selected_text(match operator {
                                    super::nodes::ThresholdOp::LessThan => "<",
                                    super::nodes::ThresholdOp::GreaterThan => ">",
                                    super::nodes::ThresholdOp::EqualWithin { .. } => "~=",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        operator,
                                        super::nodes::ThresholdOp::LessThan,
                                        "<",
                                    );
                                    ui.selectable_value(
                                        operator,
                                        super::nodes::ThresholdOp::GreaterThan,
                                        ">",
                                    );
                                    ui.selectable_value(
                                        operator,
                                        super::nodes::ThresholdOp::EqualWithin { tolerance: 0.01 },
                                        "~=",
                                    );
                                });
                            ui.add(egui::DragValue::new(value).speed(0.1));
                        });
                    }
                    TriggerCondition::PeakDetection {
                        device_id,
                        min_prominence,
                        min_height,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("  Device:");
                            ui.text_edit_singleline(device_id);
                        });
                        ui.horizontal(|ui| {
                            ui.label("  Prominence:");
                            ui.add(egui::DragValue::new(min_prominence).speed(0.1));
                        });
                        ui.horizontal(|ui| {
                            let mut has_height = min_height.is_some();
                            if ui.checkbox(&mut has_height, "Min height:").changed() {
                                *min_height = if has_height { Some(0.0) } else { None };
                            }
                            if let Some(h) = min_height {
                                ui.add(egui::DragValue::new(h).speed(0.1));
                            }
                        });
                    }
                }
                ui.separator();
            }

            // Remove trigger if requested
            if let Some(idx) = remove_idx {
                config.triggers.remove(idx);
            }

            // Add trigger button
            if ui.button("+ Add Trigger").clicked() {
                config.triggers.push(TriggerCondition::default());
            }
        });

        // Action configuration
        ui.horizontal(|ui| {
            ui.label("Action:");
            egui::ComboBox::from_id_salt("adaptive_action")
                .selected_text(match config.action {
                    AdaptiveAction::Zoom2x => "Zoom 2x",
                    AdaptiveAction::Zoom4x => "Zoom 4x",
                    AdaptiveAction::MoveToPeak => "Move to Peak",
                    AdaptiveAction::AcquireAtPeak => "Acquire at Peak",
                    AdaptiveAction::MarkAndContinue => "Mark & Continue",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut config.action, AdaptiveAction::Zoom2x, "Zoom 2x");
                    ui.selectable_value(&mut config.action, AdaptiveAction::Zoom4x, "Zoom 4x");
                    ui.selectable_value(
                        &mut config.action,
                        AdaptiveAction::MoveToPeak,
                        "Move to Peak",
                    );
                    ui.selectable_value(
                        &mut config.action,
                        AdaptiveAction::AcquireAtPeak,
                        "Acquire at Peak",
                    );
                    ui.selectable_value(
                        &mut config.action,
                        AdaptiveAction::MarkAndContinue,
                        "Mark & Continue",
                    );
                });
        });

        ui.checkbox(&mut config.require_approval, "Require approval");
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
            ExperimentNode::Loop { .. } => 2,       // Next + loop body outputs
            ExperimentNode::NestedScan { .. } => 2, // Next + body outputs
            _ => 1,                                 // Sequential flow output
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

    fn has_body(&mut self, _node: &ExperimentNode) -> bool {
        true // All nodes have inline editors
    }

    fn show_body(
        &mut self,
        node_id: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<ExperimentNode>,
    ) {
        // Wrap content with node-specific ID to prevent widget state bleed between same-type nodes
        ui.push_id(node_id, |ui| {
            // Get mutable reference to node and render appropriate editor
            if let Some(node) = snarl.get_node_mut(node_id) {
                match node {
                    ExperimentNode::Scan {
                        actuator,
                        start,
                        stop,
                        points,
                    } => {
                        self.show_scan_body(ui, node_id, actuator, start, stop, points);
                    }
                    ExperimentNode::Acquire(config) => {
                        self.show_acquire_body(ui, node_id, config);
                    }
                    ExperimentNode::Move(config) => {
                        self.show_move_body(ui, node_id, config);
                    }
                    ExperimentNode::Wait { condition } => {
                        self.show_wait_body(ui, node_id, condition);
                    }
                    ExperimentNode::Loop(config) => {
                        self.show_loop_body(ui, node_id, config);
                    }
                    ExperimentNode::NestedScan(config) => {
                        self.show_nested_scan_body(ui, node_id, config);
                    }
                    ExperimentNode::AdaptiveScan(config) => {
                        self.show_adaptive_scan_body(ui, node_id, config);
                    }
                }
            }
        });
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

    fn header_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<ExperimentNode>,
    ) -> egui::Frame {
        // Red tint for validation errors
        if self.node_errors.contains_key(&node) {
            return default.fill(egui::Color32::from_rgb(120, 40, 40));
        }

        // Execution state coloring
        if let Some(ref state) = self.execution_state {
            match state.node_state(node) {
                NodeExecutionState::Running => {
                    return default.fill(egui::Color32::from_rgb(40, 100, 40)); // Dark green
                }
                NodeExecutionState::Completed => {
                    return default.fill(egui::Color32::from_rgb(40, 60, 80)); // Dark blue
                }
                NodeExecutionState::Pending | NodeExecutionState::Skipped => {}
            }
        }

        default
    }
}
