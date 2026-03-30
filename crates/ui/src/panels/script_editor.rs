//! Script editor panel for editing Rhai scripts directly.
//!
//! This panel is used after "ejecting" from the visual graph editor.
//! Changes to the script do NOT sync back to the graph (one-way export).
//! The Run button uploads+executes the script via gRPC (bd-42fc).

use crate::runtime::Runtime;
use crate::time::{Duration, Instant};
use crate::widgets::{offline_notice, OfflineContext};
use client::DaqClient;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Maximum script size for upload (matches `common::limits::MAX_SCRIPT_SIZE`).
const MAX_SCRIPT_SIZE: usize = 1024 * 1024;

/// How often to poll execution status while running.
const EXECUTION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Results from async gRPC actions.
#[derive(Debug)]
enum ActionResult {
    /// Upload+start completed — returns execution_id on success.
    RunCompleted(Result<String, String>),
    /// Stop completed — Ok(stopped, message).
    StopCompleted(Result<(bool, String), String>),
    /// Execution status poll result — state string from `ScriptStatus`.
    StatusPoll(Result<String, String>),
}

/// Panel for editing Rhai scripts directly (ejected from visual mode).
pub struct ScriptEditorPanel {
    /// Script content
    code: String,
    /// Current file path (if saved)
    file_path: Option<std::path::PathBuf>,
    /// Color theme
    theme: ColorTheme,
    /// Whether content has unsaved changes
    dirty: bool,
    /// Status message (info/success)
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async action result sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
    /// Execution ID of the most recent run (needed for stop)
    execution_id: Option<String>,
    /// Whether execution is believed to be running
    running: bool,
    /// Last time we polled execution status
    last_poll: Instant,
}

impl ScriptEditorPanel {
    /// Create new panel with initial code from graph
    pub fn from_graph_code(code: String, source_graph: Option<std::path::PathBuf>) -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            code,
            file_path: None,
            theme: ColorTheme::GRUVBOX_DARK,
            dirty: true,
            status: source_graph.map(|p| format!("Ejected from {}", p.display())),
            error: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            execution_id: None,
            running: false,
            last_poll: Instant::now(),
        }
    }

    /// Poll for completed async action results.
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::RunCompleted(Ok(execution_id)) => {
                            let short_id = &execution_id[..8.min(execution_id.len())];
                            self.status = Some(format!("Running (execution {short_id}...)"));
                            self.execution_id = Some(execution_id);
                            self.running = true;
                            self.error = None;
                        }
                        ActionResult::RunCompleted(Err(e)) => {
                            self.error = Some(e);
                            self.running = false;
                            self.status = None;
                        }
                        ActionResult::StopCompleted(Ok(msg)) => {
                            self.running = false;
                            self.error = None;
                            self.status = Some(msg);
                        }
                        ActionResult::StopCompleted(Err(e)) => {
                            self.error = Some(format!("Stop failed: {e}"));
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

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&mut DaqClient>,
        runtime: Option<&Runtime>,
    ) {
        self.poll_async_results(ui.ctx());

        // Collect button actions (defer gRPC calls until after UI rendering)
        let mut pending_run = false;
        let mut pending_stop = false;
        let has_client = client.is_some();

        // Toolbar
        ui.horizontal(|ui| {
            ui.heading("Script Editor");

            ui.separator();

            if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                self.save();
            }

            if ui.button("Save As...").clicked() {
                self.save_as();
            }

            ui.separator();

            // Run / Stop buttons
            let busy = self.action_in_flight > 0;

            if self.running {
                // Show Stop button while running
                let can_stop = has_client && !busy;
                if ui
                    .add_enabled(can_stop, egui::Button::new("⏹ Stop"))
                    .on_hover_text("Stop the running script")
                    .clicked()
                {
                    pending_stop = true;
                }
                ui.spinner();
            } else {
                // Show Run button
                let can_run = has_client && !busy && !self.code.trim().is_empty();
                let run_btn = ui
                    .add_enabled(can_run, egui::Button::new("▶ Run"))
                    .on_hover_text(if has_client {
                        "Upload and execute this script on the daemon"
                    } else {
                        "Connect to a daemon to run scripts"
                    });
                if run_btn.clicked() {
                    pending_run = true;
                }
            }

            ui.separator();

            // Theme selector
            egui::ComboBox::from_label("Theme")
                .selected_text(self.theme_name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.theme, ColorTheme::GRUVBOX_DARK, "Gruvbox Dark");
                    ui.selectable_value(&mut self.theme, ColorTheme::GRUVBOX, "Gruvbox Light");
                    ui.selectable_value(&mut self.theme, ColorTheme::AYU_DARK, "Ayu Dark");
                });

            ui.separator();

            // File info
            if let Some(path) = &self.file_path {
                let dirty_marker = if self.dirty { "*" } else { "" };
                ui.label(format!(
                    "{}{}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    dirty_marker
                ));
            } else {
                ui.label("Unsaved*");
            }
        });

        // Status / error messages
        if let Some(err) = &self.error {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
            });
        }
        if let Some(status) = &self.status {
            ui.horizontal(|ui| {
                let color = if self.running {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::GREEN
                };
                ui.colored_label(color, status.as_str());
            });
        }

        // Offline notice (below toolbar so buttons are visible but disabled)
        offline_notice(ui, !has_client, OfflineContext::Scripts);

        ui.separator();

        // Code editor (editable)
        let response = CodeEditor::default()
            .id_source("script_editor")
            .with_rows(40)
            .with_fontsize(13.0)
            .with_theme(self.theme)
            .with_syntax(Syntax::rust())
            .with_numlines(true)
            .show(ui, &mut self.code);

        // Track dirty state
        if response.response.changed() {
            self.dirty = true;
        }

        // Dispatch pending actions after UI rendering
        if let (Some(client), Some(runtime)) = (client, runtime) {
            if pending_run {
                self.run_script(client, runtime);
            } else if pending_stop {
                self.stop_script(client, runtime);
            }
        }
    }

    /// Upload the current code and then start execution in one async task.
    fn run_script(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        self.execution_id = None;
        self.running = false;
        self.error = None;

        let code = self.code.clone();
        if code.trim().is_empty() {
            self.error = Some("Script is empty".to_string());
            return;
        }
        if code.len() > MAX_SCRIPT_SIZE {
            self.error = Some(format!(
                "Script too large ({} bytes, max {MAX_SCRIPT_SIZE})",
                code.len()
            ));
            return;
        }

        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "editor_script.rhai".to_string());

        self.status = Some("Uploading script...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            // Step 1: Upload
            let script_id = match client
                .upload_script(&name, &code, HashMap::new())
                .await
                .map_err(|e| e.to_string())
                .and_then(|resp| {
                    if resp.success {
                        Ok(resp.script_id)
                    } else {
                        Err(resp.error_message)
                    }
                }) {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx
                        .send(ActionResult::RunCompleted(Err(format!(
                            "Upload failed: {e}"
                        ))))
                        .await;
                    return;
                }
            };

            // Step 2: Start execution
            let result = client
                .start_script(&script_id, HashMap::new())
                .await
                .map(|resp| resp.execution_id)
                .map_err(|e| format!("Start failed: {e}"));

            let _ = tx.send(ActionResult::RunCompleted(result)).await;
        });
    }

    /// Stop the currently running execution.
    fn stop_script(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        let Some(execution_id) = self.execution_id.clone() else {
            self.error = Some("No execution to stop".to_string());
            return;
        };

        self.error = None;
        self.status = Some("Stopping...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .stop_script(&execution_id, false)
                .await
                .map(|resp| resp.message)
                .map_err(|e| e.to_string());

            let _ = tx.send(ActionResult::StopCompleted(result)).await;
        });
    }

    fn save(&mut self) {
        if let Some(path) = &self.file_path {
            match std::fs::write(path, &self.code) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = Some(format!("Saved to {}", path.display()));
                }
                Err(e) => {
                    self.status = Some(format!("Save failed: {}", e));
                }
            }
        } else {
            self.save_as();
        }
    }

    fn save_as(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = FileDialog::new()
            .add_filter("Rhai Script", &["rhai"])
            .set_file_name("script.rhai")
            .save_file()
        {
            self.file_path = Some(path.clone());
            self.save();
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.status = Some("File dialogs are not available in the browser".to_string());
        }
    }

    fn theme_name(&self) -> &'static str {
        self.theme.name
    }
}
