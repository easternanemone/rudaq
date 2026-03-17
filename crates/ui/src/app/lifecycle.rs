//! Application lifecycle - eframe::App impl, save/restore, Drop.

use super::*;

impl eframe::App for DaqApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- Native-only polling (ConnectionManager, logs, auto-connect) ---
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.poll_logs();
            self.poll_connect_results(ctx);
            self.update_connection_diagnostics(); // bd-j3xz.3.3
            self.process_auto_connect(ctx);
            self.detect_connection_transitions();
        }

        // --- WASM connection polling ---
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(ref mut rx) = self.wasm_connection.connect_rx {
                if let Ok(result) = rx.try_recv() {
                    self.wasm_connection.connecting = false;
                    match result {
                        Ok(client) => {
                            self.wasm_connection.status = "Connected".to_string();
                            self.client = Some(client);
                            self.was_connected = true;
                        }
                        Err(e) => {
                            self.wasm_connection.status = format!("Error: {}", e);
                        }
                    }
                    self.wasm_connection.connect_rx = None;
                }
            }
        }

        // --- Touch-friendly style for tablets (iPad/Android) ---
        // Applied once on first touch detection to avoid per-frame style_mut overhead.
        #[cfg(target_arch = "wasm32")]
        if !self.touch_style_applied && crate::layout::is_touch_device(ctx) {
            crate::layout::apply_touch_style(ctx);
            self.touch_style_applied = true;
        }

        // --- Cross-platform polling ---
        self.poll_device_reconcile(); // bd-vjzq

        // Health checks use ConnectionManager (native-only)
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.maybe_spawn_health_check();
            self.poll_health_checks();
        }

        // Check global keyboard shortcuts
        self.check_global_shortcuts(ctx);

        // Handle additional keyboard shortcuts (Ctrl+, opens settings)
        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(egui::Key::Comma) {
                self.settings_window.open();
            }
        });

        // --- Native-only rendering (menu bar, status bars, crash recovery) ---
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.render_menu_bar(ctx);
            self.render_version_warning(ctx);
            self.render_crash_recovery_banner(ctx);
            self.render_status_bar(ctx);
        }

        // --- WASM menu bar with connection UI ---
        #[cfg(target_arch = "wasm32")]
        {
            egui::TopBottomPanel::top("wasm_menu_bar").show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.label(egui::RichText::new("DAQ Control Panel").strong());
                    ui.separator();
                    ui.label("Server:");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.wasm_connection.url_input)
                            .desired_width(250.0),
                    );
                    if self.wasm_connection.connecting {
                        ui.spinner();
                    } else if self.client.is_some() {
                        if ui.button("Reconnect").clicked() {
                            self.wasm_connect(ctx);
                        }
                        if ui.button("Disconnect").clicked() {
                            self.wasm_disconnect();
                            self.wasm_connection.status = "Disconnected".to_string();
                        }
                    } else if ui.button("Connect").clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        self.wasm_connect(ctx);
                    }
                    ui.separator();
                    if self.client.is_some() {
                        ui.colored_label(egui::Color32::GREEN, "Connected");
                    } else {
                        ui.label(&self.wasm_connection.status);
                    }
                });
            });
        }

        // Render settings window
        if self.settings_window.show(ctx, &mut self.app_settings) {
            // Settings were applied - update dependent systems
            if self.theme_preference != self.app_settings.appearance.theme {
                self.theme_preference = self.app_settings.appearance.theme;
                theme::apply_theme(ctx, self.theme_preference);
            }
            // Font and UI scale changes will be applied on next frame
            ctx.set_zoom_factor(self.app_settings.appearance.ui_scale);
        }

        // --- Status bar (platform-specific) ---
        #[cfg(not(target_arch = "wasm32"))]
        {
            let error_count = self.connection.health_status().total_errors;
            let error_count = if error_count > 0 {
                Some(error_count)
            } else {
                None
            };
            self.status_bar
                .show(ctx, self.connection.state(), error_count);
        }
        #[cfg(target_arch = "wasm32")]
        self.status_bar.show_simple(ctx, self.client.is_some());

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
                device_info: Box::new(request.device_info),
            });
        }

        // Check for image viewer navigation requests from InstrumentManagerPanel
        if let Some(device_id) = self.instrument_manager_panel.take_image_viewer_request() {
            tracing::info!(
                device_id = %device_id,
                "Navigating to Image Viewer for live stream"
            );
            self.ui_actions.push(UiAction::FocusTab(Panel::ImageViewer));
            if let Some(client) = self.client.as_mut() {
                self.image_viewer_panel
                    .set_device(&device_id, client, &self.runtime);
            }
        }

        // Collect panels to close to avoid borrow conflicts
        let mut panels_to_close = Vec::new();

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
                UiAction::CloseDevicePanel { id } => {
                    // Remove panel from dock
                    dock_state.retain_tabs(|tab| {
                        !matches!(tab, Panel::DeviceControl { id: panel_id } if *panel_id == id)
                    });
                    // Defer cleanup to avoid borrow conflicts
                    panels_to_close.push(id);
                }
                UiAction::OpenDeviceControl { device_info } => {
                    let device_info = *device_info;
                    // Generate a new panel ID with saturation on overflow
                    // (practically impossible to hit usize::MAX panels, but prevents ID collisions)
                    let panel_id = self.next_device_panel_id;
                    self.next_device_panel_id = self.next_device_panel_id.saturating_add(1);

                    // Debug logging for panel routing diagnosis (bd-kj7i)
                    tracing::info!(
                        panel_id = panel_id,
                        device_id = %device_info.id,
                        device_name = %device_info.name,
                        driver_type = %device_info.driver_type,
                        is_emission_controllable = device_info.is_emission_controllable(),
                        is_shutter_controllable = device_info.is_shutter_controllable(),
                        is_wavelength_tunable = device_info.is_wavelength_tunable(),
                        is_readable = device_info.is_readable(),
                        is_movable = device_info.is_movable(),
                        "OpenDeviceControl: creating pop-out panel with capabilities"
                    );

                    // Determine panel kind from device capabilities
                    let kind = panel_kind_for_device(&device_info);

                    // Store device info (full proto with capability flags)
                    self.device_panel_info.insert(
                        panel_id,
                        DevicePanelInfo {
                            device_info: device_info.clone(),
                            availability: DeviceAvailability::Available, // Fresh from daemon
                            kind,
                        },
                    );

                    // GenericDevicePanel created lazily on first render

                    // Add the panel to the dock
                    let panel = Panel::DeviceControl { id: panel_id };
                    dock_state.main_surface_mut().push_to_focused_leaf(panel);
                }
            }
        }

        // Clean up closed panels
        for id in panels_to_close {
            self.remove_panel_data(id);
        }

        self.dock_state = Some(dock_state);

        // Render cheat sheet panel if visible
        if self.show_cheat_sheet {
            self.cheat_sheet_panel
                .show(ctx, &mut self.show_cheat_sheet, &self.shortcut_manager);
        }
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        // Persist state every 5 seconds for crash recovery (bd-izdj.30)
        std::time::Duration::from_secs(5)
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Native-only: persist daemon address, clear legacy keys, update session file
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.connection.state().is_connected() {
                self.app_settings.connection.daemon_address =
                    self.daemon_address.original().to_string();
            }
            clear_legacy_daemon_address(storage);
            write_session_file(self.daemon_address.as_str());

            // Sync current state into native preferences — only persist if changed
            let updated = crate::preferences::AppPreferences {
                daemon_url: self.app_settings.connection.daemon_address.clone(),
                theme: format!("{:?}", self.theme_preference),
            };
            if updated != self.native_prefs {
                self.native_prefs = updated;
                self.native_prefs.persist();
            }
        }

        if let Some(dock_state) = &self.dock_state {
            eframe::set_value(storage, eframe::APP_KEY, dock_state);
        }

        // Persist layout version for stale layout detection on next load
        eframe::set_value(storage, LAYOUT_VERSION_KEY, &LAYOUT_VERSION);

        eframe::set_value(storage, "theme_preference", &self.theme_preference);

        // Persist application settings
        eframe::set_value(storage, "app_settings", &self.app_settings);

        // Persist keyboard shortcuts
        eframe::set_value(storage, "shortcut_manager", &self.shortcut_manager);
        eframe::set_value(
            storage,
            "control_panel_layout_mode",
            &self.control_panel_layout_mode,
        );

        // WASM-only: persist server URL so users don't re-enter it on reload
        #[cfg(target_arch = "wasm32")]
        {
            let trimmed_url = self.wasm_connection.url_input.trim().to_string();
            eframe::set_value(storage, WASM_SERVER_URL_KEY, &trimmed_url);
        }

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
        // Native-only: mark clean shutdown and stop daemon
        #[cfg(not(target_arch = "wasm32"))]
        clear_session_file();

        tracing::debug!("DaqApp shutting down, cleaning up device panel state");

        // Collect panel IDs to avoid borrow conflicts during cleanup
        let panel_ids: Vec<usize> = self.device_panel_info.keys().copied().collect();

        // Clean up all device panel state
        for id in panel_ids {
            self.remove_panel_data(id);
        }

        // Native-only: shutdown daemon launcher if running
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(launcher) = self.daemon_launcher.take() {
            drop(launcher);
        }

        tracing::debug!("DaqApp shutdown complete");
    }
}
