//! Main application state and UI logic.

use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

struct UiLogWriter {
    buf: Arc<Mutex<Vec<String>>>,
    cur: String,
}

impl std::io::Write for UiLogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for &b in bytes {
            if b == b'\n' {
                let mut guard = self.buf.lock().unwrap();
                guard.push(self.cur.clone());
                self.cur.clear();
            } else {
                self.cur.push(b as char);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct UiLogMakeWriter {
    buf: Arc<Mutex<Vec<String>>>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for UiLogMakeWriter {
    type Writer = UiLogWriter;
    fn make_writer(&'a self) -> Self::Writer {
        UiLogWriter {
            buf: self.buf.clone(),
            cur: String::new(),
        }
    }
}

static UI_LOG_BUFFER: Lazy<Arc<Mutex<Vec<String>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

use std::time::Instant;

use crate::client::DaqClient;
use crate::connection::{resolve_address, save_to_storage, AddressSource, DaemonAddress};
use crate::daemon_launcher::{AutoConnectState, DaemonLauncher, DaemonMode};
use crate::panels::{
    ConnectionDiagnostics, ConnectionStatus as LogConnectionStatus, DevicesPanel,
    DocumentViewerPanel, GettingStartedPanel, ImageViewerPanel, InstrumentManagerPanel,
    LoggingPanel, ModulesPanel, PlanRunnerPanel, ScansPanel, ScriptsPanel, SignalPlotterPanel,
    StoragePanel,
};
use crate::reconnect::{friendly_error_message, ConnectionManager, ConnectionState};

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

    /// PVCAM live view streaming state (requires rerun_viewer + instrument_photometrics)
    /// Works in mock mode without pvcam_hardware, or with real SDK when pvcam_hardware enabled
    #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
    pvcam_streaming: bool,
    #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
    pvcam_task: Option<tokio::task::JoinHandle<()>>,
}

/// Action to perform on the UI state
enum UiAction {
    FocusTab(Panel),
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
    Storage,
    Modules,
    PlanRunner,
    DocumentViewer,
    SignalPlotter,
    ImageViewer,
    Logs,
}

impl DaqApp {
    /// Create a new application instance with the specified daemon mode
    pub fn new(cc: &eframe::CreationContext<'_>, daemon_mode: DaemonMode) -> Self {
        // Configure egui style
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        cc.egui_ctx.set_style(style);

        // Create tokio runtime for gRPC calls
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");
        // Install tracing subscriber that also feeds the UI log buffer (only once).
        static SUB_INIT: std::sync::Once = std::sync::Once::new();
        SUB_INIT.call_once(|| {
            let writer = UiLogMakeWriter {
                buf: UI_LOG_BUFFER.clone(),
            }
            .and(std::io::stdout);

            let _ = tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()),
                )
                .with_writer(writer)
                .try_init();
        });

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

        // Initialize dock state
        let dock_state = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_else(Self::default_dock_state)
        } else {
            Self::default_dock_state()
        };

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
            #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
            pvcam_streaming: false,
            #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
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
        if let Ok(addr) =
            DaemonAddress::parse(&mode.daemon_url(), AddressSource::Default)
        {
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
            egui::menu::bar(ui, |ui| {
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
                        ui.close_menu();
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
                        ui.close_menu();
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
                                ui.close_menu();
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
                                ui.close_menu();
                            }
                        }
                    } else {
                        ui.label("Remote mode - no local daemon");
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.button("Getting Started").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::GettingStarted));
                        ui.close_menu();
                    }
                    if ui.button("Devices").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Devices));
                        ui.close_menu();
                    }
                    if ui.button("Scripts").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scripts));
                        ui.close_menu();
                    }
                    if ui.button("Scans").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scans));
                        ui.close_menu();
                    }
                    if ui.button("Storage").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Storage));
                        ui.close_menu();
                    }
                    if ui.button("Modules").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Modules));
                        ui.close_menu();
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
                                ui.label("⚠");
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

    #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
    fn start_pvcam_stream(&mut self) {
        use daq_core::capabilities::FrameProducer;
        use daq_driver_pvcam::PvcamDriver;
        use rerun::archetypes::Tensor;
        use rerun::RecordingStreamBuilder;

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

            let mut rx = match driver.subscribe_frames().await {
                Some(r) => r,
                None => {
                    eprintln!("PVCAM frame subscription unavailable");
                    return;
                }
            };

            if let Err(err) = driver.start_stream().await {
                eprintln!("PVCAM start_stream failed: {err}");
                return;
            }

            // Spawn viewer or connect to existing one
            let rec = match RecordingStreamBuilder::new("PVCAM Live").spawn() {
                Ok(r) => r,
                Err(err) => {
                    eprintln!("Failed to spawn rerun viewer: {err}");
                    let _ = driver.stop_stream().await;
                    return;
                }
            };

            while let Ok(frame_arc) = rx.recv().await {
                let frame = frame_arc.as_ref();
                if frame.bit_depth != 16 {
                    continue;
                }
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

            let _ = driver.stop_stream().await;
        }));

        self.pvcam_streaming = true;
    }

    fn poll_logs(&mut self) {
        if let Ok(mut buf) = UI_LOG_BUFFER.lock() {
            for line in buf.drain(..) {
                // Strip ANSI escape codes: \x1b[...m
                let stripped = strip_ansi_codes(&line);

                // Parse tracing format: "2026-01-08T08:51:10.101171Z INFO rust_daq_gui: message"
                // Or simpler: "INFO rust_daq_gui: message"
                let (level, source, message) = parse_tracing_log(&stripped);
                self.logging_panel.log(level, source, message);
            }
        }
    }
}

/// Strip ANSI escape codes from a string
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit 'm' (end of ANSI sequence)
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(nc) = chars.next() {
                    if nc == 'm' {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse a tracing log line into (level, source, message)
fn parse_tracing_log(line: &str) -> (crate::panels::LogLevel, &str, &str) {
    use crate::panels::LogLevel;

    // Try to find level keyword in the line
    let line = line.trim();

    // Handle format with timestamp: "2026-01-08T... INFO source: message"
    // Or without: "INFO source: message"
    let level_keywords = [
        ("ERROR", LogLevel::Error),
        ("WARN", LogLevel::Warn),
        ("INFO", LogLevel::Info),
        ("DEBUG", LogLevel::Debug),
        ("TRACE", LogLevel::Trace),
    ];

    for (keyword, level) in level_keywords {
        // Find the keyword, potentially preceded by timestamp
        if let Some(idx) = line.find(keyword) {
            let after_level = &line[idx + keyword.len()..].trim_start();

            // Parse "source: message" or just "message"
            if let Some(colon_idx) = after_level.find(':') {
                let source = after_level[..colon_idx].trim();
                let message = after_level[colon_idx + 1..].trim();

                // Clean up source (remove common prefixes)
                let source = source
                    .strip_prefix("rust_daq_gui::")
                    .or_else(|| source.strip_prefix("daq_egui::"))
                    .unwrap_or(source);

                return (level, source, message);
            } else {
                return (level, "tracing", after_level);
            }
        }
    }

    // Fallback: couldn't parse, return as-is
    (LogLevel::Info, "tracing", line)
}

/// Additional DaqApp methods in a separate impl block (split for helper functions)
impl DaqApp {
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
        if self.client.is_none() {
            return;
        }

        // Mark health check as started
        self.connection.mark_health_check_started();

        // Clone what we need for the async task
        let mut client = self.client.clone().unwrap();
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
            Panel::GettingStarted => "🚀 Getting Started".into(),
            Panel::Instruments => "🔬 Instruments".into(),
            Panel::Devices => "🔧 Devices".into(),
            Panel::Scripts => "📜 Scripts".into(),
            Panel::Scans => "📊 Scans".into(),
            Panel::Storage => "💾 Storage".into(),
            Panel::Modules => "🧩 Modules".into(),
            Panel::PlanRunner => "🎯 Plan Runner".into(),
            Panel::DocumentViewer => "📄 Documents".into(),
            Panel::SignalPlotter => "📈 Signal Plotter".into(),
            Panel::ImageViewer => "🖼 Image Viewer".into(),
            Panel::Logs => "🪵 Logs".into(),
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        !matches!(tab, Panel::Nav)
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
            Panel::DocumentViewer => self
                .app
                .document_viewer_panel
                .ui(ui, self.app.client.as_mut()),
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
        }
    }
}

impl<'a> DaqTabViewer<'a> {
    fn render_nav(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Navigation");
            ui.separator();

            // Getting Started
            if ui.button("🚀 Getting Started").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::GettingStarted));
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Hardware")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if ui.button("🔬 Instruments").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::Instruments));
            }
            if ui.button("🔧 Devices").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Devices));
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Visualization")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if ui.button("📈 Signal Plotter").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::SignalPlotter));
            }
            if ui.button("🖼 Image Viewer").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::ImageViewer));
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Experiment")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if ui.button("📜 Scripts").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Scripts));
            }
            if ui.button("📊 Scans").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Scans));
            }
            if ui.button("🎯 Plan Runner").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::PlanRunner));
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Data")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if ui.button("💾 Storage").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Storage));
            }
            if ui.button("📄 Documents").clicked() {
                self.app
                    .ui_actions
                    .push(UiAction::FocusTab(Panel::DocumentViewer));
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("System")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if ui.button("🧩 Modules").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Modules));
            }
            if ui.button("🪵 Logs").clicked() {
                self.app.ui_actions.push(UiAction::FocusTab(Panel::Logs));
            }

            ui.separator();
            ui.add_space(8.0);

            // Rerun visualization button
            if ui.button("📈 Open Rerun").clicked() {
                // Launch Rerun viewer
                let _ = std::process::Command::new("rerun").spawn();
            }

            // PVCAM live view via Rerun
            #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
            {
                ui.add_space(8.0);
                let label = if self.app.pvcam_streaming {
                    "🛑 Stop PVCAM Live"
                } else {
                    "🎥 PVCAM Live to Rerun"
                };
                if ui.button(label).clicked() {
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

        // Render Dock Area
        let mut dock_state = self
            .dock_state
            .take()
            .unwrap_or_else(Self::default_dock_state);
        let mut viewer = DaqTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);

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
            }
        }

        self.dock_state = Some(dock_state);
    }

    /// Save application state (including successful daemon address) to storage.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Only persist the address if we're connected (save known-good addresses)
        if self.connection.state().is_connected() {
            save_to_storage(storage, &self.daemon_address);
        }

        // Persist dock layout
        if let Some(dock_state) = &self.dock_state {
            eframe::set_value(storage, eframe::APP_KEY, dock_state);
        }
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
