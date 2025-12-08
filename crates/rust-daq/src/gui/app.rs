use eframe::{egui, App, Frame};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use egui_dock::{DockArea, DockState, NodeIndex, Style};
use egui_plot::{Line, Plot, PlotImage, PlotPoint as EguiPlotPoint, PlotPoints};

use crate::gui::{
    create_channels, parameter_widget, spawn_backend, BackendCommand, BackendEvent,
    ConnectionStatus, DeviceInfo, ParameterDescriptor, ParameterEditState, UiChannels,
    WidgetResult,
};

/// Device row for display in the UI.
#[derive(Clone, Default)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub driver_type: String,
    pub capabilities: Vec<String>,
    pub last_value: Option<f64>,
    pub last_units: String,
    pub last_updated: Option<Instant>,
    pub error: Option<String>,
    /// Device state fields from streaming
    pub state_fields: std::collections::HashMap<String, String>,
    /// Parameter descriptors for dynamic control panels
    pub parameters: Vec<ParameterDescriptor>,
}

impl From<DeviceInfo> for DeviceRow {
    fn from(info: DeviceInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            driver_type: info.driver_type,
            capabilities: info.capabilities,
            last_value: None,
            last_units: String::new(),
            last_updated: None,
            error: None,
            state_fields: std::collections::HashMap::new(),
            parameters: Vec::new(),
        }
    }
}

/// Identifiers for dockable tabs
#[derive(Clone, Debug)]
pub enum Tab {
    DeviceList,
    DeviceDetails,
    Plot,
    Image,
    Log,
}

/// Main GUI application state.
pub struct DaqGuiApp {
    /// Address input for daemon connection
    pub daemon_addr: String,
    /// Current connection status
    pub connection_status: ConnectionStatus,
    /// Status message for display
    pub status_line: String,
    /// List of devices
    pub devices: Vec<DeviceRow>,
    /// Channels for backend communication
    pub channels: UiChannels,
    /// Last UI update time for starvation detection
    pub last_update: Instant,
    /// Backend thread handle (kept alive)
    #[cfg(not(target_arch = "wasm32"))]
    pub _backend_handle: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_arch = "wasm32")]
    pub _backend_handle: Option<()>,
    /// Whether state streaming is active
    pub is_streaming: bool,
    /// Currently selected device ID for detail panel
    pub selected_device_id: Option<String>,
    /// Parameter edit states for immediate-mode widgets
    pub param_edit_states: std::collections::HashMap<String, ParameterEditState>,

    /// Docking state
    pub dock_state: DockState<Tab>,

    /// Data history for plotting. Map device_id -> Vec of [time, value]
    pub history: std::collections::HashMap<String, Vec<[f64; 2]>>,
    /// Application start time for relative plotting
    pub start_time: Instant,
    /// Latest image data and texture for each device
    pub images: std::collections::HashMap<String, (egui::ColorImage, Option<egui::TextureHandle>)>,
}

impl DaqGuiApp {
    pub fn new() -> Self {
        // Create channels and spawn backend
        let (channels, backend_handle) = create_channels();
        let backend_thread = spawn_backend(backend_handle);

        Self::init(channels, Some(backend_thread))
    }

    /// Create a new instance with provided channels (for testing)
    pub fn new_with_channels(channels: UiChannels) -> Self {
        Self::init(channels, None)
    }

    fn init(
        channels: UiChannels,
        #[cfg(not(target_arch = "wasm32"))] backend_thread: Option<std::thread::JoinHandle<()>>,
        #[cfg(target_arch = "wasm32")] backend_thread: Option<()>,
    ) -> Self {
        // Create default dock layout
        let mut dock_state = DockState::new(vec![Tab::DeviceList]);

        {
            let surface = dock_state.main_surface_mut();

            // Split: Left (Devices 25%), Center (Plot 50%), Right (Details 25%)
            let root = NodeIndex::root();
            let [_left, rest] = surface.split_left(root, 0.25, vec![Tab::DeviceList]);
            let [center, right] = surface.split_left(rest, 0.66, vec![Tab::Plot]);
            let [_center, _right] = surface.split_below(right, 0.5, vec![Tab::DeviceDetails]);

            // Add Image tab stacked with Plot
            surface.split_right(center, 0.5, vec![Tab::Image]);

            // Add Log at bottom of center
            surface.split_below(center, 0.75, vec![Tab::Log]);
        }

        Self {
            daemon_addr: "127.0.0.1:50051".to_string(),
            connection_status: ConnectionStatus::Disconnected,
            status_line: String::from("Not connected. Enter daemon address and click Connect."),
            devices: Vec::new(),
            channels,
            last_update: Instant::now(),
            _backend_handle: backend_thread,
            is_streaming: false,
            selected_device_id: None,
            param_edit_states: std::collections::HashMap::new(),
            dock_state,
            history: std::collections::HashMap::new(),
            start_time: Instant::now(),
            images: std::collections::HashMap::new(),
        }
    }

    /// Process all pending events from the backend.
    pub fn process_backend_events(&mut self) {
        for event in self.channels.drain_events() {
            match event {
                BackendEvent::DevicesRefreshed { devices } => {
                    self.devices = devices.into_iter().map(DeviceRow::from).collect();
                    self.status_line = format!("Loaded {} devices", self.devices.len());

                    // Auto-start state streaming after devices are loaded
                    if !self.is_streaming {
                        self.channels
                            .send_command(BackendCommand::StartStateStream {
                                device_ids: vec![], // Subscribe to all devices
                            });
                    }
                }
                BackendEvent::ValueRead {
                    device_id,
                    value,
                    units,
                } => {
                    if let Some(row) = self.devices.iter_mut().find(|d| d.id == device_id) {
                        row.last_value = Some(value);
                        row.last_units = units;
                        row.last_updated = Some(Instant::now());
                        row.error = None;
                    }

                    // Update history
                    let t = Instant::now().duration_since(self.start_time).as_secs_f64();
                    let entry = self.history.entry(device_id).or_default();
                    entry.push([t, value]);
                    // Limit history size
                    if entry.len() > 1000 {
                        entry.remove(0);
                    }
                }
                BackendEvent::DeviceStateUpdated { .. } => {
                    // Legacy: ignore if received (state is now via watch channel)
                }
                BackendEvent::StateStreamStarted => {
                    self.is_streaming = true;
                    self.status_line = format!("{} (streaming)", self.status_line);
                }
                BackendEvent::StateStreamStopped => {
                    self.is_streaming = false;
                }
                BackendEvent::ParametersFetched {
                    device_id,
                    parameters,
                } => {
                    if let Some(row) = self.devices.iter_mut().find(|d| d.id == device_id) {
                        row.parameters = parameters;
                    }
                }
                BackendEvent::Error { message } => {
                    self.status_line = format!("Error: {}", message);
                }
                BackendEvent::ConnectionChanged { status } => {
                    self.connection_status = status.clone();
                    self.status_line = match &status {
                        ConnectionStatus::Disconnected => "Disconnected".to_string(),
                        ConnectionStatus::Connecting => "Connecting...".to_string(),
                        ConnectionStatus::Connected => "Connected".to_string(),
                        ConnectionStatus::Reconnecting { attempt } => {
                            format!("Reconnecting (attempt {})...", attempt)
                        }
                        ConnectionStatus::Failed { reason } => {
                            format!("Connection failed: {}", reason)
                        }
                    };

                    // Reset streaming state on disconnect
                    if matches!(
                        status,
                        ConnectionStatus::Disconnected | ConnectionStatus::Failed { .. }
                    ) {
                        self.is_streaming = false;
                    }
                }
                BackendEvent::ImageReceived {
                    device_id,
                    size,
                    data,
                } => {
                    let [w, h] = size;
                    if w * h > 0 {
                        // Determine format based on data length
                        let image = if data.len() == w * h {
                            // Grayscale
                            egui::ColorImage::from_gray([w, h], &data)
                        } else if data.len() == w * h * 3 {
                            // RGB
                            egui::ColorImage::from_rgb([w, h], &data)
                        } else if data.len() == w * h * 4 {
                            // RGBA
                            egui::ColorImage::from_rgba_unmultiplied([w, h], &data)
                        } else {
                            tracing::warn!("Invalid image data size for dimensions {}x{}", w, h);
                            return;
                        };

                        // Store image and invalidate texture (set to None)
                        self.images.insert(device_id, (image, None));
                    }
                }
            }
        }
    }

    /// Sync device state from watch channel to device rows.
    /// This pulls latest state from the watch channel (never blocks).
    pub fn sync_device_state(&mut self) {
        let snapshot = self.channels.get_state();
        for row in &mut self.devices {
            if let Some(device_state) = snapshot.devices.get(&row.id) {
                // Update state fields
                row.state_fields = device_state.fields.clone();
                if let Some(updated_at) = device_state.updated_at {
                    row.last_updated = Some(updated_at);
                }

                // Extract position if available for movable devices
                if let Some(pos_str) = row.state_fields.get("position") {
                    if let Ok(pos) = pos_str.parse::<f64>() {
                        row.last_value = Some(pos);
                        row.last_units = "pos".to_string();
                    }
                }
            }
        }
    }

    /// Check for UI starvation (frame time > 50ms).
    fn check_starvation(&mut self) {
        let elapsed = self.last_update.elapsed();
        if elapsed.as_millis() > 50 {
            tracing::warn!("UI starvation detected: {:?} since last update", elapsed);
        }
        self.last_update = Instant::now();
    }

    /// Main UI rendering logic (independent of eframe::Frame).
    pub fn ui(&mut self, ctx: &egui::Context) {
        // Check for UI starvation
        self.check_starvation();

        // Process backend events (non-blocking)
        self.process_backend_events();

        // Sync device state from watch channel (non-blocking, always latest)
        self.sync_device_state();

        // Top panel with connection controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("rust-daq Control Panel");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Daemon:");
                let addr_response = ui.text_edit_singleline(&mut self.daemon_addr);

                let is_connected = matches!(self.connection_status, ConnectionStatus::Connected);
                let is_connecting = matches!(
                    self.connection_status,
                    ConnectionStatus::Connecting | ConnectionStatus::Reconnecting { .. }
                );

                if is_connected {
                    if ui.button("Disconnect").clicked() {
                        self.channels.send_command(BackendCommand::Disconnect);
                    }
                    if ui.button("Refresh").clicked() {
                        self.channels.send_command(BackendCommand::RefreshDevices);
                        self.status_line = "Refreshing devices...".to_string();
                    }
                } else if is_connecting {
                    ui.add_enabled(false, egui::Button::new("Connecting..."));
                } else {
                    if ui.button("Connect").clicked()
                        || (addr_response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        let address = self.daemon_addr.clone();
                        self.channels
                            .send_command(BackendCommand::Connect { address });
                    }
                }

                // Connection status indicator
                let status_color = match &self.connection_status {
                    ConnectionStatus::Connected => egui::Color32::GREEN,
                    ConnectionStatus::Connecting | ConnectionStatus::Reconnecting { .. } => {
                        egui::Color32::YELLOW
                    }
                    ConnectionStatus::Disconnected => egui::Color32::GRAY,
                    ConnectionStatus::Failed { .. } => egui::Color32::RED,
                };
                ui.colored_label(status_color, "●");
            });

            ui.label(&self.status_line);
        });

        // Bottom panel with metrics
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            let metrics = self.channels.get_metrics();
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Frames dropped: {} | Stream restarts: {}",
                    metrics.frames_dropped, metrics.stream_restarts
                ));
            });
        });

        // Tab viewer logic
        struct DaqTabViewer<'a> {
            app: &'a mut DaqGuiApp,
        }

        impl<'a> egui_dock::TabViewer for DaqTabViewer<'a> {
            type Tab = Tab;

            fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
                match tab {
                    Tab::DeviceList => "Devices".into(),
                    Tab::DeviceDetails => "Details".into(),
                    Tab::Plot => "Plot".into(),
                    Tab::Image => "Camera".into(),
                    Tab::Log => "Sys Log".into(),
                }
            }

            fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
                match tab {
                    Tab::DeviceList => self.app.ui_device_list(ui),
                    Tab::DeviceDetails => self.app.ui_device_details(ui),
                    Tab::Plot => self.app.ui_plot(ui),
                    Tab::Image => self.app.ui_image(ui),
                    Tab::Log => self.app.ui_log(ui),
                }
            }
        }

        // Render DockArea
        let mut viewer = DaqTabViewer { app: self };
        let mut dock_state = std::mem::replace(&mut viewer.app.dock_state, DockState::new(vec![]));

        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);

        viewer.app.dock_state = dock_state;

        // Request repaint at ~30fps for smooth updates
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn ui_device_list(&mut self, ui: &mut egui::Ui) {
        if !matches!(self.connection_status, ConnectionStatus::Connected) {
            ui.centered_and_justified(|ui| {
                ui.label("Connect to see devices");
            });
            return;
        }

        if self.devices.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No devices found.");
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("device_grid")
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Value");
                    ui.end_row();

                    let device_data: Vec<_> = self
                        .devices
                        .iter()
                        .map(|d| {
                            (
                                d.id.clone(),
                                d.name.clone(),
                                d.last_value,
                                d.last_units.clone(),
                                d.last_updated,
                            )
                        })
                        .collect();

                    for (id, name, last_value, last_units, last_updated) in device_data {
                        let is_selected = self.selected_device_id.as_ref() == Some(&id);

                        if is_selected {
                            ui.visuals_mut().override_text_color = Some(egui::Color32::LIGHT_BLUE);
                        }

                        // Selectable label for the name
                        if ui.selectable_label(is_selected, &name).clicked() {
                            if is_selected {
                                self.selected_device_id = None;
                            } else {
                                self.selected_device_id = Some(id.clone());
                                self.channels.send_command(BackendCommand::FetchParameters {
                                    device_id: id.clone(),
                                });
                            }
                        }

                        // Value
                        if let Some(v) = last_value {
                            let age = last_updated.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                            let color = if age < 5 {
                                egui::Color32::GREEN
                            } else {
                                egui::Color32::GRAY
                            };
                            ui.colored_label(color, format!("{:.4} {}", v, last_units));
                        } else {
                            ui.label("-");
                        }

                        // Reset color
                        if is_selected {
                            ui.visuals_mut().override_text_color = None;
                        }

                        ui.end_row();
                    }
                });
        });
    }

    fn ui_device_details(&mut self, ui: &mut egui::Ui) {
        let selected_id = match &self.selected_device_id {
            Some(id) => id.clone(),
            None => {
                ui.centered_and_justified(|ui| ui.label("No device selected"));
                return;
            }
        };

        let device_data = self.devices.iter().find(|d| d.id == selected_id).map(|d| {
            (
                d.name.clone(),
                d.driver_type.clone(),
                d.capabilities.clone(),
                d.parameters.clone(),
                d.state_fields.clone(),
            )
        });

        if let Some((name, driver_type, capabilities, parameters, state_fields)) = device_data {
            ui.heading(&name);
            ui.horizontal(|ui| {
                ui.label(format!("Type: {}", driver_type));
                ui.separator();
                ui.label(format!("Caps: {}", capabilities.join(",")));
            });
            ui.separator();

            // Parameters
            if parameters.is_empty() {
                ui.label("No parameters.");
                if ui.button("Fetch").clicked() {
                    self.channels.send_command(BackendCommand::FetchParameters {
                        device_id: selected_id.clone(),
                    });
                }
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("param_grid_details")
                        .striped(true)
                        .num_columns(3)
                        .show(ui, |ui| {
                            for param in &parameters {
                                ui.label(&param.name).on_hover_text(&param.description);

                                let state_key = format!("{}:{}", param.device_id, param.name);
                                let state = self.param_edit_states.entry(state_key).or_default();

                                match parameter_widget(ui, param, state) {
                                    WidgetResult::Committed(value) => {
                                        self.channels.send_command(BackendCommand::SetParameter {
                                            device_id: param.device_id.clone(),
                                            name: param.name.clone(),
                                            value,
                                        });
                                        state.reset();
                                    }
                                    _ => {}
                                }

                                if !param.writable {
                                    ui.label("🔒");
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });
                });
            }

            // State
            if !state_fields.is_empty() {
                ui.separator();
                ui.heading("State");
                egui::Grid::new("state_grid_details")
                    .striped(true)
                    .show(ui, |ui| {
                        for (k, v) in &state_fields {
                            ui.label(k);
                            ui.label(v);
                            ui.end_row();
                        }
                    });
            }
        } else {
            ui.label("Device not found.");
        }
    }

    fn ui_plot(&mut self, ui: &mut egui::Ui) {
        let selected_id = match &self.selected_device_id {
            Some(id) => id,
            None => {
                ui.centered_and_justified(|ui| ui.label("Select a device to plot"));
                return;
            }
        };

        if let Some(data) = self.history.get(selected_id) {
            let line = Line::new(PlotPoints::from(data.clone()));
            Plot::new("device_plot")
                .view_aspect(2.0)
                .show(ui, |plot_ui| plot_ui.line(line));
        } else {
            ui.centered_and_justified(|ui| ui.label("No data for selected device"));
        }
    }

    fn ui_log(&mut self, ui: &mut egui::Ui) {
        ui.label(&self.status_line);
        // Could add a scroll area with history here
    }

    fn ui_image(&mut self, ui: &mut egui::Ui) {
        let selected_id = match &self.selected_device_id {
            Some(id) => id,
            None => {
                ui.centered_and_justified(|ui| ui.label("Select a camera to view"));
                return;
            }
        };

        if let Some((image, texture_opt)) = self.images.get_mut(selected_id) {
            // Load texture if needed
            let texture = texture_opt.get_or_insert_with(|| {
                ui.ctx().load_texture(
                    format!("img_{}", selected_id),
                    image.clone(),
                    egui::TextureOptions::NEAREST,
                )
            });

            let size = texture.size_vec2();
            Plot::new("camera_image")
                .view_aspect(1.0)
                .data_aspect(1.0)
                .show(ui, |plot_ui| {
                    plot_ui.image(PlotImage::new(
                        texture,
                        egui_plot::PlotPoint::new(0.0, 0.0),
                        size,
                    ))
                });
        } else {
            ui.centered_and_justified(|ui| ui.label("No image signal"));
        }
    }
}

impl App for DaqGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.ui(ctx);
    }
}
