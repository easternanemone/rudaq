//! Main application state and UI logic.

use eframe::egui;
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

static UI_LOG_BUFFER: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

use crate::client::DaqClient;
use crate::panels::{
    ConnectionPanel, DevicesPanel, DocumentViewerPanel, GettingStartedPanel, ModulesPanel,
    PlanRunnerPanel, ScansPanel, ScriptsPanel, StoragePanel,
};

/// Connection state to the DAQ daemon
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

enum ConnectResult {
    Connected {
        client: DaqClient,
        daemon_version: Option<String>,
        address: String,
    },
    Failed {
        error: String,
        address: String,
    },
}

/// Main application state
pub struct DaqApp {
    /// gRPC client (wrapped in Option for lazy initialization)
    client: Option<DaqClient>,

    /// Current connection state
    connection_state: ConnectionState,

    /// Target daemon address
    daemon_address: String,

    /// Daemon version (retrieved via GetDaemonInfo)
    daemon_version: Option<String>,

    /// GUI version (from CARGO_PKG_VERSION)
    gui_version: String,

    /// Active panel/tab
    active_panel: Panel,

    /// Panel states
    connection_panel: ConnectionPanel,
    getting_started_panel: GettingStartedPanel,
    devices_panel: DevicesPanel,
    scripts_panel: ScriptsPanel,
    scans_panel: ScansPanel,
    storage_panel: StoragePanel,
    modules_panel: ModulesPanel,
    plan_runner_panel: PlanRunnerPanel,
    document_viewer_panel: DocumentViewerPanel,

    /// Tokio runtime for async operations
    runtime: tokio::runtime::Runtime,
    /// Async connect result channel
    connect_tx: mpsc::Sender<ConnectResult>,
    connect_rx: mpsc::Receiver<ConnectResult>,
    /// Whether a connect attempt is in flight
    connect_pending: bool,
    /// Rolling in-app log buffer
    log_buffer: Vec<String>,
    /// PVCAM live view streaming state (requires rerun_viewer + instrument_photometrics)
    /// Works in mock mode without pvcam_hardware, or with real SDK when pvcam_hardware enabled
    #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
    pvcam_streaming: bool,
    #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
    pvcam_task: Option<tokio::task::JoinHandle<()>>,
}

/// Available panels in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    GettingStarted,
    Devices,
    Scripts,
    Scans,
    Storage,
    Modules,
    PlanRunner,
    DocumentViewer,
    Logs,
}

impl DaqApp {
    /// Create a new application instance
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
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
        let (connect_tx, connect_rx) = mpsc::channel(4);

        // Install tracing subscriber that also feeds the UI log buffer (only once).
        static SUB_INIT: std::sync::Once = std::sync::Once::new();
        SUB_INIT.call_once(|| {
            let writer = UiLogMakeWriter {
                buf: UI_LOG_BUFFER.clone(),
            }
            .and(std::io::stdout);

            let _ = tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
                .with_writer(writer)
                .try_init();
        });

        Self {
            client: None,
            connection_state: ConnectionState::Disconnected,
            daemon_address: "http://127.0.0.1:50051".to_string(),
            daemon_version: None,
            gui_version: env!("CARGO_PKG_VERSION").to_string(),
            active_panel: Panel::GettingStarted,
            connection_panel: ConnectionPanel::default(),
            getting_started_panel: GettingStartedPanel::default(),
            devices_panel: DevicesPanel::default(),
            scripts_panel: ScriptsPanel::default(),
            scans_panel: ScansPanel::default(),
            storage_panel: StoragePanel::default(),
            modules_panel: ModulesPanel::default(),
            plan_runner_panel: PlanRunnerPanel::default(),
            document_viewer_panel: DocumentViewerPanel::default(),
            runtime,
            connect_tx,
            connect_rx,
            connect_pending: false,
            log_buffer: Vec::new(),
            #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
            pvcam_streaming: false,
            #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
            pvcam_task: None,
        }
    }

    /// Attempt to connect to the daemon
    fn connect(&mut self) {
        if self.connect_pending {
            return;
        }
        self.connection_state = ConnectionState::Connecting;
        self.connect_pending = true;
        let address = self.daemon_address.clone();
        let tx = self.connect_tx.clone();

        self.runtime.spawn(async move {
            match DaqClient::connect(&address).await {
                Ok(mut client) => {
                    let daemon_version = match client.get_daemon_info().await {
                        Ok(info) => Some(info.version),
                        Err(_) => None,
                    };
                    let _ = tx
                        .send(ConnectResult::Connected {
                            client,
                            daemon_version,
                            address,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(ConnectResult::Failed {
                            error: e.to_string(),
                            address,
                        })
                        .await;
                }
            }
        });
    }

    /// Disconnect from the daemon
    fn disconnect(&mut self) {
        self.client = None;
        self.daemon_version = None;
        self.connection_state = ConnectionState::Disconnected;
        tracing::info!("Disconnected from daemon");
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
                
                ui.menu_button("View", |ui| {
                    if ui.button("Getting Started").clicked() {
                        self.active_panel = Panel::GettingStarted;
                        ui.close_menu();
                    }
                    if ui.button("Devices").clicked() {
                        self.active_panel = Panel::Devices;
                        ui.close_menu();
                    }
                    if ui.button("Scripts").clicked() {
                        self.active_panel = Panel::Scripts;
                        ui.close_menu();
                    }
                    if ui.button("Scans").clicked() {
                        self.active_panel = Panel::Scans;
                        ui.close_menu();
                    }
                    if ui.button("Storage").clicked() {
                        self.active_panel = Panel::Storage;
                        ui.close_menu();
                    }
                    if ui.button("Modules").clicked() {
                        self.active_panel = Panel::Modules;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    /// Render version mismatch warning (if applicable)
    fn render_version_warning(&self, ctx: &egui::Context) {
        // Only show warning if connected and versions don't match
        if self.connection_state == ConnectionState::Connected {
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
                // Connection status indicator
                let (color, text) = match &self.connection_state {
                    ConnectionState::Disconnected => (egui::Color32::GRAY, "Disconnected"),
                    ConnectionState::Connecting => (egui::Color32::YELLOW, "Connecting..."),
                    ConnectionState::Connected => (egui::Color32::GREEN, "Connected"),
                    ConnectionState::Error(_) => (egui::Color32::RED, "Error"),
                };
                
                ui.colored_label(color, "●");
                ui.label(text);
                
                ui.separator();
                
                // Address input
                ui.label("Daemon:");
                let response = ui.add_sized(
                    [200.0, 18.0],
                    egui::TextEdit::singleline(&mut self.daemon_address)
                        .hint_text("http://127.0.0.1:50051"),
                );
                
                // Connect/Disconnect button
                match &self.connection_state {
                    ConnectionState::Disconnected | ConnectionState::Error(_) => {
                        if ui.button("Connect").clicked() || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            self.connect();
                        }
                    }
                    ConnectionState::Connected => {
                        if ui.button("Disconnect").clicked() {
                            self.disconnect();
                        }
                    }
                    ConnectionState::Connecting => {
                        ui.spinner();
                    }
                }
                
                // Show error message if any
                if let ConnectionState::Error(msg) = &self.connection_state {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, msg);
                }
            });
        });
    }

    /// Render the left navigation panel
    fn render_nav_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav_panel")
            .resizable(false)
            .default_width(120.0)
            .show(ctx, |ui| {
                ui.heading("Navigation");
                ui.separator();
                
                ui.vertical(|ui| {
                    ui.selectable_value(&mut self.active_panel, Panel::GettingStarted, "🚀 Getting Started");
                    ui.selectable_value(&mut self.active_panel, Panel::Devices, "🔧 Devices");
                    ui.selectable_value(&mut self.active_panel, Panel::Scripts, "📜 Scripts");
                    ui.selectable_value(&mut self.active_panel, Panel::Scans, "📊 Scans");
                    ui.selectable_value(&mut self.active_panel, Panel::Storage, "💾 Storage");
                    ui.selectable_value(&mut self.active_panel, Panel::Modules, "🧩 Modules");
                    ui.selectable_value(&mut self.active_panel, Panel::PlanRunner, "🎯 Plan Runner");
                    ui.selectable_value(&mut self.active_panel, Panel::DocumentViewer, "📄 Documents");
                    ui.selectable_value(&mut self.active_panel, Panel::Logs, "🪵 Logs");
                });
                
                ui.separator();
                ui.add_space(8.0);

                // Rerun visualization button
                if ui.button("📈 Open Rerun").clicked() {
                    // Launch Rerun viewer
                    let _ = std::process::Command::new("rerun")
                        .spawn();
                }

                // PVCAM live view via Rerun (mock mode without SDK, real mode with pvcam_hardware)
                #[cfg(all(feature = "rerun_viewer", feature = "instrument_photometrics"))]
                {
                    ui.add_space(8.0);
                    let label = if self.pvcam_streaming { "🛑 Stop PVCAM Live" } else { "🎥 PVCAM Live to Rerun" };
                    if ui.button(label).clicked() {
                        if self.pvcam_streaming {
                            if let Some(task) = self.pvcam_task.take() {
                                task.abort();
                            }
                            self.pvcam_streaming = false;
                        } else {
                            self.start_pvcam_stream();
                        }
                    }
                }
            });
    }

    /// Render the main content area
    fn render_content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_panel {
                Panel::GettingStarted => {
                    self.getting_started_panel.ui(ui);
                }
                Panel::Devices => {
                    self.devices_panel.ui(ui, self.client.as_mut(), &self.runtime);
                }
                Panel::Scripts => {
                    self.scripts_panel.ui(ui, self.client.as_mut(), &self.runtime);
                }
                Panel::Scans => {
                    self.scans_panel.ui(ui, self.client.as_mut(), &self.runtime);
                }
                Panel::Storage => {
                    self.storage_panel.ui(ui, self.client.as_mut(), &self.runtime);
                }
                Panel::Modules => {
                    self.modules_panel.ui(ui, self.client.as_mut(), &self.runtime);
                }
                Panel::PlanRunner => {
                    self.plan_runner_panel.ui(ui, self.client.as_mut());
                }
                Panel::DocumentViewer => {
                    self.document_viewer_panel.ui(ui, self.client.as_mut());
                }
                Panel::Logs => {
                    self.render_logs(ui);
                }
            }
        });
    }

    fn render_logs(&mut self, ui: &mut egui::Ui) {
        use egui::ScrollArea;
        self.connection_panel.ui(ui);
        ui.separator();
        ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            for line in &self.log_buffer {
                ui.label(line);
            }
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
            if !buf.is_empty() {
                self.log_buffer.extend(buf.drain(..));
            }
        }
    }

    fn poll_connect_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;

        while let Ok(result) = self.connect_rx.try_recv() {
            self.connect_pending = false;
            match result {
                ConnectResult::Connected {
                    client,
                    daemon_version,
                    address,
                } => {
                    self.client = Some(client);
                    self.daemon_version = daemon_version.clone();
                    self.connection_state = ConnectionState::Connected;

                    tracing::info!("Connected to daemon at {}", address);
                    match daemon_version {
                        Some(daemon_ver) => {
                            tracing::info!(
                                "Daemon version: {}, GUI version: {}",
                                daemon_ver,
                                self.gui_version
                            );
                            if daemon_ver != self.gui_version {
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
                ConnectResult::Failed { error, address } => {
                    self.client = None;
                    self.daemon_version = None;
                    self.connection_state = ConnectionState::Error(error.clone());
                    tracing::error!("Failed to connect to {}: {}", address, error);
                }
            }
            updated = true;
        }

        if self.connect_pending || updated {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for DaqApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_logs();
        self.poll_connect_results(ctx);
        self.render_menu_bar(ctx);
        self.render_version_warning(ctx);
        self.render_status_bar(ctx);
        self.render_nav_panel(ctx);
        self.render_content(ctx);
    }
}
