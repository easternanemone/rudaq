//! Storage panel - HDF5 recording and acquisition management.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;

/// Pending action for storage panel
enum PendingAction {
    Refresh,
    StartRecording { name: String },
    StopRecording,
}

enum StorageActionResult {
    Refresh(Result<(Option<daq_proto::daq::StorageConfig>, Option<daq_proto::daq::RecordingStatus>, Vec<daq_proto::daq::AcquisitionSummary>), String>),
    Start(Result<String, String>),
    Stop(Result<(String, u64, u64), String>),
}

/// Storage panel state
pub struct StoragePanel {
    /// Storage configuration
    config: Option<daq_proto::daq::StorageConfig>,
    /// Current recording status
    recording_status: Option<daq_proto::daq::RecordingStatus>,
    /// List of acquisitions
    acquisitions: Vec<daq_proto::daq::AcquisitionSummary>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Recording name input
    recording_name: String,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Pending action
    pending_action: Option<PendingAction>,
    /// Async action result sender
    action_tx: mpsc::Sender<StorageActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<StorageActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
}

impl StoragePanel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        StorageActionResult::Refresh(result) => match result {
                            Ok((config, status, acquisitions)) => {
                                self.config = config;
                                self.recording_status = status;
                                self.acquisitions = acquisitions;
                                self.last_refresh = Some(std::time::Instant::now());
                                self.status = Some(format!(
                                    "Loaded {} acquisitions",
                                    self.acquisitions.len()
                                ));
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        },
                        StorageActionResult::Start(result) => match result {
                            Ok(output_path) => {
                                self.status = Some(format!("Recording started: {}", output_path));
                                self.recording_name.clear();
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        },
                        StorageActionResult::Stop(result) => match result {
                            Ok((output_path, file_size_bytes, total_samples)) => {
                                let size_mb = file_size_bytes as f64 / 1_000_000.0;
                                self.status = Some(format!(
                                    "Recording saved: {} ({:.2} MB, {} samples)",
                                    output_path, size_mb, total_samples
                                ));
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        },
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

    /// Render the storage panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());
        self.pending_action = None;
        
        ui.heading("Storage");
        
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                self.pending_action = Some(PendingAction::Refresh);
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
        
        ui.add_space(8.0);
        
        // Storage configuration
        if let Some(config) = &self.config {
            ui.group(|ui| {
                ui.heading("Configuration");
                ui.label(format!("Output directory: {}", config.output_directory));
                ui.label(format!("Flush interval: {} ms", config.flush_interval_ms));
                ui.label(format!("Max buffer: {} MB", config.max_buffer_mb));
                
                if let Some(hdf5) = &config.hdf5_config {
                    ui.separator();
                    ui.label(format!("Compression: {}", hdf5.compression));
                    if let Some(level) = hdf5.compression_level {
                        ui.label(format!("Compression level: {}", level));
                    }
                }
                
                ui.separator();
                let available_gb = config.disk_space_available_bytes as f64 / 1_000_000_000.0;
                let used_gb = config.disk_space_used_bytes as f64 / 1_000_000_000.0;
                ui.label(format!("Disk space: {:.2} GB available, {:.2} GB used", available_gb, used_gb));
            });
        }
        
        ui.add_space(8.0);
        
        // Recording control
        ui.group(|ui| {
            ui.heading("Recording Control");
            
            if let Some(status) = &self.recording_status {
                let state_color = match status.state {
                    1 => egui::Color32::GRAY,    // IDLE
                    2 => egui::Color32::GREEN,   // ACTIVE
                    3 => egui::Color32::YELLOW,  // FLUSHING
                    4 => egui::Color32::BLUE,    // FINALIZING
                    5 => egui::Color32::RED,     // ERROR
                    _ => egui::Color32::WHITE,
                };
                
                let state_name = match status.state {
                    1 => "Idle",
                    2 => "Recording",
                    3 => "Flushing",
                    4 => "Finalizing",
                    5 => "Error",
                    _ => "Unknown",
                };
                
                ui.horizontal(|ui| {
                    ui.colored_label(state_color, "●");
                    ui.label(format!("Status: {}", state_name));
                });
                
                if status.state == 2 {
                    // Recording active
                    ui.label(format!("Recording: {}", status.output_path));
                    ui.label(format!("Samples: {}", status.samples_recorded));
                    let bytes_mb = status.bytes_written as f64 / 1_000_000.0;
                    ui.label(format!("Written: {:.2} MB", bytes_mb));
                    ui.label(format!("Buffer: {}%", status.buffer_fill_percent));
                    
                    if ui.button("⏹ Stop Recording").clicked() {
                        self.pending_action = Some(PendingAction::StopRecording);
                    }
                } else {
                    // Not recording - show start controls
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.recording_name);
                    });
                    
                    if ui.button("⏺ Start Recording").clicked() {
                        let name = if self.recording_name.is_empty() {
                            format!("recording_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"))
                        } else {
                            self.recording_name.clone()
                        };
                        self.pending_action = Some(PendingAction::StartRecording { name });
                    }
                }
            } else {
                ui.label("Recording status not available");
                
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.recording_name);
                });
                
                if ui.button("⏺ Start Recording").clicked() {
                    let name = if self.recording_name.is_empty() {
                        format!("recording_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"))
                    } else {
                        self.recording_name.clone()
                    };
                    self.pending_action = Some(PendingAction::StartRecording { name });
                }
            }
        });
        
        ui.add_space(8.0);
        
        // Acquisitions list
        ui.group(|ui| {
            ui.heading("Saved Acquisitions");
            
            if self.acquisitions.is_empty() {
                ui.label("No acquisitions found");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("acquisitions_list")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for acq in &self.acquisitions {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(&acq.name);
                                    let size_mb = acq.file_size_bytes as f64 / 1_000_000.0;
                                    ui.label(format!("{:.2} MB", size_mb));
                                    ui.label(format!("{} samples", acq.sample_count));
                                });
                                ui.label(&acq.file_path);
                            });
                        }
                    });
            }
        });
        
        // Execute pending action
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }
    }
    
    /// Execute a pending action
    fn execute_action(
        &mut self,
        action: PendingAction,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        match action {
            PendingAction::Refresh => self.refresh(client, runtime),
            PendingAction::StartRecording { name } => self.start_recording(client, runtime, &name),
            PendingAction::StopRecording => self.stop_recording(client, runtime),
        }
    }
    
    /// Refresh storage info
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
                let config = client.get_storage_config().await.ok();
                let status = client.get_recording_status().await.ok();
                let acquisitions = client.list_acquisitions().await.unwrap_or_default();
                Ok::<_, anyhow::Error>((config, status, acquisitions))
            }
            .await
            .map_err(|e| e.to_string());

            let _ = tx.send(StorageActionResult::Refresh(result)).await;
        });
    }
    
    /// Start recording
    fn start_recording(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, name: &str) {
        self.error = None;
        self.status = None;
        
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };
        
        let mut client = client.clone();
        let name = name.to_string();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.start_recording(&name).await;
            let action = match result {
                Ok(response) => {
                    if response.success {
                        StorageActionResult::Start(Ok(response.output_path))
                    } else {
                        StorageActionResult::Start(Err(response.error_message))
                    }
                }
                Err(e) => StorageActionResult::Start(Err(e.to_string())),
            };
            let _ = tx.send(action).await;
        });
    }
    
    /// Stop recording
    fn stop_recording(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
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
            let result = client.stop_recording().await;
            let action = match result {
                Ok(response) => {
                    if response.success {
                        StorageActionResult::Stop(Ok((
                            response.output_path,
                            response.file_size_bytes,
                            response.total_samples,
                        )))
                    } else {
                        StorageActionResult::Stop(Err(response.error_message))
                    }
                }
                Err(e) => StorageActionResult::Stop(Err(e.to_string())),
            };
            let _ = tx.send(action).await;
        });
    }
}

impl Default for StoragePanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            config: None,
            recording_status: None,
            acquisitions: Vec::new(),
            last_refresh: None,
            recording_name: String::new(),
            error: None,
            status: None,
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
        }
    }
}
