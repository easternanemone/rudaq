//! UI rendering - menu bar, status bar, crash recovery banner, global shortcuts.

use super::*;

impl DaqApp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui
                        .button(format!("{} Settings", crate::icons::action::SETTINGS))
                        .clicked()
                    {
                        self.settings_window.open();
                        ui.close();
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

                    if ui.button("Lab Native").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabHardware { port: 50051 });
                        ui.close();
                    }

                    if ui.button("Lab Universal").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabUniversal { port: 50051 });
                        ui.close();
                    }

                    if ui.button("Lab Hybrid+DB").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabHybridDb { port: 50051 });
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

                    ui.small("Hybrid+DB requires daemon build with db-surreal feature flags.");

                    // Connection presets from gui.toml
                    if !self.gui_presets.is_empty() {
                        ui.separator();
                        ui.label("Presets");
                        let mut selected_preset_url: Option<String> = None;
                        for i in 0..self.gui_presets.len() {
                            let preset = &self.gui_presets[i];
                            let label = if preset.default {
                                format!("{} \u{2605}", preset.name)
                            } else {
                                preset.name.clone()
                            };
                            let response = ui.button(&label);
                            if !preset.description.is_empty() {
                                response.clone().on_hover_text(&preset.description);
                            }
                            if response.clicked() {
                                selected_preset_url = Some(preset.grpc_url.clone());
                                ui.close();
                            }
                        }
                        if let Some(url) = selected_preset_url {
                            self.address_input.clone_from(&url);
                            self.switch_daemon_mode(DaemonMode::Remote { url });
                        }
                    }

                    ui.separator();

                    // Daemon status
                    if let Some(ref mut launcher) = self.daemon_launcher {
                        if launcher.is_running() {
                            ui.colored_label(egui::Color32::GREEN, "Local daemon running");
                            if let Some(uptime) = launcher.uptime() {
                                ui.small(format!("Uptime: {}s", uptime.as_secs()));
                            }
                            if ui.button("Stop Daemon").clicked() {
                                launcher.stop();
                                self.disconnect();
                                ui.close();
                            }
                        } else {
                            ui.colored_label(egui::Color32::RED, "Local daemon stopped");
                            if let Some(err) = launcher.last_error() {
                                ui.small(err);
                            }
                            if ui.button("Restart Daemon").clicked() {
                                if let Err(e) = launcher.start_with_mode(&self.daemon_mode) {
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
                    if ui.button("Reset Layout").clicked() {
                        self.dock_state = Some(Self::default_dock_state());
                        ui.close();
                    }
                    ui.separator();
                    ui.label("Control Panels");
                    if ui
                        .selectable_label(
                            self.control_panel_layout_mode == ControlPanelLayoutMode::Simple,
                            "Simple",
                        )
                        .clicked()
                    {
                        self.set_control_panel_layout_mode(ControlPanelLayoutMode::Simple);
                        ui.close();
                    }
                    if ui
                        .selectable_label(
                            self.control_panel_layout_mode == ControlPanelLayoutMode::Advanced,
                            "Advanced",
                        )
                        .clicked()
                    {
                        self.set_control_panel_layout_mode(ControlPanelLayoutMode::Advanced);
                        ui.close();
                    }
                    ui.separator();

                    if ui.button("Instruments").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Instruments));
                        ui.close();
                    }
                    if ui.button("Getting Started").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::GettingStarted));
                        ui.close();
                    }
                    if ui.button("Image Viewer").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::ImageViewer));
                        ui.close();
                    }
                    if ui.button("Scripts").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scripts));
                        ui.close();
                    }
                    if ui.button("Scan Builder").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::ScanBuilder));
                        ui.close();
                    }
                    if ui.button("Experiment Designer").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::ExperimentDesigner));
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

    /// Show a transient banner when recovering from a crash (bd-izdj.30)
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn render_crash_recovery_banner(&mut self, ctx: &egui::Context) {
        if !self.recovered_from_crash {
            return;
        }

        egui::TopBottomPanel::top("crash_recovery_banner")
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color =
                        Some(egui::Color32::from_rgb(100, 200, 255));
                    ui.label(icons::status::INFO);
                    ui.label("Session restored after unexpected shutdown. Panel layout and connection settings have been recovered.");
                    if ui.small_button("Dismiss").clicked() {
                        self.recovered_from_crash = false;
                    }
                });
                ui.add_space(2.0);
            });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn render_version_warning(&self, ctx: &egui::Context) {
        // Only show warning if connected and versions don't match
        if self.connection.state().is_connected()
            && let Some(ref daemon_ver) = self.daemon_version
            && daemon_ver != &self.gui_version
        {
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

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn render_status_bar(&mut self, ctx: &egui::Context) {
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
                    ConnectionState::CircuitBreaker { last_error, .. } => {
                        Some((last_error.clone(), true))
                    }
                    _ => None,
                };
                let seconds_until_retry = self.connection.seconds_until_retry();

                // Connection status indicator — combined label for AccessKit readability
                ui.colored_label(state_color, format!("● {state_label}"));

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
        use common::capabilities::{FrameObserver, FrameProducer};
        use common::data::FrameView;
        use driver_pvcam::PvcamDriver;
        use rerun::RecordingStreamBuilder;
        use rerun::archetypes::Tensor;
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

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn poll_logs(&mut self) {
        // Drain all pending log events from the channel
        while let Ok(event) = self.log_receiver.try_recv() {
            if let Some((level, message)) = data_integrity_status_message(&event) {
                self.status_bar.set_status(message, level);
            }
            self.logging_panel
                .log(event.level, &event.target, &event.message);
        }
    }

    pub(super) fn check_global_shortcuts(&mut self, ctx: &egui::Context) {
        // Check toggle cheat sheet (Shift+?)
        if self.shortcut_manager.check_action(
            ctx,
            ShortcutContext::Global,
            ShortcutAction::ToggleCheatSheet,
        ) {
            self.show_cheat_sheet = !self.show_cheat_sheet;
        }

        // Note: Other global shortcuts (OpenSettings, SaveCurrent) will be handled
        // by specific panels or settings UI when implemented
    }
}
