//! Scan Builder panel - form-based 1D/2D scan configuration.
//!
//! This panel provides a simplified UI for scientists to configure parameter scans
//! by selecting devices from the daemon and entering scan parameters through a form.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::{offline_notice, OfflineContext};

/// Scan mode selection (1D vs 2D)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanMode {
    #[default]
    OneDimensional,
    TwoDimensional,
}

/// Pending async action
enum PendingAction {
    RefreshDevices,
}

/// Result of an async action
enum ActionResult {
    DevicesLoaded(Result<Vec<daq_proto::daq::DeviceInfo>, String>),
}

/// Scan preview calculation result
struct ScanPreview {
    total_points: u32,
    estimated_duration_secs: f64,
    valid: bool,
}

/// Scan Builder panel state
pub struct ScanBuilderPanel {
    // Device cache (refreshed from daemon)
    devices: Vec<daq_proto::daq::DeviceInfo>,
    last_device_refresh: Option<Instant>,

    // Scan mode toggle
    scan_mode: ScanMode,

    // Device selection
    selected_actuator: Option<String>,    // 1D mode
    selected_actuator_x: Option<String>,  // 2D mode (fast axis)
    selected_actuator_y: Option<String>,  // 2D mode (slow axis)
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
        }
    }
}

impl ScanBuilderPanel {
    /// Poll for completed async operations (non-blocking)
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::DevicesLoaded(result) => match result {
                            Ok(devices) => {
                                self.devices = devices;
                                self.last_device_refresh = Some(Instant::now());
                                self.status = Some(format!("Loaded {} devices", self.devices.len()));
                                self.error = None;
                            }
                            Err(e) => {
                                self.error = Some(e);
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

    /// Render the Scan Builder panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());
        self.pending_action = None;

        ui.heading("Scan Builder");

        // Show offline notice if not connected
        if offline_notice(ui, client.is_none(), OfflineContext::Experiments) {
            return;
        }

        ui.separator();

        // Toolbar: Refresh + last refresh time
        ui.horizontal(|ui| {
            if ui.button("Refresh Devices").clicked() {
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

        // Scan mode toggle
        ui.horizontal(|ui| {
            ui.label("Scan Type:");
            ui.selectable_value(&mut self.scan_mode, ScanMode::OneDimensional, "1D Line Scan");
            ui.selectable_value(&mut self.scan_mode, ScanMode::TwoDimensional, "2D Grid Scan");
        });

        ui.add_space(8.0);
        ui.separator();

        // Device selection sections
        self.render_actuator_section(ui);
        ui.add_space(8.0);
        self.render_detector_section(ui);
        ui.add_space(8.0);
        ui.separator();

        // Parameter input section
        self.render_parameters_section(ui);

        ui.add_space(8.0);
        ui.separator();

        // Scan preview
        self.render_scan_preview(ui);

        // Execute pending action
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }
    }

    /// Render the actuator (movable devices) selection section
    fn render_actuator_section(&mut self, ui: &mut egui::Ui) {
        let actuators: Vec<_> = self
            .devices
            .iter()
            .filter(|d| d.is_movable)
            .collect();

        ui.group(|ui| {
            ui.heading("Actuators");

            if actuators.is_empty() {
                ui.colored_label(egui::Color32::GRAY, "No movable devices found. Click 'Refresh Devices' to load.");
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
                                    if ui.selectable_label(
                                        self.selected_actuator.as_ref() == Some(&device.id),
                                        &label,
                                    ).clicked() {
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
                                    if ui.selectable_label(
                                        self.selected_actuator_x.as_ref() == Some(&device.id),
                                        &label,
                                    ).clicked() {
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
                                    if ui.selectable_label(
                                        self.selected_actuator_y.as_ref() == Some(&device.id),
                                        &label,
                                    ).clicked() {
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
                ui.colored_label(egui::Color32::GRAY, "No readable devices found. Click 'Refresh Devices' to load.");
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
                let response = self.render_validated_field(ui, &mut self.dwell_time_ms.clone(), "dwell_time");
                if response.changed() {
                    self.dwell_time_ms = response.text;
                }
                ui.label("ms");
            });
        });
    }

    /// Render 1D scan parameter inputs
    fn render_1d_parameters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Start:");
            let response = self.render_validated_field(ui, &mut self.start_1d.clone(), "start_1d");
            if response.changed() {
                self.start_1d = response.text;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Stop:");
            let response = self.render_validated_field(ui, &mut self.stop_1d.clone(), "stop_1d");
            if response.changed() {
                self.stop_1d = response.text;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Points:");
            let response = self.render_validated_field(ui, &mut self.points_1d.clone(), "points_1d");
            if response.changed() {
                self.points_1d = response.text;
            }
        });
    }

    /// Render 2D scan parameter inputs
    fn render_2d_parameters(&mut self, ui: &mut egui::Ui) {
        ui.label("X Axis (fast):");
        ui.horizontal(|ui| {
            ui.label("  Start:");
            let response = self.render_validated_field(ui, &mut self.x_start.clone(), "x_start");
            if response.changed() {
                self.x_start = response.text;
            }
            ui.label("Stop:");
            let response = self.render_validated_field(ui, &mut self.x_stop.clone(), "x_stop");
            if response.changed() {
                self.x_stop = response.text;
            }
            ui.label("Points:");
            let response = self.render_validated_field(ui, &mut self.x_points.clone(), "x_points");
            if response.changed() {
                self.x_points = response.text;
            }
        });

        ui.add_space(4.0);

        ui.label("Y Axis (slow):");
        ui.horizontal(|ui| {
            ui.label("  Start:");
            let response = self.render_validated_field(ui, &mut self.y_start.clone(), "y_start");
            if response.changed() {
                self.y_start = response.text;
            }
            ui.label("Stop:");
            let response = self.render_validated_field(ui, &mut self.y_stop.clone(), "y_stop");
            if response.changed() {
                self.y_stop = response.text;
            }
            ui.label("Points:");
            let response = self.render_validated_field(ui, &mut self.y_points.clone(), "y_points");
            if response.changed() {
                self.y_points = response.text;
            }
        });
    }

    /// Render a text field with validation error display
    fn render_validated_field(&mut self, ui: &mut egui::Ui, text: &mut String, field_name: &'static str) -> ValidatedFieldResponse {
        let has_error = self.validation_errors.contains_key(field_name);

        // Apply red stroke if validation error
        let mut frame = egui::Frame::NONE;
        if has_error {
            frame = frame.stroke(egui::Stroke::new(1.0, egui::Color32::RED));
        }

        let mut new_text = text.clone();
        let response = frame.show(ui, |ui| {
            ui.add_sized([80.0, 18.0], egui::TextEdit::singleline(&mut new_text))
        });

        // Show tooltip on hover if error
        if has_error {
            if let Some(err) = self.validation_errors.get(field_name) {
                response.response.on_hover_text(err);
            }
        }

        let changed = &new_text != text;
        *text = new_text.clone();

        // Re-validate on change
        if changed {
            self.validate_form();
        }

        ValidatedFieldResponse {
            text: text.clone(),
            changed,
        }
    }

    /// Validate the entire form and populate validation_errors
    fn validate_form(&mut self) {
        self.validation_errors.clear();

        // Validate actuator selection
        match self.scan_mode {
            ScanMode::OneDimensional => {
                if self.selected_actuator.is_none() {
                    self.validation_errors.insert("actuator", "Select an actuator".to_string());
                }
            }
            ScanMode::TwoDimensional => {
                if self.selected_actuator_x.is_none() {
                    self.validation_errors.insert("actuator_x", "Select X axis actuator".to_string());
                }
                if self.selected_actuator_y.is_none() {
                    self.validation_errors.insert("actuator_y", "Select Y axis actuator".to_string());
                }
                // Check for same actuator on both axes
                if let (Some(x), Some(y)) = (&self.selected_actuator_x, &self.selected_actuator_y) {
                    if x == y {
                        self.validation_errors.insert("actuator_y", "X and Y axes must be different".to_string());
                    }
                }
            }
        }

        // Validate detector selection
        if self.selected_detectors.is_empty() {
            self.validation_errors.insert("detectors", "Select at least one detector".to_string());
        }

        // Validate numeric fields based on mode
        match self.scan_mode {
            ScanMode::OneDimensional => {
                self.validate_float_field("start_1d", &self.start_1d.clone());
                self.validate_float_field("stop_1d", &self.stop_1d.clone());
                self.validate_points_field("points_1d", &self.points_1d.clone());

                // Check start != stop
                if let (Ok(start), Ok(stop)) = (self.start_1d.parse::<f64>(), self.stop_1d.parse::<f64>()) {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors.insert("stop_1d", "Stop must differ from Start".to_string());
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
                if let (Ok(start), Ok(stop)) = (self.x_start.parse::<f64>(), self.x_stop.parse::<f64>()) {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors.insert("x_stop", "Stop must differ from Start".to_string());
                    }
                }
                if let (Ok(start), Ok(stop)) = (self.y_start.parse::<f64>(), self.y_stop.parse::<f64>()) {
                    if (start - stop).abs() < f64::EPSILON {
                        self.validation_errors.insert("y_stop", "Stop must differ from Start".to_string());
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
            self.validation_errors.insert(field_name, "Must be a valid number".to_string());
        }
    }

    /// Validate a field as a positive f64
    fn validate_positive_float_field(&mut self, field_name: &'static str, value: &str) {
        match value.parse::<f64>() {
            Ok(v) if v > 0.0 => {}
            Ok(_) => {
                self.validation_errors.insert(field_name, "Must be positive".to_string());
            }
            Err(_) => {
                self.validation_errors.insert(field_name, "Must be a valid number".to_string());
            }
        }
    }

    /// Validate a field as a valid positive integer (points)
    fn validate_points_field(&mut self, field_name: &'static str, value: &str) {
        match value.parse::<u32>() {
            Ok(v) if v > 0 => {}
            Ok(_) => {
                self.validation_errors.insert(field_name, "Must be > 0".to_string());
            }
            Err(_) => {
                self.validation_errors.insert(field_name, "Must be a valid integer".to_string());
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
                if x_points == 0 || y_points == 0 || self.selected_actuator_x.is_none() || self.selected_actuator_y.is_none() {
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
}

/// Helper struct for validated field response
struct ValidatedFieldResponse {
    text: String,
    changed: bool,
}

impl ValidatedFieldResponse {
    fn changed(&self) -> bool {
        self.changed
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
