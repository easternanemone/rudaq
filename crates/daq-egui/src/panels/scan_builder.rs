//! Scan Builder panel - form-based 1D/2D scan configuration.
//!
//! This panel provides a simplified UI for scientists to configure parameter scans
//! by selecting devices from the daemon and entering scan parameters through a form.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints, Points};
use futures::StreamExt;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::widgets::{offline_notice, MetadataEditor, OfflineContext};
use daq_client::DaqClient;
use daq_proto::daq::Document;

/// Scan mode selection (1D vs 2D)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanMode {
    #[default]
    OneDimensional,
    TwoDimensional,
}

/// Execution state for the scan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionState {
    #[default]
    Idle,
    Running,
    Aborting,
}

/// Plot style selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotStyle {
    #[default]
    LineWithMarkers,
    ScatterOnly,
}

/// Pending async action
enum PendingAction {
    RefreshDevices,
    StartScan {
        plan_type: String,
        parameters: HashMap<String, String>,
        device_mapping: HashMap<String, String>,
        metadata: HashMap<String, String>,
    },
    AbortScan,
}

/// Result of an async action
enum ActionResult {
    DevicesLoaded(Result<Vec<daq_proto::daq::DeviceInfo>, String>),
    ScanStarted {
        run_uid: String,
        error: Option<String>,
    },
    ScanAborted {
        success: bool,
        error: Option<String>,
    },
}

/// Scan preview calculation result
struct ScanPreview {
    total_points: u32,
    estimated_duration_secs: f64,
    valid: bool,
}

/// Completion summary displayed after scan finishes
#[derive(Debug, Clone)]
struct CompletionSummary {
    run_uid: String,
    exit_status: String,
    total_points: u32,
    duration_secs: f64,
    saved_path: Option<String>,
}

/// Scan Builder panel state
pub struct ScanBuilderPanel {
    // Device cache (refreshed from daemon)
    devices: Vec<daq_proto::daq::DeviceInfo>,
    last_device_refresh: Option<Instant>,

    // Scan mode toggle
    scan_mode: ScanMode,

    // Device selection
    selected_actuator: Option<String>,   // 1D mode
    selected_actuator_x: Option<String>, // 2D mode (fast axis)
    selected_actuator_y: Option<String>, // 2D mode (slow axis)
    selected_detectors: Vec<String>,

    // 1D scan parameters (string fields for form input)
    start_1d: String,
    stop_1d: String,
    points_1d: String,

    // 2D scan parameters
    x_start: String,
    x_stop: String,
    x_points: String,
    y_start: String,
    y_stop: String,
    y_points: String,

    // Dwell time (shared by 1D and 2D)
    dwell_time_ms: String,

    // Validation errors (field name -> error message)
    validation_errors: HashMap<&'static str, String>,

    // Status/error display
    status: Option<String>,
    error: Option<String>,

    // Async integration (PendingAction + mpsc pattern)
    pending_action: Option<PendingAction>,
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
    action_in_flight: usize,

    // Execution state
    execution_state: ExecutionState,
    current_run_uid: Option<String>,
    start_time: Option<Instant>,

    // Progress tracking
    total_points: u32,
    current_point: u32,

    // Document streaming
    document_rx: Option<mpsc::Receiver<Result<Document, String>>>,
    subscription_task: Option<JoinHandle<()>>,

    // Live plot data: detector_id -> Vec<(actuator_position, detector_value)>
    plot_data: HashMap<String, Vec<(f64, f64)>>,
    // 2D plot data: Vec<(x_pos, y_pos, value)>
    plot_data_2d: Vec<(f64, f64, f64)>,
    // Plot display options
    show_plot: bool,
    plot_style: PlotStyle,

    // Completion summary
    show_completion_summary: bool,
    completion_summary: Option<CompletionSummary>,

    // Metadata editor
    metadata_editor: MetadataEditor,
}

impl Default for ScanBuilderPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            devices: Vec::new(),
            last_device_refresh: None,
            scan_mode: ScanMode::default(),
            selected_actuator: None,
            selected_actuator_x: None,
            selected_actuator_y: None,
            selected_detectors: Vec::new(),
            start_1d: "0.0".to_string(),
            stop_1d: "10.0".to_string(),
            points_1d: "11".to_string(),
            x_start: "0.0".to_string(),
            x_stop: "10.0".to_string(),
            x_points: "11".to_string(),
            y_start: "0.0".to_string(),
            y_stop: "10.0".to_string(),
            y_points: "11".to_string(),
            dwell_time_ms: "100.0".to_string(),
            validation_errors: HashMap::new(),
            status: None,
            error: None,
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            // Execution state
            execution_state: ExecutionState::default(),
            current_run_uid: None,
            start_time: None,
            // Progress tracking
            total_points: 0,
            current_point: 0,
            // Document streaming
            document_rx: None,
            subscription_task: None,
            // Live plot
            plot_data: HashMap::new(),
            plot_data_2d: Vec::new(),
            show_plot: true,
            plot_style: PlotStyle::default(),
            // Completion summary
            show_completion_summary: false,
            completion_summary: None,
            // Metadata editor
            metadata_editor: MetadataEditor::new(),
        }
    }
}

impl ScanBuilderPanel {
    /// Poll for completed async operations (non-blocking)
    /// Returns Some(run_uid) if a scan was started and needs document subscription
    fn poll_async_results(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut updated = false;
        let mut needs_subscription: Option<String> = None;

        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::DevicesLoaded(result) => match result {
                            Ok(devices) => {
                                self.devices = devices;
                                self.last_device_refresh = Some(Instant::now());
                                self.status =
                                    Some(format!("Loaded {} devices", self.devices.len()));
                                self.error = None;
                            }
                            Err(e) => {
                                self.error = Some(e);
                            }
                        },
                        ActionResult::ScanStarted { run_uid, error } => {
                            if let Some(err) = error {
                                self.error = Some(err);
                                self.execution_state = ExecutionState::Idle;
                            } else {
                                self.execution_state = ExecutionState::Running;
                                self.current_run_uid = Some(run_uid.clone());
                                self.start_time = Some(Instant::now());
                                self.current_point = 0;
                                // Calculate total_points based on scan mode
                                self.total_points = match self.scan_mode {
                                    ScanMode::OneDimensional => self.points_1d.parse().unwrap_or(0),
                                    ScanMode::TwoDimensional => {
                                        let x_pts: u32 = self.x_points.parse().unwrap_or(0);
                                        let y_pts: u32 = self.y_points.parse().unwrap_or(0);
                                        x_pts * y_pts
                                    }
                                };
                                self.plot_data.clear();
                                self.plot_data_2d.clear();
                                self.status = Some(format!("Run started: {}", run_uid));
                                self.error = None;

                                // Signal that we need to start document subscription
                                needs_subscription = Some(run_uid);
                            }
                        }
                        ActionResult::ScanAborted { success, error } => {
                            if success {
                                self.status = Some("Scan aborted".to_string());
                                self.execution_complete(false);
                            } else {
                                self.error = error;
                                self.execution_state = ExecutionState::Idle;
                            }
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

        needs_subscription
    }

    /// Poll documents from the subscription stream
    fn poll_documents(&mut self, ctx: &egui::Context) {
        // Collect documents first to avoid borrow issues
        let documents: Vec<_> = {
            let Some(rx) = &mut self.document_rx else {
                return;
            };

            let mut docs = Vec::new();
            while let Ok(result) = rx.try_recv() {
                docs.push(result);
            }
            docs
        };

        if documents.is_empty() {
            return;
        }

        // Process collected documents
        for result in documents {
            match result {
                Ok(doc) => self.handle_document(doc),
                Err(err) => {
                    self.error = Some(err);
                    self.execution_complete(false);
                    return;
                }
            }
        }

        // Request repaint if we're running
        if self.execution_state == ExecutionState::Running {
            ctx.request_repaint();
        }
    }

    /// Handle a received document
    fn handle_document(&mut self, doc: Document) {
        use daq_proto::daq::document::Payload;

        match doc.payload {
            Some(Payload::Start(start)) => {
                self.status = Some(format!("Run started: {}", start.run_uid));
            }
            Some(Payload::Event(event)) => {
                self.current_point = event.seq_num;
                // Process event for plot based on scan mode
                match self.scan_mode {
                    ScanMode::OneDimensional => self.process_event_for_plot(&event),
                    ScanMode::TwoDimensional => self.process_event_for_plot_2d(&event),
                }
            }
            Some(Payload::Stop(stop)) => {
                // Capture completion data
                let duration = self
                    .start_time
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);

                self.completion_summary = Some(CompletionSummary {
                    run_uid: stop.run_uid.clone(),
                    exit_status: stop.exit_status.clone(),
                    total_points: stop.num_events,
                    duration_secs: duration,
                    // DocumentWriter saves to data/{run_uid}.h5 or .csv
                    saved_path: Some(format!("data/{}.h5", stop.run_uid)),
                });
                self.show_completion_summary = true;

                let success = stop.exit_status == "success";
                self.execution_complete(success);
            }
            _ => {}
        }
    }

    /// Process an event document for 1D plot data
    fn process_event_for_plot(&mut self, event: &daq_proto::daq::EventDocument) {
        // Extract actuator position from event.data
        // The positions may be in the data map with the actuator device_id as key
        // Or we may need to use seq_num as fallback
        let actuator_pos = self
            .selected_actuator
            .as_ref()
            .and_then(|id| event.data.get(id))
            .copied()
            .unwrap_or(event.seq_num as f64);

        // Extract detector values from event.data
        for detector_id in &self.selected_detectors {
            if let Some(&value) = event.data.get(detector_id) {
                self.plot_data
                    .entry(detector_id.clone())
                    .or_default()
                    .push((actuator_pos, value));
            }
        }
    }

    /// Process an event document for 2D plot data
    fn process_event_for_plot_2d(&mut self, event: &daq_proto::daq::EventDocument) {
        // Extract X position
        let x_pos = self
            .selected_actuator_x
            .as_ref()
            .and_then(|id| event.data.get(id))
            .copied()
            .unwrap_or(0.0);

        // Extract Y position
        let y_pos = self
            .selected_actuator_y
            .as_ref()
            .and_then(|id| event.data.get(id))
            .copied()
            .unwrap_or(0.0);

        // Extract detector value (use first detector)
        let value = self
            .selected_detectors
            .first()
            .and_then(|id| event.data.get(id))
            .copied()
            .unwrap_or(0.0);

        self.plot_data_2d.push((x_pos, y_pos, value));
    }

    /// Mark execution as complete
    fn execution_complete(&mut self, success: bool) {
        self.execution_state = ExecutionState::Idle;
        if let Some(handle) = self.subscription_task.take() {
            handle.abort();
        }
        self.document_rx = None;

        if success {
            self.status = Some(format!(
                "Scan complete: {} points in {:.1}s",
                self.current_point,
                self.start_time
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0)
            ));
        }
    }

    /// Start document subscription for the current run
    fn start_document_subscription(
        &mut self,
        client: &mut DaqClient,
        runtime: &Runtime,
        run_uid: &str,
    ) {
        let (tx, rx) = mpsc::channel(100);
        self.document_rx = Some(rx);

        let mut client = client.clone();
        let run_uid = Some(run_uid.to_string());

        self.subscription_task = Some(runtime.spawn(async move {
            match client.stream_documents(run_uid, vec![]).await {
                Ok(mut stream) => {
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(doc) => {
                                if tx.send(Ok(doc)).await.is_err() {
                                    break;
                                }
                            }
                            Err(status) => {
                                let _ = tx.send(Err(format!("gRPC Error: {}", status))).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("Failed to subscribe: {}", e))).await;
                }
            }
        }));
    }

    /// Render the Scan Builder panel
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll for documents (must do this first to avoid borrow issues)
        self.poll_documents(ui.ctx());

        // Poll for action results, check if we need to start subscription
        if let Some(run_uid) = self.poll_async_results(ui.ctx()) {
            if let Some(ref mut client) = client {
                self.start_document_subscription(client, runtime, &run_uid);
            }
        }

        self.pending_action = None;

        ui.heading("Scan Builder");

        // Show offline notice if not connected
        if offline_notice(ui, client.is_none(), OfflineContext::Experiments) {
            return;
        }

        ui.separator();

        // Toolbar: Refresh + last refresh time
        ui.horizontal(|ui| {
            let refresh_enabled = self.execution_state == ExecutionState::Idle;
            if ui
                .add_enabled(refresh_enabled, egui::Button::new("Refresh Devices"))
                .clicked()
            {
                self.pending_action = Some(PendingAction::RefreshDevices);
            }

            if let Some(last) = self.last_device_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("Updated {}s ago", elapsed.as_secs()));
            }
        });

        // Show error/status messages
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.add_space(8.0);

        // Scan mode toggle (disabled during execution)
        ui.horizontal(|ui| {
            ui.label("Scan Type:");
            let enabled = self.execution_state == ExecutionState::Idle;
            ui.add_enabled_ui(enabled, |ui| {
                ui.selectable_value(
                    &mut self.scan_mode,
                    ScanMode::OneDimensional,
                    "1D Line Scan",
                );
                ui.selectable_value(
                    &mut self.scan_mode,
                    ScanMode::TwoDimensional,
                    "2D Grid Scan",
                );
            });
        });

        ui.add_space(8.0);
        ui.separator();

        // Device selection sections (disabled during execution)
        let enabled = self.execution_state == ExecutionState::Idle;
        ui.add_enabled_ui(enabled, |ui| {
            self.render_actuator_section(ui);
            ui.add_space(8.0);
            self.render_detector_section(ui);
            ui.add_space(8.0);
            ui.separator();

            // Parameter input section
            self.render_parameters_section(ui);
        });

        ui.add_space(8.0);
        ui.separator();

        // Scan preview
        self.render_scan_preview(ui);

        ui.add_space(8.0);

        // Progress bar (when running)
        self.render_progress_bar(ui);

        ui.add_space(8.0);

        // Live plot section
        ui.separator();
        ui.checkbox(&mut self.show_plot, "Show Live Plot");

        if self.show_plot {
            match self.scan_mode {
                ScanMode::OneDimensional => {
                    if !self.plot_data.is_empty() || self.execution_state == ExecutionState::Running
                    {
                        self.render_live_plot(ui);
                    } else {
                        ui.label("Plot will appear when scan starts...");
                    }
                }
                ScanMode::TwoDimensional => {
                    if !self.plot_data_2d.is_empty()
                        || self.execution_state == ExecutionState::Running
                    {
                        self.render_2d_plot(ui);
                    } else {
                        ui.label("2D plot will appear when scan starts...");
                    }
                }
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // Control buttons
        self.render_control_buttons(ui);

        // Execute pending action
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }

        // Render completion summary if shown
        if self.show_completion_summary {
            self.render_completion_summary(ui.ctx());
        }
    }

    /// Render progress bar with ETA
    fn render_progress_bar(&mut self, ui: &mut egui::Ui) {
        if self.execution_state == ExecutionState::Running && self.total_points > 0 {
            let progress = self.current_point as f32 / self.total_points as f32;

            // Calculate ETA
            let elapsed = self.start_time.map(|t| t.elapsed()).unwrap_or_default();
            let eta = if self.current_point > 0 {
                let rate = elapsed.as_secs_f64() / self.current_point as f64;
                let remaining = (self.total_points - self.current_point) as f64 * rate;
                format!(", ETA: {}", format_duration(remaining))
            } else {
                String::new()
            };

            let progress_bar = egui::ProgressBar::new(progress).text(format!(
                "{}/{} ({:.1}%){}",
                self.current_point,
                self.total_points,
                progress * 100.0,
                eta
            ));
            ui.add(progress_bar);
        }
    }

    /// Render live 1D plot
    fn render_live_plot(&mut self, ui: &mut egui::Ui) {
        // Plot style toggle
        ui.horizontal(|ui| {
            ui.label("Plot:");
            ui.selectable_value(&mut self.plot_style, PlotStyle::LineWithMarkers, "Line");
            ui.selectable_value(&mut self.plot_style, PlotStyle::ScatterOnly, "Scatter");
        });

        let actuator_name = self.selected_actuator.as_deref().unwrap_or("Position");

        Plot::new("scan_live_plot")
            .height(200.0)
            .show_axes(true)
            .show_grid(true)
            .x_axis_label(actuator_name)
            .y_axis_label("Signal")
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                // Preset colors for multiple detectors
                let colors = [
                    egui::Color32::from_rgb(255, 100, 100),
                    egui::Color32::from_rgb(100, 200, 100),
                    egui::Color32::from_rgb(100, 150, 255),
                    egui::Color32::from_rgb(255, 200, 100),
                ];

                for (idx, (detector_id, points)) in self.plot_data.iter().enumerate() {
                    let color = colors[idx % colors.len()];
                    // Convert points to Vec for reuse
                    let point_vec: Vec<[f64; 2]> = points.iter().map(|(x, y)| [*x, *y]).collect();

                    match self.plot_style {
                        PlotStyle::LineWithMarkers => {
                            // Create PlotPoints via collect for line
                            let line_points: PlotPoints = point_vec.iter().copied().collect();
                            plot_ui.line(
                                Line::new(detector_id.clone(), line_points)
                                    .color(color)
                                    .width(2.0),
                            );
                            // Create PlotPoints again for markers
                            let marker_points: PlotPoints = point_vec.iter().copied().collect();
                            plot_ui.points(
                                Points::new(format!("{} pts", detector_id), marker_points)
                                    .color(color)
                                    .radius(3.0),
                            );
                        }
                        PlotStyle::ScatterOnly => {
                            let scatter_points: PlotPoints = point_vec.iter().copied().collect();
                            plot_ui.points(
                                Points::new(detector_id.clone(), scatter_points)
                                    .color(color)
                                    .radius(4.0),
                            );
                        }
                    }
                }
            });
    }

    /// Render 2D scatter plot with color-coded values
    fn render_2d_plot(&self, ui: &mut egui::Ui) {
        if self.plot_data_2d.is_empty() {
            ui.label("Waiting for 2D scan data...");
            return;
        }

        // Calculate value range for color mapping
        let (min_val, max_val) = self.plot_data_2d.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(min, max), (_, _, v)| (min.min(*v), max.max(*v)),
        );

        let x_label = self.selected_actuator_x.as_deref().unwrap_or("X");
        let y_label = self.selected_actuator_y.as_deref().unwrap_or("Y");

        Plot::new("scan_2d_plot")
            .height(300.0)
            .data_aspect(1.0) // Square aspect for grid
            .show_axes(true)
            .show_grid(true)
            .x_axis_label(x_label)
            .y_axis_label(y_label)
            .show(ui, |plot_ui| {
                // Color each point based on value (blue -> red gradient)
                for (idx, &(x, y, value)) in self.plot_data_2d.iter().enumerate() {
                    let normalized = if max_val > min_val {
                        ((value - min_val) / (max_val - min_val)).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };

                    // Blue (cold) to Red (hot) gradient
                    let r = (normalized * 255.0) as u8;
                    let b = ((1.0 - normalized) * 255.0) as u8;
                    let color = egui::Color32::from_rgb(r, 50, b);

                    // Each point needs a unique name for proper rendering
                    let point_name = format!("pt_{}", idx);
                    let points: PlotPoints = vec![[x, y]].into_iter().collect();
                    plot_ui.points(Points::new(point_name, points).color(color).radius(5.0));
                }
            });

        // Color scale legend
        ui.horizontal(|ui| {
            ui.label(format!("Value: {:.2}", min_val));
            // Draw gradient bar
            let (rect, _) = ui.allocate_exact_size(egui::vec2(100.0, 15.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            for i in 0..100 {
                let t = i as f32 / 100.0;
                let r = (t * 255.0) as u8;
                let b = ((1.0 - t) * 255.0) as u8;
                let color = egui::Color32::from_rgb(r, 50, b);
                let x = rect.min.x + t * rect.width();
                painter.line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0, color),
                );
            }
            ui.label(format!("{:.2}", max_val));
        });
    }

    /// Render completion summary window
    fn render_completion_summary(&mut self, ctx: &egui::Context) {
        let Some(summary) = &self.completion_summary else {
            return;
        };

        let mut close_summary = false;

        egui::Window::new("Scan Complete")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // Status icon and heading
                let (icon, color) = if summary.exit_status == "success" {
                    ("OK", egui::Color32::GREEN)
                } else if summary.exit_status == "abort" {
                    ("Stopped", egui::Color32::YELLOW)
                } else {
                    ("Error", egui::Color32::RED)
                };

                ui.horizontal(|ui| {
                    ui.colored_label(color, icon);
                    ui.heading(format!("Scan {}", summary.exit_status));
                });

                ui.separator();

                // Summary details in grid
                egui::Grid::new("completion_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Run ID:");
                        ui.monospace(&summary.run_uid);
                        ui.end_row();

                        ui.label("Duration:");
                        ui.label(format_duration(summary.duration_secs));
                        ui.end_row();

                        ui.label("Total Points:");
                        ui.label(summary.total_points.to_string());
                        ui.end_row();

                        if let Some(path) = &summary.saved_path {
                            ui.label("Saved to:");
                            ui.monospace(path);
                            ui.end_row();
                        }
                    });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close_summary = true;
                    }

                    // Copy run_uid to clipboard
                    if ui.button("Copy Run ID").clicked() {
                        ui.ctx().copy_text(summary.run_uid.clone());
                    }
                });
            });

        if close_summary {
            self.show_completion_summary = false;
        }
    }

    /// Render control buttons (Start/Abort)
    fn render_control_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.execution_state {
                ExecutionState::Idle => {
                    // Validate form before enabling Start
                    self.validate_form();
                    let can_start = match self.scan_mode {
                        ScanMode::OneDimensional => {
                            self.validation_errors.is_empty()
                                && self.selected_actuator.is_some()
                                && !self.selected_detectors.is_empty()
                        }
                        ScanMode::TwoDimensional => {
                            self.validation_errors.is_empty()
                                && self.selected_actuator_x.is_some()
                                && self.selected_actuator_y.is_some()
                                && !self.selected_detectors.is_empty()
                        }
                    };

                    if ui
                        .add_enabled(can_start, egui::Button::new("Start Scan"))
                        .clicked()
                    {
                        // Build plan parameters based on scan mode
                        let (plan_type, parameters, device_mapping) = match self.scan_mode {
                            ScanMode::OneDimensional => {
                                let mut params = HashMap::new();
                                params.insert("start_position".to_string(), self.start_1d.clone());
                                params.insert("stop_position".to_string(), self.stop_1d.clone());
                                params.insert("num_points".to_string(), self.points_1d.clone());

                                let mut devices = HashMap::new();
                                if let Some(actuator) = &self.selected_actuator {
                                    devices.insert("motor".to_string(), actuator.clone());
                                }
                                if let Some(detector) = self.selected_detectors.first() {
                                    devices.insert("detector".to_string(), detector.clone());
                                }

                                ("line_scan".to_string(), params, devices)
                            }
                            ScanMode::TwoDimensional => {
                                let mut params = HashMap::new();
                                // X axis (fast/inner)
                                params.insert("x_start".to_string(), self.x_start.clone());
                                params.insert("x_stop".to_string(), self.x_stop.clone());
                                params.insert("x_points".to_string(), self.x_points.clone());
                                // Y axis (slow/outer)
                                params.insert("y_start".to_string(), self.y_start.clone());
                                params.insert("y_stop".to_string(), self.y_stop.clone());
                                params.insert("y_points".to_string(), self.y_points.clone());

                                let mut devices = HashMap::new();
                                if let Some(actuator_x) = &self.selected_actuator_x {
                                    devices.insert("x_motor".to_string(), actuator_x.clone());
                                }
                                if let Some(actuator_y) = &self.selected_actuator_y {
                                    devices.insert("y_motor".to_string(), actuator_y.clone());
                                }
                                if let Some(detector) = self.selected_detectors.first() {
                                    devices.insert("detector".to_string(), detector.clone());
                                }

                                ("grid_scan".to_string(), params, devices)
                            }
                        };

                        // Build metadata from editor + auto-add scan provenance
                        let mut metadata = self.metadata_editor.to_metadata_map();
                        metadata.insert("scan_type".to_string(), plan_type.clone());

                        // Add actuator/detector info for provenance
                        match self.scan_mode {
                            ScanMode::OneDimensional => {
                                if let Some(actuator) = &self.selected_actuator {
                                    metadata.insert("actuator".to_string(), actuator.clone());
                                }
                                if let Some(detector) = self.selected_detectors.first() {
                                    metadata.insert("detector".to_string(), detector.clone());
                                }
                            }
                            ScanMode::TwoDimensional => {
                                if let Some(actuator_x) = &self.selected_actuator_x {
                                    metadata.insert("motor_x".to_string(), actuator_x.clone());
                                }
                                if let Some(actuator_y) = &self.selected_actuator_y {
                                    metadata.insert("motor_y".to_string(), actuator_y.clone());
                                }
                                if let Some(detector) = self.selected_detectors.first() {
                                    metadata.insert("detector".to_string(), detector.clone());
                                }
                            }
                        }

                        self.pending_action = Some(PendingAction::StartScan {
                            plan_type,
                            parameters,
                            device_mapping,
                            metadata,
                        });
                    }

                    if !can_start {
                        ui.colored_label(egui::Color32::GRAY, "Complete form to enable Start");
                    }
                }
                ExecutionState::Running => {
                    if ui.button("Abort").clicked() {
                        self.pending_action = Some(PendingAction::AbortScan);
                        self.execution_state = ExecutionState::Aborting;
                    }
                }
                ExecutionState::Aborting => {
                    ui.add_enabled(false, egui::Button::new("Aborting..."));
                }
            }
        });
    }

    /// Render the actuator (movable devices) selection section
    fn render_actuator_section(&mut self, ui: &mut egui::Ui) {
        let actuators: Vec<_> = self.devices.iter().filter(|d| d.is_movable).collect();

        ui.group(|ui| {
            ui.heading("Actuators");

            if actuators.is_empty() {
                ui.colored_label(
                    egui::Color32::GRAY,
                    "No movable devices found. Click 'Refresh Devices' to load.",
                );
                return;
            }

            match self.scan_mode {
                ScanMode::OneDimensional => {
                    // Single actuator selection
                    ui.horizontal(|ui| {
                        ui.label("Motor:");
                        let selected_text = self
                            .selected_actuator
                            .as_ref()
                            .and_then(|id| actuators.iter().find(|d| &d.id == id))
                            .map(|d| format!("{} ({})", d.name, d.id))
                            .unwrap_or_else(|| "Select actuator...".to_string());

                        egui::ComboBox::from_id_salt("actuator_1d")
                            .selected_text(&selected_text)
                            .show_ui(ui, |ui| {
                                for device in &actuators {
                                    let label = format!("{} ({})", device.name, device.id);
                                    if ui
                                        .selectable_label(
                                            self.selected_actuator.as_ref() == Some(&device.id),
                                            &label,
                                        )
                                        .clicked()
                                    {
                                        self.selected_actuator = Some(device.id.clone());
                                    }
                                }
                            });

                        // Show validation error
                        if let Some(err) = self.validation_errors.get("actuator") {
                            ui.colored_label(egui::Color32::RED, err);
                        }
                    });
                }
                ScanMode::TwoDimensional => {
                    // Two actuator selection (X and Y axes)
                    ui.horizontal(|ui| {
                        ui.label("X Axis (fast):");
                        let selected_x_text = self
                            .selected_actuator_x
                            .as_ref()
                            .and_then(|id| actuators.iter().find(|d| &d.id == id))
                            .map(|d| format!("{} ({})", d.name, d.id))
                            .unwrap_or_else(|| "Select actuator...".to_string());

                        egui::ComboBox::from_id_salt("actuator_x")
                            .selected_text(&selected_x_text)
                            .show_ui(ui, |ui| {
                                for device in &actuators {
                                    let label = format!("{} ({})", device.name, device.id);
                                    if ui
                                        .selectable_label(
                                            self.selected_actuator_x.as_ref() == Some(&device.id),
                                            &label,
                                        )
                                        .clicked()
                                    {
                                        self.selected_actuator_x = Some(device.id.clone());
                                    }
                                }
                            });
                    });

                    ui.horizontal(|ui| {
                        ui.label("Y Axis (slow):");
                        let selected_y_text = self
                            .selected_actuator_y
                            .as_ref()
                            .and_then(|id| actuators.iter().find(|d| &d.id == id))
                            .map(|d| format!("{} ({})", d.name, d.id))
                            .unwrap_or_else(|| "Select actuator...".to_string());

                        egui::ComboBox::from_id_salt("actuator_y")
                            .selected_text(&selected_y_text)
                            .show_ui(ui, |ui| {
                                for device in &actuators {
                                    let label = format!("{} ({})", device.name, device.id);
                                    if ui
                                        .selectable_label(
                                            self.selected_actuator_y.as_ref() == Some(&device.id),
                                            &label,
                                        )
                                        .clicked()
                                    {
                                        self.selected_actuator_y = Some(device.id.clone());
                                    }
                                }
                            });
                    });

                    // Show validation errors for 2D mode
                    if let Some(err) = self.validation_errors.get("actuator_x") {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    if let Some(err) = self.validation_errors.get("actuator_y") {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                }
            }
        });
    }

    /// Render the detector (readable/camera devices) selection section
    fn render_detector_section(&mut self, ui: &mut egui::Ui) {
        let detectors: Vec<_> = self
            .devices
            .iter()
            .filter(|d| d.is_readable || d.is_frame_producer)
            .collect();

        ui.group(|ui| {
            ui.heading("Detectors");

            if detectors.is_empty() {
                ui.colored_label(
                    egui::Color32::GRAY,
                    "No readable devices found. Click 'Refresh Devices' to load.",
                );
                return;
            }

            // Multi-select checkboxes for detectors
            for device in &detectors {
                let mut is_selected = self.selected_detectors.contains(&device.id);
                let device_type = if device.is_frame_producer {
                    "Camera"
                } else {
                    "Sensor"
                };
                let label = format!("{} ({}) - {}", device.name, device.id, device_type);

                if ui.checkbox(&mut is_selected, &label).changed() {
                    if is_selected {
                        if !self.selected_detectors.contains(&device.id) {
                            self.selected_detectors.push(device.id.clone());
                        }
                    } else {
                        self.selected_detectors.retain(|id| id != &device.id);
                    }
                }
            }

            // Show validation error
            if let Some(err) = self.validation_errors.get("detectors") {
                ui.colored_label(egui::Color32::RED, err);
            }
        });
    }

    /// Render the scan parameters input section
    fn render_parameters_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Scan Parameters");

            match self.scan_mode {
                ScanMode::OneDimensional => {
                    self.render_1d_parameters(ui);
                }
                ScanMode::TwoDimensional => {
                    self.render_2d_parameters(ui);
                }
            }

            ui.add_space(4.0);

            // Dwell time (shared by both modes)
            ui.horizontal(|ui| {
                ui.label("Dwell Time:");
                let has_error = self.validation_errors.contains_key("dwell_time");
                let changed = Self::render_text_field(
                    ui,
                    &mut self.dwell_time_ms,
                    has_error,
                    self.validation_errors.get("dwell_time"),
                );
                if changed {
                    self.validate_form();
                }
                ui.label("ms");
            });
        });
    }

    /// Render 1D scan parameter inputs
    fn render_1d_parameters(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Start:");
            let has_error = self.validation_errors.contains_key("start_1d");
            if Self::render_text_field(
                ui,
                &mut self.start_1d,
                has_error,
                self.validation_errors.get("start_1d"),
            ) {
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Stop:");
            let has_error = self.validation_errors.contains_key("stop_1d");
            if Self::render_text_field(
                ui,
                &mut self.stop_1d,
                has_error,
                self.validation_errors.get("stop_1d"),
            ) {
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Points:");
            let has_error = self.validation_errors.contains_key("points_1d");
            if Self::render_text_field(
                ui,
                &mut self.points_1d,
                has_error,
                self.validation_errors.get("points_1d"),
            ) {
                changed = true;
            }
        });

        if changed {
            self.validate_form();
        }
    }

    /// Render 2D scan parameter inputs
    fn render_2d_parameters(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        ui.label("X Axis (fast):");
        ui.horizontal(|ui| {
            ui.label("  Start:");
            let has_error = self.validation_errors.contains_key("x_start");
            if Self::render_text_field(
                ui,
                &mut self.x_start,
                has_error,
                self.validation_errors.get("x_start"),
            ) {
                changed = true;
            }
            ui.label("Stop:");
            let has_error = self.validation_errors.contains_key("x_stop");
            if Self::render_text_field(
                ui,
                &mut self.x_stop,
                has_error,
                self.validation_errors.get("x_stop"),
            ) {
                changed = true;
            }
            ui.label("Points:");
            let has_error = self.validation_errors.contains_key("x_points");
            if Self::render_text_field(
                ui,
                &mut self.x_points,
                has_error,
                self.validation_errors.get("x_points"),
            ) {
                changed = true;
            }
        });

        ui.add_space(4.0);

        ui.label("Y Axis (slow):");
        ui.horizontal(|ui| {
            ui.label("  Start:");
            let has_error = self.validation_errors.contains_key("y_start");
            if Self::render_text_field(
                ui,
                &mut self.y_start,
                has_error,
                self.validation_errors.get("y_start"),
            ) {
                changed = true;
            }
            ui.label("Stop:");
            let has_error = self.validation_errors.contains_key("y_stop");
            if Self::render_text_field(
                ui,
                &mut self.y_stop,
                has_error,
                self.validation_errors.get("y_stop"),
            ) {
                changed = true;
            }
            ui.label("Points:");
            let has_error = self.validation_errors.contains_key("y_points");
            if Self::render_text_field(
                ui,
                &mut self.y_points,
                has_error,
                self.validation_errors.get("y_points"),
            ) {
                changed = true;
            }
        });

        if changed {
            self.validate_form();
        }
    }

    /// Render a single text field with optional error styling
    /// Returns true if the text was modified
    fn render_text_field(
        ui: &mut egui::Ui,
        text: &mut String,
        has_error: bool,
        error_msg: Option<&String>,
    ) -> bool {
        // Apply red stroke if validation error
        let mut frame = egui::Frame::NONE;
        if has_error {
            frame = frame.stroke(egui::Stroke::new(1.0, egui::Color32::RED));
        }

        let old_text = text.clone();
        let response = frame.show(ui, |ui| {
            ui.add_sized([80.0, 18.0], egui::TextEdit::singleline(text))
        });

        // Show tooltip on hover if error
        if has_error {
            if let Some(err) = error_msg {
                response.response.on_hover_text(err);
            }
        }

        text != &old_text
    }

    /// Validate the entire form and populate validation_errors
    fn validate_form(&mut self) {
        self.validation_errors.clear();

        // Validate actuator selection
        match self.scan_mode {
            ScanMode::OneDimensional => {
                if self.selected_actuator.is_none() {
                    self.validation_errors
                        .insert("actuator", "Select an actuator".to_string());
                }
            }
            ScanMode::TwoDimensional => {
                if self.selected_actuator_x.is_none() {
                    self.validation_errors
                        .insert("actuator_x", "Select X axis actuator".to_string());
                }
                if self.selected_actuator_y.is_none() {
                    self.validation_errors
                        .insert("actuator_y", "Select Y axis actuator".to_string());
                }
                // Check for same actuator on both axes
                if let (Some(x), Some(y)) = (&self.selected_actuator_x, &self.selected_actuator_y) {
                    if x == y {
                        self.validation_errors
                            .insert("actuator_y", "X and Y axes must be different".to_string());
                    }
                }
            }
        }

        // Validate detector selection
        if self.selected_detectors.is_empty() {
            self.validation_errors
                .insert("detectors", "Select at least one detector".to_string());
        }

        // Validate numeric fields based on mode
        match self.scan_mode {
            ScanMode::OneDimensional => {
                self.validate_float_field("start_1d", &self.start_1d.clone());
                self.validate_float_field("stop_1d", &self.stop_1d.clone());
                self.validate_points_field("points_1d", &self.points_1d.clone());

                // Check start != stop
                if let (Ok(start), Ok(stop)) =
                    (self.start_1d.parse::<f64>(), self.stop_1d.parse::<f64>())
                {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors
                            .insert("stop_1d", "Stop must differ from Start".to_string());
                    }
                }
            }
            ScanMode::TwoDimensional => {
                self.validate_float_field("x_start", &self.x_start.clone());
                self.validate_float_field("x_stop", &self.x_stop.clone());
                self.validate_points_field("x_points", &self.x_points.clone());
                self.validate_float_field("y_start", &self.y_start.clone());
                self.validate_float_field("y_stop", &self.y_stop.clone());
                self.validate_points_field("y_points", &self.y_points.clone());

                // Check start != stop for both axes
                if let (Ok(start), Ok(stop)) =
                    (self.x_start.parse::<f64>(), self.x_stop.parse::<f64>())
                {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors
                            .insert("x_stop", "Stop must differ from Start".to_string());
                    }
                }
                if let (Ok(start), Ok(stop)) =
                    (self.y_start.parse::<f64>(), self.y_stop.parse::<f64>())
                {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors
                            .insert("y_stop", "Stop must differ from Start".to_string());
                    }
                }
            }
        }

        // Validate dwell time
        self.validate_positive_float_field("dwell_time", &self.dwell_time_ms.clone());
    }

    /// Validate a field as a valid f64
    fn validate_float_field(&mut self, field_name: &'static str, value: &str) {
        if value.parse::<f64>().is_err() {
            self.validation_errors
                .insert(field_name, "Must be a valid number".to_string());
        }
    }

    /// Validate a field as a positive f64
    fn validate_positive_float_field(&mut self, field_name: &'static str, value: &str) {
        match value.parse::<f64>() {
            Ok(v) if v > 0.0 => {}
            Ok(_) => {
                self.validation_errors
                    .insert(field_name, "Must be positive".to_string());
            }
            Err(_) => {
                self.validation_errors
                    .insert(field_name, "Must be a valid number".to_string());
            }
        }
    }

    /// Validate a field as a valid positive integer (points)
    fn validate_points_field(&mut self, field_name: &'static str, value: &str) {
        match value.parse::<u32>() {
            Ok(v) if v > 0 => {}
            Ok(_) => {
                self.validation_errors
                    .insert(field_name, "Must be > 0".to_string());
            }
            Err(_) => {
                self.validation_errors
                    .insert(field_name, "Must be a valid integer".to_string());
            }
        }
    }

    /// Calculate and render scan preview
    fn render_scan_preview(&mut self, ui: &mut egui::Ui) {
        let preview = self.calculate_scan_preview();

        ui.group(|ui| {
            ui.heading("Scan Preview");

            if preview.valid {
                ui.label(format!(
                    "{} points, ~{}",
                    preview.total_points,
                    format_duration(preview.estimated_duration_secs)
                ));
            } else {
                ui.colored_label(egui::Color32::GRAY, "Complete form to see preview");
            }
        });
    }

    /// Calculate scan preview (total points, estimated duration)
    fn calculate_scan_preview(&self) -> ScanPreview {
        // Check if form is valid enough for preview
        let dwell_ms: f64 = self.dwell_time_ms.parse().unwrap_or(0.0);
        if dwell_ms <= 0.0 {
            return ScanPreview {
                total_points: 0,
                estimated_duration_secs: 0.0,
                valid: false,
            };
        }

        let total_points = match self.scan_mode {
            ScanMode::OneDimensional => {
                let points: u32 = self.points_1d.parse().unwrap_or(0);
                if points == 0 || self.selected_actuator.is_none() {
                    return ScanPreview {
                        total_points: 0,
                        estimated_duration_secs: 0.0,
                        valid: false,
                    };
                }
                points
            }
            ScanMode::TwoDimensional => {
                let x_points: u32 = self.x_points.parse().unwrap_or(0);
                let y_points: u32 = self.y_points.parse().unwrap_or(0);
                if x_points == 0
                    || y_points == 0
                    || self.selected_actuator_x.is_none()
                    || self.selected_actuator_y.is_none()
                {
                    return ScanPreview {
                        total_points: 0,
                        estimated_duration_secs: 0.0,
                        valid: false,
                    };
                }
                x_points * y_points
            }
        };

        if self.selected_detectors.is_empty() {
            return ScanPreview {
                total_points: 0,
                estimated_duration_secs: 0.0,
                valid: false,
            };
        }

        let estimated_duration_secs = (total_points as f64) * dwell_ms / 1000.0;

        ScanPreview {
            total_points,
            estimated_duration_secs,
            valid: true,
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
            PendingAction::RefreshDevices => self.refresh_devices(client, runtime),
            PendingAction::StartScan {
                plan_type,
                parameters,
                device_mapping,
                metadata,
            } => self.start_scan(
                client,
                runtime,
                plan_type,
                parameters,
                device_mapping,
                metadata,
            ),
            PendingAction::AbortScan => self.abort_scan(client, runtime),
        }
    }

    /// Refresh device list from daemon
    fn refresh_devices(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
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
            let result = client.list_devices().await.map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::DevicesLoaded(result)).await;
        });
    }

    /// Start a scan
    fn start_scan(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        plan_type: String,
        parameters: HashMap<String, String>,
        device_mapping: HashMap<String, String>,
        metadata: HashMap<String, String>,
    ) {
        self.error = None;
        self.status = Some("Starting scan...".to_string());

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            // Queue the plan with metadata
            match client
                .queue_plan(&plan_type, parameters, device_mapping, metadata)
                .await
            {
                Ok(queue_response) => {
                    if !queue_response.success {
                        let _ = tx
                            .send(ActionResult::ScanStarted {
                                run_uid: String::new(),
                                error: Some(queue_response.error_message),
                            })
                            .await;
                        return;
                    }

                    let run_uid = queue_response.run_uid;

                    // Start the engine
                    match client.start_engine().await {
                        Ok(start_response) => {
                            if start_response.success {
                                let _ = tx
                                    .send(ActionResult::ScanStarted {
                                        run_uid,
                                        error: None,
                                    })
                                    .await;
                            } else {
                                let _ = tx
                                    .send(ActionResult::ScanStarted {
                                        run_uid: String::new(),
                                        error: Some(start_response.error_message),
                                    })
                                    .await;
                            }
                        }
                        Err(e) => {
                            let _ = tx
                                .send(ActionResult::ScanStarted {
                                    run_uid: String::new(),
                                    error: Some(e.to_string()),
                                })
                                .await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx
                        .send(ActionResult::ScanStarted {
                            run_uid: String::new(),
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        });
    }

    /// Abort the current scan
    fn abort_scan(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            self.execution_state = ExecutionState::Idle;
            return;
        };

        // Abort document subscription
        if let Some(handle) = self.subscription_task.take() {
            handle.abort();
        }
        self.document_rx = None;

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            match client.abort_plan(None).await {
                Ok(response) => {
                    let _ = tx
                        .send(ActionResult::ScanAborted {
                            success: response.success,
                            error: if response.success {
                                None
                            } else {
                                Some(response.error_message)
                            },
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(ActionResult::ScanAborted {
                            success: false,
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        });
    }
}

/// Format duration in human-readable form
fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        format!("{:.1}min", secs / 60.0)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}
