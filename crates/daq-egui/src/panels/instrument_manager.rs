//! Instrument Manager Panel - hierarchical device tree view
//!
//! Displays registered hardware grouped by type (Cameras, Stages, Detectors, etc.)
//! with expandable nodes showing device state and quick actions.

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use std::collections::HashMap;
use std::sync::Arc;

use crate::client::DaqClient;
use daq_proto::daq::DeviceInfo;

/// Auto-refresh interval (for future auto-refresh feature)
#[allow(dead_code)]
const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Timeout for individual device state fetch (prevents stalls from hung devices)
const DEVICE_STATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Maximum concurrent device state requests (prevents overwhelming the daemon)
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// Device category for grouping in the tree view
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)] // All variants defined for completeness
pub enum DeviceCategory {
    Camera,
    Stage,
    Detector,
    Laser,
    PowerMeter,
    Other,
}

impl DeviceCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Camera => "Cameras",
            Self::Stage => "Stages",
            Self::Detector => "Detectors",
            Self::Laser => "Lasers",
            Self::PowerMeter => "Power Meters",
            Self::Other => "Other",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Camera => "📷",
            Self::Stage => "🔄",
            Self::Detector => "📊",
            Self::Laser => "🔴",
            Self::PowerMeter => "⚡",
            Self::Other => "🔧",
        }
    }

    /// Infer category from device capabilities
    pub fn from_device_info(info: &DeviceInfo) -> Self {
        if info.is_frame_producer {
            Self::Camera
        } else if info.is_movable {
            Self::Stage
        } else if info.is_readable {
            // Could be detector or power meter - check driver name
            if info.driver_type.to_lowercase().contains("power") {
                Self::PowerMeter
            } else {
                Self::Detector
            }
        } else {
            Self::Other
        }
    }
}

/// Grouped devices for tree display
#[derive(Clone)]
pub struct DeviceGroup {
    pub category: DeviceCategory,
    pub devices: Vec<DeviceInfo>,
    pub expanded: bool,
}

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

/// Async action results
enum ActionResult {
    Refresh(Result<Vec<DeviceInfo>, String>),
    GetDeviceState {
        device_id: String,
        result: Result<DeviceState, String>,
    },
}

/// Instrument Manager Panel state
pub struct InstrumentManagerPanel {
    /// Devices grouped by category
    groups: Vec<DeviceGroup>,
    /// Last refresh timestamp
    last_refresh: Option<std::time::Instant>,
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
}

impl Default for InstrumentManagerPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            groups: Vec::new(),
            last_refresh: None,
            error: None,
            status: None,
            selected_device: None,
            device_states: HashMap::new(),
            action_tx,
            action_rx,
            action_in_flight: 0,
        }
    }
}

impl InstrumentManagerPanel {
    /// Check if auto-refresh is due (for future auto-refresh feature)
    #[allow(dead_code)]
    fn should_auto_refresh(&self) -> bool {
        match self.last_refresh {
            Some(last) => last.elapsed() >= AUTO_REFRESH_INTERVAL,
            None => true, // Never refreshed
        }
    }

    /// Called each frame - triggers auto-refresh if needed (for future auto-refresh feature)
    #[allow(dead_code)]
    pub fn tick(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if self.action_in_flight == 0 && self.should_auto_refresh() {
            self.refresh(client, runtime);
        }
    }

    /// Poll for async results
    fn poll_async_results(&mut self, ctx: &egui::Context, _client: Option<&mut DaqClient>, _runtime: &Runtime) -> bool {
        let mut updated = false;
        let mut should_fetch_device_states = false;

        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::Refresh(result) => match result {
                            Ok(devices) => {
                                self.update_groups(devices);
                                self.last_refresh = Some(std::time::Instant::now());
                                self.status = Some(format!("Loaded {} devices",
                                    self.groups.iter().map(|g| g.devices.len()).sum::<usize>()));
                                self.error = None;
                                should_fetch_device_states = true;
                            }
                            Err(e) => self.error = Some(e),
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
        let old_expanded: HashMap<DeviceCategory, bool> = self.groups
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
        let Some(client) = client else { return };

        // Collect all device IDs first
        let device_ids: Vec<String> = self.groups.iter()
            .flat_map(|g| g.devices.iter().map(|d| d.id.clone()))
            .collect();

        if device_ids.is_empty() {
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
                    ).await {
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
                let _ = tx.send(ActionResult::GetDeviceState {
                    device_id: device_id_for_panic,
                    result,
                }).await;
            });
        }
    }

    /// Refresh device list from daemon
    ///
    /// Uses catch_unwind to ensure action_in_flight is always decremented (bd-tjwm.5)
    pub fn refresh(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime) {
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
            use futures::FutureExt;

            let work = std::panic::AssertUnwindSafe(async {
                client.list_devices().await.map_err(|e| e.to_string())
            });

            let result = match work.catch_unwind().await {
                Ok(r) => r,
                Err(_) => Err("Task panicked".to_string()),
            };

            let _ = tx.send(ActionResult::Refresh(result)).await;
        });
    }

    /// Render the instrument manager panel
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        let should_fetch_states = self.poll_async_results(ui.ctx(), client.as_deref_mut(), runtime);


        // Fetch device states if refresh completed
        if should_fetch_states {
            self.refresh_device_states(client.as_deref_mut(), runtime);
        }

        ui.heading("Instruments");

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

        // Device tree
        if self.groups.is_empty() {
            ui.label("No devices. Click Refresh to load.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("instrument_tree")
                .show(ui, |ui| {
                    self.render_tree(ui);
                });
        }
    }

    /// Render the device tree
    fn render_tree(&mut self, ui: &mut egui::Ui) {
        // Clone groups to avoid borrow checker issues
        let groups = self.groups.clone();

        for group in groups {
            let header = format!("{} {} ({})",
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
            response.context_menu(|ui| {
                if ui.button("⚙️ Configure").clicked() {
                    // TODO: Open config dialog
                    self.status = Some(format!("Configure {} (not yet implemented)", device.name));
                    ui.close_menu();
                }
                if ui.button("🔌 Test Connection").clicked() {
                    // TODO: Trigger connection test
                    self.status = Some(format!("Testing connection to {} (not yet implemented)", device.name));
                    ui.close_menu();
                }
                if ui.button("📋 View Parameters").clicked() {
                    // TODO: Show parameters panel
                    self.status = Some(format!("View parameters for {} (not yet implemented)", device.name));
                    ui.close_menu();
                }
            });

            // Show device details on hover
            response.on_hover_ui(|ui| {
                ui.label(format!("ID: {}", device.id));
                ui.label(format!("Driver: {}", device.driver_type));

                // Capabilities
                ui.separator();
                ui.label("Capabilities:");
                if device.is_movable { ui.label("• Movable"); }
                if device.is_readable { ui.label("• Readable"); }
                if device.is_frame_producer { ui.label("• Frame Producer"); }

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
}
