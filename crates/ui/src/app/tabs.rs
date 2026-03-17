//! Tab viewer implementation - DaqTabViewer, navigation, device control panels.

use super::*;

pub(super) struct DaqTabViewer<'a> {
    pub(super) app: &'a mut DaqApp,
}

impl TabViewer for DaqTabViewer<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Panel::Nav => "Navigation".into(),
            Panel::GettingStarted => {
                format!("{} Getting Started", icons::nav::GETTING_STARTED).into()
            }
            Panel::Instruments => format!("{} Instruments", icons::nav::INSTRUMENT_MANAGER).into(),
            Panel::Scripts => format!("{} Scripts", icons::nav::SCRIPTS).into(),
            Panel::ScanBuilder => "Scan Builder".into(),
            Panel::ExperimentDesigner => "Experiment Designer".into(),
            Panel::Storage => format!("{} Storage", icons::nav::STORAGE).into(),
            Panel::RunHistory => "📚 Run History".into(),
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
        // Constrain each dock tab to its own available width to avoid
        // content driving slight horizontal overflow.
        ui.set_max_width(ui.available_width());

        match tab {
            Panel::Nav => self.render_nav(ui),
            Panel::GettingStarted => self.app.getting_started_panel.ui(ui),
            Panel::Instruments => self.app.instrument_manager_panel.ui(
                ui,
                self.app.client.as_mut(),
                &self.app.runtime,
            ),
            Panel::Scripts => {
                self.app
                    .scripts_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::ScanBuilder => {
                self.app
                    .scan_builder_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::ExperimentDesigner => self.app.experiment_designer_panel.ui(
                ui,
                self.app.client.as_mut(),
                Some(&self.app.runtime),
            ),
            Panel::Storage => {
                self.app
                    .storage_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::RunHistory => {
                self.app
                    .run_history_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Modules => {
                self.app
                    .modules_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::PlanRunner => {
                self.app
                    .plan_runner_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::DocumentViewer => {
                self.app
                    .document_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::SignalPlotter => {
                self.app.signal_plotter_panel.drain_updates();
                self.app.signal_plotter_panel.ui(ui);
            }
            Panel::ImageViewer => {
                self.app
                    .image_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Logs => self.app.logging_panel.ui(ui),
            Panel::DeviceControl { id } => {
                self.render_device_control(ui, *id);
            }
        }
    }
}

impl DaqTabViewer<'_> {
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
            self.nav_button(ui, icons::nav::SCANS, "Scan Builder", Panel::ScanBuilder);
            self.nav_button(
                ui,
                icons::nav::SCANS,
                "Experiment Designer",
                Panel::ExperimentDesigner,
            );
            self.nav_button(
                ui,
                icons::nav::PLAN_RUNNER,
                "Plan Runner",
                Panel::PlanRunner,
            );

            Self::section_label(ui, "Data");
            self.nav_button(ui, icons::nav::STORAGE, "Storage", Panel::Storage);
            self.nav_button(ui, "📚", "Run History", Panel::RunHistory);
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

            #[cfg(not(target_arch = "wasm32"))]
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

        // Gate rendering based on availability
        match info.availability {
            DeviceAvailability::Pending => {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.spinner();
                    ui.label("Validating device with daemon...");
                    ui.add_space(10.0);
                    ui.label(format!("Device: {}", device_info.name));
                    ui.label(format!("ID: {}", device_info.id));
                });
                return;
            }
            DeviceAvailability::Missing => {
                ui.vertical(|ui| {
                    ui.add_space(10.0);
                    ui.colored_label(egui::Color32::RED, "⚠ Device Not Found");
                    ui.add_space(10.0);

                    ui.label(format!("Device: {}", device_info.name));
                    ui.label(format!("ID: {}", device_info.id));
                    #[cfg(not(target_arch = "wasm32"))]
                    ui.label(format!("Daemon: {}", self.app.daemon_address));
                    #[cfg(target_arch = "wasm32")]
                    ui.label(format!("Daemon: {}", self.app.wasm_connection.url_input));

                    ui.add_space(10.0);
                    ui.label("This device is not available on the connected daemon.");
                    ui.label("The daemon may have been restarted with a different configuration.");

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Refresh").clicked() {
                            self.app.start_device_reconcile();
                        }
                        if ui.button("Close Panel").clicked() {
                            self.app
                                .ui_actions
                                .push(UiAction::CloseDevicePanel { id: panel_id });
                        }
                    });
                });
                return;
            }
            DeviceAvailability::Available => {
                // Continue to normal rendering below
            }
        }

        // --- Priority 0 & 1: Config-driven panels (cross-platform, uses hardware schema types) ---
        {
            // Priority 0: gRPC-driven panel from device metadata
            let grpc_config = self
                .app
                .grpc_ui_config_cache
                .entry(panel_id)
                .or_insert_with(|| {
                    crate::panels::instrument_manager::dispatch::try_grpc_ui_config(device_info)
                });
            if let Some(panel_config) = grpc_config {
                let panel = self
                    .app
                    .docked_config_driven_panels
                    .entry(panel_id)
                    .or_insert_with(|| ConfigDrivenPanel::new(panel_config.clone()));
                ui.push_id(("docked", panel_id), |ui| {
                    panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                });
                return;
            }

            // --- Priority 1: Config-driven panel from local TOML ---
            if let Some(panel_config) = self
                .app
                .config_cache
                .get_ui_config_for_driver(&device_info.driver_type)
            {
                let panel = self
                    .app
                    .docked_config_driven_panels
                    .entry(panel_id)
                    .or_insert_with(|| ConfigDrivenPanel::new(panel_config.clone()));
                ui.push_id(("docked", panel_id), |ui| {
                    panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                });
                return;
            }
        }

        let layout_mode = self.app.control_panel_layout_mode;
        ui.push_id(("docked", panel_id), |ui| match layout_mode {
            ControlPanelLayoutMode::Simple => {
                let panel = self
                    .app
                    .docked_panels
                    .entry(panel_id)
                    .or_insert_with(|| GenericDevicePanel::from_device_info(device_info));
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            }
            ControlPanelLayoutMode::Advanced => {
                match docked_advanced_panel_kind_for_device(device_info) {
                    DockedAdvancedPanelKind::MaiTai => {
                        let panel = self.app.docked_maitai_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Comedi => {
                        let did = device_info.id.clone();
                        let panel = self
                            .app
                            .docked_comedi_panels
                            .entry(panel_id)
                            .or_insert_with(|| ComediPanel::new(&did));
                        panel.ui(ui, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::PowerMeter => {
                        let panel = self
                            .app
                            .docked_power_meter_panels
                            .entry(panel_id)
                            .or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Rotator => {
                        let panel = self.app.docked_rotator_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Stage => {
                        let panel = self.app.docked_stage_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Generic => {
                        let panel =
                            self.app.docked_panels.entry(panel_id).or_insert_with(|| {
                                GenericDevicePanel::from_device_info(device_info)
                            });
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                }

                let mut command_widgets = self
                    .app
                    .docked_command_widgets
                    .remove(&panel_id)
                    .unwrap_or_default();
                command_widgets.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                self.app
                    .docked_command_widgets
                    .insert(panel_id, command_widgets);
            }
        });
    }
}
