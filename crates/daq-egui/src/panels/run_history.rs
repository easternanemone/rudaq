//! Run history panel - browse and filter past experiment acquisitions.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use daq_client::DaqClient;

/// Pending action for run history panel
#[allow(dead_code)]
enum PendingAction {
    Refresh,
    SaveAnnotation { file_path: String },
    LoadAnnotation { file_path: String },
}

/// Result from an async action
enum ActionResult {
    Refresh(Result<Vec<protocol::daq::AcquisitionSummary>, String>),
    #[cfg(feature = "storage_hdf5")]
    SaveAnnotation(Result<(), String>),
    #[cfg(feature = "storage_hdf5")]
    LoadAnnotation(Result<Option<daq_storage::RunAnnotation>, String>),
}

/// Run history panel state
pub struct RunHistoryPanel {
    /// All acquisitions loaded from server
    acquisitions: Vec<protocol::daq::AcquisitionSummary>,
    /// Filtered acquisitions (after search)
    filtered_acquisitions: Vec<protocol::daq::AcquisitionSummary>,
    /// Search query text
    search_query: String,
    /// Selected run index in filtered_acquisitions
    selected_run_idx: Option<usize>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Error message
    error: Option<String>,
    /// Pending action
    pending_action: Option<PendingAction>,
    /// Async action result sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
    /// Annotation notes text
    annotation_notes: String,
    /// Annotation tags (comma-separated)
    annotation_tags: String,
    /// Annotation status message
    annotation_status: Option<String>,
}

impl Default for RunHistoryPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            acquisitions: Vec::new(),
            filtered_acquisitions: Vec::new(),
            search_query: String::new(),
            selected_run_idx: None,
            last_refresh: None,
            error: None,
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            annotation_notes: String::new(),
            annotation_tags: String::new(),
            annotation_status: None,
        }
    }
}

impl RunHistoryPanel {
    /// Poll for async results and update state
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::Refresh(result) => match result {
                            Ok(acquisitions) => {
                                self.acquisitions = acquisitions;
                                self.apply_filter();
                                self.last_refresh = Some(std::time::Instant::now());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        },
                        #[cfg(feature = "storage_hdf5")]
                        ActionResult::SaveAnnotation(result) => match result {
                            Ok(()) => {
                                self.annotation_status = Some("Annotations saved ✓".to_string())
                            }
                            Err(e) => self.annotation_status = Some(format!("Error: {}", e)),
                        },
                        #[cfg(feature = "storage_hdf5")]
                        ActionResult::LoadAnnotation(result) => match result {
                            Ok(Some(annotation)) => {
                                self.annotation_notes = annotation.notes;
                                self.annotation_tags = annotation.tags.join(", ");
                                self.annotation_status = None;
                            }
                            Ok(None) => {
                                // No existing annotations
                                self.annotation_notes.clear();
                                self.annotation_tags.clear();
                                self.annotation_status = None;
                            }
                            Err(e) => {
                                self.annotation_status = Some(format!("Load error: {}", e));
                            }
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

    /// Refresh acquisition list from server
    fn refresh(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        let Some(client) = client else {
            self.error = Some("Not connected".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight += 1;

        runtime.spawn(async move {
            let result = client.list_acquisitions().await.map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Refresh(result)).await;
        });
    }

    /// Save annotation to HDF5 file
    #[cfg(feature = "storage_hdf5")]
    fn save_annotation(&mut self, file_path: String, runtime: &Runtime) {
        let notes = self.annotation_notes.clone();
        let tags = self
            .annotation_tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        let tx = self.action_tx.clone();
        self.action_in_flight += 1;

        runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                use std::path::PathBuf;
                let path = PathBuf::from(file_path);
                let annotation = daq_storage::RunAnnotation { notes, tags };
                daq_storage::add_run_annotation(&path, &annotation)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));

            let _ = tx.send(ActionResult::SaveAnnotation(result)).await;
        });
    }

    /// Load annotation from HDF5 file
    #[cfg(feature = "storage_hdf5")]
    fn load_annotation(&mut self, file_path: String, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        self.action_in_flight += 1;

        runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                use std::path::PathBuf;
                let path = PathBuf::from(file_path);
                daq_storage::read_run_annotations(&path)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));

            let _ = tx.send(ActionResult::LoadAnnotation(result)).await;
        });
    }

    /// Apply search filter to acquisitions
    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_acquisitions = self.acquisitions.clone();
        } else {
            let query_lower = self.search_query.to_lowercase();
            self.filtered_acquisitions = self
                .acquisitions
                .iter()
                .filter(|acq| {
                    acq.name.to_lowercase().contains(&query_lower)
                        || acq.acquisition_id.to_lowercase().contains(&query_lower)
                })
                .cloned()
                .collect();
        }
        self.selected_run_idx = None; // Clear selection when filter changes
    }

    /// Render the run history panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());
        self.pending_action = None;

        ui.heading("Run History");

        // Show offline notice if not connected
        if offline_notice(ui, client.is_none(), OfflineContext::Storage) {
            return;
        }

        // Trigger initial refresh
        if self.last_refresh.is_none() {
            self.pending_action = Some(PendingAction::Refresh);
        }

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

        // Show error message
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }

        ui.add_space(8.0);

        // Search bar
        ui.horizontal(|ui| {
            ui.label("Search:");
            if ui.text_edit_singleline(&mut self.search_query).changed() {
                self.apply_filter();
            }
            if ui.button("Clear").clicked() {
                self.search_query.clear();
                self.apply_filter();
            }
        });

        ui.add_space(8.0);

        // Table with acquisitions
        use egui_extras::{Column, TableBuilder};

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(80.0)) // UID
            .column(Column::auto().at_least(140.0)) // Date
            .column(Column::auto().at_least(80.0)) // Samples
            .column(Column::auto().at_least(80.0)) // Size
            .column(Column::remainder()) // Name
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Run UID");
                });
                header.col(|ui| {
                    ui.strong("Date");
                });
                header.col(|ui| {
                    ui.strong("Samples");
                });
                header.col(|ui| {
                    ui.strong("Size (MB)");
                });
                header.col(|ui| {
                    ui.strong("Name");
                });
            })
            .body(|mut body| {
                for (idx, acq) in self.filtered_acquisitions.iter().enumerate() {
                    body.row(18.0, |mut row| {
                        let is_selected = self.selected_run_idx == Some(idx);

                        row.col(|ui| {
                            if ui
                                .selectable_label(
                                    is_selected,
                                    &acq.acquisition_id[..8.min(acq.acquisition_id.len())],
                                )
                                .clicked()
                            {
                                self.selected_run_idx = Some(idx);
                            }
                        });
                        row.col(|ui| {
                            ui.label(format_timestamp(acq.created_at_ns));
                        });
                        row.col(|ui| {
                            ui.label(acq.sample_count.to_string());
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.2}", acq.file_size_bytes as f64 / 1_000_000.0));
                        });
                        row.col(|ui| {
                            ui.label(&acq.name);
                        });
                    });
                }
            });

        ui.separator();

        // Detail view for selected run
        if let Some(idx) = self.selected_run_idx {
            if let Some(acq) = self.filtered_acquisitions.get(idx) {
                ui.heading("Run Details");

                egui::Grid::new("run_details_grid")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Run UID:");
                        ui.label(&acq.acquisition_id);
                        ui.end_row();

                        ui.label("Name:");
                        ui.label(&acq.name);
                        ui.end_row();

                        ui.label("Created:");
                        ui.label(format_timestamp(acq.created_at_ns));
                        ui.end_row();

                        ui.label("File Path:");
                        ui.label(&acq.file_path);
                        ui.end_row();

                        ui.label("File Size:");
                        ui.label(format!(
                            "{:.2} MB",
                            acq.file_size_bytes as f64 / 1_000_000.0
                        ));
                        ui.end_row();

                        ui.label("Sample Count:");
                        ui.label(acq.sample_count.to_string());
                        ui.end_row();
                    });

                // TODO: Display run metadata when AcquisitionSummary includes metadata field
                // This will be populated from HDF5 attributes in future enhancement

                ui.separator();
                ui.heading("Annotations");

                // Trigger load if annotations are empty (indicates new selection or not loaded)
                #[cfg(feature = "storage_hdf5")]
                if self.annotation_notes.is_empty() && self.annotation_tags.is_empty() {
                    // Check if we need to load annotations for this selection
                    // Only trigger if not already loading
                    if self
                        .pending_action
                        .as_ref()
                        .map_or(true, |a| !matches!(a, PendingAction::LoadAnnotation { .. }))
                    {
                        self.pending_action = Some(PendingAction::LoadAnnotation {
                            file_path: acq.file_path.clone(),
                        });
                    }
                }

                ui.label("Notes:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.annotation_notes)
                        .desired_width(f32::INFINITY),
                );

                ui.horizontal(|ui| {
                    ui.label("Tags (comma-separated):");
                    ui.text_edit_singleline(&mut self.annotation_tags);
                });

                #[cfg(feature = "storage_hdf5")]
                if ui.button("💾 Save Annotations").clicked() {
                    self.pending_action = Some(PendingAction::SaveAnnotation {
                        file_path: acq.file_path.clone(),
                    });
                }

                #[cfg(not(feature = "storage_hdf5"))]
                ui.colored_label(
                    egui::Color32::GRAY,
                    "Annotation feature requires storage_hdf5 feature flag",
                );

                if let Some(status) = &self.annotation_status {
                    ui.label(status);
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Copy Run UID").clicked() {
                        ui.ctx().copy_text(acq.acquisition_id.clone());
                    }

                    if ui.button("Copy File Path").clicked() {
                        ui.ctx().copy_text(acq.file_path.clone());
                    }
                });
            }
        } else {
            ui.label("Select a run to view details");
        }

        // Handle pending actions
        if let Some(action) = self.pending_action.take() {
            match action {
                PendingAction::Refresh => self.refresh(client, runtime),
                #[cfg(feature = "storage_hdf5")]
                PendingAction::SaveAnnotation { file_path } => {
                    self.save_annotation(file_path, runtime);
                }
                #[cfg(feature = "storage_hdf5")]
                PendingAction::LoadAnnotation { file_path } => {
                    self.load_annotation(file_path, runtime);
                }
                #[cfg(not(feature = "storage_hdf5"))]
                PendingAction::SaveAnnotation { file_path: _ }
                | PendingAction::LoadAnnotation { file_path: _ } => {
                    self.annotation_status =
                        Some("Annotation feature requires storage_hdf5".to_string());
                }
            }
        }
    }
}

/// Format timestamp (nanoseconds) to human-readable date
fn format_timestamp(ns: u64) -> String {
    use chrono::{TimeZone, Utc};
    let secs = ns / 1_000_000_000;
    Utc.timestamp_opt(secs as i64, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
