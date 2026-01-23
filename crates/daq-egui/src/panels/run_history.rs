//! Run history panel - browse and filter past experiment acquisitions.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::{offline_notice, OfflineContext};

/// Pending action for run history panel
enum PendingAction {
    Refresh,
}

/// Result from an async action
enum ActionResult {
    Refresh(Result<Vec<daq_proto::daq::AcquisitionSummary>, String>),
}

/// Run history panel state
pub struct RunHistoryPanel {
    /// All acquisitions loaded from server
    acquisitions: Vec<daq_proto::daq::AcquisitionSummary>,
    /// Filtered acquisitions (after search)
    filtered_acquisitions: Vec<daq_proto::daq::AcquisitionSummary>,
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
                            ui.label(format!(
                                "{:.2}",
                                acq.file_size_bytes as f64 / 1_000_000.0
                            ));
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
                        ui.label(format!("{:.2} MB", acq.file_size_bytes as f64 / 1_000_000.0));
                        ui.end_row();

                        ui.label("Sample Count:");
                        ui.label(acq.sample_count.to_string());
                        ui.end_row();
                    });

                // TODO: Display run metadata when AcquisitionSummary includes metadata field
                // This will be populated from HDF5 attributes in future enhancement

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
