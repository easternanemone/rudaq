//! Devices panel - list and control hardware devices.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::icons;
use crate::layout;
use crate::widgets::{
    filter_parameters, group_parameters_by_prefix, offline_notice, OfflineContext, ParameterCache,
};

const LAYOUT_CHANGING_PARAMS: &[&str] =
    &["readout.port", "readout.speed_mode", "readout.gain_mode"];

/// Result of an async parameter load operation
struct ParamLoadResult {
    device_id: String,
    params: Vec<ParameterCache>,
    errors: Vec<(String, String)>, // (param_name, error)
}

/// Result of an async parameter set operation
struct ParamSetResult {
    device_id: String,
    param_name: String,
    success: bool,
    actual_value: String,
    error: Option<String>,
}

/// Cached device information
#[derive(Clone)]
struct DeviceCache {
    info: daq_proto::daq::DeviceInfo,
    state: Option<daq_proto::daq::DeviceStateResponse>,
    /// Cached parameters for this device (bd-cdh5.1)
    parameters: Vec<ParameterCache>,
    /// Whether parameters have been loaded
    parameters_loaded: bool,
}

/// Pending action to execute after UI rendering
enum PendingAction {
    Refresh,
    MoveAbsolute {
        device_id: String,
        value: f64,
    },
    MoveRelative {
        device_id: String,
        value: f64,
    },
    ReadValue {
        device_id: String,
    },
    /// Load parameters for a device (bd-cdh5.1)
    LoadParameters {
        device_id: String,
    },
    /// Set a parameter value (bd-cdh5.1)
    SetParameter {
        device_id: String,
        name: String,
        value: String,
    },
    /// Execute a device command (bd-cdh5.4)
    ExecuteCommand {
        device_id: String,
        command: String,
        args: String,
    },
}

/// Registration failure from the daemon
#[derive(Clone)]
struct RegistrationFailureCache {
    #[allow(dead_code)]
    device_id: String,
    device_name: String,
    driver_type: String,
    error: String,
}

/// Result of an async device action
enum DeviceActionResult {
    Refresh(Result<(Vec<DeviceCache>, Vec<RegistrationFailureCache>), String>),
    Move {
        device_id: String,
        success: bool,
        final_position: f64,
        error: Option<String>,
    },
    Read {
        device_id: String,
        success: bool,
        value: f64,
        units: String,
        error: Option<String>,
    },
    Command {
        command: String,
        success: bool,
        error: Option<String>,
    },
}

/// Devices panel state
pub struct DevicesPanel {
    /// Cached device list
    devices: Vec<DeviceCache>,
    /// Devices that failed to register (for debugging visibility)
    registration_failures: Vec<RegistrationFailureCache>,
    /// Selected device ID
    selected_device: Option<String>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Whether initial auto-refresh has been triggered
    initial_refresh_done: bool,
    /// Move target position
    move_target: f64,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Pending action to execute
    pending_action: Option<PendingAction>,
    /// Async action result sender
    action_tx: mpsc::Sender<DeviceActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<DeviceActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
    /// Parameter search filter (bd-cdh5.1)
    param_filter: String,
    /// Parameter edit buffers keyed by (device_id, param_name) (bd-cdh5.1)
    /// Separate from DeviceCache to allow mutation during UI rendering
    param_edit_buffers: std::collections::HashMap<(String, String), String>,
    /// Parameter errors keyed by (device_id, param_name) (bd-cdh5.1)
    param_errors: std::collections::HashMap<(String, String), String>,
    /// Receiver for async parameter load results
    param_load_rx: Option<mpsc::Receiver<ParamLoadResult>>,
    /// Sender for async parameter set results (persistent, cloned per request)
    param_set_tx: mpsc::Sender<ParamSetResult>,
    /// Receiver for async parameter set results
    param_set_rx: mpsc::Receiver<ParamSetResult>,
    /// Device ID currently loading parameters (for UI indicator)
    loading_params_device: Option<String>,
    /// Parameters currently being set (device_id, param_name) for UI indicator
    setting_params: std::collections::HashSet<(String, String)>,
    /// Show advanced parameters (bd-cdh5.4)
    show_advanced: bool,
    /// PP feature editor (bd-cdh5.4)
    pp_editor: crate::widgets::PPEditor,
    /// Smart streaming editor (bd-cdh5.4)
    smart_stream_editor: crate::widgets::SmartStreamEditor,
}

impl Default for DevicesPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        // Persistent channel for parameter set results - sender is cloned per request
        let (param_set_tx, param_set_rx) = mpsc::channel(32);
        Self {
            devices: Vec::new(),
            registration_failures: Vec::new(),
            selected_device: None,
            last_refresh: None,
            initial_refresh_done: false,
            move_target: 0.0,
            error: None,
            status: None,
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            param_filter: String::new(),
            param_edit_buffers: std::collections::HashMap::new(),
            param_errors: std::collections::HashMap::new(),
            param_load_rx: None,
            param_set_tx,
            param_set_rx,
            loading_params_device: None,
            setting_params: std::collections::HashSet::new(),
            show_advanced: false,
            pp_editor: crate::widgets::PPEditor::new(),
            smart_stream_editor: crate::widgets::SmartStreamEditor::new(),
        }
    }
}

fn infer_device_icon(info: &daq_proto::daq::DeviceInfo) -> &'static str {
    if info.is_frame_producer {
        return icons::device::CAMERA;
    }
    if info.is_movable {
        return icons::device::MOTOR;
    }

    let name_lower = info.name.to_lowercase();
    let driver_lower = info.driver_type.to_lowercase();

    if name_lower.contains("camera") || driver_lower.contains("pvcam") {
        icons::device::CAMERA
    } else if name_lower.contains("stage")
        || name_lower.contains("motor")
        || driver_lower.contains("esp")
    {
        icons::device::MOTOR
    } else if name_lower.contains("sensor") || name_lower.contains("meter") || info.is_readable {
        icons::device::SENSOR
    } else if name_lower.contains("laser") || driver_lower.contains("maitai") {
        icons::device::LASER
    } else if name_lower.contains("daq") || driver_lower.contains("comedi") {
        icons::device::DAQ
    } else {
        icons::device::GENERIC
    }
}

impl DevicesPanel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        // Poll device action results
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        DeviceActionResult::Refresh(result) => match result {
                            Ok((devices, failures)) => {
                                self.devices = devices;
                                self.registration_failures = failures;
                                self.last_refresh = Some(std::time::Instant::now());
                                if self.registration_failures.is_empty() {
                                    self.status =
                                        Some(format!("Loaded {} devices", self.devices.len()));
                                } else {
                                    self.status = Some(format!(
                                        "Loaded {} devices ({} failed to register)",
                                        self.devices.len(),
                                        self.registration_failures.len()
                                    ));
                                }
                                self.error = None;
                            }
                            Err(e) => {
                                self.error = Some(e);
                            }
                        },
                        DeviceActionResult::Move {
                            device_id,
                            success,
                            final_position,
                            error,
                        } => {
                            if success {
                                self.status =
                                    Some(format!("Moved {} to {:.4}", device_id, final_position));
                                self.error = None;
                            } else {
                                self.error = error;
                            }
                        }
                        DeviceActionResult::Read {
                            device_id,
                            success,
                            value,
                            units,
                            error,
                        } => {
                            if success {
                                self.status =
                                    Some(format!("{}: {:.4} {}", device_id, value, units));
                                self.error = None;
                            } else {
                                self.error = error;
                            }
                        }
                        DeviceActionResult::Command {
                            command,
                            success,
                            error,
                        } => {
                            if success {
                                self.status =
                                    Some(format!("Command '{}' executed successfully", command));
                                self.error = None;
                            } else {
                                self.error = error;
                            }
                        }
                    }
                    updated = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        // Poll parameter load results
        if let Some(rx) = &mut self.param_load_rx {
            match rx.try_recv() {
                Ok(result) => {
                    // Store any load errors
                    for (param_name, error) in result.errors {
                        let key = (result.device_id.clone(), param_name);
                        self.param_errors
                            .insert(key, format!("Load failed: {}", error));
                    }

                    // Update device cache
                    if let Some(device) = self
                        .devices
                        .iter_mut()
                        .find(|d| d.info.id == result.device_id)
                    {
                        device.parameters = result.params;
                        device.parameters_loaded = true;
                    }

                    self.loading_params_device = None;
                    ctx.request_repaint();
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // Still loading, request repaint to poll again
                    ctx.request_repaint();
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Channel closed, clear loading state
                    self.loading_params_device = None;
                    self.param_load_rx = None;
                }
            }
        }

        // Poll parameter set results (persistent channel - drain all available)
        loop {
            match self.param_set_rx.try_recv() {
                Ok(result) => {
                    let key = (result.device_id.clone(), result.param_name.clone());
                    self.setting_params.remove(&key);

                    if result.success {
                        // Update cached value
                        if let Some(device) = self
                            .devices
                            .iter_mut()
                            .find(|d| d.info.id == result.device_id)
                        {
                            if let Some(param) = device
                                .parameters
                                .iter_mut()
                                .find(|p| p.descriptor.name == result.param_name)
                            {
                                param.update_value(result.actual_value.clone());
                            }
                        }
                        // Update edit buffer
                        let unquoted = result.actual_value.trim_matches('"').to_string();
                        self.param_edit_buffers.insert(key.clone(), unquoted);
                        self.param_errors.remove(&key);
                    } else if let Some(err) = result.error {
                        self.param_errors.insert(key, err);
                    }

                    ctx.request_repaint();
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // No more results available
                    break;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Channel closed (shouldn't happen with persistent channel)
                    self.setting_params.clear();
                    break;
                }
            }
        }

        // Request repaint if we're waiting for parameter set results
        if !self.setting_params.is_empty() {
            ctx.request_repaint();
        }

        if self.action_in_flight > 0 || updated {
            ctx.request_repaint();
        }
    }

    /// Render the devices panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll for completed async operations (non-blocking)
        self.poll_async_results(ui.ctx());

        // Clear pending action
        self.pending_action = None;

        ui.heading("Devices");

        // Show offline notice if not connected (bd-j3xz.4.4)
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        // Auto-refresh on first render when connected
        tracing::debug!(
            initial_refresh_done = self.initial_refresh_done,
            client_connected = client.is_some(),
            action_in_flight = self.action_in_flight,
            "Devices panel: checking auto-refresh conditions"
        );
        if !self.initial_refresh_done && client.is_some() && self.action_in_flight == 0 {
            tracing::info!("Devices panel: triggering auto-refresh on first render");
            self.initial_refresh_done = true;
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

        // Show error/status messages
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        // Show registration failures warning if any devices failed to register
        if !self.registration_failures.is_empty() {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(80, 40, 40))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 180, 100),
                            format!(
                                "⚠ {} device(s) failed to register",
                                self.registration_failures.len()
                            ),
                        );
                    });
                    egui::CollapsingHeader::new("Show details")
                        .default_open(false)
                        .show(ui, |ui| {
                            for failure in &self.registration_failures {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&failure.device_name)
                                            .color(egui::Color32::WHITE)
                                            .strong(),
                                    );
                                    ui.label(
                                        egui::RichText::new(format!("({})", failure.driver_type))
                                            .color(egui::Color32::GRAY),
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(&failure.error)
                                        .color(egui::Color32::from_rgb(255, 150, 150))
                                        .small(),
                                );
                                ui.add_space(4.0);
                            }
                        });
                });
            ui.add_space(4.0);
        }

        // Clone selected device for rendering (avoids borrow issues)
        let selected_device = self
            .selected_device
            .as_ref()
            .and_then(|id| self.devices.iter().find(|d| &d.info.id == id).cloned());

        // Device list and details in two columns
        ui.columns(2, |columns| {
            // Left column: device list
            columns[0].heading("Device List");
            columns[0].separator();

            if self.devices.is_empty() {
                // Empty state with icon (bd-ennc)
                columns[0].vertical_centered(|ui| {
                    ui.add_space(layout::SECTION_SPACING);
                    ui.label(
                        egui::RichText::new(icons::status::INFO).size(layout::ICON_SIZE_LARGE),
                    );
                    ui.add_space(4.0);
                    ui.label("No devices found");
                    ui.weak("Click Refresh to load devices");
                });
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("device_list")
                    .show(&mut columns[0], |ui| {
                        for (idx, device) in self.devices.iter().enumerate() {
                            if idx > 0 {
                                ui.add_space(layout::SECTION_SPACING);
                            }

                            let selected = self.selected_device.as_ref() == Some(&device.info.id);
                            let is_online = device.state.as_ref().map_or(false, |s| s.online);

                            let mut frame = layout::card_frame(ui);
                            if selected {
                                frame =
                                    frame.stroke(egui::Stroke::new(2.0, layout::colors::ACCENT));
                            }

                            let response = frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let icon = infer_device_icon(&device.info);
                                    ui.label(
                                        egui::RichText::new(icon).size(layout::ICON_SIZE_BUTTON),
                                    );

                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            let status_color = if is_online {
                                                layout::colors::CONNECTED
                                            } else {
                                                layout::colors::DISCONNECTED
                                            };
                                            let dot = egui::RichText::new("●").color(status_color);
                                            ui.label(dot);
                                            ui.strong(&device.info.name);
                                        });
                                        ui.weak(&device.info.driver_type);
                                    });
                                });
                            });

                            if response.response.interact(egui::Sense::click()).clicked() {
                                self.selected_device = Some(device.info.id.clone());
                            }
                        }
                    });
            }

            // Right column: device details
            columns[1].heading("Device Details");
            columns[1].separator();

            if let Some(device) = &selected_device {
                self.render_device_details(&mut columns[1], device);
            } else {
                columns[1].label("Select a device to view details");
            }
        });

        // Execute pending action after UI is done borrowing self
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }
    }

    /// Render details for a selected device
    fn render_device_details(&mut self, ui: &mut egui::Ui, device: &DeviceCache) {
        let info = &device.info;

        ui.group(|ui| {
            ui.heading(&info.name);
            ui.label(format!("ID: {}", info.id));
            ui.label(format!("Driver: {}", info.driver_type));

            ui.separator();
            ui.label("Capabilities:");
            ui.horizontal(|ui| {
                if info.is_movable {
                    ui.label("📐 Movable");
                }
                if info.is_readable {
                    ui.label("📖 Readable");
                }
                if info.is_triggerable {
                    ui.label("⚡ Triggerable");
                }
                if info.is_frame_producer {
                    ui.label("📷 Camera");
                }
                if info.is_exposure_controllable {
                    ui.label("⏱ Exposure");
                }
                if info.is_shutter_controllable {
                    ui.label("🚪 Shutter");
                }
                if info.is_wavelength_tunable {
                    ui.label("🌈 Wavelength");
                }
                if info.is_emission_controllable {
                    ui.label("💡 Emission");
                }
            });
        });

        // State display
        if let Some(state) = &device.state {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Current State");
                ui.label(format!(
                    "Online: {}",
                    if state.online { "✅" } else { "❌" }
                ));

                if let Some(pos) = state.position {
                    ui.label(format!("Position: {:.4}", pos));
                }
                if let Some(reading) = state.last_reading {
                    ui.label(format!("Last reading: {:.4}", reading));
                }
                if let Some(armed) = state.armed {
                    ui.label(format!("Armed: {}", armed));
                }
                if let Some(exposure) = state.exposure_ms {
                    ui.label(format!("Exposure: {:.2} ms", exposure));
                }
            });
        }

        // Control section for movable devices
        if info.is_movable {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Motion Control");

                ui.horizontal(|ui| {
                    ui.label("Target:");
                    ui.add(
                        egui::DragValue::new(&mut self.move_target)
                            .speed(0.1)
                            .suffix(" units"),
                    );
                });

                ui.horizontal(|ui| {
                    if ui.button("Move Absolute").clicked() {
                        self.pending_action = Some(PendingAction::MoveAbsolute {
                            device_id: info.id.clone(),
                            value: self.move_target,
                        });
                    }
                    if ui.button("Move Relative").clicked() {
                        self.pending_action = Some(PendingAction::MoveRelative {
                            device_id: info.id.clone(),
                            value: self.move_target,
                        });
                    }
                });

                // Quick move buttons
                ui.horizontal(|ui| {
                    for delta in [-10.0, -1.0, -0.1, 0.1, 1.0, 10.0] {
                        let label = if delta > 0.0 {
                            format!("+{}", delta)
                        } else {
                            format!("{}", delta)
                        };
                        if ui.button(label).clicked() {
                            self.pending_action = Some(PendingAction::MoveRelative {
                                device_id: info.id.clone(),
                                value: delta,
                            });
                        }
                    }
                });
            });
        }

        // Read button for readable devices
        if info.is_readable {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Read Value");
                if ui.button("📖 Read Now").clicked() {
                    self.pending_action = Some(PendingAction::ReadValue {
                        device_id: info.id.clone(),
                    });
                }
            });
        }

        // Properties section (bd-cdh5.1)
        ui.add_space(8.0);
        let header_id = egui::Id::new("properties_header").with(&info.id);
        egui::CollapsingHeader::new("Properties")
            .id_salt(header_id)
            .default_open(false)
            .show(ui, |ui| {
                // Check if loading
                let is_loading = self.loading_params_device.as_ref() == Some(&info.id);

                // Load parameters if not yet loaded
                if !device.parameters_loaded {
                    if is_loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading parameters...");
                        });
                    } else if ui.button("Load Parameters").clicked() {
                        self.pending_action = Some(PendingAction::LoadParameters {
                            device_id: info.id.clone(),
                        });
                    }
                    return;
                }

                if device.parameters.is_empty() {
                    ui.label("No parameters available");
                    return;
                }

                // Search filter and refresh
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut self.param_filter);

                    ui.checkbox(&mut self.show_advanced, "Advanced");

                    if is_loading {
                        ui.spinner();
                        ui.weak("Refreshing...");
                    } else if ui.button("Refresh").clicked() {
                        self.pending_action = Some(PendingAction::LoadParameters {
                            device_id: info.id.clone(),
                        });
                    }
                });

                ui.separator();

                // Render parameters - collect any changes to apply later
                let mut filtered = filter_parameters(&device.parameters, &self.param_filter);

                // If not showing advanced, filter by basic whitelist
                if !self.show_advanced {
                    let basic_whitelist = [
                        "exposure_ms",
                        "gain_mode",
                        "temperature",
                        "binning",
                        "roi",
                        "trigger_mode",
                    ];
                    filtered.retain(|p| {
                        let name = &p.descriptor.name;
                        // Match either exact name or group.name where name is in whitelist
                        basic_whitelist
                            .iter()
                            .any(|&w| name == w || name.ends_with(&format!(".{}", w)))
                    });
                }

                let params_vec: Vec<_> = filtered.iter().cloned().cloned().collect();
                let groups = group_parameters_by_prefix(&params_vec);

                // Use device-scoped ID to prevent state bleed between devices
                egui::ScrollArea::vertical()
                    .id_salt(egui::Id::new("parameters_scroll").with(&info.id))
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (group_name, group_params) in groups {
                            // Device-scoped group ID
                            let group_id = egui::Id::new("param_group")
                                .with(&info.id)
                                .with(&group_name);

                            egui::CollapsingHeader::new(&group_name)
                                .id_salt(group_id)
                                .default_open(true)
                                .show(ui, |ui| {
                                    for param in group_params {
                                        // Uses separate edit buffers to allow mutation during rendering
                                        self.render_single_parameter(ui, &info.id, param);
                                    }
                                });
                        }
                    });
            });

        // Specialized PVCAM Editors (bd-cdh5.4)
        if info.driver_type == "pvcam" {
            ui.add_space(8.0);
            egui::CollapsingHeader::new("✨ PP Features")
                .id_salt(egui::Id::new("pp_header").with(&info.id))
                .show(ui, |ui| {
                    if ui.button("🔄 Reset All to Defaults").clicked() {
                        self.pending_action = Some(PendingAction::ExecuteCommand {
                            device_id: info.id.clone(),
                            command: "reset_pp".into(),
                            args: "{}".into(),
                        });
                    }
                    self.pp_editor.ui(ui, &info.id, &device.parameters);
                });

            ui.add_space(8.0);
            egui::CollapsingHeader::new("🚀 Smart Streaming")
                .id_salt(egui::Id::new("ss_header").with(&info.id))
                .show(ui, |ui| {
                    if self.smart_stream_editor.ui(ui, &info.id) {
                        // Apply button clicked
                        let args = serde_json::json!({
                            "exposures": self.smart_stream_editor.exposures
                        });
                        self.pending_action = Some(PendingAction::ExecuteCommand {
                            device_id: info.id.clone(),
                            command: "upload_smart_stream".into(),
                            args: args.to_string(),
                        });
                    }
                });
        }
    }

    /// Render a single parameter (helper for Properties section)
    /// Uses separate edit buffers to allow mutation during rendering
    fn render_single_parameter(
        &mut self,
        ui: &mut egui::Ui,
        device_id: &str,
        param: &ParameterCache,
    ) {
        let desc = &param.descriptor;
        let buffer_key = (device_id.to_string(), desc.name.clone());

        // Check if this parameter is currently being set
        let is_setting = self.setting_params.contains(&buffer_key);
        if is_setting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&desc.name);
                ui.weak("(updating...)");
            });
            return;
        }

        // Show read-only parameters as labels
        if !desc.writable {
            ui.horizontal(|ui| {
                ui.label(&desc.name);
                ui.label(format!(": {}", param.current_value));
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }
            });
            return;
        }

        // For editable parameters, check for enum values FIRST (takes precedence over dtype)
        if !desc.enum_values.is_empty() {
            let current = param.current_value.trim_matches('"').to_string();
            let mut selected = current.clone();

            ui.horizontal(|ui| {
                ui.label(&desc.name);

                let combo_id = egui::Id::new("param_combo")
                    .with(device_id)
                    .with(&desc.name);

                egui::ComboBox::from_id_salt(combo_id)
                    .selected_text(&selected)
                    .show_ui(ui, |ui| {
                        for option in &desc.enum_values {
                            ui.selectable_value(&mut selected, option.clone(), option);
                        }
                    });

                if selected != current {
                    // Use serde_json for proper escaping
                    let json_value = serde_json::to_string(&selected)
                        .unwrap_or_else(|_| format!("\"{}\"", selected));
                    self.pending_action = Some(PendingAction::SetParameter {
                        device_id: device_id.to_string(),
                        name: desc.name.clone(),
                        value: json_value,
                    });
                }
            });

            // Show error if any
            if let Some(err) = self.param_errors.get(&buffer_key) {
                ui.colored_label(egui::Color32::RED, err);
            }
            return;
        }

        // Render based on dtype
        match desc.dtype.as_str() {
            "bool" => {
                let mut value = param.current_value.parse::<bool>().unwrap_or(false);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut value, &desc.name).changed() {
                        self.pending_action = Some(PendingAction::SetParameter {
                            device_id: device_id.to_string(),
                            name: desc.name.clone(),
                            value: value.to_string(),
                        });
                    }
                });
            }
            "int" => {
                // Use persistent edit buffer for integers
                let edit_buffer = self
                    .param_edit_buffers
                    .entry(buffer_key.clone())
                    .or_insert_with(|| param.current_value.clone());

                let mut value: i64 = edit_buffer.parse().unwrap_or(0);
                let original: i64 = param.current_value.parse().unwrap_or(0);

                ui.horizontal(|ui| {
                    ui.label(&desc.name);
                    let mut drag = egui::DragValue::new(&mut value).speed(1);

                    // Apply constraints if available
                    if let Some(min) = desc.min_value {
                        drag = drag.range(min as i64..=i64::MAX);
                    }
                    if let Some(max) = desc.max_value {
                        drag = drag.range(i64::MIN..=max as i64);
                    }
                    if let (Some(min), Some(max)) = (desc.min_value, desc.max_value) {
                        drag = drag.range(min as i64..=max as i64);
                    }

                    let response = ui.add(drag);

                    if !desc.units.is_empty() {
                        ui.weak(&desc.units);
                    }

                    // Update buffer while editing
                    *self.param_edit_buffers.get_mut(&buffer_key).unwrap() = value.to_string();

                    // Commit on focus lost or drag stopped
                    if (response.lost_focus() || response.drag_stopped()) && value != original {
                        self.pending_action = Some(PendingAction::SetParameter {
                            device_id: device_id.to_string(),
                            name: desc.name.clone(),
                            value: value.to_string(),
                        });
                    }
                });
            }
            "float" => {
                // Use persistent edit buffer for floats
                let edit_buffer = self
                    .param_edit_buffers
                    .entry(buffer_key.clone())
                    .or_insert_with(|| param.current_value.clone());

                let mut value: f64 = edit_buffer.parse().unwrap_or(0.0);
                let original: f64 = param.current_value.parse().unwrap_or(0.0);

                ui.horizontal(|ui| {
                    ui.label(&desc.name);
                    let mut drag = egui::DragValue::new(&mut value).speed(0.01);

                    // Apply constraints if available (handle single-sided bounds)
                    if let (Some(min), Some(max)) = (desc.min_value, desc.max_value) {
                        drag = drag.range(min..=max);
                    } else if let Some(min) = desc.min_value {
                        drag = drag.range(min..=f64::MAX);
                    } else if let Some(max) = desc.max_value {
                        drag = drag.range(f64::MIN..=max);
                    }

                    let response = ui.add(drag);

                    if !desc.units.is_empty() {
                        ui.weak(&desc.units);
                    }

                    // Update buffer while editing
                    *self.param_edit_buffers.get_mut(&buffer_key).unwrap() = value.to_string();

                    // Commit on focus lost or drag stopped
                    if (response.lost_focus() || response.drag_stopped())
                        && (value - original).abs() > f64::EPSILON
                    {
                        self.pending_action = Some(PendingAction::SetParameter {
                            device_id: device_id.to_string(),
                            name: desc.name.clone(),
                            value: value.to_string(),
                        });
                    }
                });
            }
            "string" => {
                // Use persistent edit buffer for strings - this fixes the per-frame recreation bug
                let current_unquoted = param.current_value.trim_matches('"').to_string();
                let edit_buffer = self
                    .param_edit_buffers
                    .entry(buffer_key.clone())
                    .or_insert_with(|| current_unquoted.clone());

                ui.horizontal(|ui| {
                    ui.label(&desc.name);
                    let response = ui.text_edit_singleline(edit_buffer);

                    // Commit on focus lost
                    if response.lost_focus() && *edit_buffer != current_unquoted {
                        // Use serde_json for proper escaping of quotes/backslashes
                        let json_value = serde_json::to_string(&**edit_buffer)
                            .unwrap_or_else(|_| format!("\"{}\"", edit_buffer));
                        self.pending_action = Some(PendingAction::SetParameter {
                            device_id: device_id.to_string(),
                            name: desc.name.clone(),
                            value: json_value,
                        });
                    }
                });
            }
            _ => {
                // Unknown type - show as read-only
                ui.horizontal(|ui| {
                    ui.label(&desc.name);
                    ui.label(format!(": {}", param.current_value));
                });
            }
        }

        // Show error if any (from separate error store)
        if let Some(err) = self.param_errors.get(&buffer_key) {
            ui.colored_label(egui::Color32::RED, err);
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
            PendingAction::Refresh => self.refresh_devices(client, runtime),
            PendingAction::MoveAbsolute { device_id, value } => {
                self.move_device(client, runtime, &device_id, value, false);
            }
            PendingAction::MoveRelative { device_id, value } => {
                self.move_device(client, runtime, &device_id, value, true);
            }
            PendingAction::ReadValue { device_id } => {
                self.read_device(client, runtime, &device_id);
            }
            PendingAction::LoadParameters { device_id } => {
                self.load_parameters(client, runtime, &device_id);
            }
            PendingAction::SetParameter {
                device_id,
                name,
                value,
            } => {
                let _needs_refresh = LAYOUT_CHANGING_PARAMS.contains(&name.as_str());
                self.set_parameter(client, runtime, &device_id, &name, &value);
            }
            PendingAction::ExecuteCommand {
                device_id,
                command,
                args,
            } => {
                self.execute_command(client, runtime, &device_id, &command, &args);
            }
        }
    }

    /// Execute a device command (async, non-blocking)
    fn execute_command(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        command: &str,
        args: &str,
    ) {
        self.error = None;
        self.status = None;

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let device_id = device_id.to_string();
        let command = command.to_string();
        let args = args.to_string();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .execute_device_command(&device_id, &command, &args)
                .await;
            let action = match result {
                Ok(response) => DeviceActionResult::Command {
                    command,
                    success: response.success,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => DeviceActionResult::Command {
                    command,
                    success: false,
                    error: Some(e.to_string()),
                },
            };
            let _ = tx.send(action).await;
        });
    }

    /// Refresh the device list from the daemon
    fn refresh_devices(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.error = None;
        self.status = None;

        // Clear stale edit buffers and errors on refresh
        self.param_edit_buffers.clear();
        self.param_errors.clear();

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        tracing::info!("Refreshing device list from daemon");
        runtime.spawn(async move {
            let result = async {
                let (devices, failures) = client.list_devices_full().await?;
                let device_count = devices.len();
                let failure_count = failures.len();
                tracing::info!(device_count, failure_count, "Discovered devices from daemon");

                if failure_count > 0 {
                    for f in &failures {
                        tracing::warn!(
                            device_id = %f.device_id,
                            device_name = %f.device_name,
                            driver_type = %f.driver_type,
                            error = %f.error,
                            "Device failed to register"
                        );
                    }
                }

                let mut cached = Vec::new();

                for info in devices {
                    tracing::debug!(device_id = %info.id, device_name = %info.name, "Loading device state");
                    let state = client.get_device_state(&info.id).await.ok();
                    cached.push(DeviceCache {
                        info,
                        state,
                        parameters: Vec::new(),
                        parameters_loaded: false,
                    });
                }

                // Convert proto failures to cache
                let failures_cache: Vec<RegistrationFailureCache> = failures
                    .into_iter()
                    .map(|f| RegistrationFailureCache {
                        device_id: f.device_id,
                        device_name: f.device_name,
                        driver_type: f.driver_type,
                        error: f.error,
                    })
                    .collect();

                Ok::<_, anyhow::Error>((cached, failures_cache))
            }
            .await
            .map_err(|e| e.to_string());

            let _ = tx.send(DeviceActionResult::Refresh(result)).await;
        });
    }

    /// Move a device
    fn move_device(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        value: f64,
        relative: bool,
    ) {
        self.error = None;
        self.status = None;

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let device_id = device_id.to_string();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = if relative {
                client.move_relative(&device_id, value).await
            } else {
                client.move_absolute(&device_id, value).await
            };

            let action = match result {
                Ok(response) => DeviceActionResult::Move {
                    device_id,
                    success: response.success,
                    final_position: response.final_position,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => DeviceActionResult::Move {
                    device_id,
                    success: false,
                    final_position: 0.0,
                    error: Some(e.to_string()),
                },
            };
            let _ = tx.send(action).await;
        });
    }

    /// Read value from a device
    fn read_device(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        self.error = None;
        self.status = None;

        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let device_id = device_id.to_string();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.read_value(&device_id).await;
            let action = match result {
                Ok(response) => DeviceActionResult::Read {
                    device_id,
                    success: response.success,
                    value: response.value,
                    units: response.units,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => DeviceActionResult::Read {
                    device_id,
                    success: false,
                    value: 0.0,
                    units: String::new(),
                    error: Some(e.to_string()),
                },
            };
            let _ = tx.send(action).await;
        });
    }

    // =========================================================================
    // Parameter Methods (bd-cdh5.1) - Async with background tasks
    // =========================================================================

    /// Load parameters for a device (async, non-blocking)
    /// Spawns a background task and uses channel to return results
    fn load_parameters(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        // Don't start another load if already loading
        if self.loading_params_device.is_some() {
            return;
        }

        let mut client = client.clone();
        let device_id_str = device_id.to_string();

        // Clear existing edit buffers and errors for this device
        self.param_edit_buffers
            .retain(|(dev_id, _), _| dev_id != device_id);
        self.param_errors
            .retain(|(dev_id, _), _| dev_id != device_id);

        // Set loading state
        self.loading_params_device = Some(device_id_str.clone());

        // Create channel for result
        let (tx, rx) = mpsc::channel(1);
        self.param_load_rx = Some(rx);

        // Spawn async task to load parameters in background
        runtime.spawn(async move {
            let device_id_for_error = device_id_str.clone();

            let result = async {
                let descriptors = client.list_parameters(&device_id_str).await?;

                // Parallel fetch of all parameter values (fixes N+1 pattern)
                let fetch_futures: Vec<_> = descriptors
                    .iter()
                    .map(|desc| {
                        let mut client = client.clone();
                        let device_id = device_id_str.clone();
                        let param_name = desc.name.clone();
                        async move {
                            let value = client.get_parameter(&device_id, &param_name).await;
                            (param_name, value)
                        }
                    })
                    .collect();

                let fetch_results = futures::future::join_all(fetch_futures).await;

                // Combine descriptors with fetched values
                let mut params = Vec::new();
                let mut load_errors = Vec::new();

                for (desc, (param_name, value_result)) in descriptors.into_iter().zip(fetch_results)
                {
                    match value_result {
                        Ok(v) => {
                            params.push(ParameterCache::new(desc, v.value));
                        }
                        Err(e) => {
                            load_errors.push((param_name, e.to_string()));
                            params.push(ParameterCache::new(desc, String::new()));
                        }
                    }
                }

                Ok::<_, anyhow::Error>(ParamLoadResult {
                    device_id: device_id_str,
                    params,
                    errors: load_errors,
                })
            }
            .await;

            match result {
                Ok(load_result) => {
                    let _ = tx.send(load_result).await;
                }
                Err(e) => {
                    // Send empty result with error (device_id needed for cleanup)
                    let _ = tx
                        .send(ParamLoadResult {
                            device_id: device_id_for_error,
                            params: Vec::new(),
                            errors: vec![("_load".to_string(), e.to_string())],
                        })
                        .await;
                }
            }
        });
    }

    /// Set a parameter value (async, non-blocking)
    /// Spawns a background task and uses channel to return result
    fn set_parameter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        name: &str,
        value: &str,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let device_id_str = device_id.to_string();
        let name_str = name.to_string();
        let value_str = value.to_string();
        let buffer_key = (device_id_str.clone(), name_str.clone());

        // Clear any previous error for this parameter
        self.param_errors.remove(&buffer_key);

        // Mark as setting
        self.setting_params.insert(buffer_key);

        // Clone the persistent sender - this preserves all in-flight responses
        let tx = self.param_set_tx.clone();

        // Spawn async task
        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id_str, &name_str, &value_str)
                .await;

            let set_result = match result {
                Ok(response) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: response.success,
                    actual_value: response.actual_value,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: false,
                    actual_value: String::new(),
                    error: Some(e.to_string()),
                },
            };

            let _ = tx.send(set_result).await;
        });
    }
}
