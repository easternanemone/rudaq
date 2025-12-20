//! Scripts panel - manage and execute Rhai scripts.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;

/// Scripts panel state
pub struct ScriptsPanel {
    /// Cached script list
    scripts: Vec<daq_proto::daq::ScriptInfo>,
    /// Cached execution list
    executions: Vec<daq_proto::daq::ScriptStatus>,
    /// Selected script ID
    selected_script: Option<String>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Async action result sender
    action_tx: mpsc::Sender<Result<(Vec<daq_proto::daq::ScriptInfo>, Vec<daq_proto::daq::ScriptStatus>), String>>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<Result<(Vec<daq_proto::daq::ScriptInfo>, Vec<daq_proto::daq::ScriptStatus>), String>>,
    /// Number of in-flight async actions
    action_in_flight: usize,
}

impl ScriptsPanel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        Ok((scripts, executions)) => {
                            self.scripts = scripts;
                            self.executions = executions;
                            self.last_refresh = Some(std::time::Instant::now());
                            self.status = Some(format!(
                                "Loaded {} scripts, {} executions",
                                self.scripts.len(),
                                self.executions.len()
                            ));
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
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

    /// Render the scripts panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());
        ui.heading("Scripts");
        
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                self.refresh(client, runtime);
            }
            
            if let Some(last) = self.last_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("Updated {}s ago", elapsed.as_secs()));
            }
        });
        
        ui.separator();
        
        // Show error/status messages
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }
        
        // Scripts and executions in tabs
        ui.horizontal(|ui| {
            ui.heading("Uploaded Scripts");
        });
        
        if self.scripts.is_empty() {
            ui.label("No scripts found. Upload scripts via gRPC or CLI.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("scripts_list")
                .max_height(200.0)
                .show(ui, |ui| {
                    for script in &self.scripts {
                        ui.group(|ui| {
                            let selected = self.selected_script.as_ref() == Some(&script.script_id);
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(selected, &script.name)
                                    .clicked()
                                {
                                    self.selected_script = Some(script.script_id.clone());
                                }
                                ui.label(format!("ID: {}", script.script_id));
                            });
                        });
                    }
                });
        }
        
        ui.separator();
        ui.heading("Executions");
        
        if self.executions.is_empty() {
            ui.label("No executions found.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("executions_list")
                .max_height(300.0)
                .show(ui, |ui| {
                    for exec in &self.executions {
                        let state_color = match exec.state.as_str() {
                            "RUNNING" => egui::Color32::YELLOW,
                            "COMPLETED" => egui::Color32::GREEN,
                            "ERROR" => egui::Color32::RED,
                            "STOPPED" => egui::Color32::GRAY,
                            _ => egui::Color32::WHITE,
                        };
                        
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(state_color, &exec.state);
                                ui.label(format!("ID: {}", exec.execution_id));
                                if exec.progress_percent > 0 {
                                    ui.label(format!("{}%", exec.progress_percent));
                                }
                            });
                            
                            if !exec.error_message.is_empty() {
                                ui.colored_label(egui::Color32::RED, &exec.error_message);
                            }
                        });
                    }
                });
        }
    }
    
    /// Refresh scripts and executions
    fn refresh(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.error = None;
        self.status = None;
        
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

            let _ = tx.send(result).await;
        });
    }
}

impl Default for ScriptsPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(8);
        Self {
            scripts: Vec::new(),
            executions: Vec::new(),
            selected_script: None,
            last_refresh: None,
            error: None,
            status: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
        }
    }
}
