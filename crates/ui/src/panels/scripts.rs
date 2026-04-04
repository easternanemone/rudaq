//! Scripts panel - manage and execute Rhai scripts.
//!
//! Phase 6 (bd-r8uq): Enhanced with Run/Stop controls, progress bars, and execution status.
//! File upload and inline editor for writing/uploading scripts from the GUI.

use crate::runtime::Runtime;
use eframe::egui;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::widgets::{OfflineContext, offline_notice};
use client::DaqClient;

/// Maximum script size for upload (matches `common::limits::MAX_SCRIPT_SIZE`).
const MAX_SCRIPT_SIZE: usize = 1024 * 1024;

/// Result types for different async actions
#[derive(Debug)]
enum ActionResult {
    /// Refresh completed with script and execution lists
    Refresh(
        Result<
            (
                Vec<protocol::daq::ScriptInfo>,
                Vec<protocol::daq::ScriptStatus>,
            ),
            String,
        >,
    ),
    /// Script started with execution ID
    Started(Result<String, String>),
    /// Script stopped
    Stopped(Result<String, String>),
    /// Script uploaded — returns (script_id, name) on success
    Uploaded(Result<(String, String), String>),
}

/// Scripts panel state
pub struct ScriptsPanel {
    /// Cached script list
    scripts: Vec<protocol::daq::ScriptInfo>,
    /// Cached execution list
    executions: Vec<protocol::daq::ScriptStatus>,
    /// Selected script ID
    selected_script: Option<String>,
    /// Selected execution ID (for stop action)
    selected_execution: Option<String>,
    /// Last refresh timestamp
    last_refresh: Option<crate::time::Instant>,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Async action result sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
    /// Auto-refresh interval for running executions
    auto_refresh_enabled: bool,
    /// Last auto-refresh time
    last_auto_refresh: Option<crate::time::Instant>,
    /// Whether the inline editor is visible
    editor_visible: bool,
    /// Editor code content
    editor_code: String,
    /// Script name for upload
    editor_name: String,
    /// Editor color theme
    editor_theme: ColorTheme,
    /// Whether editor has unsaved changes
    editor_dirty: bool,
    /// Local file path if opened from disk
    editor_file_path: Option<PathBuf>,
    /// Editor status message
    editor_status: Option<String>,
    /// Chain upload → start execution
    run_after_upload: bool,
    /// Script ID to auto-run after upload completes
    auto_start_after_upload: Option<String>,
    /// Trigger refresh after upload
    needs_refresh: bool,
}

impl ScriptsPanel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::Refresh(Ok((scripts, executions))) => {
                            self.scripts = scripts;
                            self.executions = executions;
                            self.last_refresh = Some(crate::time::Instant::now());
                            // Only show status if not auto-refreshing
                            if !self.auto_refresh_enabled {
                                self.status = Some(format!(
                                    "Loaded {} scripts, {} executions",
                                    self.scripts.len(),
                                    self.executions.len()
                                ));
                            }
                            self.error = None;
                        }
                        ActionResult::Refresh(Err(e)) => {
                            self.error = Some(e);
                        }
                        ActionResult::Started(Ok(execution_id)) => {
                            self.status = Some(format!("Started execution: {}", execution_id));
                            self.error = None;
                            // Auto-enable refresh to track progress
                            self.auto_refresh_enabled = true;
                            // Immediately refresh so has_running_executions() sees the new state
                            self.needs_refresh = true;
                        }
                        ActionResult::Started(Err(e)) => {
                            self.error = Some(format!("Failed to start: {}", e));
                        }
                        ActionResult::Stopped(Ok(msg)) => {
                            self.status = Some(msg);
                            self.error = None;
                        }
                        ActionResult::Stopped(Err(e)) => {
                            self.error = Some(format!("Failed to stop: {}", e));
                        }
                        ActionResult::Uploaded(Ok((script_id, name))) => {
                            self.status = Some(format!(
                                "Uploaded '{}' ({})",
                                name,
                                &script_id[..8.min(script_id.len())]
                            ));
                            self.error = None;
                            self.editor_dirty = false;
                            self.editor_status = Some("Uploaded successfully".to_string());
                            self.needs_refresh = true;
                            if self.run_after_upload {
                                self.run_after_upload = false;
                                self.auto_start_after_upload = Some(script_id);
                            }
                        }
                        ActionResult::Uploaded(Err(e)) => {
                            self.run_after_upload = false;
                            self.error = Some(format!("Upload failed: {}", e));
                            self.editor_status = Some(format!("Upload failed: {}", e));
                        }
                    }
                    updated = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        if self.action_in_flight > 0 || updated {
            ctx.request_repaint();
        }
    }

    /// Check if any execution is currently running
    fn has_running_executions(&self) -> bool {
        self.executions.iter().any(|e| e.state == "RUNNING")
    }

    /// Render the scripts panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());

        // Track pending actions
        let mut pending_refresh = false;
        let mut pending_start: Option<String> = None;
        let mut pending_upload: Option<(String, String)> = None; // (name, content)

        // Post-upload refresh
        if self.needs_refresh {
            self.needs_refresh = false;
            pending_refresh = true;
        }

        // Auto-start after upload
        if let Some(script_id) = self.auto_start_after_upload.take() {
            pending_start = Some(script_id);
        }

        // Auto-refresh when executions are running
        if self.auto_refresh_enabled && self.has_running_executions() {
            let should_refresh = self
                .last_auto_refresh
                .map(|t| t.elapsed().as_secs() >= 2)
                .unwrap_or(true);

            if should_refresh && self.action_in_flight == 0 {
                self.last_auto_refresh = Some(crate::time::Instant::now());
                pending_refresh = true;
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        } else if self.auto_refresh_enabled && !self.has_running_executions() && !pending_refresh {
            // Disable auto-refresh when no executions running and no refresh pending
            // (a pending refresh may bring in fresh execution data showing RUNNING state)
            self.auto_refresh_enabled = false;
        }

        ui.heading("Scripts");

        // Show offline notice if not connected (bd-j3xz.4.4)
        if offline_notice(ui, client.is_none(), OfflineContext::Scripts) {
            return;
        }

        // Toolbar
        let has_client = client.is_some();
        let mut file_upload_requested = false;
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                pending_refresh = true;
            }

            ui.separator();

            // Run button - enabled when script is selected and no action in flight (bd-tjwm.6)
            let can_run =
                self.selected_script.is_some() && has_client && self.action_in_flight == 0;
            if ui
                .add_enabled(can_run, egui::Button::new("▶ Run"))
                .clicked()
            {
                if let Some(script_id) = &self.selected_script {
                    pending_start = Some(script_id.clone());
                }
            }

            ui.separator();

            // Upload File button — pick .rhai from disk, upload to daemon
            if ui
                .add_enabled(has_client, egui::Button::new("📤 Upload File"))
                .on_hover_text("Upload a .rhai file from disk")
                .clicked()
            {
                file_upload_requested = true;
            }

            // New Script button — toggle inline editor
            let new_label = if self.editor_visible {
                "✏ Hide Editor"
            } else {
                "✏ New Script"
            };
            if ui.button(new_label).clicked() {
                self.editor_visible = !self.editor_visible;
                if self.editor_visible && self.editor_code.is_empty() {
                    self.editor_name = "new_script.rhai".to_string();
                }
            }

            ui.separator();

            if let Some(last) = self.last_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("Updated {}s ago", elapsed.as_secs()));
            }

            if self.action_in_flight > 0 {
                ui.spinner();
            }
        });

        // Handle file upload from dialog (outside toolbar closure to satisfy borrow checker)
        if file_upload_requested {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(path) = FileDialog::new()
                    .add_filter("Rhai Script", &["rhai"])
                    .pick_file()
                {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if name.trim().is_empty() {
                                self.error =
                                    Some("Selected file has an empty or invalid name".to_string());
                            } else if content.trim().is_empty() {
                                self.error = Some(format!("File '{}' is empty", name));
                            } else if content.len() > MAX_SCRIPT_SIZE {
                                self.error = Some(format!(
                                    "Script too large ({} bytes, max {})",
                                    content.len(),
                                    MAX_SCRIPT_SIZE
                                ));
                            } else {
                                pending_upload = Some((name, content));
                            }
                        }
                        Err(e) => {
                            self.error = Some(format!("Failed to read file: {}", e));
                        }
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.error = Some("File dialogs are not available in the browser".to_string());
            }
        }

        ui.separator();

        // Show error/status messages
        if let Some(err) = &self.error {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("⚠").color(egui::Color32::RED));
                ui.colored_label(egui::Color32::RED, err);
            });
        }
        if let Some(status) = &self.status {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("✓").color(egui::Color32::GREEN));
                ui.colored_label(egui::Color32::GREEN, status);
            });
        }

        // Two-column layout - render without client reference
        let pending_stop = self.render_panels(ui);

        // Inline editor section
        if self.editor_visible {
            ui.separator();
            if let Some((name, code)) = self.render_editor(ui) {
                pending_upload = Some((name, code));
            }
        }

        // Execute pending actions with client
        if let Some(client) = client {
            if pending_refresh {
                self.refresh_internal(Some(client), runtime);
            }
            if let Some(script_id) = pending_start {
                self.start_script(script_id, Some(client), runtime);
            }
            if let Some(exec_id) = pending_stop {
                self.stop_script(exec_id, false, Some(client), runtime);
            }
            if let Some((name, content)) = pending_upload {
                self.upload_script_async(name, content, client, runtime);
            }
        } else if pending_refresh
            || pending_start.is_some()
            || pending_stop.is_some()
            || pending_upload.is_some()
        {
            self.error = Some("Not connected to daemon".to_string());
        }
    }

    /// Render the panels and return any pending stop action
    fn render_panels(&mut self, ui: &mut egui::Ui) -> Option<String> {
        let mut pending_stop: Option<String> = None;

        ui.columns(2, |columns| {
            // Left column: Scripts
            columns[0].heading("📜 Scripts");
            self.render_scripts_list(&mut columns[0]);

            // Right column: Executions
            columns[1].heading("⚡ Executions");
            pending_stop = self.render_executions_list_inner(&mut columns[1]);
        });

        pending_stop
    }

    /// Render the scripts list
    fn render_scripts_list(&mut self, ui: &mut egui::Ui) {
        if self.scripts.is_empty() {
            ui.label("No scripts found.");
            ui.label(
                egui::RichText::new("Use 'Upload File' or 'New Script' above to get started")
                    .small()
                    .weak(),
            );
        } else {
            egui::ScrollArea::vertical()
                .id_salt("scripts_list")
                .max_height(300.0)
                .show(ui, |ui| {
                    for script in &self.scripts {
                        let selected = self.selected_script.as_ref() == Some(&script.script_id);

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // Selection indicator
                                if selected {
                                    ui.label(
                                        egui::RichText::new("▸").color(egui::Color32::LIGHT_BLUE),
                                    );
                                } else {
                                    ui.label("  ");
                                }

                                // Script name (clickable)
                                if ui.selectable_label(selected, &script.name).clicked() {
                                    self.selected_script = Some(script.script_id.clone());
                                }
                            });

                            // Script ID in smaller text
                            ui.label(
                                egui::RichText::new(format!(
                                    "ID: {}",
                                    &script.script_id[..8.min(script.script_id.len())]
                                ))
                                .small()
                                .weak(),
                            );
                        });
                    }
                });
        }
    }

    /// Render the executions list with progress bars and stop buttons
    /// Returns execution ID to stop, if any
    fn render_executions_list_inner(&mut self, ui: &mut egui::Ui) -> Option<String> {
        if self.executions.is_empty() {
            ui.label("No executions.");
            ui.label(
                egui::RichText::new("Select a script and click Run")
                    .small()
                    .weak(),
            );
            return None;
        }

        // Pending stop action
        let mut stop_execution_id: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("executions_list")
            .max_height(300.0)
            .show(ui, |ui| {
                for exec in &self.executions {
                    let (state_icon, state_color) = match exec.state.as_str() {
                        "PENDING" => ("⏳", egui::Color32::GRAY),
                        "RUNNING" => ("▶", egui::Color32::YELLOW),
                        "COMPLETED" => ("✓", egui::Color32::GREEN),
                        "ERROR" => ("✗", egui::Color32::RED),
                        "STOPPED" => ("⏹", egui::Color32::GRAY),
                        _ => ("?", egui::Color32::WHITE),
                    };

                    let is_running = exec.state == "RUNNING";
                    let selected = self.selected_execution.as_ref() == Some(&exec.execution_id);

                    ui.group(|ui| {
                        // Header row with state and controls
                        ui.horizontal(|ui| {
                            // State icon and label
                            ui.label(egui::RichText::new(state_icon).color(state_color));
                            ui.colored_label(state_color, &exec.state);

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Stop button for running executions (bd-tjwm.6: disable when action in flight)
                                    let can_stop = is_running && self.action_in_flight == 0;
                                    if ui
                                        .add_enabled(can_stop, egui::Button::new("⏹ Stop"))
                                        .clicked()
                                    {
                                        stop_execution_id = Some(exec.execution_id.clone());
                                    }
                                },
                            );
                        });

                        // Execution ID
                        let id_display = if exec.execution_id.len() > 12 {
                            format!("{}...", &exec.execution_id[..12])
                        } else {
                            exec.execution_id.clone()
                        };

                        if ui
                            .selectable_label(selected, egui::RichText::new(id_display).small())
                            .clicked()
                        {
                            self.selected_execution = Some(exec.execution_id.clone());
                        }

                        // Progress bar for running/pending executions
                        if is_running || exec.state == "PENDING" {
                            #[allow(clippy::cast_precision_loss)]
                            let progress = exec.progress_percent as f32 / 100.0;
                            ui.add(
                                egui::ProgressBar::new(progress)
                                    .text(format!("{}%", exec.progress_percent))
                                    .animate(is_running),
                            );
                        }

                        // Current line being executed
                        if is_running && !exec.current_line.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("Line: {}", exec.current_line))
                                    .small()
                                    .weak(),
                            );
                        }

                        // Error message if present
                        if !exec.error_message.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("⚠").color(egui::Color32::RED));
                                ui.colored_label(egui::Color32::RED, &exec.error_message);
                            });
                        }
                    });
                }
            });

        stop_execution_id
    }

    /// Start executing a script
    fn start_script(
        &mut self,
        script_id: String,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.error = None;
        self.status = Some(format!(
            "Starting script {}...",
            &script_id[..8.min(script_id.len())]
        ));

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .start_script(&script_id, HashMap::new())
                .await
                .map(|resp| resp.execution_id)
                .map_err(|e| e.to_string());

            let _ = tx.send(ActionResult::Started(result)).await;
        });
    }

    /// Stop a running script execution
    fn stop_script(
        &mut self,
        execution_id: String,
        force: bool,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.error = None;
        self.status = Some(format!(
            "Stopping execution {}...",
            &execution_id[..8.min(execution_id.len())]
        ));

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .stop_script(&execution_id, force)
                .await
                .map(|resp| resp.message)
                .map_err(|e| e.to_string());

            let _ = tx.send(ActionResult::Stopped(result)).await;
        });
    }

    /// Refresh scripts and executions
    fn refresh_internal(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = async {
                let scripts = client.list_scripts().await?;
                let executions = client.list_executions().await?;
                Ok::<_, anyhow::Error>((scripts, executions))
            }
            .await
            .map_err(|e| e.to_string());

            let _ = tx.send(ActionResult::Refresh(result)).await;
        });
    }

    /// Render the inline editor. Returns `(name, code)` if an upload action is requested.
    fn render_editor(&mut self, ui: &mut egui::Ui) -> Option<(String, String)> {
        let mut upload_action: Option<(String, String)> = None;

        // Editor toolbar
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.add(
                egui::TextEdit::singleline(&mut self.editor_name)
                    .desired_width(200.0)
                    .hint_text("script_name.rhai"),
            );

            ui.separator();

            if ui
                .button("Open...")
                .on_hover_text("Load a .rhai file into editor")
                .clicked()
            {
                self.open_file_into_editor();
            }

            if ui
                .button("Save Local")
                .on_hover_text("Save editor content to disk")
                .clicked()
            {
                self.save_editor_to_file();
            }

            ui.separator();

            if ui
                .button("📤 Upload to Daemon")
                .on_hover_text("Upload current editor content")
                .clicked()
            {
                if self.editor_name.trim().is_empty() {
                    self.editor_status = Some("Script name cannot be empty".to_string());
                } else if self.editor_code.trim().is_empty() {
                    self.editor_status = Some("Editor is empty".to_string());
                } else if self.editor_code.len() > MAX_SCRIPT_SIZE {
                    self.editor_status = Some(format!(
                        "Script too large ({} bytes, max {})",
                        self.editor_code.len(),
                        MAX_SCRIPT_SIZE
                    ));
                } else {
                    self.run_after_upload = false;
                    upload_action = Some((self.editor_name.clone(), self.editor_code.clone()));
                }
            }

            if ui
                .button("▶ Upload & Run")
                .on_hover_text("Upload and immediately execute")
                .clicked()
            {
                if self.editor_name.trim().is_empty() {
                    self.editor_status = Some("Script name cannot be empty".to_string());
                } else if self.editor_code.trim().is_empty() {
                    self.editor_status = Some("Editor is empty".to_string());
                } else if self.editor_code.len() > MAX_SCRIPT_SIZE {
                    self.editor_status = Some(format!(
                        "Script too large ({} bytes, max {})",
                        self.editor_code.len(),
                        MAX_SCRIPT_SIZE
                    ));
                } else {
                    self.run_after_upload = true;
                    upload_action = Some((self.editor_name.clone(), self.editor_code.clone()));
                }
            }

            ui.separator();

            // Theme selector
            egui::ComboBox::from_id_salt("editor_theme")
                .selected_text(self.editor_theme.name)
                .width(100.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.editor_theme,
                        ColorTheme::GRUVBOX_DARK,
                        "Gruvbox Dark",
                    );
                    ui.selectable_value(
                        &mut self.editor_theme,
                        ColorTheme::GRUVBOX,
                        "Gruvbox Light",
                    );
                    ui.selectable_value(&mut self.editor_theme, ColorTheme::AYU_DARK, "Ayu Dark");
                });

            // File info
            if let Some(path) = &self.editor_file_path {
                let dirty_marker = if self.editor_dirty { " *" } else { "" };
                ui.label(
                    egui::RichText::new(format!(
                        "{}{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        dirty_marker
                    ))
                    .weak(),
                );
            } else if self.editor_dirty {
                ui.label(egui::RichText::new("unsaved *").weak());
            }
        });

        // Editor status
        if let Some(status) = &self.editor_status {
            ui.label(egui::RichText::new(status).small().weak());
        }

        // Code editor widget
        let available = ui.available_height().max(200.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = ((available / 16.0) as usize).max(10);
        let response = CodeEditor::default()
            .id_source("scripts_inline_editor")
            .with_rows(rows)
            .with_fontsize(13.0)
            .with_theme(self.editor_theme)
            .with_syntax(Syntax::rust())
            .with_numlines(true)
            .show(ui, &mut self.editor_code);

        if response.response.changed() {
            self.editor_dirty = true;
        }

        upload_action
    }

    /// Open a .rhai file into the editor
    fn open_file_into_editor(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(path) = FileDialog::new()
                .add_filter("Rhai Script", &["rhai"])
                .pick_file()
            {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        if content.len() > MAX_SCRIPT_SIZE {
                            self.editor_status = Some(format!(
                                "File too large ({} bytes, max {})",
                                content.len(),
                                MAX_SCRIPT_SIZE
                            ));
                        } else {
                            self.editor_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            self.editor_code = content;
                            self.editor_file_path = Some(path);
                            self.editor_dirty = false;
                            self.editor_status = Some("File loaded".to_string());
                            self.editor_visible = true;
                        }
                    }
                    Err(e) => {
                        self.editor_status = Some(format!("Failed to read: {}", e));
                    }
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.editor_status = Some("File dialogs are not available in the browser".to_string());
        }
    }

    /// Save editor content to a local file
    fn save_editor_to_file(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dialog = FileDialog::new()
                .add_filter("Rhai Script", &["rhai"])
                .set_file_name(&self.editor_name);

            // If we have an existing path, start there
            let dialog = if let Some(path) = &self.editor_file_path {
                if let Some(parent) = path.parent() {
                    dialog.set_directory(parent)
                } else {
                    dialog
                }
            } else {
                dialog
            };

            if let Some(path) = dialog.save_file() {
                match std::fs::write(&path, &self.editor_code) {
                    Ok(()) => {
                        self.editor_file_path = Some(path.clone());
                        self.editor_dirty = false;
                        self.editor_status = Some(format!("Saved to {}", path.display()));
                    }
                    Err(e) => {
                        self.editor_status = Some(format!("Save failed: {}", e));
                    }
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.editor_status = Some("File dialogs are not available in the browser".to_string());
        }
    }

    /// Upload script content to the daemon asynchronously
    fn upload_script_async(
        &mut self,
        name: String,
        content: String,
        client: &mut DaqClient,
        runtime: &Runtime,
    ) {
        self.error = None;
        self.status = Some(format!("Uploading '{}'...", name));

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .upload_script(&name, &content, HashMap::new())
                .await
                .map_err(|e| e.to_string())
                .and_then(|resp| {
                    if resp.success {
                        Ok((resp.script_id, name))
                    } else {
                        Err(resp.error_message)
                    }
                });

            let _ = tx.send(ActionResult::Uploaded(result)).await;
        });
    }
}

impl Default for ScriptsPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            scripts: Vec::new(),
            executions: Vec::new(),
            selected_script: None,
            selected_execution: None,
            last_refresh: None,
            error: None,
            status: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            auto_refresh_enabled: false,
            last_auto_refresh: None,
            editor_visible: false,
            editor_code: String::new(),
            editor_name: "new_script.rhai".to_string(),
            editor_theme: ColorTheme::GRUVBOX_DARK,
            editor_dirty: false,
            editor_file_path: None,
            editor_status: None,
            run_after_upload: false,
            auto_start_after_upload: None,
            needs_refresh: false,
        }
    }
}
