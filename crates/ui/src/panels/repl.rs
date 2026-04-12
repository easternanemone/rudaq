//! Interactive Rhai script REPL panel.
//!
//! Provides a scrollback output area and single-line input with command history.
//! Scripts are executed via the gRPC `RunScript` (upload + start) workflow on the
//! connected daemon. Designed for quick interactive experimentation (bd-moq9).

use crate::runtime::Runtime;
use crate::widgets::{OfflineContext, offline_notice};
use client::DaqClient;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Maximum number of output lines retained in scrollback.
const MAX_OUTPUT_LINES: usize = 2000;

/// Result from an async script execution.
struct ReplResult {
    output: Result<String, String>,
}

/// A single line in the REPL output scrollback.
enum ReplLine {
    /// User input command (rendered with ">" prompt).
    Input(String),
    /// Successful output value.
    Output(String),
    /// Error message (rendered in red).
    Error(String),
}

/// Interactive Rhai REPL panel state.
pub struct ReplPanel {
    /// Current input field text.
    input: String,
    /// Command history (newest last).
    history: Vec<String>,
    /// Current position when navigating history with up/down arrows.
    /// `None` means not navigating; `Some(i)` indexes into `history`.
    history_index: Option<usize>,
    /// Saved input text before history navigation started.
    saved_input: String,
    /// Scrollback output buffer.
    output_lines: Vec<ReplLine>,
    /// Whether a script execution is in flight.
    pending_execution: bool,
    /// Channel receiver for async execution results.
    result_rx: mpsc::Receiver<ReplResult>,
    /// Channel sender cloned into async tasks.
    result_tx: mpsc::Sender<ReplResult>,
}

impl Default for ReplPanel {
    fn default() -> Self {
        let (result_tx, result_rx) = mpsc::channel(16);
        Self {
            input: String::new(),
            history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            output_lines: Vec::new(),
            pending_execution: false,
            result_rx,
            result_tx,
        }
    }
}

impl ReplPanel {
    /// Poll for completed async execution results.
    fn poll_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        while let Ok(result) = self.result_rx.try_recv() {
            self.pending_execution = false;
            match result.output {
                Ok(value) => {
                    let display = if value.trim().is_empty() {
                        "(ok)".to_string()
                    } else {
                        value
                    };
                    self.output_lines.push(ReplLine::Output(display));
                }
                Err(e) => {
                    self.output_lines.push(ReplLine::Error(e));
                }
            }
            updated = true;
        }

        // Trim scrollback if it exceeds the limit
        if self.output_lines.len() > MAX_OUTPUT_LINES {
            let excess = self.output_lines.len() - MAX_OUTPUT_LINES;
            self.output_lines.drain(..excess);
        }

        if self.pending_execution || updated {
            ctx.request_repaint();
        }
    }

    /// Execute the current input line.
    fn execute(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        let command = self.input.trim().to_string();
        if command.is_empty() {
            return;
        }

        // Push to history (skip if duplicate of last entry)
        if self.history.last() != Some(&command) {
            self.history.push(command.clone());
        }
        self.history_index = None;
        self.saved_input.clear();

        // Show the input in scrollback
        self.output_lines.push(ReplLine::Input(command.clone()));

        // Clear input field
        self.input.clear();

        // Mark as pending
        self.pending_execution = true;

        // Spawn async execution: upload script then start it
        let mut client = client.clone();
        let tx = self.result_tx.clone();

        runtime.spawn(async move {
            // Upload
            let script_id = match client
                .upload_script("repl_snippet.rhai", &command, HashMap::new())
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
                        .send(ReplResult {
                            output: Err(format!("Upload failed: {e}")),
                        })
                        .await;
                    return;
                }
            };

            // Start execution
            let result = client
                .start_script(&script_id, HashMap::new())
                .await
                .map(|resp| resp.execution_id)
                .map_err(|e| format!("Execution failed: {e}"));

            let _ = tx.send(ReplResult { output: result }).await;
        });
    }

    /// Handle up/down arrow history navigation.
    fn navigate_history(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                if up {
                    // Start navigating: save current input, go to last history entry
                    self.saved_input = self.input.clone();
                    let idx = self.history.len() - 1;
                    self.history_index = Some(idx);
                    self.input.clone_from(&self.history[idx]);
                }
                // Down with no history navigation active: do nothing
            }
            Some(idx) => {
                if up {
                    if idx > 0 {
                        let new_idx = idx - 1;
                        self.history_index = Some(new_idx);
                        self.input.clone_from(&self.history[new_idx]);
                    }
                    // Already at oldest entry: do nothing
                } else {
                    // Down arrow
                    if idx + 1 < self.history.len() {
                        let new_idx = idx + 1;
                        self.history_index = Some(new_idx);
                        self.input.clone_from(&self.history[new_idx]);
                    } else {
                        // Past the end: restore saved input
                        self.history_index = None;
                        self.input.clone_from(&self.saved_input);
                        self.saved_input.clear();
                    }
                }
            }
        }
    }

    /// Main UI entry point.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&mut DaqClient>,
        runtime: Option<&Runtime>,
    ) {
        self.poll_results(ui.ctx());

        let has_client = client.is_some();

        ui.heading("REPL");
        ui.label("Interactive Rhai script console");

        // Offline notice
        offline_notice(ui, !has_client, OfflineContext::Scripts);

        ui.separator();

        // --- Output scrollback area ---
        let available = ui.available_size();
        // Reserve space for the input row (~30px) plus separator
        let output_height = (available.y - 40.0).max(60.0);

        egui::ScrollArea::vertical()
            .max_height(output_height)
            .stick_to_bottom(true)
            .id_salt("repl_output")
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.output_lines.is_empty() {
                    ui.colored_label(
                        egui::Color32::GRAY,
                        egui::RichText::new("Type a Rhai expression and press Enter to evaluate.")
                            .monospace(),
                    );
                } else {
                    for line in &self.output_lines {
                        match line {
                            ReplLine::Input(cmd) => {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new("> ")
                                            .monospace()
                                            .color(egui::Color32::from_rgb(80, 200, 80)),
                                    );
                                    ui.label(
                                        egui::RichText::new(cmd)
                                            .monospace()
                                            .color(egui::Color32::from_rgb(80, 200, 80)),
                                    );
                                });
                            }
                            ReplLine::Output(val) => {
                                ui.label(egui::RichText::new(val).monospace());
                            }
                            ReplLine::Error(err) => {
                                ui.label(
                                    egui::RichText::new(err)
                                        .monospace()
                                        .color(egui::Color32::from_rgb(255, 80, 80)),
                                );
                            }
                        }
                    }
                }
            });

        ui.separator();

        // --- Input row ---
        let mut should_execute = false;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(">>>").monospace().strong());

            let input_id = ui.id().with("repl_input");

            // Check for up/down arrow keys before rendering the text edit
            let (pressed_up, pressed_down) = ui.ctx().input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                )
            });

            if pressed_up {
                self.navigate_history(true);
            }
            if pressed_down {
                self.navigate_history(false);
            }

            let response = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .id(input_id)
                    .desired_width(ui.available_width() - 40.0)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Enter Rhai expression..."),
            );

            // Enter key submits
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                should_execute = true;
                // Re-focus the input field for continuous typing
                ui.memory_mut(|mem| mem.request_focus(input_id));
            }

            // Pending spinner or run button
            if self.pending_execution {
                ui.spinner();
            } else {
                let can_run = has_client && !self.input.trim().is_empty();
                if ui
                    .add_enabled(can_run, egui::Button::new("▶"))
                    .on_hover_text("Execute (Enter)")
                    .clicked()
                {
                    should_execute = true;
                }
            }
        });

        // Dispatch execution after UI rendering (avoids borrow conflicts)
        if should_execute && !self.pending_execution {
            if let (Some(client), Some(runtime)) = (client, runtime) {
                self.execute(client, runtime);
            } else {
                self.output_lines.push(ReplLine::Input(self.input.clone()));
                self.output_lines
                    .push(ReplLine::Error("Not connected to daemon".to_string()));
                self.input.clear();
            }
        }
    }
}
