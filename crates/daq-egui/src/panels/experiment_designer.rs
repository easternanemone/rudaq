//! Experiment Designer panel with node graph editor.

use std::path::PathBuf;

use egui_snarl::ui::{get_selected_nodes, SnarlStyle};
use egui_snarl::{NodeId, Snarl};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use undo::Record;

use crate::client::DaqClient;
use crate::graph::commands::{AddNodeData, GraphEdit, ModifyNodeData};
use crate::graph::{
    load_graph, save_graph, EngineStateLocal, ExecutionState, ExperimentNode, ExperimentViewer,
    GraphFile, GraphMetadata, GraphPlan, GRAPH_FILE_EXTENSION,
};
use crate::widgets::node_palette::{NodePalette, NodeType};
use crate::widgets::{EditableParameter, PropertyInspector, RuntimeParameterEditResult, RuntimeParameterEditor};
use daq_experiment::Plan;

/// Actions from async execution operations
enum ExecutionAction {
    Started { run_uid: String, total_events: u32 },
    StatusUpdate { state: i32, current_event: Option<u32>, total_events: Option<u32> },
    Completed,
    Error(String),
}

/// Panel for visual experiment design using a node graph editor.
pub struct ExperimentDesignerPanel {
    snarl: Snarl<ExperimentNode>,
    viewer: ExperimentViewer,
    style: SnarlStyle,
    /// Track if a node is being dragged from palette
    dragging_node: Option<NodeType>,
    /// Position to add context menu node
    context_menu_pos: Option<egui::Pos2>,
    /// Counter for generating unique node positions to avoid overlap
    node_count: usize,
    /// Undo/redo history
    history: Record<GraphEdit>,
    /// Cache of selected node ID (updated from egui-snarl state)
    selected_node: Option<NodeId>,
    /// Current file path (None if unsaved)
    current_file: Option<PathBuf>,
    /// Graph metadata
    metadata: GraphMetadata,
    /// Status message for save/load feedback (message, timestamp)
    status_message: Option<(String, std::time::Instant)>,
    /// Execution state for visual feedback
    execution_state: ExecutionState,
    /// Channel for async action results
    action_tx: mpsc::Sender<ExecutionAction>,
    action_rx: mpsc::Receiver<ExecutionAction>,
    /// Last error message
    last_error: Option<String>,
    /// Parameters available for editing while paused
    editable_params: Vec<EditableParameter>,
}

impl Default for ExperimentDesignerPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);
        Self {
            snarl: Snarl::new(),
            viewer: ExperimentViewer::new(),
            style: SnarlStyle::default(),
            dragging_node: None,
            context_menu_pos: None,
            node_count: 0,
            history: Record::new(),
            selected_node: None,
            current_file: None,
            metadata: GraphMetadata::default(),
            status_message: None,
            execution_state: ExecutionState::new(),
            action_tx,
            action_rx,
            last_error: None,
            editable_params: Vec::new(),
        }
    }
}

impl ExperimentDesignerPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: Option<&Runtime>) {
        // Poll for async results
        self.poll_execution_actions();

        // Clone client for use in multiple places (DaqClient is Clone)
        // We clone first to preserve client for later use
        let client_clone: Option<DaqClient> = client.as_ref().map(|c| (*c).clone());

        // Poll engine status when execution is active (every 500ms)
        if self.execution_state.is_active()
            && self.execution_state.last_update.elapsed() > std::time::Duration::from_millis(500)
        {
            self.poll_engine_status(client_clone.as_ref(), runtime);
        }

        // Handle keyboard shortcuts FIRST (before any UI that might consume keys)
        self.handle_keyboard(ui);

        // Update selected node from egui-snarl state
        self.update_selected_node(ui);

        // Top toolbar with file operations and undo/redo buttons
        ui.horizontal(|ui| {
            ui.label("Experiment Designer");
            ui.separator();

            // File operations
            if ui.button("New").on_hover_text("Start a new graph").clicked() {
                self.new_graph();
            }

            if ui
                .button("Open...")
                .on_hover_text("Ctrl+O - Open a saved graph")
                .clicked()
            {
                self.open_file_dialog();
            }

            if ui
                .button("Save")
                .on_hover_text("Ctrl+S - Save current graph")
                .clicked()
            {
                self.save_current();
            }

            if ui
                .button("Save As...")
                .on_hover_text("Save graph to a new file")
                .clicked()
            {
                self.save_file_dialog();
            }

            ui.separator();

            // Undo button
            let can_undo = self.history.can_undo();
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                self.undo();
            }

            // Redo button
            let can_redo = self.history.can_redo();
            if ui
                .add_enabled(can_redo, egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Y or Ctrl+Shift+Z")
                .clicked()
            {
                self.redo();
            }

            ui.separator();

            // Show current file name
            if let Some(path) = &self.current_file {
                if let Some(name) = path.file_name() {
                    ui.label(format!("File: {}", name.to_string_lossy()));
                }
            } else {
                ui.label("Unsaved");
            }

            // Show status message (auto-fades after 3 seconds)
            if let Some((msg, time)) = &self.status_message {
                if time.elapsed().as_secs() < 3 {
                    ui.separator();
                    ui.label(msg);
                } else {
                    self.status_message = None;
                }
            }

            ui.separator();

            // Show drag hint if dragging
            if let Some(node_type) = &self.dragging_node {
                ui.colored_label(node_type.color(), format!("Dragging: {}", node_type.name()));
            }
        });

        // Execution controls (separate row for more space)
        ui.horizontal(|ui| {
            self.show_execution_toolbar(ui, client_clone.as_ref(), runtime);
        });

        ui.separator();

        // Run validation each frame (cheap check)
        self.validate_graph();

        // Bottom status bar with validation status
        egui::TopBottomPanel::bottom("validation_status_bar")
            .show_inside(ui, |ui| {
                self.show_validation_status_bar(ui);
            });

        // Three-panel layout: Palette | Canvas | Inspector
        egui::SidePanel::left("node_palette_panel")
            .resizable(true)
            .default_width(180.0)
            .min_width(150.0)
            .max_width(300.0)
            .show_inside(ui, |ui| {
                // Check if drag started from palette
                if let Some(node_type) = NodePalette::show(ui) {
                    self.dragging_node = Some(node_type);
                }
            });

        // Clone again for right panel
        let client_for_panel = client_clone.clone();

        egui::SidePanel::right("property_inspector_panel")
            .resizable(true)
            .default_width(200.0)
            .min_width(150.0)
            .max_width(400.0)
            .show_inside(ui, |ui| {
                self.show_property_inspector(ui, client_for_panel.as_ref(), runtime);
            });

        // Main canvas area
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Sync execution state to viewer for node highlighting
            if self.execution_state.is_active() {
                self.viewer.execution_state = Some(self.execution_state.clone());
            } else {
                self.viewer.execution_state = None;
            }

            // Handle context menu for adding nodes
            self.handle_context_menu(ui);

            // Handle drop onto canvas
            self.handle_canvas_drop(ui);

            // Graph canvas - takes remaining space
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let id = egui::Id::new("experiment_graph");
                self.snarl.show(&mut self.viewer, &self.style, id, ui);

                // TODO: Add visual node highlighting when egui-snarl supports custom header colors
                // For now, execution state is tracked but not visually shown on nodes
                // Alternative: Could add status icons/badges to node titles
            });
        });
    }

    fn show_property_inspector(&mut self, ui: &mut egui::Ui, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        ui.heading("Properties");
        ui.separator();

        if let Some(node_id) = self.selected_node {
            // Show validation error for selected node prominently
            if let Some(error) = self.viewer.node_errors.get(&node_id) {
                ui.group(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), "Validation Error:");
                    ui.label(error);
                });
                ui.add_space(8.0);
            }

            if let Some(node) = self.snarl.get_node(node_id) {
                // Clone the node before passing to inspector
                let node_clone = node.clone();

                // Show inspector and check for modifications
                // TODO: Fetch and cache device list from DaqClient for autocomplete
                let device_ids: Vec<String> = Vec::new(); // Empty for now, falls back to text field
                if let Some(modified_node) = PropertyInspector::show(ui, &node_clone, &device_ids) {
                    // Create undo-tracked modification
                    self.history.edit(
                        &mut self.snarl,
                        GraphEdit::ModifyNode(ModifyNodeData {
                            node_id,
                            old_data: node_clone,
                            new_data: modified_node,
                        }),
                    );
                }
            } else {
                // Node was deleted, clear selection
                self.selected_node = None;
                PropertyInspector::show_empty(ui);
            }
        } else {
            PropertyInspector::show_empty(ui);
        }

        // Show parameter editor when paused
        if self.execution_state.is_paused() || self.execution_state.is_running() {
            ui.add_space(8.0);
            self.show_parameter_editor_panel(ui, client, runtime);
        }
    }

    fn handle_keyboard(&mut self, ui: &mut egui::Ui) {
        // Save: Ctrl+S (Cmd+S on Mac)
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save_current();
        }

        // Open: Ctrl+O (Cmd+O on Mac)
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.open_file_dialog();
        }

        // Undo: Ctrl+Z (Cmd+Z on Mac)
        let undo_pressed = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z));
        let shift_held = ui.input(|i| i.modifiers.shift);

        if undo_pressed {
            if shift_held {
                // Ctrl+Shift+Z = Redo
                self.redo();
            } else {
                // Ctrl+Z = Undo
                self.undo();
            }
        }

        // Redo: Ctrl+Y
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y)) {
            self.redo();
        }

        // Delete: Delete or Backspace to remove selected node
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            if let Some(node_id) = self.selected_node.take() {
                // For now, just remove directly (could use RemoveNode command for undo)
                // We do direct removal because RemoveNode would need the node position
                // which we'd need to look up, making it more complex
                self.snarl.remove_node(node_id);
            }
        }
    }

    fn update_selected_node(&mut self, ui: &egui::Ui) {
        // Get selected nodes from egui-snarl's internal state
        let snarl_id = egui::Id::new("experiment_graph");
        let selected = get_selected_nodes(snarl_id, ui.ctx());

        // Update our cached selection (take first selected node if any)
        self.selected_node = selected.first().copied();
    }

    fn undo(&mut self) {
        self.history.undo(&mut self.snarl);
    }

    fn redo(&mut self) {
        self.history.redo(&mut self.snarl);
    }

    fn handle_context_menu(&mut self, ui: &mut egui::Ui) {
        // Check for right-click to open context menu
        let response = ui.interact(
            ui.available_rect_before_wrap(),
            egui::Id::new("canvas_context"),
            egui::Sense::click(),
        );

        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.context_menu_pos = Some(pos);
            }
        }

        // Show context menu popup
        if let Some(pos) = self.context_menu_pos {
            let popup_id = egui::Id::new("add_node_menu");

            egui::Area::new(popup_id)
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(120.0);
                        ui.label("Add Node");
                        ui.separator();

                        let mut close_menu = false;

                        for node_type in NodeType::all() {
                            if ui.button(node_type.name()).clicked() {
                                // Add node at reasonable position with undo tracking
                                let node_pos = self.next_node_position();
                                let node = node_type.create_node();
                                self.history.edit(
                                    &mut self.snarl,
                                    GraphEdit::AddNode(AddNodeData {
                                        node,
                                        position: node_pos,
                                        node_id: None,
                                    }),
                                );
                                close_menu = true;
                            }
                        }

                        // Close menu on click outside or after adding node
                        if close_menu
                            || ui.input(|i| i.pointer.any_click())
                                && !ui.rect_contains_pointer(ui.min_rect())
                        {
                            self.context_menu_pos = None;
                        }
                    });
                });

            // Close menu when clicking elsewhere
            if ui.input(|i| {
                i.pointer.any_click() && i.pointer.hover_pos().is_some_and(|p| p != pos)
            }) {
                self.context_menu_pos = None;
            }
        }
    }

    fn handle_canvas_drop(&mut self, ui: &mut egui::Ui) {
        // Check if we're dragging and the mouse was released over the canvas
        if let Some(node_type) = self.dragging_node {
            let response = ui.interact(
                ui.available_rect_before_wrap(),
                egui::Id::new("canvas_drop_zone"),
                egui::Sense::hover(),
            );

            if response.hovered() {
                // Show drop indicator
                ui.painter().rect_stroke(
                    response.rect,
                    egui::CornerRadius::same(4),
                    egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
            }

            // Check if drag ended (mouse released)
            if !ui.input(|i| i.pointer.any_down()) {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    if response.rect.contains(pos) {
                        // Create node at drop position with undo tracking
                        let node_pos = self.next_node_position();
                        let node = node_type.create_node();
                        self.history.edit(
                            &mut self.snarl,
                            GraphEdit::AddNode(AddNodeData {
                                node,
                                position: node_pos,
                                node_id: None,
                            }),
                        );
                    }
                }
                self.dragging_node = None;
            }
        }
    }

    /// Generate a position for the next node to avoid overlapping.
    fn next_node_position(&mut self) -> egui::Pos2 {
        let x = 50.0 + (self.node_count % 5) as f32 * 180.0;
        let y = 50.0 + (self.node_count / 5) as f32 * 120.0;
        self.node_count += 1;
        egui::pos2(x, y)
    }

    // ========== File Operations ==========

    /// Create a new empty graph.
    fn new_graph(&mut self) {
        self.snarl = Snarl::new();
        self.history = Record::new();
        self.current_file = None;
        self.metadata = GraphMetadata::default();
        self.selected_node = None;
        self.node_count = 0;
        self.set_status("New graph created");
    }

    /// Open file dialog and load selected file.
    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Experiment Graph", &[GRAPH_FILE_EXTENSION])
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            self.load_from_path(&path);
        }
    }

    /// Load graph from the specified path.
    fn load_from_path(&mut self, path: &std::path::Path) {
        match load_graph(path) {
            Ok(file) => {
                self.snarl = file.graph;
                self.metadata = file.metadata;
                self.current_file = Some(path.to_path_buf());
                self.history = Record::new(); // Clear history for loaded file
                self.selected_node = None;
                // Reset node count based on loaded graph
                self.node_count = self.snarl.node_ids().count();
                self.set_status(format!("Loaded: {}", path.display()));
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
    }

    /// Save to current file, or open save dialog if no current file.
    fn save_current(&mut self) {
        if let Some(path) = self.current_file.clone() {
            self.save_to_path(&path);
        } else {
            self.save_file_dialog();
        }
    }

    /// Open save dialog and save to selected file.
    fn save_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Experiment Graph", &[GRAPH_FILE_EXTENSION])
            .set_file_name("experiment.expgraph")
            .save_file()
        {
            self.save_to_path(&path);
        }
    }

    /// Save graph to the specified path.
    fn save_to_path(&mut self, path: &std::path::Path) {
        // Update modification timestamp
        self.metadata.modified = Some(chrono::Utc::now().to_rfc3339());
        if self.metadata.created.is_none() {
            self.metadata.created = self.metadata.modified.clone();
        }

        let file = GraphFile::new(self.snarl.clone()).with_metadata(self.metadata.clone());

        match save_graph(path, &file) {
            Ok(()) => {
                self.current_file = Some(path.to_path_buf());
                self.set_status(format!("Saved: {}", path.display()));
            }
            Err(e) => {
                self.set_status(format!("Error: {e}"));
            }
        }
    }

    /// Set a status message that auto-fades after 3 seconds.
    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), std::time::Instant::now()));
    }

    // ========== Validation ==========

    /// Show validation status bar at the bottom of the panel.
    fn show_validation_status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let error_count = self.viewer.error_count();
            if error_count > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 100, 100),
                    format!(
                        "{} validation error{}",
                        error_count,
                        if error_count == 1 { "" } else { "s" }
                    ),
                );

                // Show first error as summary
                if let Some((node_id, error)) = self.viewer.node_errors.iter().next() {
                    if let Some(node) = self.snarl.get_node(*node_id) {
                        ui.label(format!("- {}: {}", node.node_name(), error));
                    }
                }
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(100, 200, 100),
                    "Graph valid",
                );
            }
        });
    }

    /// Validate the entire graph and update error display.
    fn validate_graph(&mut self) {
        self.viewer.clear_all_errors();

        // Check for cycles first (graph-level validation)
        if let Some(cycle_error) = crate::graph::validation::validate_graph_structure(&self.snarl)
        {
            // Set error on first node as a way to show the error
            if let Some((first_id, _)) = self.snarl.node_ids().next() {
                self.viewer.set_node_error(first_id, cycle_error);
            }
            return; // Don't do per-node validation if there's a cycle
        }

        // Per-node validation (existing code)
        let errors: Vec<_> = self
            .snarl
            .node_ids()
            .filter_map(|(node_id, node)| {
                self.validate_node(node).map(|error| (node_id, error))
            })
            .collect();

        // Apply errors after iteration
        for (node_id, error) in errors {
            self.viewer.set_node_error(node_id, error);
        }
    }

    /// Validate a single node, returning an error message if invalid.
    fn validate_node(&self, node: &ExperimentNode) -> Option<String> {
        match node {
            ExperimentNode::Scan {
                actuator, points, ..
            } => {
                if actuator.is_empty() {
                    return Some("Actuator not set".to_string());
                }
                if *points == 0 {
                    return Some("Points must be > 0".to_string());
                }
            }
            ExperimentNode::Acquire(config) => {
                if config.detector.is_empty() {
                    return Some("Detector not set".to_string());
                }
                if config.frame_count == 0 {
                    return Some("Frame count must be > 0".to_string());
                }
            }
            ExperimentNode::Move(config) => {
                if config.device.is_empty() {
                    return Some("Device not set".to_string());
                }
            }
            ExperimentNode::Wait { condition } => {
                use crate::graph::nodes::WaitCondition;
                match condition {
                    WaitCondition::Duration { milliseconds } => {
                        if *milliseconds <= 0.0 {
                            return Some("Duration must be > 0".to_string());
                        }
                    }
                    WaitCondition::Threshold { timeout_ms, .. } => {
                        if *timeout_ms <= 0.0 {
                            return Some("Timeout must be > 0".to_string());
                        }
                    }
                    WaitCondition::Stability { timeout_ms, .. } => {
                        if *timeout_ms <= 0.0 {
                            return Some("Timeout must be > 0".to_string());
                        }
                    }
                }
            }
            ExperimentNode::Loop(config) => {
                use crate::graph::nodes::LoopTermination;
                match &config.termination {
                    LoopTermination::Count { iterations } => {
                        if *iterations == 0 {
                            return Some("Iterations must be > 0".to_string());
                        }
                    }
                    LoopTermination::Condition { max_iterations, .. } => {
                        if *max_iterations == 0 {
                            return Some("Max iterations must be > 0".to_string());
                        }
                    }
                    LoopTermination::Infinite { max_iterations } => {
                        if *max_iterations == 0 {
                            return Some("Max iterations must be > 0".to_string());
                        }
                    }
                }
            }
        }
        None
    }

    // ========== Execution Controls ==========

    fn show_execution_toolbar(&mut self, ui: &mut egui::Ui, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let has_errors = self.viewer.error_count() > 0;
        let is_running = self.execution_state.is_running();
        let is_paused = self.execution_state.is_paused();
        let is_idle = !self.execution_state.is_active();
        let is_empty = self.snarl.node_ids().count() == 0;

        // Run button - enabled when idle, no validation errors, and graph is not empty
        let can_run = is_idle && !has_errors && !is_empty;
        let run_hover_text = if is_empty {
            "Add nodes to graph first"
        } else if has_errors {
            "Fix validation errors first"
        } else if !is_idle {
            "Execution already in progress"
        } else {
            "Execute the experiment"
        };
        let run_clicked = ui.add_enabled(can_run, egui::Button::new("▶ Run"))
            .on_hover_text(run_hover_text)
            .clicked();

        // Pause button - enabled when running
        let pause_clicked = ui.add_enabled(is_running, egui::Button::new("⏸ Pause"))
            .on_hover_text("Pause at next checkpoint")
            .clicked();

        // Resume button - enabled when paused
        let resume_clicked = ui.add_enabled(is_paused, egui::Button::new("▶ Resume"))
            .on_hover_text("Resume execution")
            .clicked();

        // Abort button - enabled when running or paused
        let abort_clicked = ui.add_enabled(is_running || is_paused, egui::Button::new("⏹ Abort"))
            .on_hover_text("Abort execution")
            .clicked();

        // Handle button clicks AFTER all UI (only one can be clicked per frame)
        if run_clicked {
            self.run_experiment(client, runtime);
        } else if pause_clicked {
            self.pause_experiment(client, runtime);
        } else if resume_clicked {
            self.resume_experiment(client, runtime);
        } else if abort_clicked {
            self.abort_experiment(client, runtime);
        }

        // Progress display
        if self.execution_state.is_active() {
            ui.separator();
            let progress = self.execution_state.progress();
            ui.add(egui::ProgressBar::new(progress).show_percentage());

            let status_text = match self.execution_state.engine_state {
                EngineStateLocal::Running => {
                    format!("Running: {}/{}", self.execution_state.current_event, self.execution_state.total_events)
                }
                EngineStateLocal::Paused => "Paused".to_string(),
                _ => String::new(),
            };
            ui.label(status_text);

            // ETA
            if let Some(eta) = self.execution_state.estimated_remaining() {
                ui.label(format!("ETA: {:.0}s", eta.as_secs_f64()));
            }
        }

        // Error display
        if let Some(err) = &self.last_error {
            ui.colored_label(egui::Color32::RED, err);
        }
    }

    fn run_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        // Clear previous errors
        self.last_error = None;

        // Check connection
        let Some(_client) = client else {
            self.last_error = Some("Not connected to daemon".to_string());
            return;
        };
        let Some(_runtime) = runtime else {
            self.last_error = Some("No runtime available".to_string());
            return;
        };

        // Check for empty graph
        if self.snarl.node_ids().count() == 0 {
            self.last_error = Some("Graph is empty - add nodes first".to_string());
            return;
        }

        // Check for validation errors (including cycles)
        self.validate_graph();
        if self.viewer.error_count() > 0 {
            self.last_error = Some(format!(
                "{} validation error(s) - fix before running",
                self.viewer.error_count()
            ));
            return;
        }

        // Translate graph to plan
        let plan = match GraphPlan::from_snarl(&self.snarl) {
            Ok(p) => p,
            Err(e) => {
                self.last_error = Some(format!("Translation error: {}", e));
                return;
            }
        };

        // Check plan has events to execute
        let total_events = plan.num_points() as u32;
        if total_events == 0 {
            self.last_error = Some("Graph produces no events - add Scan or Acquire nodes".to_string());
            return;
        }

        // Start execution
        self.execution_state.start_execution("pending".to_string(), total_events);
        self.set_status(format!("Starting experiment with {} events", total_events));

        // TODO: Queue plan via gRPC
        // For full implementation, need to either:
        // 1. Serialize GraphPlan and send via QueuePlan with plan_type="graph_plan"
        // 2. Or convert to an existing plan type the server understands
        //
        // For now, the UI shows execution state for demo purposes.
        // Full server integration would require:
        // - Server accepting GraphPlan or serialized commands
        // - Or translating to LineScan/GridScan based on graph content
    }

    fn pause_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else { return; };
        let Some(runtime) = runtime else { return; };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.pause_engine(true).await {
                Ok(_) => { let _ = tx.send(ExecutionAction::StatusUpdate { state: 2, current_event: None, total_events: None }).await; }
                Err(e) => { let _ = tx.send(ExecutionAction::Error(e.to_string())).await; }
            }
        });
    }

    fn resume_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else { return; };
        let Some(runtime) = runtime else { return; };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.resume_engine().await {
                Ok(_) => { let _ = tx.send(ExecutionAction::StatusUpdate { state: 1, current_event: None, total_events: None }).await; }
                Err(e) => { let _ = tx.send(ExecutionAction::Error(e.to_string())).await; }
            }
        });
    }

    fn abort_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else { return; };
        let Some(runtime) = runtime else { return; };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.abort_plan(None).await {
                Ok(_) => { let _ = tx.send(ExecutionAction::Completed).await; }
                Err(e) => { let _ = tx.send(ExecutionAction::Error(e.to_string())).await; }
            }
        });
    }

    fn poll_execution_actions(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                ExecutionAction::Started { run_uid, total_events } => {
                    self.execution_state.start_execution(run_uid, total_events);
                }
                ExecutionAction::StatusUpdate { state, current_event, total_events } => {
                    self.execution_state.update_from_status(state, current_event, total_events);
                }
                ExecutionAction::Completed => {
                    self.execution_state.reset();
                    self.set_status("Execution completed");
                }
                ExecutionAction::Error(e) => {
                    self.last_error = Some(e);
                    self.execution_state.reset();
                }
            }
        }
    }

    /// Poll engine status to keep execution state in sync with daemon
    fn poll_engine_status(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else { return; };
        let Some(runtime) = runtime else { return; };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.get_engine_status().await {
                Ok(status) => {
                    let _ = tx.send(ExecutionAction::StatusUpdate {
                        state: status.state,
                        current_event: status.current_event_number,
                        total_events: status.total_events_expected,
                    }).await;
                }
                Err(_) => {
                    // Ignore polling errors - transient network issues shouldn't disrupt UI
                }
            }
        });

        // Mark that we've initiated a poll (update timestamp to avoid rapid polling)
        self.execution_state.last_update = std::time::Instant::now();
    }

    // ========== Runtime Parameter Editing ==========

    /// Collect editable parameters from graph nodes
    fn collect_editable_parameters(&self) -> Vec<EditableParameter> {
        let mut params = Vec::new();

        for (_, node) in self.snarl.node_ids() {
            match node {
                ExperimentNode::Scan {
                    actuator, start, ..
                } => {
                    if !actuator.is_empty() {
                        params.push(EditableParameter::float(
                            actuator,
                            "position",
                            &format!("{} Position", actuator),
                            *start,
                        ));
                    }
                }
                ExperimentNode::Acquire(config) => {
                    if !config.detector.is_empty() {
                        let exposure = config.exposure_ms.unwrap_or(100.0);
                        params.push(EditableParameter::float_ranged(
                            &config.detector,
                            "exposure_ms",
                            &format!("{} Exposure (ms)", config.detector),
                            exposure,
                            1.0,
                            10000.0,
                        ));
                    }
                }
                ExperimentNode::Move(config) => {
                    if !config.device.is_empty() {
                        params.push(EditableParameter::float(
                            &config.device,
                            "position",
                            &format!("{} Position", config.device),
                            config.position,
                        ));
                    }
                }
                ExperimentNode::Wait { condition } => {
                    use crate::graph::nodes::WaitCondition;
                    match condition {
                        WaitCondition::Duration { milliseconds } => {
                            params.push(EditableParameter::float_ranged(
                                "",
                                "wait_duration",
                                "Wait Duration (ms)",
                                *milliseconds,
                                0.0,
                                60000.0,
                            ));
                        }
                        _ => {
                            // Condition-based waits not editable at runtime yet
                        }
                    }
                }
                ExperimentNode::Loop(..) => {
                    // Loop iterations not typically editable at runtime
                }
            }
        }

        // Deduplicate by device_id + name
        params.sort_by(|a, b| (&a.device_id, &a.name).cmp(&(&b.device_id, &b.name)));
        params.dedup_by(|a, b| a.device_id == b.device_id && a.name == b.name);

        params
    }

    /// Show parameter editor panel when execution is active
    fn show_parameter_editor_panel(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) {
        let is_paused = self.execution_state.is_paused();

        ui.group(|ui| {
            ui.heading("Runtime Parameters");
            if !is_paused {
                ui.label("(Pause execution to modify parameters)");
            }
            ui.separator();

            // Populate parameters if we just entered paused state
            if is_paused && self.editable_params.is_empty() {
                self.editable_params = self.collect_editable_parameters();
            }

            // Clear when not active
            if !self.execution_state.is_active() && !self.editable_params.is_empty() {
                self.editable_params.clear();
            }

            // Show parameter editors
            let results = RuntimeParameterEditor::show_group(
                ui,
                "Device Parameters",
                &mut self.editable_params,
                is_paused,
            );

            // Handle modifications
            for result in results {
                if let RuntimeParameterEditResult::Modified {
                    ref device_id,
                    ref param_name,
                    ref new_value,
                } = result
                {
                    self.send_parameter_update(device_id, param_name, new_value, client, runtime);
                }
            }
        });
    }

    /// Send parameter update to daemon via gRPC
    fn send_parameter_update(
        &mut self,
        device_id: &str,
        param_name: &str,
        new_value: &str,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) {
        if device_id.is_empty() {
            // Local parameter (like wait duration) - just update status
            self.set_status(format!("Updated {} to {}", param_name, new_value));
            return;
        }

        let Some(client) = client else {
            self.last_error = Some("Not connected".to_string());
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        // Clone values for status message before moving into closure
        let device_id_display = device_id.to_string();
        let param_name_display = param_name.to_string();
        let new_value_display = new_value.to_string();

        let tx = self.action_tx.clone();
        let mut client = client.clone();
        let device_id = device_id.to_string();
        let param_name = param_name.to_string();
        let new_value = new_value.to_string();

        runtime.spawn(async move {
            match client.set_parameter(&device_id, &param_name, &new_value).await {
                Ok(_) => {
                    // Success - no action needed, value already updated in UI
                }
                Err(e) => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!(
                            "Failed to set {}: {}",
                            param_name, e
                        )))
                        .await;
                }
            }
        });

        self.set_status(format!(
            "Set {}.{} = {}",
            device_id_display, param_name_display, new_value_display
        ));
    }
}
