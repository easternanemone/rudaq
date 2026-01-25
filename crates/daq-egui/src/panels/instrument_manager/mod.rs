//! Instrument Manager Panel - hierarchical device tree view
//!
//! Displays registered hardware grouped by type (Cameras, Stages, Detectors, etc.)
//! with expandable nodes showing device state and quick actions.

mod dispatch;
mod types;

// Note: dispatch module contains PanelType and determine_panel_type for future panel routing
// Currently the panel selection logic is inline in render_device_control_panel
pub use types::{DeviceCategory, DeviceGroup, ParameterInfo, PopOutRequest};

use eframe::egui;
use egui_extras::{Size, StripBuilder};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::{
    offline_notice, DeviceControlWidget, MaiTaiControlPanel, OfflineContext,
    PowerMeterControlPanel, RotatorControlPanel, StageControlPanel,
};
use daq_proto::daq::DeviceInfo;

/// Timeout for individual device state fetch (prevents stalls from hung devices)
const DEVICE_STATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Maximum concurrent device state requests (prevents overwhelming the daemon)
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Device state information
#[derive(Debug, Clone, Default)]
struct DeviceState {
    position: Option<f64>,
    reading: Option<f64>,
    armed: Option<bool>,
    streaming: Option<bool>,
    exposure_ms: Option<f64>,
    online: bool,
}

/// Async action results from background device operations
enum ActionResult {
    Refresh(Result<Vec<DeviceInfo>, String>),
    GetDeviceState {
        device_id: String,
        result: Result<DeviceState, String>,
    },
    TestConnection {
        _device_id: String,
        device_name: String,
        result: Result<bool, String>,
    },
    ListParameters {
        device_id: String,
        device_name: String,
        result: Result<Vec<ParameterInfo>, String>,
    },
    SetParameter {
        _device_id: String,
        param_name: String,
        result: Result<String, String>,
    },
    // Device control actions
    MoveDevice {
        device_id: String,
        result: Result<(), String>,
    },
    ReadDevice {
        device_id: String,
        result: Result<f64, String>,
    },
    StartStream {
        device_id: String,
        result: Result<(), String>,
    },
    StopStream {
        device_id: String,
        result: Result<(), String>,
    },
}

/// Instrument Manager Panel state
pub struct InstrumentManagerPanel {
    /// Devices grouped by category
    groups: Vec<DeviceGroup>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
    /// Whether initial auto-refresh has been triggered
    initial_refresh_done: bool,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Selected device ID
    selected_device: Option<String>,
    /// Device state cache
    device_states: HashMap<String, DeviceState>,
    /// Async action channel
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
    action_in_flight: usize,
    /// Parameter viewer state
    params_viewer_open: bool,
    params_viewer_device_id: Option<String>,
    params_viewer_device_name: Option<String>,
    params_viewer_params: Vec<ParameterInfo>,
    params_viewer_loading: bool,
    params_viewer_error: Option<String>,
    /// Parameter edit state (for Configure dialog)
    param_edit_values: HashMap<String, String>,
    /// Pending context menu action (device_id, device_name, action_type)
    pending_action: Option<(String, String, ContextAction)>,

    // Control panel state
    /// Move target position input (keyed by device_id)
    move_target: HashMap<String, String>,
    /// Relative move amount input (keyed by device_id)
    jog_amount: HashMap<String, String>,
    /// Exposure input (ms) for cameras
    exposure_input: HashMap<String, String>,
    /// Last read value (keyed by device_id)
    last_reading: HashMap<String, (f64, std::time::Instant)>,
    /// Operation in progress (keyed by device_id)
    operation_pending: HashMap<String, String>,

    // Device-specific control panels (keyed by device_id)
    /// MaiTai laser control panels
    maitai_panels: HashMap<String, MaiTaiControlPanel>,
    /// Power meter control panels
    power_meter_panels: HashMap<String, PowerMeterControlPanel>,
    /// Rotator control panels
    rotator_panels: HashMap<String, RotatorControlPanel>,
    /// Stage control panels
    stage_panels: HashMap<String, StageControlPanel>,

    /// Pending pop-out request containing full device info
    /// Checked by DaqApp after each ui() call
    pending_pop_out: Option<DeviceInfo>,
}

/// Context menu actions
#[derive(Clone, Debug)]
enum ContextAction {
    TestConnection,
    ViewParameters,
    Configure,
}

/// Control panel actions (collected during UI render, executed after)
#[derive(Clone, Debug)]
enum ControlAction {
    MoveAbs(String, f64),     // device_id, position
    MoveRel(String, f64),     // device_id, delta
    Read(String),             // device_id
    StartStream(String),      // device_id
    StopStream(String),       // device_id
    SetExposure(String, f64), // device_id, exposure_ms
    RefreshState(String),     // device_id
}

impl Default for InstrumentManagerPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            groups: Vec::new(),
            last_refresh: None,
            initial_refresh_done: false,
            error: None,
            status: None,
            selected_device: None,
            device_states: HashMap::new(),
            action_tx,
            action_rx,
            action_in_flight: 0,
            params_viewer_open: false,
            params_viewer_device_id: None,
            params_viewer_device_name: None,
            params_viewer_params: Vec::new(),
            params_viewer_loading: false,
            params_viewer_error: None,
            param_edit_values: HashMap::new(),
            pending_action: None,
            // Control panel state
            move_target: HashMap::new(),
            jog_amount: HashMap::new(),
            exposure_input: HashMap::new(),
            last_reading: HashMap::new(),
            operation_pending: HashMap::new(),
            // Device-specific control panels
            maitai_panels: HashMap::new(),
            power_meter_panels: HashMap::new(),
            rotator_panels: HashMap::new(),
            stage_panels: HashMap::new(),
            pending_pop_out: None,
        }
    }
}

impl InstrumentManagerPanel {
    /// Take a pending pop-out request (if any).
    /// Called by DaqApp after each ui() call to handle pop-out actions.
    pub fn take_pop_out_request(&mut self) -> Option<PopOutRequest> {
        self.pending_pop_out
            .take()
            .map(|device_info| PopOutRequest { device_info })
    }

    /// Reset the refresh state to trigger a new auto-refresh.
    /// Called when the connection is re-established after a disconnect.
    pub fn reset_refresh_state(&mut self) {
        tracing::info!("InstrumentManagerPanel: resetting refresh state for reconnect");
        self.initial_refresh_done = false;
        self.groups.clear();
        self.device_states.clear();
        self.error = None;
        self.status = None;
    }

    /// Poll for async results
    fn poll_async_results(
        &mut self,
        ctx: &egui::Context,
        _client: Option<&mut DaqClient>,
        _runtime: &Runtime,
    ) -> bool {
        let mut updated = false;
        let mut should_fetch_device_states = false;

        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::Refresh(result) => match result {
                            Ok(devices) => {
                                let device_count = devices.len();
                                tracing::info!(
                                    device_count,
                                    "InstrumentManagerPanel: refresh succeeded, received devices"
                                );
                                self.update_groups(devices);
                                self.last_refresh = Some(std::time::Instant::now());
                                self.status = Some(format!(
                                    "Loaded {} devices",
                                    self.groups.iter().map(|g| g.devices.len()).sum::<usize>()
                                ));
                                self.error = None;
                                should_fetch_device_states = true;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "InstrumentManagerPanel: refresh failed"
                                );
                                self.error = Some(e);
                            }
                        },
                        ActionResult::GetDeviceState { device_id, result } => match result {
                            Ok(state) => {
                                self.device_states.insert(device_id, state);
                            }
                            Err(_e) => {
                                // Silently fail device state updates - not critical
                                // Device might be offline or temporarily unavailable
                            }
                        },
                        ActionResult::TestConnection {
                            device_name,
                            result,
                            ..
                        } => match result {
                            Ok(online) => {
                                if online {
                                    self.status =
                                        Some(format!("{}: Connection successful", device_name));
                                } else {
                                    self.error =
                                        Some(format!("{}: Device is offline", device_name));
                                }
                            }
                            Err(e) => {
                                self.error =
                                    Some(format!("{}: Connection failed - {}", device_name, e));
                            }
                        },
                        ActionResult::ListParameters {
                            device_id,
                            device_name,
                            result,
                        } => {
                            self.params_viewer_loading = false;
                            match result {
                                Ok(params) => {
                                    self.params_viewer_device_id = Some(device_id);
                                    self.params_viewer_device_name = Some(device_name);
                                    self.params_viewer_params = params.clone();
                                    self.params_viewer_open = true;
                                    self.params_viewer_error = None;
                                    // Initialize edit values from current values
                                    self.param_edit_values.clear();
                                    for p in &params {
                                        if let Some(ref val) = p.current_value {
                                            self.param_edit_values
                                                .insert(p.name.clone(), val.clone());
                                        }
                                    }
                                }
                                Err(e) => {
                                    self.params_viewer_error = Some(e);
                                }
                            }
                        }
                        ActionResult::SetParameter {
                            param_name, result, ..
                        } => match result {
                            Ok(actual_value) => {
                                self.status = Some(format!(
                                    "Set {} = {} successfully",
                                    param_name, actual_value
                                ));
                                // Update the cached value
                                if let Some(p) = self
                                    .params_viewer_params
                                    .iter_mut()
                                    .find(|p| p.name == param_name)
                                {
                                    p.current_value = Some(actual_value.clone());
                                }
                                self.param_edit_values.insert(param_name, actual_value);
                            }
                            Err(e) => {
                                self.error = Some(format!("Failed to set {}: {}", param_name, e));
                            }
                        },
                        // Device control action results
                        ActionResult::MoveDevice { device_id, result } => {
                            self.operation_pending.remove(&device_id);
                            match result {
                                Ok(()) => {
                                    self.status = Some("Move completed".to_string());
                                    // Refresh device state after move
                                    should_fetch_device_states = true;
                                }
                                Err(e) => {
                                    self.error = Some(format!("Move failed: {}", e));
                                }
                            }
                        }
                        ActionResult::ReadDevice { device_id, result } => {
                            self.operation_pending.remove(&device_id);
                            match result {
                                Ok(value) => {
                                    self.last_reading
                                        .insert(device_id, (value, std::time::Instant::now()));
                                    self.status = Some(format!("Read: {:.4}", value));
                                }
                                Err(e) => {
                                    self.error = Some(format!("Read failed: {}", e));
                                }
                            }
                        }
                        ActionResult::StartStream { device_id, result } => {
                            self.operation_pending.remove(&device_id);
                            match result {
                                Ok(()) => {
                                    self.status = Some("Streaming started".to_string());
                                    should_fetch_device_states = true;
                                }
                                Err(e) => {
                                    self.error = Some(format!("Failed to start stream: {}", e));
                                }
                            }
                        }
                        ActionResult::StopStream { device_id, result } => {
                            self.operation_pending.remove(&device_id);
                            match result {
                                Ok(()) => {
                                    self.status = Some("Streaming stopped".to_string());
                                    should_fetch_device_states = true;
                                }
                                Err(e) => {
                                    self.error = Some(format!("Failed to stop stream: {}", e));
                                }
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

        should_fetch_device_states
    }

    /// Update device groups from flat list
    fn update_groups(&mut self, devices: Vec<DeviceInfo>) {
        let mut by_category: HashMap<DeviceCategory, Vec<DeviceInfo>> = HashMap::new();

        for device in devices {
            let category = DeviceCategory::from_device_info(&device);
            by_category.entry(category).or_default().push(device);
        }

        // Preserve expansion state
        let old_expanded: HashMap<DeviceCategory, bool> = self
            .groups
            .iter()
            .map(|g| (g.category, g.expanded))
            .collect();

        self.groups = by_category
            .into_iter()
            .map(|(category, devices)| DeviceGroup {
                category,
                devices,
                expanded: old_expanded.get(&category).copied().unwrap_or(true),
            })
            .collect();

        // Sort by category order
        self.groups.sort_by_key(|g| match g.category {
            DeviceCategory::Camera => 0,
            DeviceCategory::Stage => 1,
            DeviceCategory::Detector => 2,
            DeviceCategory::Laser => 3,
            DeviceCategory::PowerMeter => 4,
            DeviceCategory::Other => 5,
        });
    }

    /// Refresh device states for all known devices
    ///
    /// Uses bounded concurrency and per-device timeouts to prevent:
    /// - Stalls from hung devices blocking auto-refresh
    /// - Overwhelming the daemon with too many concurrent requests
    /// - action_in_flight counter getting stuck
    fn refresh_device_states(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        let Some(client) = client else {
            tracing::debug!("refresh_device_states: no client");
            return;
        };

        // Collect all device IDs first
        let device_ids: Vec<String> = self
            .groups
            .iter()
            .flat_map(|g| g.devices.iter().map(|d| d.id.clone()))
            .collect();

        tracing::info!(
            device_count = device_ids.len(),
            "refresh_device_states: fetching states for devices"
        );

        if device_ids.is_empty() {
            tracing::debug!("refresh_device_states: no devices to query");
            return;
        }

        // Track total requests for this batch
        let batch_size = device_ids.len();
        self.action_in_flight = self.action_in_flight.saturating_add(batch_size);

        let client = client.clone();
        let tx = self.action_tx.clone();

        // Create semaphore for concurrency limiting
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_REQUESTS));

        // Spawn all requests with bounded concurrency and timeouts
        // Uses catch_unwind to ensure action_in_flight is always decremented (bd-tjwm.5)
        for device_id in device_ids {
            let mut client = client.clone();
            let tx = tx.clone();
            let semaphore = semaphore.clone();

            runtime.spawn(async move {
                use futures::FutureExt;

                let device_id_for_panic = device_id.clone();

                // Wrap the work in catch_unwind to handle panics
                let work = std::panic::AssertUnwindSafe(async {
                    // Acquire semaphore permit (limits concurrency)
                    let _permit = semaphore.acquire().await;

                    // Fetch with timeout
                    match tokio::time::timeout(
                        DEVICE_STATE_TIMEOUT,
                        client.get_device_state(&device_id),
                    )
                    .await
                    {
                        Ok(Ok(proto_state)) => Ok(DeviceState {
                            position: proto_state.position,
                            reading: proto_state.last_reading,
                            armed: proto_state.armed,
                            streaming: proto_state.streaming,
                            exposure_ms: proto_state.exposure_ms,
                            online: proto_state.online,
                        }),
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(_) => Err(format!("Timeout after {}s", DEVICE_STATE_TIMEOUT.as_secs())),
                    }
                });

                let result = match work.catch_unwind().await {
                    Ok(r) => r,
                    Err(_) => Err("Task panicked".to_string()),
                };

                // Always send result (success, error, timeout, or panic) to decrement action_in_flight
                let _ = tx
                    .send(ActionResult::GetDeviceState {
                        device_id: device_id_for_panic,
                        result,
                    })
                    .await;
            });
        }
    }

    /// Refresh device list from daemon
    ///
    /// Uses catch_unwind to ensure action_in_flight is always decremented (bd-tjwm.5)
    pub fn refresh(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        tracing::info!(
            client_available = client.is_some(),
            action_in_flight = self.action_in_flight,
            "InstrumentManagerPanel::refresh() called"
        );

        self.error = None;
        self.status = Some("Refreshing devices...".to_string());

        let Some(client) = client else {
            tracing::warn!("InstrumentManagerPanel::refresh() - no client available");
            self.error = Some("Not connected to daemon".to_string());
            self.status = None;
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        tracing::debug!(
            action_in_flight = self.action_in_flight,
            "InstrumentManagerPanel: spawning list_devices task"
        );

        runtime.spawn(async move {
            use futures::FutureExt;

            tracing::debug!("InstrumentManagerPanel: list_devices task starting");

            let work = std::panic::AssertUnwindSafe(async {
                client.list_devices().await.map_err(|e| e.to_string())
            });

            let result = match work.catch_unwind().await {
                Ok(r) => {
                    match &r {
                        Ok(devices) => tracing::debug!(
                            device_count = devices.len(),
                            "InstrumentManagerPanel: list_devices succeeded"
                        ),
                        Err(e) => tracing::warn!(
                            error = %e,
                            "InstrumentManagerPanel: list_devices failed"
                        ),
                    }
                    r
                }
                Err(_) => {
                    tracing::error!("InstrumentManagerPanel: list_devices task panicked");
                    Err("Task panicked".to_string())
                }
            };

            let _ = tx.send(ActionResult::Refresh(result)).await;
        });
    }

    /// Render the instrument manager panel
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        let should_fetch_states = self.poll_async_results(ui.ctx(), client.as_deref_mut(), runtime);

        // Fetch device states if refresh completed
        if should_fetch_states {
            tracing::info!("ui: should_fetch_states=true, calling refresh_device_states");
            self.refresh_device_states(client.as_deref_mut(), runtime);
        }

        // Handle pending context menu actions
        if let Some((device_id, device_name, action)) = self.pending_action.take() {
            match action {
                ContextAction::TestConnection => {
                    self.status = Some(format!("Testing connection to {}...", device_name));
                    self.test_connection(client.as_deref_mut(), runtime, device_id, device_name);
                }
                ContextAction::ViewParameters | ContextAction::Configure => {
                    self.status = Some(format!("Loading parameters for {}...", device_name));
                    self.load_parameters(client.as_deref_mut(), runtime, device_id, device_name);
                }
            }
        }

        // Render parameter viewer window (if open)
        let set_param_action = self.render_params_viewer(ui.ctx());

        // Handle parameter set action from viewer
        if let Some((param_name, value)) = set_param_action {
            if let Some(device_id) = self.params_viewer_device_id.clone() {
                self.set_parameter(client.as_deref_mut(), runtime, device_id, param_name, value);
            }
        }

        ui.heading("Instruments");

        // Show offline notice if not connected (bd-j3xz.4.4)
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        // Auto-refresh on first render when connected
        if !self.initial_refresh_done && client.is_some() && self.action_in_flight == 0 {
            tracing::info!(
                groups_count = self.groups.len(),
                "InstrumentManagerPanel: triggering auto-refresh on first render (client connected)"
            );
            self.initial_refresh_done = true;
            self.refresh(client.as_deref_mut(), runtime);
        } else if !self.initial_refresh_done && client.is_none() {
            tracing::debug!(
                "InstrumentManagerPanel: waiting for client connection before auto-refresh"
            );
        }

        // Toolbar
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh").clicked() {
                self.refresh(client.as_deref_mut(), runtime);
            }

            if let Some(last) = self.last_refresh {
                let elapsed = last.elapsed();
                ui.label(format!("{}s ago", elapsed.as_secs()));
            }
        });

        ui.separator();

        // Status/error messages
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.add_space(4.0);

        // Split view: device tree on left, control panel on right
        // Uses horizontal StripBuilder for proper height allocation
        let available_height = ui.available_height();
        StripBuilder::new(ui)
            .size(Size::exact(250.0)) // Left column: Device tree
            .size(Size::exact(4.0)) // Separator
            .size(Size::remainder()) // Right column: Control panel
            .horizontal(|mut strip| {
                // Left side: Device tree
                strip.cell(|ui| {
                    ui.set_min_height(available_height);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Device Tree").strong());
                        ui.separator();

                        if self.groups.is_empty() {
                            ui.label("No devices. Click Refresh to load.");
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("instrument_tree")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.render_tree(ui);
                                });
                        }
                    });
                });

                // Separator
                strip.cell(|ui| {
                    ui.separator();
                });

                // Right side: Control panel
                strip.cell(|ui| {
                    ui.set_min_height(available_height);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Control Panel").strong());
                        ui.separator();

                        egui::ScrollArea::vertical()
                            .id_salt("control_panel")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                self.render_device_control_panel(ui, client, runtime);
                            });
                    });
                });
            });
    }

    /// Render the device tree
    fn render_tree(&mut self, ui: &mut egui::Ui) {
        // Clone groups to avoid borrow checker issues
        let groups = self.groups.clone();

        for group in groups {
            let header = format!(
                "{} {} ({})",
                group.category.icon(),
                group.category.label(),
                group.devices.len()
            );

            let id = ui.make_persistent_id(format!("group_{:?}", group.category));
            egui::CollapsingHeader::new(header)
                .id_salt(id)
                .default_open(group.expanded)
                .show(ui, |ui| {
                    for device in &group.devices {
                        self.render_device_row(ui, device);
                    }
                });
        }
    }

    /// Render a single device row with status, capabilities, and context menu
    fn render_device_row(&mut self, ui: &mut egui::Ui, device: &DeviceInfo) {
        let selected = self.selected_device.as_ref() == Some(&device.id);

        // Get device state from cache
        let state = self.device_states.get(&device.id);

        ui.horizontal(|ui| {
            // Status indicator - green if online, gray if offline/unknown
            let status_color = if state.map(|s| s.online).unwrap_or(false) {
                egui::Color32::GREEN
            } else {
                egui::Color32::GRAY
            };
            ui.colored_label(status_color, "●");

            // Build device label with state
            let mut label = device.name.clone();
            if let Some(state) = state {
                let mut state_parts = Vec::new();

                if !state.online {
                    state_parts.push("offline".to_string());
                } else {
                    if let Some(pos) = state.position {
                        state_parts.push(format!("{:.2}", pos));
                    }
                    if let Some(reading) = state.reading {
                        state_parts.push(format!("{:.2}", reading));
                    }
                    if state.streaming == Some(true) {
                        state_parts.push("streaming".to_string());
                    }
                    if state.armed == Some(true) {
                        state_parts.push("armed".to_string());
                    }
                }

                if !state_parts.is_empty() {
                    label.push_str(&format!(" [{}]", state_parts.join(", ")));
                }
            }

            // Device name (selectable)
            let response = ui.selectable_label(selected, label);
            if response.clicked() {
                self.selected_device = Some(device.id.clone());
            }

            // Capability badges
            if device.is_movable {
                ui.label("🔄");
            }
            if device.is_readable {
                ui.label("📖");
            }
            if device.is_frame_producer {
                ui.label("📷");
            }

            // Context menu on right-click
            let device_id = device.id.clone();
            let device_name = device.name.clone();
            response.context_menu(|ui| {
                if ui.button("📋 View Parameters").clicked() {
                    self.pending_action = Some((
                        device_id.clone(),
                        device_name.clone(),
                        ContextAction::ViewParameters,
                    ));
                    ui.close();
                }
                if ui.button("🔌 Test Connection").clicked() {
                    self.pending_action = Some((
                        device_id.clone(),
                        device_name.clone(),
                        ContextAction::TestConnection,
                    ));
                    ui.close();
                }
                if ui.button("⚙️ Configure").clicked() {
                    // Configure opens parameters in edit mode
                    self.pending_action = Some((
                        device_id.clone(),
                        device_name.clone(),
                        ContextAction::Configure,
                    ));
                    ui.close();
                }
            });

            // Show device details on hover
            response.on_hover_ui(|ui| {
                ui.label(format!("ID: {}", device.id));
                ui.label(format!("Driver: {}", device.driver_type));

                // Capabilities
                ui.separator();
                ui.label("Capabilities:");
                if device.is_movable {
                    ui.label("• Movable");
                }
                if device.is_readable {
                    ui.label("• Readable");
                }
                if device.is_frame_producer {
                    ui.label("• Frame Producer");
                }

                // Current state
                if let Some(state) = state {
                    ui.separator();
                    ui.label("Current State:");
                    ui.label(format!("Online: {}", state.online));
                    if let Some(pos) = state.position {
                        ui.label(format!("Position: {:.3}", pos));
                    }
                    if let Some(reading) = state.reading {
                        ui.label(format!("Reading: {:.3}", reading));
                    }
                    if let Some(armed) = state.armed {
                        ui.label(format!("Armed: {}", armed));
                    }
                    if let Some(streaming) = state.streaming {
                        ui.label(format!("Streaming: {}", streaming));
                    }
                    if let Some(exp) = state.exposure_ms {
                        ui.label(format!("Exposure: {:.1} ms", exp));
                    }
                }
            });
        });
    }

    /// Get currently selected device ID (for future device detail view)
    #[allow(dead_code)]
    pub fn selected_device(&self) -> Option<&str> {
        self.selected_device.as_deref()
    }

    /// Test connection to a device
    fn test_connection(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        device_name: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = match client.get_device_state(&device_id).await {
                Ok(state) => Ok(state.online),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx
                .send(ActionResult::TestConnection {
                    _device_id: device_id,
                    device_name,
                    result,
                })
                .await;
        });
    }

    /// Load parameters for a device (opens the parameter viewer)
    fn load_parameters(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        device_name: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.params_viewer_loading = true;
        self.params_viewer_error = None;

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            // First get the parameter descriptors
            let descriptors = match client.list_parameters(&device_id).await {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx
                        .send(ActionResult::ListParameters {
                            device_id,
                            device_name,
                            result: Err(e.to_string()),
                        })
                        .await;
                    return;
                }
            };

            // Then get current values for each parameter
            let mut params = Vec::new();
            for desc in descriptors {
                let current_value = if desc.readable {
                    match client.get_parameter(&device_id_clone, &desc.name).await {
                        Ok(v) => Some(v.value),
                        Err(_) => None,
                    }
                } else {
                    None
                };

                params.push(ParameterInfo {
                    name: desc.name,
                    description: desc.description,
                    dtype: desc.dtype,
                    units: desc.units,
                    readable: desc.readable,
                    writable: desc.writable,
                    min_value: desc.min_value,
                    max_value: desc.max_value,
                    enum_values: desc.enum_values,
                    current_value,
                });
            }

            let _ = tx
                .send(ActionResult::ListParameters {
                    device_id: device_id_clone,
                    device_name,
                    result: Ok(params),
                })
                .await;
        });
    }

    /// Set a parameter value
    fn set_parameter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        param_name: String,
        value: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = match client.set_parameter(&device_id, &param_name, &value).await {
                Ok(resp) => {
                    if resp.success {
                        Ok(resp.actual_value)
                    } else {
                        Err(resp.error_message)
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx
                .send(ActionResult::SetParameter {
                    _device_id: device_id,
                    param_name,
                    result,
                })
                .await;
        });
    }

    /// Render the parameters viewer window
    /// Returns an optional (param_name, value) to set after rendering
    fn render_params_viewer(&mut self, ctx: &egui::Context) -> Option<(String, String)> {
        if !self.params_viewer_open {
            return None;
        }

        let device_name = self
            .params_viewer_device_name
            .clone()
            .unwrap_or_else(|| "Device".to_string());

        let mut action_to_perform: Option<(String, String)> = None;
        let mut open = self.params_viewer_open;

        egui::Window::new(format!("Parameters: {}", device_name))
            .id(egui::Id::new("params_viewer"))
            .open(&mut open)
            .resizable(true)
            .default_width(500.0)
            .show(ctx, |ui| {
                if self.params_viewer_loading {
                    ui.spinner();
                    ui.label("Loading parameters...");
                    return;
                }

                if let Some(ref err) = self.params_viewer_error {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
                    return;
                }

                if self.params_viewer_params.is_empty() {
                    ui.label("No parameters available for this device.");
                    return;
                }

                // Parameters table
                egui::ScrollArea::vertical()
                    .id_salt("params_scroll")
                    .show(ui, |ui| {
                        egui::Grid::new("params_grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                // Header
                                ui.strong("Parameter");
                                ui.strong("Value");
                                ui.strong("Units");
                                ui.strong("Actions");
                                ui.end_row();

                                // Parameters - clone to avoid borrow issues
                                let params = self.params_viewer_params.clone();
                                for param in params {
                                    ui.label(&param.name);

                                    // Value display/edit
                                    if param.writable {
                                        let edit_value = self
                                            .param_edit_values
                                            .entry(param.name.clone())
                                            .or_insert_with(|| {
                                                param.current_value.clone().unwrap_or_default()
                                            });

                                        // Use appropriate widget based on dtype
                                        if !param.enum_values.is_empty() {
                                            // Enum: dropdown
                                            egui::ComboBox::from_id_salt(&param.name)
                                                .selected_text(edit_value.as_str())
                                                .show_ui(ui, |ui| {
                                                    for v in &param.enum_values {
                                                        ui.selectable_value(
                                                            edit_value,
                                                            v.clone(),
                                                            v,
                                                        );
                                                    }
                                                });
                                        } else if param.dtype == "bool" {
                                            // Bool: checkbox
                                            let mut checked = edit_value == "true";
                                            if ui.checkbox(&mut checked, "").changed() {
                                                *edit_value = checked.to_string();
                                            }
                                        } else {
                                            // Text input
                                            let response = ui.add(
                                                egui::TextEdit::singleline(edit_value)
                                                    .desired_width(100.0),
                                            );

                                            // Show tooltip with range info
                                            if param.min_value.is_some()
                                                || param.max_value.is_some()
                                            {
                                                response.on_hover_text(format!(
                                                    "Range: {} to {}",
                                                    param
                                                        .min_value
                                                        .map(|v| v.to_string())
                                                        .unwrap_or_else(|| "-".to_string()),
                                                    param
                                                        .max_value
                                                        .map(|v| v.to_string())
                                                        .unwrap_or_else(|| "-".to_string())
                                                ));
                                            }
                                        }
                                    } else {
                                        // Read-only
                                        ui.label(param.current_value.as_deref().unwrap_or("-"));
                                    }

                                    ui.label(&param.units);

                                    // Action buttons
                                    ui.horizontal(|ui| {
                                        if param.writable {
                                            let current_edit =
                                                self.param_edit_values.get(&param.name);
                                            let has_changes = current_edit
                                                .map(|v| Some(v) != param.current_value.as_ref())
                                                .unwrap_or(false);

                                            if ui
                                                .add_enabled(has_changes, egui::Button::new("Set"))
                                                .clicked()
                                            {
                                                if let Some(value) = current_edit.cloned() {
                                                    action_to_perform =
                                                        Some((param.name.clone(), value));
                                                }
                                            }
                                        }
                                    });

                                    ui.end_row();
                                }
                            });
                    });

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        self.params_viewer_open = false;
                    }
                });
            });

        self.params_viewer_open = open;
        action_to_perform
    }

    // =========================================================================
    // Device Control Methods
    // =========================================================================

    /// Move a device to a target position
    fn move_device(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        position: f64,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.operation_pending
            .insert(device_id.clone(), "Moving...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.move_absolute(&device_id, position).await;
            let _ = tx
                .send(ActionResult::MoveDevice {
                    device_id: device_id_clone,
                    result: result.map(|_| ()).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Move a device relative to current position
    fn move_device_relative(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        delta: f64,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.operation_pending
            .insert(device_id.clone(), "Moving...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.move_relative(&device_id, delta).await;
            let _ = tx
                .send(ActionResult::MoveDevice {
                    device_id: device_id_clone,
                    result: result.map(|_| ()).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Read a value from a device
    fn read_device(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.operation_pending
            .insert(device_id.clone(), "Reading...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.read_value(&device_id).await;
            let _ = tx
                .send(ActionResult::ReadDevice {
                    device_id: device_id_clone,
                    result: result.map(|r| r.value).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Start streaming on a camera
    fn start_stream(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.operation_pending
            .insert(device_id.clone(), "Starting stream...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.start_stream(&device_id, None).await;
            let _ = tx
                .send(ActionResult::StartStream {
                    device_id: device_id_clone,
                    result: result.map(|_| ()).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Stop streaming on a camera
    fn stop_stream(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        self.operation_pending
            .insert(device_id.clone(), "Stopping stream...".to_string());

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id_clone = device_id.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client.stop_stream(&device_id).await;
            let _ = tx
                .send(ActionResult::StopStream {
                    device_id: device_id_clone,
                    result: result.map(|_| ()).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Get the DeviceInfo for a device ID
    fn get_device_info(&self, device_id: &str) -> Option<DeviceInfo> {
        for group in &self.groups {
            for device in &group.devices {
                if device.id == device_id {
                    return Some(device.clone());
                }
            }
        }
        None
    }

    /// Render the control panel for the selected device
    fn render_device_control_panel(
        &mut self,
        ui: &mut egui::Ui,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let Some(device_id) = self.selected_device.clone() else {
            ui.colored_label(
                egui::Color32::GRAY,
                "Select a device above to show controls",
            );
            return;
        };

        let Some(device) = self.get_device_info(&device_id) else {
            ui.label("Device not found");
            return;
        };

        // Pop Out button header
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("⬜ Pop Out")
                    .on_hover_text("Open in separate dockable panel")
                    .clicked()
                {
                    self.pending_pop_out = Some(device.clone());
                }
            });
        });

        ui.separator();

        // Determine which device-specific panel to use based on driver type and capabilities
        let driver_lower = device.driver_type.to_lowercase();

        // Check for MaiTai laser
        if driver_lower.contains("maitai")
            || driver_lower.contains("mai_tai")
            || (device.is_wavelength_tunable && device.is_emission_controllable)
        {
            let panel = self.maitai_panels.entry(device_id.clone()).or_default();
            // Use push_id to avoid widget ID collisions with docked panels
            ui.push_id(("instr_mgr", &device_id), |ui| {
                panel.ui(ui, &device, client.as_deref_mut(), runtime);
            });
            return;
        }

        // Check for power meter
        if driver_lower.contains("1830")
            || driver_lower.contains("power_meter")
            || (device.is_readable && !device.is_movable && !device.is_frame_producer)
        {
            let panel = self
                .power_meter_panels
                .entry(device_id.clone())
                .or_default();
            // Use push_id to avoid widget ID collisions with docked panels
            ui.push_id(("instr_mgr", &device_id), |ui| {
                panel.ui(ui, &device, client.as_deref_mut(), runtime);
            });
            return;
        }

        // Check for ELL14 rotator
        if driver_lower.contains("ell14") || driver_lower.contains("thorlabs") {
            let panel = self.rotator_panels.entry(device_id.clone()).or_default();
            // Use push_id to avoid widget ID collisions with docked panels
            ui.push_id(("instr_mgr", &device_id), |ui| {
                panel.ui(ui, &device, client.as_deref_mut(), runtime);
            });
            return;
        }

        // Check for ESP300 stage or other movable devices
        if device.is_movable {
            let panel = self.stage_panels.entry(device_id.clone()).or_default();
            // Use push_id to avoid widget ID collisions with docked panels
            ui.push_id(("instr_mgr", &device_id), |ui| {
                panel.ui(ui, &device, client.as_deref_mut(), runtime);
            });
            return;
        }

        // Fallback: use the original generic control panel
        self.render_generic_control_panel(ui, &device, client, runtime);
    }

    /// Render the generic (legacy) control panel for devices without specialized panels
    fn render_generic_control_panel(
        &mut self,
        ui: &mut egui::Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let device_id = device.id.clone();

        // Clone state to avoid borrow issues
        let state = self.device_states.get(&device_id).cloned();
        let is_online = state.as_ref().map(|s| s.online).unwrap_or(false);
        let op_pending = self.operation_pending.get(&device_id).cloned();

        ui.horizontal(|ui| {
            ui.heading(&device.name);
            if is_online {
                ui.colored_label(egui::Color32::GREEN, "● Online");
            } else {
                ui.colored_label(egui::Color32::RED, "● Offline");
            }
            if let Some(op) = &op_pending {
                ui.spinner();
                ui.label(op);
            }
        });

        ui.separator();

        // Collect actions from all control panels
        let mut actions = Vec::new();

        // Show appropriate controls based on device capabilities
        if device.is_movable {
            if let Some(action) = self.render_motion_controls(ui, &device_id, state.as_ref()) {
                actions.push(action);
            }
        }

        if device.is_readable {
            if let Some(action) = self.render_read_controls(ui, &device_id) {
                actions.push(action);
            }
        }

        if device.is_frame_producer {
            if let Some(action) = self.render_camera_controls(ui, &device_id, state.as_ref()) {
                actions.push(action);
            }
        }

        // Quick actions
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("📋 Parameters").clicked() {
                self.pending_action = Some((
                    device_id.clone(),
                    device.name.clone(),
                    ContextAction::ViewParameters,
                ));
            }
            if ui.button("🔄 Refresh State").clicked() {
                actions.push(ControlAction::RefreshState(device_id.clone()));
            }
        });

        // Execute collected actions
        for action in actions {
            self.execute_control_action(action, client.as_deref_mut(), runtime);
        }
    }

    /// Render motion controls for movable devices
    /// Returns an optional action to perform (MoveAbs(pos), MoveRel(delta))
    fn render_motion_controls(
        &mut self,
        ui: &mut egui::Ui,
        device_id: &str,
        state: Option<&DeviceState>,
    ) -> Option<ControlAction> {
        let mut action = None;

        ui.group(|ui| {
            ui.label(egui::RichText::new("🔄 Motion Control").strong());

            // Current position
            if let Some(pos) = state.and_then(|s| s.position) {
                ui.horizontal(|ui| {
                    ui.label("Position:");
                    ui.label(
                        egui::RichText::new(format!("{:.4}", pos))
                            .monospace()
                            .strong(),
                    );
                });
            }

            ui.add_space(4.0);

            // Absolute move
            ui.horizontal(|ui| {
                ui.label("Move to:");
                let target = self
                    .move_target
                    .entry(device_id.to_string())
                    .or_insert_with(|| "0.0".to_string());
                ui.add(egui::TextEdit::singleline(target).desired_width(80.0));

                let is_busy = self.operation_pending.contains_key(device_id);
                if ui.add_enabled(!is_busy, egui::Button::new("Go")).clicked() {
                    if let Ok(pos) = target.parse::<f64>() {
                        action = Some(ControlAction::MoveAbs(device_id.to_string(), pos));
                    } else {
                        self.error = Some("Invalid position value".to_string());
                    }
                }
            });

            // Jog controls
            ui.horizontal(|ui| {
                ui.label("Jog:");
                let jog = self
                    .jog_amount
                    .entry(device_id.to_string())
                    .or_insert_with(|| "1.0".to_string());
                ui.add(egui::TextEdit::singleline(jog).desired_width(60.0));

                let is_busy = self.operation_pending.contains_key(device_id);
                let jog_val: f64 = jog.parse().unwrap_or(1.0);

                if ui.add_enabled(!is_busy, egui::Button::new("◀ -")).clicked() {
                    action = Some(ControlAction::MoveRel(device_id.to_string(), -jog_val));
                }
                if ui.add_enabled(!is_busy, egui::Button::new("+ ▶")).clicked() {
                    action = Some(ControlAction::MoveRel(device_id.to_string(), jog_val));
                }
            });
        });

        action
    }

    /// Render read controls for readable devices
    /// Returns an optional action to perform
    fn render_read_controls(
        &mut self,
        ui: &mut egui::Ui,
        device_id: &str,
    ) -> Option<ControlAction> {
        let mut action = None;

        ui.group(|ui| {
            ui.label(egui::RichText::new("📖 Read Value").strong());

            // Last reading
            if let Some((value, when)) = self.last_reading.get(device_id) {
                ui.horizontal(|ui| {
                    ui.label("Value:");
                    ui.label(
                        egui::RichText::new(format!("{:.6}", value))
                            .monospace()
                            .strong(),
                    );
                    ui.label(format!("({}s ago)", when.elapsed().as_secs()));
                });
            }

            let is_busy = self.operation_pending.contains_key(device_id);
            if ui
                .add_enabled(!is_busy, egui::Button::new("📖 Read Now"))
                .clicked()
            {
                action = Some(ControlAction::Read(device_id.to_string()));
            }
        });

        action
    }

    /// Render camera controls for frame producers
    /// Returns an optional action to perform
    fn render_camera_controls(
        &mut self,
        ui: &mut egui::Ui,
        device_id: &str,
        state: Option<&DeviceState>,
    ) -> Option<ControlAction> {
        let mut action = None;

        ui.group(|ui| {
            ui.label(egui::RichText::new("📷 Camera Control").strong());

            let is_streaming = state.and_then(|s| s.streaming).unwrap_or(false);
            let exposure_ms = state.and_then(|s| s.exposure_ms);

            // Current state
            ui.horizontal(|ui| {
                ui.label("Status:");
                if is_streaming {
                    ui.colored_label(egui::Color32::GREEN, "Streaming");
                } else {
                    ui.label("Idle");
                }
                if let Some(exp) = exposure_ms {
                    ui.label(format!("| Exposure: {:.1} ms", exp));
                }
            });

            ui.add_space(4.0);

            // Stream controls
            let is_busy = self.operation_pending.contains_key(device_id);
            ui.horizontal(|ui| {
                if is_streaming {
                    if ui
                        .add_enabled(!is_busy, egui::Button::new("⏹ Stop Stream"))
                        .clicked()
                    {
                        action = Some(ControlAction::StopStream(device_id.to_string()));
                    }
                } else if ui
                    .add_enabled(!is_busy, egui::Button::new("▶ Start Stream"))
                    .clicked()
                {
                    action = Some(ControlAction::StartStream(device_id.to_string()));
                }
            });

            // Exposure control
            ui.horizontal(|ui| {
                ui.label("Exposure (ms):");
                let exp_input = self
                    .exposure_input
                    .entry(device_id.to_string())
                    .or_insert_with(|| {
                        exposure_ms
                            .map(|e| e.to_string())
                            .unwrap_or("10.0".to_string())
                    });
                ui.add(egui::TextEdit::singleline(exp_input).desired_width(80.0));

                if ui.button("Set").clicked() {
                    if let Ok(exp) = exp_input.parse::<f64>() {
                        action = Some(ControlAction::SetExposure(device_id.to_string(), exp));
                    } else {
                        self.error = Some("Invalid exposure value".to_string());
                    }
                }
            });
        });

        action
    }

    /// Execute a control action
    fn execute_control_action(
        &mut self,
        action: ControlAction,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        match action {
            ControlAction::MoveAbs(device_id, position) => {
                self.move_device(client, runtime, device_id, position);
            }
            ControlAction::MoveRel(device_id, delta) => {
                self.move_device_relative(client, runtime, device_id, delta);
            }
            ControlAction::Read(device_id) => {
                self.read_device(client, runtime, device_id);
            }
            ControlAction::StartStream(device_id) => {
                self.start_stream(client, runtime, device_id);
            }
            ControlAction::StopStream(device_id) => {
                self.stop_stream(client, runtime, device_id);
            }
            ControlAction::SetExposure(device_id, exposure_ms) => {
                self.set_exposure(client, runtime, device_id, exposure_ms);
            }
            ControlAction::RefreshState(device_id) => {
                self.refresh_single_device(client, runtime, device_id);
            }
        }
    }

    /// Set exposure for a camera
    fn set_exposure(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
        exposure_ms: f64,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = client
                .set_parameter(
                    &device_id,
                    "acquisition.exposure_ms",
                    &exposure_ms.to_string(),
                )
                .await;
            let _ = tx
                .send(ActionResult::SetParameter {
                    _device_id: device_id,
                    param_name: "acquisition.exposure_ms".to_string(),
                    result: result.map(|r| r.actual_value).map_err(|e| e.to_string()),
                })
                .await;
        });
    }

    /// Refresh state for a single device
    fn refresh_single_device(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: String,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        runtime.spawn(async move {
            let result = match client.get_device_state(&device_id).await {
                Ok(proto_state) => Ok(DeviceState {
                    position: proto_state.position,
                    reading: proto_state.last_reading,
                    armed: proto_state.armed,
                    streaming: proto_state.streaming,
                    exposure_ms: proto_state.exposure_ms,
                    online: proto_state.online,
                }),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx
                .send(ActionResult::GetDeviceState { device_id, result })
                .await;
        });
    }
}
