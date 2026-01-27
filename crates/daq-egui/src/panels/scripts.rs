//! Scripts panel - manage and execute Rhai scripts.
//!
//! Phase 6 (bd-r8uq): Enhanced with Run/Stop controls, progress bars, and execution status.

use eframe::egui;
use std::collections::HashMap;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use daq_client::DaqClient;

/// Result types for different async actions
#[derive(Debug)]
enum ActionResult {
    /// Refresh completed with script and execution lists
    Refresh(
        Result<
            (
                Vec<daq_proto::daq::ScriptInfo>,
                Vec<daq_proto::daq::ScriptStatus>,
            ),
            String,
        >,
    ),
    /// Script started with execution ID
    Started(Result<String, String>),
    /// Script stopped
    Stopped(Result<String, String>),
}

/// Scripts panel state
pub struct ScriptsPanel {
    /// Cached script list
    scripts: Vec<daq_proto::daq::ScriptInfo>,
    /// Cached execution list
    executions: Vec<daq_proto::daq::ScriptStatus>,
    /// Selected script ID
    selected_script: Option<String>,
    /// Selected execution ID (for stop action)
    selected_execution: Option<String>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
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
    last_auto_refresh: Option<std::time::Instant>,
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
                            self.last_refresh = Some(std::time::Instant::now());
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

        // Auto-refresh when executions are running
        if self.auto_refresh_enabled && self.has_running_executions() {
            let should_refresh = self
                .last_auto_refresh
                .map(|t| t.elapsed().as_secs() >= 2)
                .unwrap_or(true);

            if should_refresh && self.action_in_flight == 0 {
                self.last_auto_refresh = Some(std::time::Instant::now());
                pending_refresh = true;
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        } else if self.auto_refresh_enabled && !self.has_running_executions() {
            // Disable auto-refresh when no executions running
            self.auto_refresh_enabled = false;
        }

        ui.heading("Scripts");

        // Show offline notice if not connected (bd-j3xz.4.4)
        if offline_notice(ui, client.is_none(), OfflineContext::Scripts) {
            return;
        }

        // Toolbar
        let has_client = client.is_some();
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

            if let Some(last) = self.last_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("Updated {}s ago", elapsed.as_secs()));
            }

            if self.action_in_flight > 0 {
                ui.spinner();
            }
        });

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

        // Execute pending actions with client
        if let Some(client) = client {
            if pending_refresh {
                self.refresh_internal(Some(client), runtime);
            } else if let Some(script_id) = pending_start {
                self.start_script(script_id, Some(client), runtime);
            } else if let Some(exec_id) = pending_stop {
                self.stop_script(exec_id, false, Some(client), runtime);
            }
        } else if pending_refresh || pending_start.is_some() || pending_stop.is_some() {
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
                egui::RichText::new("Upload via CLI: rust-daq-daemon upload <file.rhai>")
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
        }
    }
}
