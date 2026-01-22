//! Main application state and UI logic.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::connection::{resolve_address, save_to_storage, AddressSource, DaemonAddress};
use crate::daemon_launcher::{AutoConnectState, DaemonLauncher, DaemonMode};
use crate::icons;
use crate::layout;
use crate::panels::{
    ConnectionDiagnostics, ConnectionStatus as LogConnectionStatus, DevicesPanel,
    DocumentViewerPanel, GettingStartedPanel, ImageViewerPanel, InstrumentManagerPanel,
    LoggingPanel, ModulesPanel, PlanRunnerPanel, ScanBuilderPanel, ScansPanel, ScriptsPanel,
    SignalPlotterPanel, StoragePanel,
};
use crate::reconnect::{friendly_error_message, ConnectionManager, ConnectionState};
use crate::theme::{self, ThemePreference};
use crate::widgets::{
    DeviceControlWidget, MaiTaiControlPanel, PowerMeterControlPanel, RotatorControlPanel,
    StageControlPanel, StatusBar,
};
use daq_proto::daq::DeviceInfo;

/// Result of a health check sent through the channel (bd-j3xz.3.3: includes RTT).
enum HealthCheckResult {
    /// Health check succeeded with round-trip time in milliseconds.
    Success { rtt_ms: f64 },
    /// Health check failed with error message.
    Failed(String),
}

/// Main application state
pub struct DaqApp {
    /// gRPC client (wrapped in Option for lazy initialization)
    client: Option<DaqClient>,

    /// Connection manager (handles state machine and auto-reconnect)
    connection: ConnectionManager,

    /// Validated daemon address (normalized, with source tracking)
    daemon_address: DaemonAddress,

    /// Text input field for address (may be invalid during editing)
    address_input: String,

    /// Address validation error (shown in UI)
    address_error: Option<String>,

    /// Daemon version (retrieved via GetDaemonInfo)
    daemon_version: Option<String>,

    /// GUI version (from CARGO_PKG_VERSION)
    gui_version: String,

    /// Dock state for panel management
    dock_state: Option<DockState<Panel>>,

    /// Queue for deferred UI actions (e.g. opening tabs from Nav panel)
    ui_actions: Vec<UiAction>,

    /// Panel states
    getting_started_panel: GettingStartedPanel,
    devices_panel: DevicesPanel,
    scripts_panel: ScriptsPanel,
    scans_panel: ScansPanel,
    storage_panel: StoragePanel,
    modules_panel: ModulesPanel,
    plan_runner_panel: PlanRunnerPanel,
    scan_builder_panel: ScanBuilderPanel,
    document_viewer_panel: DocumentViewerPanel,
    instrument_manager_panel: InstrumentManagerPanel,
    signal_plotter_panel: SignalPlotterPanel,
    image_viewer_panel: ImageViewerPanel,
    logging_panel: LoggingPanel,

    /// Tokio runtime for async operations
    runtime: tokio::runtime::Runtime,

    /// Channel for health check results
    health_tx: mpsc::Sender<HealthCheckResult>,
    health_rx: mpsc::Receiver<HealthCheckResult>,

    /// Previous connection state (for detecting transitions)
    was_connected: bool,

    /// Daemon mode configuration (local auto-start, remote, or lab hardware)
    daemon_mode: DaemonMode,

    /// Daemon process launcher (for LocalAuto mode)
    daemon_launcher: Option<DaemonLauncher>,

    /// Auto-connect lifecycle state
    auto_connect_state: AutoConnectState,

    /// Receiver for tracing log events (forwarded to logging panel)
    log_receiver: std::sync::mpsc::Receiver<crate::gui_log_layer::GuiLogEvent>,

    /// Theme preference (light/dark/system)
    theme_preference: ThemePreference,

    /// Status bar widget for connection indicator and version display
    status_bar: StatusBar,

    /// Device control panel ID to device info mapping (for dockable device panels)
    device_panel_info: HashMap<usize, DevicePanelInfo>,

    /// Next available device panel ID
    next_device_panel_id: usize,

    /// Docked MaiTai control panels (keyed by panel ID)
    docked_maitai_panels: HashMap<usize, MaiTaiControlPanel>,

    /// Docked power meter control panels (keyed by panel ID)
    docked_power_meter_panels: HashMap<usize, PowerMeterControlPanel>,

    /// Docked rotator control panels (keyed by panel ID)
    docked_rotator_panels: HashMap<usize, RotatorControlPanel>,

    /// Docked stage control panels (keyed by panel ID)
    docked_stage_panels: HashMap<usize, StageControlPanel>,

    /// PVCAM live view streaming state (requires rerun_viewer + instrument_photometrics)
    /// Works in mock mode without pvcam_hardware, or with real SDK when pvcam_hardware enabled
    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    pvcam_streaming: bool,
    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    pvcam_task: Option<tokio::task::JoinHandle<()>>,
}

/// Action to perform on the UI state
enum UiAction {
    FocusTab(Panel),
    /// Open a device control panel as a docked tab
    OpenDeviceControl {
        /// Full device info with capability flags
        device_info: DeviceInfo,
    },
}

/// Info about a docked device control panel (runtime state)
#[derive(Debug, Clone)]
pub(crate) struct DevicePanelInfo {
    /// Full device info with capability flags (avoids inferring capabilities from driver_type)
    device_info: DeviceInfo,
}

/// Serializable version of device panel info for layout persistence.
/// Contains only the fields needed to restore the panel on app restart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedPanelInfo {
    device_id: String,
    device_name: String,
    driver_type: String,
    // Capability flags for panel type determination
    is_emission_controllable: bool,
    is_shutter_controllable: bool,
    is_wavelength_tunable: bool,
    is_readable: bool,
    is_movable: bool,
}

impl From<&DeviceInfo> for PersistedPanelInfo {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            device_id: info.id.clone(),
            device_name: info.name.clone(),
            driver_type: info.driver_type.clone(),
            is_emission_controllable: info.is_emission_controllable,
            is_shutter_controllable: info.is_shutter_controllable,
            is_wavelength_tunable: info.is_wavelength_tunable,
            is_readable: info.is_readable,
            is_movable: info.is_movable,
        }
    }
}

impl From<PersistedPanelInfo> for DeviceInfo {
    fn from(info: PersistedPanelInfo) -> Self {
        Self {
            id: info.device_id,
            name: info.device_name,
            driver_type: info.driver_type,
            category: 0, // Will be updated when daemon connects
            is_movable: info.is_movable,
            is_readable: info.is_readable,
            is_triggerable: false,
            is_frame_producer: false,
            is_exposure_controllable: false,
            is_shutter_controllable: info.is_shutter_controllable,
            is_wavelength_tunable: info.is_wavelength_tunable,
            is_emission_controllable: info.is_emission_controllable,
            metadata: None,
        }
    }
}

/// Available panels in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Panel {
    Nav,
    GettingStarted,
    Instruments,
    Devices,
    Scripts,
    Scans,
    ScanBuilder,
    Storage,
    Modules,
    PlanRunner,
    DocumentViewer,
    SignalPlotter,
    ImageViewer,
    Logs,
    /// Dockable device control panel (uses id to lookup device_id in app state)
    DeviceControl {
        id: usize,
    },
}

impl DaqApp {
    /// Create a new application instance with the specified daemon mode
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        daemon_mode: DaemonMode,
        log_receiver: std::sync::mpsc::Receiver<crate::gui_log_layer::GuiLogEvent>,
    ) -> Self {
        // Load phosphor icons into egui fonts
        let mut fonts = egui::FontDefinitions::default();
        icons::add_to_fonts(&mut fonts);
        cc.egui_ctx.set_fonts(fonts);

        // Load or default theme preference
        let theme_preference: ThemePreference = cc
            .storage
            .and_then(|s| eframe::get_value(s, "theme_preference"))
            .unwrap_or_default();
        theme::apply_theme(&cc.egui_ctx, theme_preference);

        // Configure egui style with consistent spacing
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = layout::ITEM_SPACING;
        cc.egui_ctx.set_style(style);

        // Create tokio runtime for gRPC calls
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");
        // Start daemon launcher if in LocalAuto mode
        let daemon_launcher = if daemon_mode.should_auto_start() {
            let port = daemon_mode.port().unwrap_or(50051);
            let mut launcher = DaemonLauncher::new(port);
            if let Err(e) = launcher.start() {
                tracing::error!("Failed to start daemon: {}", e);
            }
            Some(launcher)
        } else {
            None
        };

        // Determine auto-connect state based on mode
        let auto_connect_state = if daemon_mode.should_auto_start() {
            AutoConnectState::WaitingForDaemon {
                since: Instant::now(),
            }
        } else {
            // For remote mode, we can try to connect immediately
            AutoConnectState::ReadyToConnect
        };

        // Use daemon mode URL as the address, or fall back to stored/env/default
        let daemon_address = if matches!(daemon_mode, DaemonMode::Remote { .. }) {
            // For remote mode, use the provided URL directly
            DaemonAddress::parse(&daemon_mode.daemon_url(), AddressSource::UserInput)
                .unwrap_or_else(|_| resolve_address(None, cc.storage))
        } else {
            // For local modes, use the generated URL
            DaemonAddress::parse(&daemon_mode.daemon_url(), AddressSource::Default)
                .unwrap_or_else(|_| resolve_address(None, cc.storage))
        };
        let address_input = daemon_address.original().to_string();

        // Create health check channel
        let (health_tx, health_rx) = mpsc::channel(4);

        // Load persisted device panel info
        let (
            device_panel_info,
            next_device_panel_id,
            docked_maitai_panels,
            docked_power_meter_panels,
            docked_rotator_panels,
            docked_stage_panels,
        ) = if let Some(storage) = cc.storage {
            let persisted: HashMap<usize, PersistedPanelInfo> =
                eframe::get_value(storage, "device_panel_info").unwrap_or_default();
            let next_id: usize = eframe::get_value(storage, "next_device_panel_id").unwrap_or(0);

            // Convert persisted panels to runtime structures and create panel widgets
            let mut device_info_map = HashMap::new();
            let mut maitai = HashMap::new();
            let mut power_meter = HashMap::new();
            let mut rotator = HashMap::new();
            let mut stage = HashMap::new();

            for (id, persisted_info) in persisted {
                let device_info: DeviceInfo = persisted_info.clone().into();

                // Create the appropriate panel widget based on capability flags
                if device_info.is_emission_controllable
                    || device_info.is_shutter_controllable
                    || device_info.is_wavelength_tunable
                {
                    maitai.insert(id, MaiTaiControlPanel::default());
                } else if device_info.is_readable && !device_info.is_movable {
                    power_meter.insert(id, PowerMeterControlPanel::default());
                } else if device_info.is_movable {
                    let driver_lower = device_info.driver_type.to_lowercase();
                    if driver_lower.contains("ell14")
                        || driver_lower.contains("rotator")
                        || driver_lower.contains("thorlabs")
                    {
                        rotator.insert(id, RotatorControlPanel::default());
                    } else {
                        stage.insert(id, StageControlPanel::default());
                    }
                } else {
                    stage.insert(id, StageControlPanel::default());
                }

                device_info_map.insert(id, DevicePanelInfo { device_info });
            }

            (
                device_info_map,
                next_id,
                maitai,
                power_meter,
                rotator,
                stage,
            )
        } else {
            (
                HashMap::new(),
                0,
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
            )
        };

        // Initialize dock state and filter out orphaned DeviceControl panels
        let mut dock_state = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_else(Self::default_dock_state)
        } else {
            Self::default_dock_state()
        };

        // Remove DeviceControl panels that have no matching device_panel_info
        // (can happen if storage is corrupted or panels were manually edited)
        let orphaned_ids: Vec<usize> = dock_state
            .iter_all_tabs()
            .filter_map(|(_, tab)| {
                if let Panel::DeviceControl { id } = tab {
                    if !device_panel_info.contains_key(id) {
                        Some(*id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        for id in orphaned_ids {
            dock_state.retain_tabs(
                |tab| !matches!(tab, Panel::DeviceControl { id: panel_id } if *panel_id == id),
            );
        }

        Self {
            client: None,
            connection: ConnectionManager::new(),
            daemon_address,
            address_input,
            address_error: None,
            daemon_version: None,
            gui_version: env!("CARGO_PKG_VERSION").to_string(),
            dock_state: Some(dock_state),
            ui_actions: Vec::new(),
            getting_started_panel: GettingStartedPanel::default(),
            devices_panel: DevicesPanel::default(),
            scripts_panel: ScriptsPanel::default(),
            scans_panel: ScansPanel::default(),
            storage_panel: StoragePanel::default(),
            modules_panel: ModulesPanel::default(),
            plan_runner_panel: PlanRunnerPanel::default(),
            scan_builder_panel: ScanBuilderPanel::default(),
            document_viewer_panel: DocumentViewerPanel::default(),
            instrument_manager_panel: InstrumentManagerPanel::default(),
            signal_plotter_panel: SignalPlotterPanel::new(),
            image_viewer_panel: ImageViewerPanel::new(),
            logging_panel: LoggingPanel::new(),
            runtime,
            health_tx,
            health_rx,
            was_connected: false,
            daemon_mode,
            daemon_launcher,
            auto_connect_state,
            log_receiver,
            theme_preference,
            status_bar: StatusBar::new(),
            device_panel_info,
            next_device_panel_id,
            docked_maitai_panels,
            docked_power_meter_panels,
            docked_rotator_panels,
            docked_stage_panels,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_streaming: false,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_task: None,
        }
    }

    /// Create the default dock layout
    fn default_dock_state() -> DockState<Panel> {
        let mut dock_state = DockState::new(vec![Panel::GettingStarted]);
        let surface = dock_state.main_surface_mut();

        // Split left for Nav
        let [_nav, content] = surface.split_left(NodeIndex::root(), 0.15, vec![Panel::Nav]);

        // Split bottom of content for Logs
        let [_content, _logs] = surface.split_below(content, 0.75, vec![Panel::Logs]);

        dock_state
    }

    /// Attempt to connect to the daemon
    fn connect(&mut self) {
        if self.connection.is_busy() {
            return;
        }

        // Validate and normalize the address input
        match DaemonAddress::parse(&self.address_input, AddressSource::UserInput) {
            Ok(addr) => {
                self.daemon_address = addr;
                self.address_error = None;
            }
            Err(e) => {
                self.address_error = Some(e.to_string());
                self.logging_panel
                    .error("Connection", &format!("Invalid address: {}", e));
                return;
            }
        }

        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
        self.logging_panel.info(
            "Connection",
            &format!(
                "Connecting to {} ({})",
                self.daemon_address,
                self.daemon_address.source().label()
            ),
        );

        self.connection
            .connect(self.daemon_address.clone(), &self.runtime);
    }

    /// Disconnect from the daemon
    fn disconnect(&mut self) {
        self.client = None;
        self.daemon_version = None;
        self.connection.disconnect();
        self.logging_panel.connection_status = LogConnectionStatus::Disconnected;
        self.logging_panel
            .info("Connection", "Disconnected from daemon");
    }

    /// Switch to a different daemon mode
    fn switch_daemon_mode(&mut self, mode: DaemonMode) {
        tracing::info!("Switching daemon mode to: {}", mode.label());

        // Stop existing daemon if we're switching away from LocalAuto
        if let Some(ref mut launcher) = self.daemon_launcher {
            launcher.stop();
        }
        self.daemon_launcher = None;

        // Disconnect current connection
        self.disconnect();

        // Update daemon mode
        self.daemon_mode = mode.clone();

        // Update address
        if let Ok(addr) = DaemonAddress::parse(&mode.daemon_url(), AddressSource::Default) {
            self.daemon_address = addr;
            self.address_input = self.daemon_address.original().to_string();
        }

        // Start new daemon if needed
        if mode.should_auto_start() {
            let port = mode.port().unwrap_or(50051);
            let mut launcher = DaemonLauncher::new(port);
            if let Err(e) = launcher.start() {
                self.logging_panel
                    .error("Daemon", &format!("Failed to start: {}", e));
            }
            self.daemon_launcher = Some(launcher);
            self.auto_connect_state = AutoConnectState::WaitingForDaemon {
                since: Instant::now(),
            };
        } else {
            // For remote mode, try to connect immediately
            self.auto_connect_state = AutoConnectState::ReadyToConnect;
        }

        self.logging_panel
            .info("Daemon", &format!("Switched to {} mode", mode.label()));
    }

    /// Render the top menu bar
    fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                // Daemon menu for mode selection and control
                ui.menu_button("Daemon", |ui| {
                    // Current mode indicator
                    ui.label(format!("Mode: {}", self.daemon_mode.label()));
                    ui.separator();

                    // Mode selection buttons
                    if ui.button("Local (Mock)").clicked() {
                        self.switch_daemon_mode(DaemonMode::LocalAuto { port: 50051 });
                        ui.close();
                    }

                    // Remote connection - use the address input
                    if ui.button("Use Remote Address").clicked() {
                        // Parse current address input as remote URL
                        if let Ok(addr) =
                            DaemonAddress::parse(&self.address_input, AddressSource::UserInput)
                        {
                            self.switch_daemon_mode(DaemonMode::Remote {
                                url: addr.to_string(),
                            });
                        }
                        ui.close();
                    }

                    // Lab Hardware - placeholder for future implementation
                    ui.add_enabled_ui(false, |ui| {
                        ui.button("Lab Hardware (TODO)")
                            .on_hover_text("Not yet implemented");
                    });

                    ui.separator();

                    // Daemon status
                    if let Some(ref mut launcher) = self.daemon_launcher {
                        if launcher.is_running() {
                            ui.colored_label(egui::Color32::GREEN, "● Local daemon running");
                            if let Some(uptime) = launcher.uptime() {
                                ui.small(format!("Uptime: {}s", uptime.as_secs()));
                            }
                            if ui.button("Stop Daemon").clicked() {
                                launcher.stop();
                                self.disconnect();
                                ui.close();
                            }
                        } else {
                            ui.colored_label(egui::Color32::RED, "● Local daemon stopped");
                            if let Some(err) = launcher.last_error() {
                                ui.small(err);
                            }
                            if ui.button("Restart Daemon").clicked() {
                                if let Err(e) = launcher.start() {
                                    self.logging_panel.error("Daemon", &e);
                                } else {
                                    self.auto_connect_state = AutoConnectState::WaitingForDaemon {
                                        since: Instant::now(),
                                    };
                                }
                                ui.close();
                            }
                        }
                    } else {
                        ui.label("Remote mode - no local daemon");
                    }
                });

                if theme::theme_toggle_button(ui, &mut self.theme_preference) {
                    theme::apply_theme(ctx, self.theme_preference);
                }

                ui.menu_button("View", |ui| {
                    if ui.button("Getting Started").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::GettingStarted));
                        ui.close();
                    }
                    if ui.button("Devices").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Devices));
                        ui.close();
                    }
                    if ui.button("Scripts").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scripts));
                        ui.close();
                    }
                    if ui.button("Scans").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scans));
                        ui.close();
                    }
                    if ui.button("Scan Builder").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::ScanBuilder));
                        ui.close();
                    }
                    if ui.button("Storage").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Storage));
                        ui.close();
                    }
                    if ui.button("Modules").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Modules));
                        ui.close();
                    }
                });
            });
        });
    }

    /// Render version mismatch warning (if applicable)
    fn render_version_warning(&self, ctx: &egui::Context) {
        // Only show warning if connected and versions don't match
        if self.connection.state().is_connected() {
            if let Some(ref daemon_ver) = self.daemon_version {
                if daemon_ver != &self.gui_version {
                    egui::TopBottomPanel::top("version_warning")
                        .show_separator_line(false)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(255, 200, 0));
                                ui.label(icons::status::WARNING);
                                ui.label(format!(
                                    "Version mismatch: Daemon {} ≠ GUI {}. Some features may not work correctly.",
                                    daemon_ver, self.gui_version
                                ));
                            });
                            ui.add_space(2.0);
                        });
                }
            }
        }
    }

    /// Render the connection status bar
    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Show auto-connect status if active
                match &self.auto_connect_state {
                    AutoConnectState::WaitingForDaemon { since } => {
                        ui.spinner();
                        ui.label(format!(
                            "Starting daemon... ({:.0}s)",
                            since.elapsed().as_secs_f64()
                        ));
                        ui.separator();
                        ui.label(format!("Mode: {}", self.daemon_mode.label()));
                        return; // Don't show rest of status bar during startup
                    }
                    AutoConnectState::ReadyToConnect => {
                        ui.spinner();
                        ui.label("Connecting...");
                        ui.separator();
                        ui.label(format!("Mode: {}", self.daemon_mode.label()));
                        return; // Don't show rest of status bar during startup
                    }
                    AutoConnectState::Complete | AutoConnectState::Skipped => {
                        // Continue with normal status bar
                    }
                }

                // Extract state info upfront to avoid borrow conflicts
                let state_color = self.connection.state().color();
                let state_label = self.connection.state().label();
                let is_connected = self.connection.state().is_connected();
                let is_connecting = self.connection.state().is_connecting();
                let is_disconnected =
                    matches!(self.connection.state(), ConnectionState::Disconnected);
                let error_info = match self.connection.state() {
                    ConnectionState::Error { message, retriable } => {
                        Some((message.clone(), *retriable))
                    }
                    _ => None,
                };
                let seconds_until_retry = self.connection.seconds_until_retry();

                // Connection status indicator
                ui.colored_label(state_color, "●");
                ui.label(state_label);

                // Show reconnect countdown if reconnecting
                if let Some(secs) = seconds_until_retry {
                    ui.label(format!("({:.0}s)", secs));
                }

                ui.separator();

                // Address input with source indicator
                ui.label("Daemon:");

                // Show source as tooltip on the label
                let source_label = format!("[{}]", self.daemon_address.source().label());
                ui.label(
                    egui::RichText::new(&source_label)
                        .small()
                        .color(egui::Color32::GRAY),
                )
                .on_hover_text(format!("Source: {}", self.daemon_address.source()));

                // Text input - show with error highlight if invalid
                let text_color = if self.address_error.is_some() {
                    Some(egui::Color32::RED)
                } else {
                    None
                };
                let mut text_edit = egui::TextEdit::singleline(&mut self.address_input)
                    .hint_text("http://127.0.0.1:50051");
                if let Some(color) = text_color {
                    text_edit = text_edit.text_color(color);
                }
                let response = ui.add_sized([200.0, 18.0], text_edit);

                // Check for Enter key press before potentially consuming response
                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                // Show resolved URL as tooltip when connected
                if is_connected {
                    response.on_hover_text(format!("Resolved: {}", self.daemon_address.as_str()));
                }

                // Connect/Disconnect/Cancel buttons based on state
                if is_disconnected {
                    if ui.button("Connect").clicked() || enter_pressed {
                        self.connect();
                    }
                } else if let Some((_, retriable)) = &error_info {
                    if *retriable {
                        if ui.button("Retry").clicked() || enter_pressed {
                            self.connection
                                .retry(self.daemon_address.clone(), &self.runtime);
                            self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                        }
                    } else if ui.button("Connect").clicked() || enter_pressed {
                        self.connect();
                    }
                } else if is_connected {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                } else if is_connecting {
                    if ui.button("Cancel").clicked() {
                        self.connection.cancel();
                        self.logging_panel.connection_status = LogConnectionStatus::Disconnected;
                        self.logging_panel
                            .info("Connection", "Connection attempt cancelled");
                    }
                    ui.spinner();
                }

                // Show validation error
                if let Some(ref err) = self.address_error {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, err);
                }
                // Show connection error with friendly message
                else if let Some((err_msg, _)) = &error_info {
                    ui.separator();
                    let friendly = friendly_error_message(err_msg);
                    ui.colored_label(egui::Color32::RED, &friendly)
                        .on_hover_text(format!("Raw error: {}", err_msg)); // Show raw error on hover
                }
            });
        });
    }

    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    fn start_pvcam_stream(&mut self) {
        use daq_core::capabilities::{FrameObserver, FrameProducer};
        use daq_core::data::FrameView;
        use daq_driver_pvcam::PvcamDriver;
        use rerun::archetypes::Tensor;
        use rerun::RecordingStreamBuilder;
        use std::sync::atomic::{AtomicU64, Ordering};

        /// Frame data with dimensions for channel transport
        struct PreviewFrame {
            data: Vec<u8>,
            width: u32,
            height: u32,
        }

        /// Observer that sends frame copies to Rerun for GUI preview (bd-0dax.6.2)
        ///
        /// Implements the FrameObserver pattern for tap-based frame delivery.
        /// Uses a bounded channel with try_send to avoid blocking the frame loop.
        struct RerunPreviewObserver {
            tx: tokio::sync::mpsc::Sender<PreviewFrame>,
            /// Counter for decimation (send every Nth frame)
            counter: AtomicU64,
            /// Decimation interval (1 = every frame, 10 = every 10th)
            decimation: u64,
        }

        impl FrameObserver for RerunPreviewObserver {
            fn on_frame(&self, frame: &FrameView<'_>) {
                // Only process 16-bit frames
                if frame.bit_depth != 16 {
                    return;
                }

                // Decimation: skip frames based on interval
                let count = self.counter.fetch_add(1, Ordering::Relaxed);
                if count % self.decimation != 0 {
                    return;
                }

                // Non-blocking send with copy (taps must copy, not hold references)
                if let Ok(permit) = self.tx.try_reserve() {
                    permit.send(PreviewFrame {
                        data: frame.pixels().to_vec(),
                        width: frame.width,
                        height: frame.height,
                    });
                }
                // If channel is full, we just drop this frame (backpressure)
            }

            fn name(&self) -> &str {
                "rerun_preview"
            }
        }

        let handle = self.runtime.handle().clone();
        self.pvcam_task = Some(handle.spawn(async move {
            // Connect PVCAM driver and open rerun stream
            let driver = match PvcamDriver::new_async("PrimeBSI".to_string()).await {
                Ok(d) => d,
                Err(err) => {
                    eprintln!("PVCAM init failed: {err}");
                    return;
                }
            };

            // Create channel for frame data (bounded to prevent memory buildup)
            let (tx, mut rx) = tokio::sync::mpsc::channel::<PreviewFrame>(4);

            // Create observer
            let observer = RerunPreviewObserver {
                tx,
                counter: AtomicU64::new(0),
                decimation: 1, // Send every frame (adjust for lower preview FPS)
            };

            // Register the observer using the tap system (replaces deprecated subscribe_frames)
            let observer_handle = match driver.register_observer(Box::new(observer)).await {
                Ok(h) => h,
                Err(err) => {
                    eprintln!("Failed to register frame observer: {err}");
                    return;
                }
            };

            if let Err(err) = driver.start_stream().await {
                eprintln!("PVCAM start_stream failed: {err}");
                let _ = driver.unregister_observer(observer_handle).await;
                return;
            }

            // Spawn viewer or connect to existing one
            let rec = match RecordingStreamBuilder::new("PVCAM Live").spawn() {
                Ok(r) => r,
                Err(err) => {
                    eprintln!("Failed to spawn rerun viewer: {err}");
                    let _ = driver.stop_stream().await;
                    let _ = driver.unregister_observer(observer_handle).await;
                    return;
                }
            };

            // Process frames from the observer channel
            while let Some(frame) = rx.recv().await {
                // Convert raw bytes to u16 slice and create tensor
                let u16_data: &[u16] = bytemuck::cast_slice(&frame.data);
                let shape = vec![frame.height as u64, frame.width as u64];
                let tensor_data = rerun::TensorData::new(
                    shape,
                    rerun::TensorBuffer::U16(u16_data.to_vec().into()),
                );
                let tensor = Tensor::new(tensor_data);
                let _ = rec.log("/pvcam/image", &tensor);
            }

            // Cleanup
            let _ = driver.stop_stream().await;
            let _ = driver.unregister_observer(observer_handle).await;
        }));

        self.pvcam_streaming = true;
    }

    fn poll_logs(&mut self) {
        // Drain all pending log events from the channel
        while let Ok(event) = self.log_receiver.try_recv() {
            self.logging_panel
                .log(event.level, &event.target, &event.message);
        }
    }
}

/// Additional DaqApp methods in a separate impl block (split for helper functions)
impl DaqApp {
    /// Remove all state associated with a device control panel.
    ///
    /// Returns the removed DevicePanelInfo if the panel existed, None otherwise.
    /// Used for cleanup when panels are closed or during app shutdown.
    pub(crate) fn remove_panel_data(&mut self, id: usize) -> Option<DevicePanelInfo> {
        // Remove from all panel-type-specific maps
        self.docked_maitai_panels.remove(&id);
        self.docked_power_meter_panels.remove(&id);
        self.docked_rotator_panels.remove(&id);
        self.docked_stage_panels.remove(&id);

        // Remove and return the panel info
        self.device_panel_info.remove(&id)
    }

    fn poll_connect_results(&mut self, ctx: &egui::Context) {
        // Poll connection manager for results
        if let Some((client, daemon_version)) =
            self.connection.poll(&self.runtime, &self.daemon_address)
        {
            self.client = Some(client);
            self.daemon_version = daemon_version.clone();
            self.logging_panel.connection_status = LogConnectionStatus::Connected;
            self.logging_panel.info(
                "Connection",
                &format!(
                    "Connected to {} ({})",
                    self.daemon_address.as_str(),
                    self.daemon_address.source().label()
                ),
            );

            // Log version info
            match daemon_version {
                Some(ref daemon_ver) => {
                    tracing::info!(
                        "Daemon version: {}, GUI version: {}",
                        daemon_ver,
                        self.gui_version
                    );
                    if daemon_ver != &self.gui_version {
                        tracing::warn!(
                            "Version mismatch detected! Daemon: {}, GUI: {}. Some features may not work correctly.",
                            daemon_ver,
                            self.gui_version
                        );
                    }
                }
                None => {
                    tracing::warn!("Connected but failed to get daemon version");
                }
            }
        }

        // Update logging panel status based on connection state
        match self.connection.state() {
            ConnectionState::Error { .. } => {
                if self.logging_panel.connection_status != LogConnectionStatus::Error {
                    self.logging_panel.connection_status = LogConnectionStatus::Error;
                    if let Some(err) = self.connection.state().error_message() {
                        self.logging_panel
                            .error("Connection", &format!("Connection failed: {}", err));
                    }
                }
            }
            ConnectionState::Reconnecting { attempt, .. } => {
                self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                if let Some(err) = self.connection.state().error_message() {
                    self.logging_panel.warn(
                        "Connection",
                        &format!("Reconnecting (attempt {}): {}", attempt, err),
                    );
                }
            }
            _ => {}
        }

        // Request repaint if connection attempt is in progress
        if self.connection.is_busy() || self.connection.seconds_until_retry().is_some() {
            ctx.request_repaint();
        }
    }

    /// Check if a health check should be spawned and spawn it.
    fn maybe_spawn_health_check(&mut self) {
        if !self.connection.should_health_check() {
            return;
        }
        let Some(ref client) = self.client else {
            return;
        };

        // Mark health check as started
        self.connection.mark_health_check_started();

        // Clone what we need for the async task
        let mut client = client.clone();
        let tx = self.health_tx.clone();

        self.runtime.spawn(async move {
            // Measure RTT for the health check (bd-j3xz.3.3)
            let start = std::time::Instant::now();
            match client.health_check().await {
                Ok(()) => {
                    let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let _ = tx.send(HealthCheckResult::Success { rtt_ms }).await;
                }
                Err(e) => {
                    let _ = tx.send(HealthCheckResult::Failed(e.to_string())).await;
                }
            }
        });
    }

    /// Poll for health check results.
    fn poll_health_checks(&mut self) {
        while let Ok(result) = self.health_rx.try_recv() {
            match result {
                HealthCheckResult::Success { rtt_ms } => {
                    self.connection.record_health_success(rtt_ms);
                }
                HealthCheckResult::Failed(error) => {
                    let should_reconnect = self.connection.record_health_failure(&error);

                    if should_reconnect {
                        // Clear client - connection is stale
                        self.client = None;
                        self.daemon_version = None;
                        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                        self.logging_panel.warn(
                            "Connection",
                            &format!("Connection lost ({}), reconnecting...", error),
                        );

                        // Trigger reconnect
                        self.connection
                            .trigger_health_reconnect(self.daemon_address.clone(), &self.runtime);
                    }
                }
            }
        }
    }

    /// Update the logging panel's connection diagnostics from the ConnectionManager (bd-j3xz.3.3).
    fn update_connection_diagnostics(&mut self) {
        let health_status = self.connection.health_status();

        // Calculate relative times
        let secs_since_last_success = health_status
            .last_success
            .map(|t| t.elapsed().as_secs_f64());
        let secs_since_last_error = health_status
            .last_error_at
            .map(|t| t.elapsed().as_secs_f64());

        self.logging_panel.connection_diagnostics = ConnectionDiagnostics {
            last_rtt_ms: health_status.last_rtt_ms,
            total_errors: health_status.total_errors,
            secs_since_last_error,
            last_error_message: health_status.last_error_message.clone(),
            secs_since_last_success,
            consecutive_failures: health_status.consecutive_failures,
        };
    }

    /// Process auto-connect state machine
    fn process_auto_connect(&mut self, ctx: &egui::Context) {
        use std::time::Duration;

        match &self.auto_connect_state {
            AutoConnectState::WaitingForDaemon { since } => {
                let elapsed = since.elapsed();

                // Check if daemon process has started
                if let Some(ref mut launcher) = self.daemon_launcher {
                    if launcher.is_running() && elapsed > Duration::from_millis(500) {
                        // Give daemon time to start listening
                        tracing::info!("Daemon is running, initiating auto-connect");
                        self.auto_connect_state = AutoConnectState::ReadyToConnect;
                    } else if elapsed > Duration::from_secs(10) {
                        // Timeout - daemon didn't start
                        tracing::error!("Timeout waiting for daemon to start");
                        self.auto_connect_state = AutoConnectState::Skipped;
                        self.logging_panel
                            .error("Daemon", "Timeout waiting for daemon to start");
                    }
                } else {
                    // No launcher but in WaitingForDaemon - shouldn't happen, skip
                    self.auto_connect_state = AutoConnectState::Skipped;
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            AutoConnectState::ReadyToConnect => {
                if !self.connection.is_busy() {
                    tracing::info!("Auto-connecting to daemon at {}", self.daemon_address);
                    self.connect();
                    self.auto_connect_state = AutoConnectState::Complete;
                }
            }
            AutoConnectState::Complete | AutoConnectState::Skipped => {
                // No action needed
            }
        }
    }

    /// Called when connection is established - trigger panel refreshes
    fn on_connection_established(&mut self) {
        tracing::info!("Connection established - triggering panel refreshes");

        // Reset panels to force them to refresh their data
        // This clears cached data and triggers new loads on next render
        self.devices_panel = DevicesPanel::default();
        self.scripts_panel = ScriptsPanel::default();
        self.modules_panel = ModulesPanel::default();
        self.storage_panel = StoragePanel::default();

        self.logging_panel
            .info("Connection", "Connected - panels will refresh data");
    }

    /// Detect connection state transitions and handle them
    fn detect_connection_transitions(&mut self) {
        let is_connected = self.connection.state().is_connected();

        if is_connected && !self.was_connected {
            // Just connected - trigger panel refreshes
            self.on_connection_established();
        }

        self.was_connected = is_connected;
    }
}

struct DaqTabViewer<'a> {
    app: &'a mut DaqApp,
}

impl<'a> TabViewer for DaqTabViewer<'a> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Panel::Nav => "Navigation".into(),
            Panel::GettingStarted => {
                format!("{} Getting Started", icons::nav::GETTING_STARTED).into()
            }
            Panel::Instruments => format!("{} Instruments", icons::nav::INSTRUMENT_MANAGER).into(),
            Panel::Devices => format!("{} Devices", icons::nav::DEVICES).into(),
            Panel::Scripts => format!("{} Scripts", icons::nav::SCRIPTS).into(),
            Panel::Scans => format!("{} Scans", icons::nav::SCANS).into(),
            Panel::ScanBuilder => "Scan Builder".into(),
            Panel::Storage => format!("{} Storage", icons::nav::STORAGE).into(),
            Panel::Modules => format!("{} Modules", icons::nav::MODULES).into(),
            Panel::PlanRunner => format!("{} Plan Runner", icons::nav::PLAN_RUNNER).into(),
            Panel::DocumentViewer => format!("{} Documents", icons::nav::DOCUMENT_VIEWER).into(),
            Panel::SignalPlotter => format!("{} Signal Plotter", icons::nav::SIGNAL_PLOTTER).into(),
            Panel::ImageViewer => format!("{} Image Viewer", icons::nav::IMAGE_VIEWER).into(),
            Panel::Logs => format!("{} Logs", icons::nav::LOGGING).into(),
            Panel::DeviceControl { id } => {
                // Look up device name from the panel ID mapping
                if let Some(info) = self.app.device_panel_info.get(id) {
                    format!("🎛 {}", info.device_info.name).into()
                } else {
                    "🎛 Device".into()
                }
            }
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        !matches!(tab, Panel::Nav)
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> OnCloseResponse {
        // Clean up device panel state when a DeviceControl tab is closed
        if let Panel::DeviceControl { id } = tab {
            self.app.remove_panel_data(*id);
        }
        OnCloseResponse::Close
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Panel::Nav => self.render_nav(ui),
            Panel::GettingStarted => self.app.getting_started_panel.ui(ui),
            Panel::Instruments => self.app.instrument_manager_panel.ui(
                ui,
                self.app.client.as_mut(),
                &self.app.runtime,
            ),
            Panel::Devices => {
                self.app
                    .devices_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::Scripts => {
                self.app
                    .scripts_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::Scans => {
                self.app
                    .scans_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::ScanBuilder => {
                self.app
                    .scan_builder_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::Storage => {
                self.app
                    .storage_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::Modules => {
                self.app
                    .modules_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::PlanRunner => {
                self.app
                    .plan_runner_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::DocumentViewer => {
                self.app
                    .document_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::SignalPlotter => {
                self.app.signal_plotter_panel.drain_updates();
                self.app.signal_plotter_panel.ui(ui);
            }
            Panel::ImageViewer => {
                self.app
                    .image_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime)
            }
            Panel::Logs => self.app.logging_panel.ui(ui),
            Panel::DeviceControl { id } => {
                self.render_device_control(ui, *id);
            }
        }
    }
}

impl<'a> DaqTabViewer<'a> {
    fn nav_button(&mut self, ui: &mut egui::Ui, icon: &str, label: &str, panel: Panel) {
        let text = format!("{} {}", icon, label);
        if ui.button(text).clicked() {
            self.app.ui_actions.push(UiAction::FocusTab(panel));
        }
    }

    fn section_label(ui: &mut egui::Ui, text: &str) {
        ui.add_space(layout::SECTION_SPACING / 2.0);
        ui.label(
            egui::RichText::new(text)
                .small()
                .color(layout::colors::MUTED),
        );
    }

    fn render_nav(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Navigation");
            ui.separator();

            self.nav_button(
                ui,
                icons::nav::GETTING_STARTED,
                "Getting Started",
                Panel::GettingStarted,
            );

            Self::section_label(ui, "Hardware");
            self.nav_button(
                ui,
                icons::nav::INSTRUMENT_MANAGER,
                "Instruments",
                Panel::Instruments,
            );
            self.nav_button(ui, icons::nav::DEVICES, "Devices", Panel::Devices);

            Self::section_label(ui, "Visualization");
            self.nav_button(
                ui,
                icons::nav::SIGNAL_PLOTTER,
                "Signal Plotter",
                Panel::SignalPlotter,
            );
            self.nav_button(
                ui,
                icons::nav::IMAGE_VIEWER,
                "Image Viewer",
                Panel::ImageViewer,
            );

            Self::section_label(ui, "Experiment");
            self.nav_button(ui, icons::nav::SCRIPTS, "Scripts", Panel::Scripts);
            self.nav_button(ui, icons::nav::SCANS, "Scans", Panel::Scans);
            self.nav_button(ui, icons::nav::SCANS, "Scan Builder", Panel::ScanBuilder);
            self.nav_button(
                ui,
                icons::nav::PLAN_RUNNER,
                "Plan Runner",
                Panel::PlanRunner,
            );

            Self::section_label(ui, "Data");
            self.nav_button(ui, icons::nav::STORAGE, "Storage", Panel::Storage);
            self.nav_button(
                ui,
                icons::nav::DOCUMENT_VIEWER,
                "Documents",
                Panel::DocumentViewer,
            );

            Self::section_label(ui, "System");
            self.nav_button(ui, icons::nav::MODULES, "Modules", Panel::Modules);
            self.nav_button(ui, icons::nav::LOGGING, "Logs", Panel::Logs);

            ui.separator();
            ui.add_space(layout::SECTION_SPACING / 2.0);

            if ui
                .button(format!("{} Open Rerun", icons::CHART_LINE))
                .clicked()
            {
                let _ = std::process::Command::new("rerun").spawn();
            }

            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            {
                ui.add_space(layout::SECTION_SPACING / 2.0);
                let (icon, label) = if self.app.pvcam_streaming {
                    (icons::action::STOP, "Stop PVCAM Live")
                } else {
                    (icons::action::RECORD, "PVCAM Live to Rerun")
                };
                if ui.button(format!("{} {}", icon, label)).clicked() {
                    if self.app.pvcam_streaming {
                        if let Some(task) = self.app.pvcam_task.take() {
                            task.abort();
                        }
                        self.app.pvcam_streaming = false;
                    } else {
                        self.app.start_pvcam_stream();
                    }
                }
            }
        });
    }

    /// Render a docked device control panel
    fn render_device_control(&mut self, ui: &mut egui::Ui, panel_id: usize) {
        // Get device info for this panel (stored with full capability flags)
        let Some(info) = self.app.device_panel_info.get(&panel_id).cloned() else {
            ui.label("Device panel not found");
            return;
        };

        let device_info = &info.device_info;

        // Render the appropriate panel widget based on which HashMap contains the panel
        // (panel type was determined at creation time using capability flags)
        // Use push_id to avoid widget ID collisions with instrument manager panels
        ui.push_id(("docked", panel_id), |ui| {
            if let Some(panel) = self.app.docked_maitai_panels.get_mut(&panel_id) {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            } else if let Some(panel) = self.app.docked_power_meter_panels.get_mut(&panel_id) {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            } else if let Some(panel) = self.app.docked_rotator_panels.get_mut(&panel_id) {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            } else if let Some(panel) = self.app.docked_stage_panels.get_mut(&panel_id) {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            }
        });
    }
}

impl eframe::App for DaqApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_logs();
        self.poll_connect_results(ctx);
        self.maybe_spawn_health_check();
        self.poll_health_checks();
        self.update_connection_diagnostics(); // bd-j3xz.3.3

        // Process auto-connect state machine
        self.process_auto_connect(ctx);

        // Detect connection state transitions (for panel refresh on connect)
        self.detect_connection_transitions();

        self.render_menu_bar(ctx);
        self.render_version_warning(ctx);
        self.render_status_bar(ctx);

        let error_count = self.connection.health_status().total_errors;
        let error_count = if error_count > 0 {
            Some(error_count)
        } else {
            None
        };
        self.status_bar
            .show(ctx, self.connection.state(), error_count);

        // Render Dock Area
        let mut dock_state = self
            .dock_state
            .take()
            .unwrap_or_else(Self::default_dock_state);
        let mut viewer = DaqTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);

        // Check for pop-out requests from InstrumentManagerPanel
        if let Some(request) = self.instrument_manager_panel.take_pop_out_request() {
            self.ui_actions.push(UiAction::OpenDeviceControl {
                device_info: request.device_info,
            });
        }

        // Process deferred UI actions
        for action in self.ui_actions.drain(..) {
            match action {
                UiAction::FocusTab(panel) => {
                    if let Some((surface, node, tab)) = dock_state.find_tab(&panel) {
                        dock_state.set_active_tab((surface, node, tab));
                        dock_state.set_focused_node_and_surface((surface, node));
                    } else {
                        // Add to focused leaf or fallback to root
                        dock_state.main_surface_mut().push_to_focused_leaf(panel);
                    }
                }
                UiAction::OpenDeviceControl { device_info } => {
                    // Generate a new panel ID with wrapping on overflow
                    // (practically impossible to hit 2^64 panels, but safe regardless)
                    let panel_id = self.next_device_panel_id;
                    self.next_device_panel_id = self.next_device_panel_id.wrapping_add(1);

                    // Store device info (full proto with capability flags)
                    self.device_panel_info.insert(
                        panel_id,
                        DevicePanelInfo {
                            device_info: device_info.clone(),
                        },
                    );

                    // Create the appropriate panel widget based on capability flags
                    // Priority: laser controls > readable (power meter) > movable (rotator/stage)
                    if device_info.is_emission_controllable
                        || device_info.is_shutter_controllable
                        || device_info.is_wavelength_tunable
                    {
                        // Laser with full control capabilities
                        self.docked_maitai_panels
                            .insert(panel_id, MaiTaiControlPanel::default());
                    } else if device_info.is_readable && !device_info.is_movable {
                        // Pure readable device (power meter, sensor)
                        self.docked_power_meter_panels
                            .insert(panel_id, PowerMeterControlPanel::default());
                    } else if device_info.is_movable {
                        // Check driver_type for rotator vs stage distinction
                        // (both are movable, but rotators have different UI)
                        let driver_lower = device_info.driver_type.to_lowercase();
                        if driver_lower.contains("ell14")
                            || driver_lower.contains("rotator")
                            || driver_lower.contains("thorlabs")
                        {
                            self.docked_rotator_panels
                                .insert(panel_id, RotatorControlPanel::default());
                        } else {
                            self.docked_stage_panels
                                .insert(panel_id, StageControlPanel::default());
                        }
                    } else {
                        // Fallback to stage panel for unknown devices
                        self.docked_stage_panels
                            .insert(panel_id, StageControlPanel::default());
                    }

                    // Add the panel to the dock
                    let panel = Panel::DeviceControl { id: panel_id };
                    dock_state.main_surface_mut().push_to_focused_leaf(panel);
                }
            }
        }

        self.dock_state = Some(dock_state);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.connection.state().is_connected() {
            save_to_storage(storage, &self.daemon_address);
        }

        if let Some(dock_state) = &self.dock_state {
            eframe::set_value(storage, eframe::APP_KEY, dock_state);
        }

        eframe::set_value(storage, "theme_preference", &self.theme_preference);

        // Persist device panel info for layout restoration
        let persisted_panels: HashMap<usize, PersistedPanelInfo> = self
            .device_panel_info
            .iter()
            .map(|(id, info)| (*id, PersistedPanelInfo::from(&info.device_info)))
            .collect();
        eframe::set_value(storage, "device_panel_info", &persisted_panels);
        eframe::set_value(storage, "next_device_panel_id", &self.next_device_panel_id);
    }
}

impl Drop for DaqApp {
    fn drop(&mut self) {
        tracing::debug!("DaqApp shutting down, cleaning up device panel state");

        // Collect panel IDs to avoid borrow conflicts during cleanup
        let panel_ids: Vec<usize> = self.device_panel_info.keys().copied().collect();

        // Clean up all device panel state
        for id in panel_ids {
            self.remove_panel_data(id);
        }

        // Shutdown daemon launcher if running
        if let Some(launcher) = self.daemon_launcher.take() {
            drop(launcher); // DaemonLauncher should have its own Drop that terminates the process
        }

        tracing::debug!("DaqApp shutdown complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_serialization() {
        let panel = Panel::Nav;
        let serialized = serde_json::to_string(&panel).unwrap();
        assert_eq!(serialized, "\"Nav\"");

        let deserialized: Panel = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, Panel::Nav);
    }

    #[test]
    fn test_default_dock_layout() {
        let dock_state = DaqApp::default_dock_state();

        let mut found_nav = false;
        let mut found_logs = false;
        let mut found_getting_started = false;

        for ((_surface, _node), tab) in dock_state.iter_all_tabs() {
            match tab {
                Panel::Nav => found_nav = true,
                Panel::Logs => found_logs = true,
                Panel::GettingStarted => found_getting_started = true,
                _ => {}
            }
        }

        assert!(found_nav, "Navigation panel missing from default layout");
        assert!(found_logs, "Logs panel missing from default layout");
        assert!(
            found_getting_started,
            "Getting Started panel missing from default layout"
        );
    }
}
