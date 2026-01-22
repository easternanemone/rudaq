//! Experiment Designer panel with node graph editor.

use std::path::PathBuf;

use egui_snarl::ui::{get_selected_nodes, SnarlStyle};
use egui_snarl::{NodeId, Snarl};
use undo::Record;

use crate::graph::commands::{AddNodeData, GraphEdit, ModifyNodeData};
use crate::graph::{
    load_graph, save_graph, ExperimentNode, ExperimentViewer, GraphFile, GraphMetadata,
    GRAPH_FILE_EXTENSION,
};
use crate::widgets::node_palette::{NodePalette, NodeType};
use crate::widgets::PropertyInspector;

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
}

impl Default for ExperimentDesignerPanel {
    fn default() -> Self {
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
        }
    }
}

impl ExperimentDesignerPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
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

        egui::SidePanel::right("property_inspector_panel")
            .resizable(true)
            .default_width(200.0)
            .min_width(150.0)
            .max_width(400.0)
            .show_inside(ui, |ui| {
                self.show_property_inspector(ui);
            });

        // Main canvas area
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Handle context menu for adding nodes
            self.handle_context_menu(ui);

            // Handle drop onto canvas
            self.handle_canvas_drop(ui);

            // Graph canvas - takes remaining space
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let id = egui::Id::new("experiment_graph");
                self.snarl.show(&mut self.viewer, &self.style, id, ui);
            });
        });
    }

    fn show_property_inspector(&mut self, ui: &mut egui::Ui) {
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
                if let Some(modified_node) = PropertyInspector::show(ui, &node_clone) {
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
            ExperimentNode::Acquire { detector, .. } => {
                if detector.is_empty() {
                    return Some("Detector not set".to_string());
                }
            }
            ExperimentNode::Move { device, .. } => {
                if device.is_empty() {
                    return Some("Device not set".to_string());
                }
            }
            ExperimentNode::Wait { duration_ms } => {
                if *duration_ms <= 0.0 {
                    return Some("Duration must be > 0".to_string());
                }
            }
            ExperimentNode::Loop { iterations } => {
                if *iterations == 0 {
                    return Some("Iterations must be > 0".to_string());
                }
            }
        }
        None
    }
}
