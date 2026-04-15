// Panel under active development; not yet fully integrated (bd-m5fh.4.2).
#![allow(dead_code)]
//! Experiment Designer panel with node graph editor.

use std::path::PathBuf;

use crate::runtime::Runtime;
use egui_snarl::ui::{BackgroundPattern, SnarlStyle, SnarlWidget, WireLayer, WireStyle};
use egui_snarl::{NodeId, Snarl};
use tokio::sync::mpsc;
use undo::Record;

use crate::graph::commands::{AddNodeData, GraphEdit, ModifyNodeData};
use crate::graph::{
    EngineStateLocal, ExecutionState, ExperimentNode, ExperimentViewer, GRAPH_FILE_EXTENSION,
    GraphFile, GraphMetadata, GraphPlan, graph_to_rhai_script, load_graph, save_graph,
};
use crate::panels::{
    CodePreviewPanel, DataUpdate, DataUpdateSender, FrameUpdate, FrameUpdateSender,
    LiveVisualizationPanel, SpectrumUpdate, SpectrumUpdateSender, data_channel, frame_channel,
    spectrum_channel,
};
use crate::widgets::node_palette::{NodePalette, NodeType};
use crate::widgets::{
    AdaptiveAlertData, AdaptiveAlertResponse, EditableParameter, PropertyInspector,
    RuntimeParameterEditResult, RuntimeParameterEditor, show_adaptive_alert,
};
use client::DaqClient;
use experiment::Plan;
use futures::StreamExt;
use protocol::daq::StreamQuality;

/// Type alias for camera detector info: (device_id, title)
type CameraInfo = (String, String);

/// Type alias for plot detector info: (device_id, label, title)
type PlotInfo = (String, String, String);

/// Type alias for spectrum detector info: (device_id, label, title)
type SpectrumInfo = (String, String, String);

/// Actions from async execution operations
enum ExecutionAction {
    Started {
        run_uid: String,
        total_events: u32,
    },
    StatusUpdate {
        state: i32,
        current_event: Option<u32>,
        total_events: Option<u32>,
    },
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
    status_message: Option<(String, crate::time::Instant)>,
    /// Execution state for visual feedback
    execution_state: ExecutionState,
    /// Channel for async action results
    action_tx: mpsc::Sender<ExecutionAction>,
    action_rx: mpsc::Receiver<ExecutionAction>,
    /// Last error message
    last_error: Option<String>,
    /// Parameters available for editing while paused
    editable_params: Vec<EditableParameter>,
    /// Live visualization panel (shown during execution)
    visualization_panel: Option<LiveVisualizationPanel>,
    /// Frame update sender (for camera data)
    frame_tx: Option<FrameUpdateSender>,
    /// Data update sender (for plot data)
    data_tx: Option<DataUpdateSender>,
    /// Spectrum update sender (for streamed spectrum data)
    spectrum_tx: Option<SpectrumUpdateSender>,
    /// Camera streaming tasks (for cleanup)
    camera_stream_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Document stream task (for cleanup)
    document_stream_task: Option<tokio::task::JoinHandle<()>>,
    /// Spectrum streaming tasks (for cleanup)
    spectrum_stream_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Metadata editor for run metadata
    metadata_editor: crate::widgets::MetadataEditor,
    /// Whether to show eject confirmation dialog
    show_eject_confirmation: bool,
    /// Script editor panel (Some when ejected)
    script_editor: Option<crate::panels::ScriptEditorPanel>,
    /// Code preview panel (shows generated Rhai)
    code_preview: CodePreviewPanel,
    /// Graph version counter (incremented on each edit)
    graph_version: u64,
    /// Cached device IDs for dropdown selectors
    cached_device_ids: Vec<String>,
    /// Last time device list was fetched
    last_device_fetch: Option<crate::time::Instant>,
    /// Whether to show flattened progress (vs nested) for multi-dimensional scans
    show_flattened_progress: bool,
    /// Active adaptive alert (if any)
    adaptive_alert: Option<AdaptiveAlertData>,
    /// Timestamp when auto-proceed should trigger (for non-approval alerts)
    adaptive_alert_auto_proceed_at: Option<crate::time::Instant>,
}

/// Create custom SnarlStyle for the experiment designer.
fn create_node_style() -> SnarlStyle {
    SnarlStyle {
        // Larger pins for easier mouse targeting
        pin_size: Some(8.0),

        // Orthogonal wires (cleaner for DAQ flow graphs)
        wire_style: Some(WireStyle::AxisAligned { corner_radius: 4.0 }),
        wire_width: Some(2.0),
        wire_layer: Some(WireLayer::BehindNodes), // Don't obscure inline editors

        // No grid background (cleaner)
        bg_pattern: Some(BackgroundPattern::NoPattern),

        // Better selection visibility (note: API typo is intentional)
        select_stoke: Some(egui::Stroke::new(
            2.0,
            egui::Color32::from_rgb(100, 150, 255),
        )),

        ..Default::default()
    }
}

impl Default for ExperimentDesignerPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);
        Self {
            snarl: Snarl::new(),
            viewer: ExperimentViewer::new(),
            style: create_node_style(),
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
            visualization_panel: None,
            frame_tx: None,
            data_tx: None,
            spectrum_tx: None,
            camera_stream_tasks: Vec::new(),
            document_stream_task: None,
            spectrum_stream_tasks: Vec::new(),
            metadata_editor: crate::widgets::MetadataEditor::new(),
            show_eject_confirmation: false,
            script_editor: None,
            code_preview: CodePreviewPanel::new(),
            graph_version: 0,
            cached_device_ids: Vec::new(),
            last_device_fetch: None,
            show_flattened_progress: false,
            adaptive_alert: None,
            adaptive_alert_auto_proceed_at: None,
        }
    }
}

impl ExperimentDesignerPanel {
    fn calibration_staleness_warning(error: &str) -> Option<String> {
        let marker = "CalibrationStalenessGate:";
        error
            .find(marker)
            .map(|idx| error[idx..].trim().to_string())
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&mut DaqClient>,
        runtime: Option<&Runtime>,
    ) {
        // If ejected, show script editor instead
        if let Some(editor) = &mut self.script_editor {
            // Track if user wants to return to graph mode
            let mut return_to_graph = false;

            // Add "New Graph" button (creates new graph, loses script changes)
            ui.horizontal(|ui| {
                if ui
                    .button("New Graph")
                    .on_hover_text("Start a new visual graph (script changes will be lost)")
                    .clicked()
                {
                    return_to_graph = true;
                }
            });
            ui.separator();

            // Show editor (safe borrow after button logic)
            editor.ui(ui, client, runtime);

            // Handle return to graph mode
            if return_to_graph {
                self.script_editor = None;
                self.new_graph();
            }

            return;
        }

        // Poll for async results
        self.poll_execution_actions();

        // Clone client for use in multiple places (DaqClient is Clone)
        // We clone first to preserve client for later use
        let client_clone: Option<DaqClient> = client.as_deref().cloned();

        // Poll engine status when execution is active (every 500ms)
        if self.execution_state.is_active()
            && self.execution_state.last_update.elapsed() > std::time::Duration::from_millis(500)
        {
            self.poll_engine_status(client_clone.as_ref(), runtime);
        }

        // Fetch device list periodically (every 10s) for dropdown selectors
        let should_fetch_devices = self
            .last_device_fetch
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(10));
        if should_fetch_devices {
            self.update_device_list(client_clone.as_ref(), runtime);
        }

        // Handle keyboard shortcuts FIRST (before any UI that might consume keys)
        self.handle_keyboard(ui);

        // Update code preview if visible
        self.code_preview.update(&self.snarl, self.graph_version);

        // Render code preview panel (right side) BEFORE main panel so it claims space
        self.code_preview.ui_inside(ui);

        // Top toolbar with file operations and undo/redo buttons
        ui.horizontal(|ui| {
            ui.label("Experiment Designer");
            ui.separator();

            // File operations
            if ui
                .button("New")
                .on_hover_text("Start a new graph")
                .clicked()
            {
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

            // Code preview toggle
            let code_label = if self.code_preview.is_visible() {
                "Hide Code"
            } else {
                "Show Code"
            };
            if ui
                .button(code_label)
                .on_hover_text("Toggle generated Rhai code preview (CODE-01)")
                .clicked()
            {
                self.code_preview.toggle();
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

            // Export Rhai button
            if ui
                .button("Export Rhai...")
                .on_hover_text("Export as standalone Rhai script file (CODE-02)")
                .clicked()
            {
                self.export_rhai_dialog();
            }

            // Eject to Script button
            if ui
                .button("Eject to Script")
                .on_hover_text(
                    "Convert to editable script (one-way, cannot return to graph) (CODE-03)",
                )
                .clicked()
            {
                self.show_eject_confirmation = true;
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

        // Run metadata section (collapsible, before execution controls)
        egui::CollapsingHeader::new("Run Metadata (Optional)")
            .default_open(false)
            .show(ui, |ui| {
                self.metadata_editor.ui(ui);
            });

        ui.separator();

        // Execution controls (separate row for more space)
        ui.horizontal(|ui| {
            self.show_execution_toolbar(ui, client_clone.as_ref(), runtime);
        });

        ui.separator();

        // Live visualization (shown during execution)
        if let Some(ref mut panel) = self.visualization_panel {
            egui::CollapsingHeader::new("Live Visualization")
                .default_open(true)
                .show(ui, |ui| {
                    panel.show(ui);
                });
            ui.separator();
        }

        // Eject confirmation dialog
        if self.show_eject_confirmation {
            egui::Window::new("Eject to Script Mode?")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("This will convert your visual graph to an editable Rhai script.");
                    ui.label("");
                    ui.label("WARNING: This is one-way. You cannot convert the script back to a visual graph.");
                    ui.label("Your .expgraph file will remain unchanged.");
                    ui.label("");

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.show_eject_confirmation = false;
                        }

                        if ui.button("Eject").clicked() {
                            self.eject_to_script();
                            self.show_eject_confirmation = false;
                        }
                    });
                });
        }

        // Show adaptive alert modal if present
        if let Some(ref alert_data) = self.adaptive_alert {
            let response = show_adaptive_alert(ui.ctx(), alert_data);

            match response {
                AdaptiveAlertResponse::Approved => {
                    // Resume execution with approved action; keep modal open on failure
                    if self.confirm_adaptive_action(client_clone.as_ref(), runtime) {
                        self.adaptive_alert = None;
                        self.adaptive_alert_auto_proceed_at = None;
                    }
                }
                AdaptiveAlertResponse::Cancelled => {
                    // Cancel adaptive action and abort the plan; keep modal open on failure
                    if self.cancel_adaptive_action(client_clone.as_ref(), runtime) {
                        self.adaptive_alert = None;
                        self.adaptive_alert_auto_proceed_at = None;
                    }
                }
                AdaptiveAlertResponse::Pending => {
                    // Check auto-proceed timeout for non-approval alerts
                    if let Some(auto_time) = self.adaptive_alert_auto_proceed_at
                        && crate::time::Instant::now() >= auto_time
                        && self.confirm_adaptive_action(client_clone.as_ref(), runtime)
                    {
                        self.adaptive_alert = None;
                        self.adaptive_alert_auto_proceed_at = None;
                    }
                }
            }
        }

        // Run validation each frame (cheap check)
        self.validate_graph();

        // Bottom status bar with validation status
        egui::TopBottomPanel::bottom("validation_status_bar").show_inside(ui, |ui| {
            self.show_validation_status_bar(ui);
        });

        // Two-panel layout: Palette | Canvas (properties now inline in nodes)
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

        // Main canvas area
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Sync execution state to viewer for node highlighting
            if self.execution_state.is_active() {
                self.viewer.execution_state = Some(self.execution_state.clone());
            } else {
                self.viewer.execution_state = None;
            }

            // Sync device list to viewer for dropdown selectors
            self.viewer.device_ids.clone_from(&self.cached_device_ids);

            // Capture canvas rect BEFORE widget consumes space (for drop detection)
            let canvas_rect = ui.available_rect_before_wrap();

            // Define SnarlWidget
            let snarl_id = egui::Id::new("experiment_graph");
            let widget = SnarlWidget::new().id(snarl_id).style(self.style);

            // 1. Render Graph FIRST (so it's the background/base layer)
            widget.show(&mut self.snarl, &mut self.viewer, ui);

            // 2. Render Overlays/Handlers AFTER (so they are on top)
            // Handle context menu for adding nodes
            self.handle_context_menu(ui);

            // Handle drop onto canvas (using pre-captured rect)
            self.handle_canvas_drop_at(ui, canvas_rect);

            // 3. Query selection using the widget instance (ensures ID consistency)
            let selected = widget.get_selected_nodes(ui);

            // DEBUG: print on every click
            if ui.input(|i| i.pointer.any_click()) {
                eprintln!(
                    "CLICK - dragging: {:?}, selected: {:?}",
                    self.dragging_node.as_ref().map(|n| n.name()),
                    selected
                );
            }

            self.selected_node = selected.first().copied();
        });
    }

    fn show_property_inspector(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) {
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
                if let Some(modified_node) =
                    PropertyInspector::show(ui, &node_clone, &self.cached_device_ids)
                {
                    // Create undo-tracked modification
                    self.history.edit(
                        &mut self.snarl,
                        GraphEdit::ModifyNode(ModifyNodeData {
                            node_id,
                            old_data: node_clone,
                            new_data: modified_node,
                        }),
                    );
                    self.graph_version = self.graph_version.wrapping_add(1);
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
        // Only process if no text widget has focus (otherwise user is editing text)
        let text_edit_has_focus =
            ui.ctx().memory(|mem| mem.focused().is_some()) && ui.ctx().wants_keyboard_input();
        if !text_edit_has_focus
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && let Some(node_id) = self.selected_node.take()
        {
            // For now, just remove directly (could use RemoveNode command for undo)
            // We do direct removal because RemoveNode would need the node position
            // which we'd need to look up, making it more complex
            self.snarl.remove_node(node_id);
            self.graph_version = self.graph_version.wrapping_add(1);
        }
    }

    fn undo(&mut self) {
        self.history.undo(&mut self.snarl);
        self.graph_version = self.graph_version.wrapping_add(1);
    }

    fn redo(&mut self) {
        self.history.redo(&mut self.snarl);
        self.graph_version = self.graph_version.wrapping_add(1);
    }

    fn handle_context_menu(&mut self, ui: &mut egui::Ui) {
        // Check for right-click to open context menu
        // Don't use ui.interact() at all - just check input state directly
        // This avoids consuming any clicks that should go to snarl
        let canvas_rect = ui.available_rect_before_wrap();

        if ui.input(|i| i.pointer.secondary_clicked())
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            && canvas_rect.contains(pos)
        {
            self.context_menu_pos = Some(pos);
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
                                self.graph_version = self.graph_version.wrapping_add(1);
                                close_menu = true;
                            }
                        }

                        // Close menu on click outside or after adding node
                        // Use primary_clicked() instead of any_click() to avoid closing
                        // on the same right-click that opened the menu
                        if close_menu
                            || (ui.input(|i| i.pointer.primary_clicked())
                                && !ui.rect_contains_pointer(ui.min_rect()))
                        {
                            self.context_menu_pos = None;
                        }
                    });
                });

            // Close menu when clicking elsewhere
            // Ignore secondary click to prevent closing immediately on the same frame it opens
            if ui.input(|i| {
                i.pointer.any_click()
                    && !i.pointer.secondary_clicked()
                    && i.pointer.hover_pos().is_some_and(|p| p != pos)
            }) {
                self.context_menu_pos = None;
            }
        }
    }

    fn handle_canvas_drop_at(&mut self, ui: &mut egui::Ui, canvas_rect: egui::Rect) {
        // Check if we're dragging and the mouse was released over the canvas
        if let Some(node_type) = self.dragging_node {
            // Use the pre-captured canvas rect (before widget consumed space)
            let response = ui.interact(
                canvas_rect,
                egui::Id::new("canvas_drop_zone"),
                egui::Sense::hover(),
            );

            if response.hovered() {
                // Show drop indicator
                ui.painter().rect_stroke(
                    canvas_rect,
                    egui::CornerRadius::same(4),
                    egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
            }

            // Check if drag ended (mouse released)
            if !ui.input(|i| i.pointer.any_down()) {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
                    && canvas_rect.contains(pos)
                {
                    // Create node at drop position (convert screen coords to canvas-relative)
                    let node_pos = pos - canvas_rect.min.to_vec2();
                    self.node_count += 1; // Keep counter in sync for context menu adds
                    let node = node_type.create_node();
                    self.history.edit(
                        &mut self.snarl,
                        GraphEdit::AddNode(AddNodeData {
                            node,
                            position: node_pos,
                            node_id: None,
                        }),
                    );
                    self.graph_version = self.graph_version.wrapping_add(1);
                }
                self.dragging_node = None;
            }
        }
    }

    /// Generate a position for the next node to avoid overlapping.
    fn next_node_position(&mut self) -> egui::Pos2 {
        #[allow(clippy::cast_precision_loss)]
        let x = 50.0 + (self.node_count % 5) as f32 * 180.0;
        #[allow(clippy::cast_precision_loss)]
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
        self.graph_version = self.graph_version.wrapping_add(1);
        self.set_status("New graph created");
    }

    /// Open file dialog and load selected file.
    fn open_file_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Experiment Graph", &[GRAPH_FILE_EXTENSION])
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            self.load_from_path(&path);
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.set_status("File dialogs are not available in the browser");
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
                self.graph_version = self.graph_version.wrapping_add(1);
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
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Experiment Graph", &[GRAPH_FILE_EXTENSION])
            .set_file_name("experiment.expgraph")
            .save_file()
        {
            self.save_to_path(&path);
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.set_status("File dialogs are not available in the browser");
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
        self.status_message = Some((msg.into(), crate::time::Instant::now()));
    }

    /// Open file dialog to export graph as Rhai script.
    fn export_rhai_dialog(&mut self) {
        // Generate code first
        let source_name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string());
        let code = graph_to_rhai_script(&self.snarl, source_name.as_deref());

        // Suggest filename based on current graph file
        let suggested_name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.rhai", s.to_string_lossy()))
            .unwrap_or_else(|| "experiment.rhai".to_string());

        // Open save dialog
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Rhai Script", &["rhai"])
            .set_file_name(&suggested_name)
            .save_file()
        {
            match std::fs::write(&path, &code) {
                Ok(()) => {
                    self.set_status(format!("Exported to {}", path.display()));
                }
                Err(e) => {
                    self.set_status(format!("Export failed: {}", e));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (code, suggested_name); // suppress unused variable warnings
            self.set_status("File dialogs are not available in the browser");
        }
    }

    /// Eject to script editor mode (one-way conversion).
    fn eject_to_script(&mut self) {
        let source_name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string());
        let code = graph_to_rhai_script(&self.snarl, source_name.as_deref());

        self.script_editor = Some(crate::panels::ScriptEditorPanel::from_graph_code(
            code,
            self.current_file.clone(),
        ));
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
                if let Some((node_id, error)) = self.viewer.node_errors.iter().next()
                    && let Some(node) = self.snarl.get_node(*node_id)
                {
                    ui.label(format!("- {}: {}", node.node_name(), error));
                }
            } else {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "Graph valid");
            }
        });
    }

    /// Validate the entire graph and update error display.
    fn validate_graph(&mut self) {
        self.viewer.clear_all_errors();

        // Check for cycles first (graph-level validation)
        if let Some(cycle_error) = crate::graph::validation::validate_graph_structure(&self.snarl) {
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
            .filter_map(|(node_id, node)| self.validate_node(node).map(|error| (node_id, error)))
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
            ExperimentNode::NestedScan(config) => {
                if config.outer.actuator.is_empty() {
                    return Some("Outer actuator not set".to_string());
                }
                if config.inner.actuator.is_empty() {
                    return Some("Inner actuator not set".to_string());
                }
                if config.outer.points == 0 {
                    return Some("Outer points must be > 0".to_string());
                }
                if config.inner.points == 0 {
                    return Some("Inner points must be > 0".to_string());
                }
            }
            ExperimentNode::AdaptiveScan(config) => {
                if config.scan.actuator.is_empty() {
                    return Some("Scan actuator not set".to_string());
                }
                if config.scan.points == 0 {
                    return Some("Scan points must be > 0".to_string());
                }
                if config.triggers.is_empty() {
                    return Some("At least one trigger required".to_string());
                }
            }
        }
        None
    }

    // ========== Execution Controls ==========

    fn show_execution_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) {
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
        let run_clicked = ui
            .add_enabled(can_run, egui::Button::new("▶ Run"))
            .on_hover_text(run_hover_text)
            .clicked();

        // Pause button - enabled when running
        let pause_clicked = ui
            .add_enabled(is_running, egui::Button::new("⏸ Pause"))
            .on_hover_text("Pause at next checkpoint")
            .clicked();

        // Resume button - enabled when paused
        let resume_clicked = ui
            .add_enabled(is_paused, egui::Button::new("▶ Resume"))
            .on_hover_text("Resume execution")
            .clicked();

        // Abort button - enabled when running or paused
        let abort_clicked = ui
            .add_enabled(is_running || is_paused, egui::Button::new("⏹ Abort"))
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

            // Progress bar always uses flat values for accurate percentage
            let progress = if let Some(ref nested) = self.execution_state.nested_progress {
                nested.progress()
            } else {
                self.execution_state.progress()
            };
            ui.add(egui::ProgressBar::new(progress).show_percentage());

            // Status text with optional nested/flat toggle
            let status_text = match self.execution_state.engine_state {
                EngineStateLocal::Running => {
                    if let Some(ref nested) = self.execution_state.nested_progress {
                        // Multi-dimensional scan: show nested or flat based on toggle
                        if self.show_flattened_progress {
                            format!("Running: {}", nested.format_flat())
                        } else {
                            format!("Running: {}", nested.format_nested())
                        }
                    } else {
                        // Simple scan: show flat progress
                        format!(
                            "Running: {}/{}",
                            self.execution_state.current_event + 1,
                            self.execution_state.total_events
                        )
                    }
                }
                EngineStateLocal::Paused => "Paused".to_string(),
                _ => String::new(),
            };
            ui.label(&status_text);

            // Toggle button for nested vs flat view (only for multi-dimensional scans)
            if self.execution_state.nested_progress.is_some() {
                let toggle_label = if self.show_flattened_progress {
                    "Show Nested"
                } else {
                    "Show Flat"
                };
                if ui.small_button(toggle_label).clicked() {
                    self.show_flattened_progress = !self.show_flattened_progress;
                }
            }

            // ETA
            if let Some(eta) = self.execution_state.estimated_remaining() {
                ui.label(format!("ETA: {:.0}s", eta.as_secs_f64()));
            }
        }

        // Error display
        if let Some(err) = &self.last_error {
            if let Some(warning) = Self::calibration_staleness_warning(err) {
                ui.colored_label(egui::Color32::from_rgb(255, 200, 80), warning);
            } else {
                ui.colored_label(egui::Color32::RED, err);
            }
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
        #[allow(clippy::cast_possible_truncation)]
        let total_events = plan.num_points() as u32;
        if total_events == 0 {
            self.last_error =
                Some("Graph produces no events - add Scan or Acquire nodes".to_string());
            return;
        }

        // Start execution
        self.execution_state
            .start_execution("pending".to_string(), total_events);
        self.set_status(format!("Starting experiment with {} events", total_events));

        // Start visualization (client and runtime already validated as Some above)
        let client = client.unwrap();
        let runtime = runtime.unwrap();
        self.start_visualization(client, runtime);

        // Save plan to database and queue by plan_id (bd-yz1w)
        let mut metadata = self.metadata_editor.to_metadata_map();
        metadata.insert(
            "graph_node_count".to_string(),
            self.snarl.node_ids().count().to_string(),
        );
        metadata.insert(
            "graph_file".to_string(),
            self.current_file
                .as_ref()
                .and_then(|p| p.file_name().and_then(|n| n.to_str()))
                .unwrap_or("unsaved")
                .to_string(),
        );

        // Generate a plan_id from graph name or file, falling back to a timestamp
        let plan_id = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
            .map(|s| format!("graph_{s}"))
            .unwrap_or_else(|| format!("graph_{}", common::time::now_ns() / 1_000_000_000));

        // Serialize commands and graph data for DB storage
        let commands_json = match serde_json::to_string(plan.commands()) {
            Ok(json) => json,
            Err(e) => {
                self.last_error = Some(format!("Failed to serialize commands: {e}"));
                return;
            }
        };

        let graph_data_json = {
            let graph_file = crate::graph::serialization::GraphFile::new(self.snarl.clone());
            match serde_json::to_string(&graph_file) {
                Ok(json) => json,
                Err(e) => {
                    self.last_error = Some(format!("Failed to serialize graph: {e}"));
                    return;
                }
            }
        };

        let plan_name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
            .unwrap_or("Untitled Graph")
            .to_string();

        // Build SavePlanRequest
        let save_request = protocol::daq::SavePlanRequest {
            plan_id: plan_id.clone(),
            name: plan_name,
            description: None,
            plan_type: "graph_plan".to_string(),
            commands_json: Some(commands_json),
            movers: plan.movers(),
            detectors: plan.detectors(),
            num_points: Some(total_events),
            graph_data_json: Some(graph_data_json),
            parameters: Default::default(),
            device_mapping: Default::default(),
        };

        // Spawn async task to save plan, then queue and start
        let tx = self.action_tx.clone();
        let mut client = client.clone();
        runtime.spawn(async move {
            // 1. Save plan to DB
            match client.save_plan(save_request).await {
                Ok(resp) if !resp.success => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!(
                            "Failed to save plan: {}",
                            resp.error_message
                        )))
                        .await;
                    return;
                }
                Err(e) => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!("Failed to save plan: {e}")))
                        .await;
                    return;
                }
                Ok(_) => {}
            }

            // 2. Queue the stored plan
            let run_uid = match client.queue_stored_plan(&plan_id, metadata).await {
                Ok(resp) if resp.success => resp.run_uid,
                Ok(resp) => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!(
                            "Failed to queue plan: {}",
                            resp.error_message
                        )))
                        .await;
                    return;
                }
                Err(e) => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!("Failed to queue plan: {e}")))
                        .await;
                    return;
                }
            };

            // 3. Start the engine
            match client.start_engine().await {
                Ok(_) => {
                    let _ = tx
                        .send(ExecutionAction::Started {
                            run_uid,
                            total_events,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(ExecutionAction::Error(format!(
                            "Failed to start engine: {e}"
                        )))
                        .await;
                }
            }
        });
    }

    fn pause_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else {
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.pause_engine(true).await {
                Ok(_) => {
                    let _ = tx
                        .send(ExecutionAction::StatusUpdate {
                            state: 2,
                            current_event: None,
                            total_events: None,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(ExecutionAction::Error(e.to_string())).await;
                }
            }
        });
    }

    fn resume_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else {
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.resume_engine().await {
                Ok(_) => {
                    let _ = tx
                        .send(ExecutionAction::StatusUpdate {
                            state: 1,
                            current_event: None,
                            total_events: None,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(ExecutionAction::Error(e.to_string())).await;
                }
            }
        });
    }

    fn abort_experiment(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else {
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        // Stop visualization immediately
        self.stop_visualization();

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.abort_plan(None).await {
                Ok(_) => {
                    let _ = tx.send(ExecutionAction::Completed).await;
                }
                Err(e) => {
                    let _ = tx.send(ExecutionAction::Error(e.to_string())).await;
                }
            }
        });
    }

    fn poll_execution_actions(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                ExecutionAction::Started {
                    run_uid,
                    total_events,
                } => {
                    self.execution_state.start_execution(run_uid, total_events);
                }
                ExecutionAction::StatusUpdate {
                    state,
                    current_event,
                    total_events,
                } => {
                    self.execution_state
                        .update_from_status(state, current_event, total_events);
                }
                ExecutionAction::Completed => {
                    self.stop_visualization();
                    self.execution_state.reset();
                    self.set_status("Execution completed");
                }
                ExecutionAction::Error(e) => {
                    self.stop_visualization();
                    self.last_error = Some(e);
                    self.execution_state.reset();
                }
            }
        }
    }

    /// Poll engine status to keep execution state in sync with daemon
    fn poll_engine_status(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else {
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        let tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.get_engine_status().await {
                Ok(status) => {
                    let _ = tx
                        .send(ExecutionAction::StatusUpdate {
                            state: status.state,
                            current_event: status.current_event_number,
                            total_events: status.total_events_expected,
                        })
                        .await;
                }
                Err(_) => {
                    // Ignore polling errors - transient network issues shouldn't disrupt UI
                }
            }
        });

        // Mark that we've initiated a poll (update timestamp to avoid rapid polling)
        self.execution_state.last_update = crate::time::Instant::now();
    }

    /// Update the cached device list from daemon
    fn update_device_list(&mut self, client: Option<&DaqClient>, runtime: Option<&Runtime>) {
        let Some(client) = client else {
            // No client, clear device list
            self.cached_device_ids.clear();
            self.last_device_fetch = Some(crate::time::Instant::now());
            return;
        };
        let Some(runtime) = runtime else {
            return;
        };

        // Spawn async task to fetch devices
        let mut client = client.clone();

        // Use a channel to get results back (simple approach for egui)
        // For now, just do a blocking fetch since it's infrequent
        let devices_result = runtime.block_on(async move { client.list_devices().await });

        match devices_result {
            Ok(devices) => {
                self.cached_device_ids = devices.into_iter().map(|d| d.id).collect();
            }
            Err(_) => {
                // Keep existing list on error
            }
        }
        self.last_device_fetch = Some(crate::time::Instant::now());
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
                ExperimentNode::NestedScan(config) => {
                    // Add outer and inner position parameters
                    if !config.outer.actuator.is_empty() {
                        params.push(EditableParameter::float(
                            &config.outer.actuator,
                            "position",
                            &format!("{} Position", config.outer.dimension_name),
                            config.outer.start,
                        ));
                    }
                    if !config.inner.actuator.is_empty() {
                        params.push(EditableParameter::float(
                            &config.inner.actuator,
                            "position",
                            &format!("{} Position", config.inner.dimension_name),
                            config.inner.start,
                        ));
                    }
                }
                ExperimentNode::AdaptiveScan(config) => {
                    if !config.scan.actuator.is_empty() {
                        params.push(EditableParameter::float(
                            &config.scan.actuator,
                            "position",
                            &format!("{} Position", config.scan.actuator),
                            config.scan.start,
                        ));
                    }
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
            match client
                .set_parameter(&device_id, &param_name, &new_value)
                .await
            {
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

    // ========== Live Visualization Integration ==========

    /// Extract detectors from graph Acquire nodes.
    /// Returns (cameras, plots, spectra) where:
    /// - cameras: Vec<CameraInfo> - (device_id, title)
    /// - plots: Vec<PlotInfo> - (device_id, label, title)
    /// - spectra: Vec<SpectrumInfo> - (device_id, label, title)
    fn extract_detectors(&self) -> (Vec<CameraInfo>, Vec<PlotInfo>, Vec<SpectrumInfo>) {
        let mut cameras = Vec::new();
        let mut plots = Vec::new();

        for (_, node) in self.snarl.node_ids() {
            if let ExperimentNode::Acquire(config) = node
                && !config.detector.is_empty()
            {
                // Simple heuristic: device IDs containing "camera" or "cam" are cameras
                // Everything else is a plot (power meter, photodiode, etc.)
                let device_id = &config.detector;
                let device_lower = device_id.to_lowercase();

                if device_lower.contains("camera") || device_lower.contains("cam") {
                    cameras.push((device_id.clone(), device_id.clone()));
                } else {
                    // For plots, use device_id as both identifier and label
                    plots.push((device_id.clone(), device_id.clone(), device_id.clone()));
                }
            }
        }

        // Deduplicate
        cameras.sort_unstable();
        cameras.dedup();
        plots.sort_unstable();
        plots.dedup();

        let spectra = cameras
            .iter()
            .map(|(device_id, title)| {
                (
                    device_id.clone(),
                    "Spectrum".to_string(),
                    format!("{title} Spectrum"),
                )
            })
            .collect();

        (cameras, plots, spectra)
    }

    /// Start visualization when experiment execution begins.
    fn start_visualization(&mut self, client: &DaqClient, runtime: &Runtime) {
        // Extract detectors from graph
        let (cameras, plots, spectrum_plots) = self.extract_detectors();

        // Only create visualization if there are detectors
        if cameras.is_empty() && plots.is_empty() && spectrum_plots.is_empty() {
            return;
        }

        // Clear any previous stream tasks (cleanup for repeated calls)
        for handle in self.camera_stream_tasks.drain(..) {
            handle.abort();
        }
        if let Some(handle) = self.document_stream_task.take() {
            handle.abort();
        }
        for handle in self.spectrum_stream_tasks.drain(..) {
            handle.abort();
        }

        // Create channels (LOCAL variables)
        let (frame_tx, frame_rx) = frame_channel();
        let (data_tx, data_rx) = data_channel();
        let (spectrum_tx, spectrum_rx) = spectrum_channel();

        // Create and configure panel
        let mut panel = LiveVisualizationPanel::new();
        panel.configure_detectors_with_spectra(
            cameras.clone(),
            plots.clone(),
            spectrum_plots.clone(),
        );
        panel.set_frame_receiver(frame_rx);
        panel.set_data_receiver(data_rx);
        panel.set_spectrum_receiver(spectrum_rx);
        panel.start();

        // Spawn camera streaming tasks BEFORE storing senders
        // Use LOCAL frame_tx, not self.frame_tx
        for (camera_id, _) in cameras {
            let tx = frame_tx.clone(); // Clone LOCAL variable
            let mut client = client.clone();

            let handle = runtime.spawn(async move {
                // Start stream (30 FPS preview quality for live viz)
                match client.stream_frames(&camera_id, 30, StreamQuality::Preview).await {
                    Ok(mut stream) => {
                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(frame_data) => {
                                    let update = FrameUpdate {
                                        device_id: camera_id.clone(),
                                        width: frame_data.width,
                                        height: frame_data.height,
                                        data: frame_data.data,
                                        frame_number: frame_data.frame_number,
                                        timestamp_ns: frame_data.timestamp_ns,
                                    };
                                    if tx.try_send(update).is_err() {
                                        // Channel full or closed - continue trying
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(device = %camera_id, error = %e, "Camera stream error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(device = %camera_id, error = %e, "Failed to start camera stream");
                    }
                }
            });
            self.camera_stream_tasks.push(handle);
        }

        // Spawn document stream for plots (still using LOCAL data_tx)
        if !plots.is_empty() {
            let tx = data_tx.clone(); // Clone LOCAL variable
            let mut client = client.clone();
            let plot_ids: Vec<String> = plots.iter().map(|(id, _, _)| id.clone()).collect();

            let handle = runtime.spawn(async move {
                // Subscribe to all documents (filter events client-side)
                match client.stream_documents(None, vec![]).await {
                    Ok(mut stream) => {
                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(doc) => {
                                    use protocol::daq::document::Payload;
                                    if let Some(Payload::Event(event)) = doc.payload {
                                        // Extract values for configured plot devices
                                        #[allow(clippy::cast_precision_loss)]
                                        let timestamp_secs = event.time_ns as f64 / 1_000_000_000.0;
                                        for plot_id in &plot_ids {
                                            if let Some(&value) = event.data.get(plot_id) {
                                                let update = DataUpdate {
                                                    device_id: plot_id.clone(),
                                                    value,
                                                    timestamp_secs,
                                                };
                                                if tx.try_send(update).is_err() {
                                                    // Channel full or closed
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "Document stream error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to start document stream");
                    }
                }
            });
            self.document_stream_task = Some(handle);
        }

        // Spawn per-camera spectrum streams for server-side extraction products.
        for (camera_id, _, _) in &spectrum_plots {
            let tx = spectrum_tx.clone();
            let mut client = client.clone();
            let camera_id = camera_id.clone();

            let handle = runtime.spawn(async move {
                match client.stream_spectra(vec![camera_id.clone()], 15).await {
                    Ok(mut stream) => {
                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(payload) => {
                                    let device_id = if payload.device_id.is_empty() {
                                        camera_id.clone()
                                    } else {
                                        payload.device_id.clone()
                                    };
                                    let label = if payload.name.is_empty() {
                                        if payload.merged {
                                            "Merged".to_string()
                                        } else {
                                            format!("Order {}", payload.order_index)
                                        }
                                    } else {
                                        payload.name.clone()
                                    };

                                    let update = SpectrumUpdate {
                                        device_id,
                                        label,
                                        x_values: payload.wavelengths,
                                        y_values: payload.intensities,
                                        ivar: payload.ivar,
                                        x_unit: (!payload.wavelength_unit.is_empty())
                                            .then_some(payload.wavelength_unit),
                                        y_unit: (!payload.intensity_unit.is_empty())
                                            .then_some(payload.intensity_unit),
                                        merged: payload.merged,
                                        order_index: payload.order_index,
                                        calibration_profile_id: (!payload.calibration_profile_id
                                            .is_empty())
                                        .then_some(payload.calibration_profile_id),
                                        quality_flags: payload.quality_flags,
                                        snr_estimate: payload.snr_estimate,
                                        timestamp_ns: payload.timestamp_ns,
                                    };

                                    if tx.try_send(update).is_err() {
                                        // Channel full or closed - keep receiving newer spectra.
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(device = %camera_id, error = %e, "Spectrum stream error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            device = %camera_id,
                            error = %e,
                            "No server-side spectrum stream available for camera"
                        );
                    }
                }
            });
            self.spectrum_stream_tasks.push(handle);
        }

        // THEN store panel and senders for later cleanup
        self.visualization_panel = Some(panel);
        self.frame_tx = Some(frame_tx);
        self.data_tx = Some(data_tx);
        self.spectrum_tx = Some(spectrum_tx);
    }

    /// Stop visualization when experiment completes.
    fn stop_visualization(&mut self) {
        // Abort all streaming tasks
        for handle in self.camera_stream_tasks.drain(..) {
            handle.abort();
        }
        if let Some(handle) = self.document_stream_task.take() {
            handle.abort();
        }
        for handle in self.spectrum_stream_tasks.drain(..) {
            handle.abort();
        }

        if let Some(panel) = &mut self.visualization_panel {
            panel.stop();
        }
        // Keep panel visible but mark as inactive
        // Don't drop channels yet - they may have pending data
    }

    // ========== Adaptive Alert Handling ==========

    /// Show an adaptive trigger alert.
    #[allow(dead_code)]
    fn show_adaptive_trigger_alert(&mut self, data: AdaptiveAlertData) {
        if !data.requires_approval {
            // Auto-proceed after 3 seconds
            self.adaptive_alert_auto_proceed_at =
                Some(crate::time::Instant::now() + std::time::Duration::from_secs(3));
        }
        self.adaptive_alert = Some(data);
    }

    /// Confirm adaptive action and resume execution.
    ///
    /// Sends `ResumeEngine` to the daemon so the `RunEngine` proceeds with the
    /// adaptive action that triggered the pause.  Returns `true` when the RPC
    /// was successfully dispatched.
    fn confirm_adaptive_action(
        &mut self,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) -> bool {
        if self.require_connected(client, runtime, "confirm").is_none() {
            return false;
        }
        tracing::info!("Adaptive action approved");
        self.set_status("Adaptive action approved - proceeding");
        self.resume_experiment(client, runtime);
        true
    }

    /// Cancel adaptive action.
    ///
    /// Sends `AbortPlan` to the daemon so the `RunEngine` stops the current
    /// plan instead of executing the adaptive action.  Returns `true` when the
    /// RPC was successfully dispatched.
    fn cancel_adaptive_action(
        &mut self,
        client: Option<&DaqClient>,
        runtime: Option<&Runtime>,
    ) -> bool {
        if self.require_connected(client, runtime, "cancel").is_none() {
            return false;
        }
        tracing::info!("Adaptive action cancelled");
        self.set_status("Adaptive action cancelled");
        self.abort_experiment(client, runtime);
        true
    }

    /// Check that both client and runtime are available. Sets `last_error` and
    /// returns `None` if either is missing. `action` is used in the error
    /// message (e.g. "confirm", "cancel").
    fn require_connected<'a>(
        &mut self,
        client: Option<&'a DaqClient>,
        runtime: Option<&'a Runtime>,
        action: &str,
    ) -> Option<(&'a DaqClient, &'a Runtime)> {
        let Some(c) = client else {
            self.last_error = Some(format!("Cannot {action}: not connected to daemon"));
            return None;
        };
        let Some(r) = runtime else {
            self.last_error = Some(format!("Cannot {action}: async runtime unavailable"));
            return None;
        };
        Some((c, r))
    }
}
