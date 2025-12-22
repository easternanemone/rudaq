//! DAQ Control Panel with embedded Rerun Viewer
//!
//! This binary embeds the Rerun viewer alongside DAQ control panels.
//! Run with: cargo run --bin daq-rerun --features rerun_viewer

#[cfg(feature = "rerun_viewer")]
mod rerun_app {
    use rerun::external::{eframe, egui, re_viewer, re_log, re_crash_handler, re_memory, tokio};
    use rerun::external::re_grpc_client;
    use rerun::external::re_uri::ProxyUri;
    use daq_egui::client::DaqClient;
    use tokio::sync::mpsc;
    use egui_dock::{DockArea, DockState, Style, TabViewer};

    /// Simplified Instrument Manager Panel for Rerun viewer
    /// (uses Rerun's egui version)
    #[derive(Default)]
    struct InstrumentManagerPanel {
        // Simplified state - just a placeholder for now
        last_refresh: Option<std::time::Instant>,
    }

    impl InstrumentManagerPanel {
        fn ui(&mut self, ui: &mut egui::Ui, _client: Option<&mut DaqClient>, _runtime: &tokio::runtime::Runtime) {
            ui.heading("Instruments");
            ui.label("Instrument manager panel");
            ui.small("(Simplified version for Rerun viewer)");

            if let Some(last) = self.last_refresh {
                ui.label(format!("Last refresh: {:.1}s ago", last.elapsed().as_secs_f32()));
            }

            if ui.button("Refresh").clicked() {
                self.last_refresh = Some(std::time::Instant::now());
            }
        }
    }

    /// Simplified Signal Plotter Panel for Rerun viewer
    /// (uses Rerun's egui version)
    #[derive(Default)]
    struct SignalPlotterPanel {
        // Simplified state - just a placeholder for now
        paused: bool,
    }

    impl SignalPlotterPanel {
        fn ui(&mut self, ui: &mut egui::Ui) {
            ui.heading("Signal Scope");
            ui.label("Signal plotter panel");
            ui.small("(Simplified version for Rerun viewer)");

            ui.horizontal(|ui| {
                let label = if self.paused { "▶ Resume" } else { "⏸ Pause" };
                ui.toggle_value(&mut self.paused, label);
            });

            ui.separator();
            ui.label("Use Rerun viewer for primary data visualization");
        }
    }

    // Use Rerun's memory tracking allocator
    #[global_allocator]
    static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
        re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

    /// DAQ Control state shared with the viewer
    pub struct DaqControlState {
        pub daemon_address: String,
        pub rerun_url: String,
        pub connection_status: String,
        pub devices: Vec<DeviceInfo>,
        pub selected_device: Option<usize>,
        pub move_target: f64,
        pub status_message: Option<String>,
        pub error_message: Option<String>,
    }

    #[derive(Clone)]
    pub struct DeviceInfo {
        pub id: String,
        pub name: String,
        pub driver: String,
        pub is_movable: bool,
        pub is_readable: bool,
        pub is_frame_producer: bool,
        pub position: Option<f64>,
        pub reading: Option<f64>,
    }

    impl Default for DaqControlState {
        fn default() -> Self {
            Self {
                daemon_address: std::env::var("DAQ_DAEMON_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:50051".to_string()),
                rerun_url: std::env::var("RERUN_URL")
                    .unwrap_or_else(|_| "rerun+http://127.0.0.1:9876/proxy".to_string()),
                connection_status: "Disconnected".to_string(),
                devices: Vec::new(),
                selected_device: None,
                move_target: 0.0,
                status_message: None,
                error_message: None,
            }
        }
    }

    /// App that wraps Rerun viewer with DAQ control panels
    pub struct DaqRerunApp {
        rerun_app: re_viewer::App,
        daq_state: DaqControlState,
        client: Option<DaqClient>,
        runtime: tokio::runtime::Runtime,
        action_tx: mpsc::Sender<RerunActionResult>,
        action_rx: mpsc::Receiver<RerunActionResult>,
        action_in_flight: usize,

        // Dockable panel state
        dock_state: DockState<PanelKind>,
        instrument_panel: InstrumentManagerPanel,
        signal_plotter: SignalPlotterPanel,
    }

    enum RerunActionResult {
        Connect(Result<DaqClient, String>),
        Refresh(Result<Vec<DeviceInfo>, String>),
        Move { device_id: String, result: Result<f64, String> },
        Read { device_id: String, result: Result<(f64, String), String> },
        StartStream { device_id: String, frame_count: Option<u32>, result: Result<(), String> },
        StopStream { device_id: String, result: Result<u32, String> },
    }

    /// Panel types that can be docked
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    enum PanelKind {
        DaqControl,
        InstrumentManager,
        SignalPlotter,
    }

    /// Storage key for dock state persistence
    const DOCK_STATE_KEY: &str = "daq_dock_state";

    impl PanelKind {
        fn title(&self) -> &'static str {
            match self {
                Self::DaqControl => "DAQ Control",
                Self::InstrumentManager => "Instruments",
                Self::SignalPlotter => "Signal Scope",
            }
        }
    }

    /// Tab viewer for dockable panels
    struct DaqTabViewer<'a> {
        daq_state: &'a mut DaqControlState,
        client: &'a mut Option<DaqClient>,
        runtime: &'a tokio::runtime::Runtime,
        action_tx: &'a mpsc::Sender<RerunActionResult>,
        action_in_flight: &'a mut usize,
        instrument_panel: &'a mut InstrumentManagerPanel,
        signal_plotter: &'a mut SignalPlotterPanel,
    }

    impl<'a> DaqTabViewer<'a> {
        fn render_daq_control(&mut self, ui: &mut egui::Ui) {
            ui.heading("DAQ Control");

            // Connection
            ui.group(|ui| {
                ui.label("Control Plane (gRPC):");
                ui.horizontal(|ui| {
                    ui.label("Daemon:");
                    ui.text_edit_singleline(&mut self.daq_state.daemon_address);
                });

                ui.horizontal(|ui| {
                    let connected = self.client.is_some();
                    let status_color = if connected { egui::Color32::GREEN } else { egui::Color32::GRAY };
                    ui.colored_label(status_color, "●");
                    ui.label(&self.daq_state.connection_status);

                    if connected {
                        if ui.button("Disconnect").clicked() {
                            *self.client = None;
                            self.daq_state.connection_status = "Disconnected".to_string();
                            self.daq_state.devices.clear();
                        }
                    } else {
                        if ui.button("Connect").clicked() {
                            let address = self.daq_state.daemon_address.clone();
                            let tx = self.action_tx.clone();
                            *self.action_in_flight = self.action_in_flight.saturating_add(1);
                            self.daq_state.connection_status = "Connecting...".to_string();

                            self.runtime.spawn(async move {
                                let result = DaqClient::connect(&address).await.map_err(|e| e.to_string());
                                let _ = tx.send(RerunActionResult::Connect(result)).await;
                            });
                        }
                    }
                });

                ui.separator();
                ui.label("Data Plane (Rerun):");
                ui.horizontal(|ui| {
                    ui.label("Rerun URL:");
                    ui.text_edit_singleline(&mut self.daq_state.rerun_url);
                });
                ui.small("Note: Rerun connection established at startup. Restart to change.");
            });

            // Messages
            if let Some(err) = &self.daq_state.error_message {
                ui.colored_label(egui::Color32::RED, err);
            }
            if let Some(status) = &self.daq_state.status_message {
                ui.colored_label(egui::Color32::GREEN, status);
            }

            ui.separator();

            // Devices
            if ui.button("🔄 Refresh Devices").clicked() {
                let Some(client) = self.client else {
                    self.daq_state.error_message = Some("Not connected".to_string());
                    return;
                };

                let mut client = client.clone();
                let tx = self.action_tx.clone();
                *self.action_in_flight = self.action_in_flight.saturating_add(1);

                self.runtime.spawn(async move {
                    let result = async {
                        let devices = client.list_devices().await?;
                        let mut infos = Vec::new();
                        for d in devices {
                            let state = client.get_device_state(&d.id).await.ok();
                            infos.push(DeviceInfo {
                                id: d.id,
                                name: d.name,
                                driver: d.driver_type,
                                is_movable: d.is_movable,
                                is_readable: d.is_readable,
                                is_frame_producer: d.is_frame_producer,
                                position: state.as_ref().and_then(|s| s.position),
                                reading: state.as_ref().and_then(|s| s.last_reading),
                            });
                        }
                        Ok::<_, anyhow::Error>(infos)
                    }
                    .await
                    .map_err(|e| e.to_string());

                    let _ = tx.send(RerunActionResult::Refresh(result)).await;
                });
            }

            if self.daq_state.devices.is_empty() {
                ui.label("No devices");
            } else {
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        let devices = self.daq_state.devices.clone();
                        for (i, device) in devices.iter().enumerate() {
                            let selected = self.daq_state.selected_device == Some(i);
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    if ui.selectable_label(selected, &device.name).clicked() {
                                        self.daq_state.selected_device = Some(i);
                                    }
                                    ui.label(format!("({})", device.driver));
                                });

                                if let Some(pos) = device.position {
                                    ui.label(format!("Position: {:.4}", pos));
                                }
                                if let Some(reading) = device.reading {
                                    ui.label(format!("Reading: {:.4}", reading));
                                }

                                ui.horizontal(|ui| {
                                    if device.is_movable {
                                        ui.add(egui::DragValue::new(&mut self.daq_state.move_target).speed(0.1));
                                        if ui.small_button("Go").clicked() {
                                            self.move_device(&device.id, self.daq_state.move_target, false);
                                        }
                                        for delta in [-1.0, -0.1, 0.1, 1.0] {
                                            let label = if delta > 0.0 { format!("+{}", delta) } else { format!("{}", delta) };
                                            if ui.small_button(label).clicked() {
                                                self.move_device(&device.id, delta, true);
                                            }
                                        }
                                    }
                                    if device.is_readable {
                                        if ui.small_button("📖").clicked() {
                                            self.read_device(&device.id);
                                        }
                                    }
                                    if device.is_frame_producer {
                                        if ui.small_button("▶ Stream 10").clicked() {
                                            self.start_stream(&device.id, Some(10));
                                        }
                                        if ui.small_button("▶ Stream").clicked() {
                                            self.start_stream(&device.id, None);
                                        }
                                        if ui.small_button("⏹ Stop").clicked() {
                                            self.stop_stream(&device.id);
                                        }
                                    }
                                });
                            });
                        }
                    });
            }

            // Data visualization info
            ui.separator();
            ui.heading("Visualization");
            ui.small("Camera frames and measurements logged by the daemon");
            ui.small("appear automatically in the Rerun viewer panel.");
        }

        fn move_device(&mut self, device_id: &str, value: f64, relative: bool) {
            let Some(client) = self.client else { return };

            let mut client = client.clone();
            let device_id = device_id.to_string();
            let tx = self.action_tx.clone();
            *self.action_in_flight = self.action_in_flight.saturating_add(1);

            self.runtime.spawn(async move {
                let result = if relative {
                    client.move_relative(&device_id, value).await
                } else {
                    client.move_absolute(&device_id, value).await
                };
                let action = match result {
                    Ok(response) if response.success => {
                        RerunActionResult::Move {
                            device_id,
                            result: Ok(response.final_position),
                        }
                    }
                    Ok(response) => RerunActionResult::Move {
                        device_id,
                        result: Err(response.error_message),
                    },
                    Err(e) => RerunActionResult::Move {
                        device_id,
                        result: Err(e.to_string()),
                    },
                };
                let _ = tx.send(action).await;
            });
        }

        fn read_device(&mut self, device_id: &str) {
            let Some(client) = self.client else { return };

            let mut client = client.clone();
            let device_id = device_id.to_string();
            let tx = self.action_tx.clone();
            *self.action_in_flight = self.action_in_flight.saturating_add(1);

            self.runtime.spawn(async move {
                let result = client.read_value(&device_id).await;
                let action = match result {
                    Ok(response) if response.success => RerunActionResult::Read {
                        device_id,
                        result: Ok((response.value, response.units)),
                    },
                    Ok(response) => RerunActionResult::Read {
                        device_id,
                        result: Err(response.error_message),
                    },
                    Err(e) => RerunActionResult::Read {
                        device_id,
                        result: Err(e.to_string()),
                    },
                };
                let _ = tx.send(action).await;
            });
        }

        fn start_stream(&mut self, device_id: &str, frame_count: Option<u32>) {
            let Some(client) = self.client else { return };

            let mut client = client.clone();
            let device_id = device_id.to_string();
            let tx = self.action_tx.clone();
            *self.action_in_flight = self.action_in_flight.saturating_add(1);

            self.runtime.spawn(async move {
                let result = client.start_stream(&device_id, frame_count).await;
                let action = match result {
                    Ok(response) if response.success => RerunActionResult::StartStream {
                        device_id,
                        frame_count,
                        result: Ok(()),
                    },
                    Ok(response) => RerunActionResult::StartStream {
                        device_id,
                        frame_count,
                        result: Err(response.error_message),
                    },
                    Err(e) => RerunActionResult::StartStream {
                        device_id,
                        frame_count,
                        result: Err(e.to_string()),
                    },
                };
                let _ = tx.send(action).await;
            });
        }

        fn stop_stream(&mut self, device_id: &str) {
            let Some(client) = self.client else { return };

            let mut client = client.clone();
            let device_id = device_id.to_string();
            let tx = self.action_tx.clone();
            *self.action_in_flight = self.action_in_flight.saturating_add(1);

            self.runtime.spawn(async move {
                let result = client.stop_stream(&device_id).await;
                let action = match result {
                    Ok(response) if response.success => RerunActionResult::StopStream {
                        device_id,
                        result: Ok(response.frames_captured as u32),
                    },
                    Ok(_response) => RerunActionResult::StopStream {
                        device_id,
                        result: Err("Failed to stop stream".to_string()),
                    },
                    Err(e) => RerunActionResult::StopStream {
                        device_id,
                        result: Err(e.to_string()),
                    },
                };
                let _ = tx.send(action).await;
            });
        }
    }

    impl<'a> TabViewer for DaqTabViewer<'a> {
        type Tab = PanelKind;

        fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
            tab.title().into()
        }

        fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
            match tab {
                PanelKind::DaqControl => {
                    self.render_daq_control(ui);
                }
                PanelKind::InstrumentManager => {
                    self.instrument_panel.ui(ui, self.client.as_mut(), self.runtime);
                }
                PanelKind::SignalPlotter => {
                    self.signal_plotter.ui(ui);
                }
            }
        }
    }

    impl DaqRerunApp {
        pub fn new(
            cc: &eframe::CreationContext<'_>,
            rerun_app: re_viewer::App,
        ) -> Self {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");
            let (action_tx, action_rx) = mpsc::channel(16);

            // Try to restore dock state from storage, otherwise create default layout
            let dock_state = cc.storage
                .and_then(|storage| eframe::get_value::<DockState<PanelKind>>(storage, DOCK_STATE_KEY))
                .unwrap_or_else(Self::default_dock_layout);

            Self {
                rerun_app,
                daq_state: DaqControlState::default(),
                client: None,
                runtime,
                action_tx,
                action_rx,
                action_in_flight: 0,
                dock_state,
                instrument_panel: InstrumentManagerPanel::default(),
                signal_plotter: SignalPlotterPanel::default(),
            }
        }

        /// Creates the default dock layout with all panels as tabs
        fn default_dock_layout() -> DockState<PanelKind> {
            let mut dock_state = DockState::new(vec![PanelKind::DaqControl]);
            let surface = dock_state.main_surface_mut();
            surface.push_to_first_leaf(PanelKind::InstrumentManager);
            surface.push_to_first_leaf(PanelKind::SignalPlotter);
            dock_state
        }

        fn poll_async_results(&mut self, ctx: &egui::Context) {
            let mut updated = false;
            loop {
                match self.action_rx.try_recv() {
                    Ok(result) => {
                        self.action_in_flight = self.action_in_flight.saturating_sub(1);
                        match result {
                            RerunActionResult::Connect(result) => match result {
                                Ok(client) => {
                                    self.client = Some(client);
                                    self.daq_state.connection_status = "Connected".to_string();
                                    self.daq_state.status_message = Some("Connected to daemon".to_string());
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => {
                                    self.client = None;
                                    self.daq_state.connection_status = "Error".to_string();
                                    self.daq_state.error_message = Some(e);
                                }
                            },
                            RerunActionResult::Refresh(result) => match result {
                                Ok(devices) => {
                                    self.daq_state.status_message = Some(format!("Loaded {} devices", devices.len()));
                                    self.daq_state.devices = devices;
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => self.daq_state.error_message = Some(e),
                            },
                            RerunActionResult::Move { device_id, result } => match result {
                                Ok(position) => {
                                    self.daq_state.status_message =
                                        Some(format!("Moved to {:.4}", position));
                                    if let Some(dev) =
                                        self.daq_state.devices.iter_mut().find(|d| d.id == device_id)
                                    {
                                        dev.position = Some(position);
                                    }
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => self.daq_state.error_message = Some(e),
                            },
                            RerunActionResult::Read { device_id, result } => match result {
                                Ok((value, units)) => {
                                    self.daq_state.status_message =
                                        Some(format!("{}: {:.4} {}", device_id, value, units));
                                    if let Some(dev) =
                                        self.daq_state.devices.iter_mut().find(|d| d.id == device_id)
                                    {
                                        dev.reading = Some(value);
                                    }
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => self.daq_state.error_message = Some(e),
                            },
                            RerunActionResult::StartStream { device_id, frame_count, result } => match result {
                                Ok(()) => {
                                    let msg = if let Some(n) = frame_count {
                                        format!("Started streaming {} frames from {}", n, device_id)
                                    } else {
                                        format!("Started streaming from {}", device_id)
                                    };
                                    self.daq_state.status_message = Some(msg);
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => self.daq_state.error_message = Some(e),
                            },
                            RerunActionResult::StopStream { device_id, result } => match result {
                                Ok(frames) => {
                                    self.daq_state.status_message = Some(format!(
                                        "Stopped streaming from {} ({} frames captured)",
                                        device_id, frames
                                    ));
                                    self.daq_state.error_message = None;
                                }
                                Err(e) => self.daq_state.error_message = Some(e),
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

    }

    impl eframe::App for DaqRerunApp {
        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            // Save dock panel layout
            eframe::set_value(storage, DOCK_STATE_KEY, &self.dock_state);
            // Save Rerun app state
            self.rerun_app.save(storage);
        }

        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.poll_async_results(ctx);

            // Top menu bar for layout control
            egui::TopBottomPanel::top("daq_menu_bar").show(ctx, |ui| {
                #[allow(deprecated)] // egui 0.33 deprecated menu::bar, but rerun uses 0.33
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("View", |ui| {
                        if ui.button("Reset Panel Layout").clicked() {
                            self.dock_state = Self::default_dock_layout();
                            ui.close_menu();
                        }
                    });
                });
            });

            // Left panel with dockable tabs
            egui::SidePanel::left("daq_dock_panel")
                .default_width(350.0)
                .resizable(true)
                .show(ctx, |ui| {
                    let mut viewer = DaqTabViewer {
                        daq_state: &mut self.daq_state,
                        client: &mut self.client,
                        runtime: &self.runtime,
                        action_tx: &self.action_tx,
                        action_in_flight: &mut self.action_in_flight,
                        instrument_panel: &mut self.instrument_panel,
                        signal_plotter: &mut self.signal_plotter,
                    };

                    DockArea::new(&mut self.dock_state)
                        .style(Style::from_egui(ui.style().as_ref()))
                        .show_inside(ui, &mut viewer);
                });

            // Rest is Rerun viewer
            self.rerun_app.update(ctx, frame);
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Create tokio runtime BEFORE eframe - required for re_grpc_client and re_viewer
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        // Enter the runtime context so tokio::spawn works from eframe callbacks
        let _guard = runtime.enter();

        let main_thread_token = re_viewer::MainThreadToken::i_promise_i_am_on_the_main_thread();

        re_log::setup_logging();
        re_crash_handler::install_crash_handlers(re_viewer::build_info());

        // Parse the daemon's Rerun server URI from env var or use default
        let rerun_url = std::env::var("RERUN_URL")
            .unwrap_or_else(|_| "rerun+http://127.0.0.1:9876/proxy".to_string());

        eprintln!("Connecting to Rerun server at: {}", rerun_url);

        // Parse URI - stream creation happens inside eframe callback
        let uri: ProxyUri = rerun_url.parse()
            .expect("Invalid Rerun proxy URI");

        let mut native_options = re_viewer::native::eframe_options(None);
        native_options.viewport = native_options
            .viewport
            .with_app_id("daq_control_panel")
            .with_inner_size([1400.0, 900.0]);

        let startup_options = re_viewer::StartupOptions::default();
        let app_env = re_viewer::AppEnvironment::Custom("DAQ Control Panel".to_owned());

        eframe::run_native(
            "DAQ Control Panel + Rerun",
            native_options,
            Box::new(move |cc| {
                re_viewer::customize_eframe_and_setup_renderer(cc)?;

                let mut rerun_app = re_viewer::App::new(
                    main_thread_token,
                    re_viewer::build_info(),
                    app_env,
                    startup_options,
                    cc,
                    None,
                    re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
                );

                // Connect to daemon's Rerun gRPC server
                // Returns LogReceiver for streaming data from remote daemon
                let data_rx = re_grpc_client::stream(uri, None);
                rerun_app.add_log_receiver(data_rx);

                Ok(Box::new(DaqRerunApp::new(cc, rerun_app)))
            }),
        )?;

        Ok(())
    }
}

#[cfg(feature = "rerun_viewer")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    rerun_app::run()
}

#[cfg(not(feature = "rerun_viewer"))]
fn main() {
    eprintln!("This binary requires the 'rerun_viewer' feature.");
    eprintln!("Run with: cargo run --bin daq-rerun --features rerun_viewer");
    std::process::exit(1);
}
