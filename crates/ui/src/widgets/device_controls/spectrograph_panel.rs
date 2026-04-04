//! Andor Shamrock spectrograph control panel.
//!
//! Provides:
//! - Grating selector (turret position)
//! - Center wavelength control
//! - Slit width adjustment
//! - Shutter toggle
//! - Filter wheel position
//! - Flipper mirror position

use crate::runtime::Runtime;
use egui::Ui;

use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Spectrograph state cached from the daemon.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // `online` reserved for future online/offline indicator
struct SpectrographState {
    grating: Option<u32>,
    num_gratings: u32,
    wavelength_nm: Option<f64>,
    slit_width_um: Option<f64>,
    shutter_open: Option<bool>,
    filter_position: Option<u32>,
    flipper_position: Option<u32>,
    online: bool,
}

/// Async action results for the spectrograph panel.
enum ActionResult {
    FetchState(Result<SpectrographState, String>),
    SetParameter(Result<String, String>),
    SetShutter(Result<bool, String>),
    SetWavelength(Result<f64, String>),
}

/// Andor Shamrock spectrograph control panel.
pub struct SpectrographPanel {
    panel_state: DevicePanelState<ActionResult>,
    state: SpectrographState,
    wavelength_input: String,
    slit_width_input: String,
    grating_idx: usize,
}

impl Default for SpectrographPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            state: SpectrographState::default(),
            wavelength_input: "500.0".to_string(),
            slit_width_input: "100.0".to_string(),
            grating_idx: 0,
        }
    }
}

impl SpectrographPanel {
    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            self.panel_state.action_completed();

            match result {
                ActionResult::FetchState(result) => match result {
                    Ok(state) => {
                        if let Some(wl) = state.wavelength_nm {
                            self.wavelength_input = format!("{wl:.2}");
                        }
                        if let Some(sw) = state.slit_width_um {
                            self.slit_width_input = format!("{sw:.0}");
                        }
                        if let Some(g) = state.grating {
                            self.grating_idx = g.saturating_sub(1) as usize;
                        }
                        self.state = state;
                        self.panel_state.error = None;
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Failed to fetch state: {e}"));
                    }
                },
                ActionResult::SetParameter(result) => match result {
                    Ok(msg) => {
                        self.panel_state.set_status(msg);
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Set failed: {e}"));
                    }
                },
                ActionResult::SetShutter(result) => match result {
                    Ok(is_open) => {
                        self.state.shutter_open = Some(is_open);
                        let label = if is_open { "opened" } else { "closed" };
                        self.panel_state.set_status(format!("Shutter {label}"));
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Shutter failed: {e}"));
                    }
                },
                ActionResult::SetWavelength(result) => match result {
                    Ok(wl) => {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_input = format!("{wl:.2}");
                        self.panel_state
                            .set_status(format!("Wavelength: {wl:.2} nm"));
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Wavelength failed: {e}"));
                    }
                },
            }
        }
    }

    fn fetch_state(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            // Query parameters sequentially (serial devices can't handle parallel)
            let mut state = SpectrographState {
                online: true,
                num_gratings: 3,
                ..Default::default()
            };

            // Helper: get parameter value, ignoring errors for unsupported params
            async fn get_param(
                client: &mut DaqClient,
                device_id: &str,
                name: &str,
            ) -> Option<String> {
                client
                    .get_parameter(device_id, name)
                    .await
                    .ok()
                    .map(|pv| pv.value)
            }

            state.grating = get_param(&mut client, &device_id, "grating")
                .await
                .and_then(|v| v.parse::<u32>().ok());
            if let Some(n) = get_param(&mut client, &device_id, "num_gratings")
                .await
                .and_then(|v| v.parse::<u32>().ok())
            {
                state.num_gratings = n;
            }
            // Use the typed gRPC method for wavelength (maps to WavelengthTunable)
            state.wavelength_nm = client.get_wavelength(&device_id).await.ok();
            state.slit_width_um = get_param(&mut client, &device_id, "slit_width")
                .await
                .and_then(|v| v.parse::<f64>().ok());
            // Use the typed gRPC method for shutter (maps to ShutterControl)
            state.shutter_open = client.get_shutter(&device_id).await.ok();
            state.filter_position = get_param(&mut client, &device_id, "filter_position")
                .await
                .and_then(|v| v.parse::<u32>().ok());
            state.flipper_position = get_param(&mut client, &device_id, "flipper_position")
                .await
                .and_then(|v| v.parse::<u32>().ok());

            let _ = tx.send(ActionResult::FetchState(Ok(state))).await;
        });
    }

    fn set_parameter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        param: &str,
        value: &str,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();
        let param = param.to_string();
        let value = value.to_string();

        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id, &param, &value)
                .await
                .map(|_| format!("{param} set to {value}"))
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetParameter(result)).await;
        });
    }

    fn set_wavelength(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        wavelength_nm: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_wavelength(&device_id, wavelength_nm)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetWavelength(result)).await;
        });
    }

    fn set_shutter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        open: bool,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_shutter(&device_id, open)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetShutter(result)).await;
        });
    }
}

impl DeviceControlWidget for SpectrographPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.poll_results();

        let device_id = device.id.clone();
        self.panel_state.device_id = Some(device_id.clone());

        // Initial fetch
        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Auto-refresh every 3 seconds
        if self
            .panel_state
            .should_refresh(std::time::Duration::from_secs(3))
        {
            self.panel_state.mark_refreshed();
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("Shamrock Spectrograph");
            if self.panel_state.is_busy() {
                ui.spinner();
            }
        });

        if let Some(ref err) = self.panel_state.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.panel_state.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        let is_busy = self.panel_state.is_busy();

        // Grating selector
        ui.label(egui::RichText::new("Grating").strong());
        ui.horizontal(|ui| {
            let prev = self.grating_idx;
            egui::ComboBox::from_id_salt("grating")
                .selected_text(format!("Grating {}", self.grating_idx + 1))
                .show_ui(ui, |ui| {
                    for i in 0..self.state.num_gratings as usize {
                        ui.selectable_value(&mut self.grating_idx, i, format!("Grating {}", i + 1));
                    }
                });
            if self.grating_idx != prev && !is_busy {
                let grating_num = self.grating_idx + 1;
                self.set_parameter(
                    client.as_deref_mut(),
                    runtime,
                    &device_id,
                    "grating",
                    &grating_num.to_string(),
                );
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Center wavelength
        ui.label(egui::RichText::new("Wavelength").strong());
        ui.horizontal(|ui| {
            ui.label("Center (nm):");
            let response =
                ui.add(egui::TextEdit::singleline(&mut self.wavelength_input).desired_width(80.0));
            if ui.add_enabled(!is_busy, egui::Button::new("Go")).clicked() {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                } else {
                    self.panel_state.set_error("Invalid wavelength value");
                }
            }
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !is_busy
                && let Ok(wl) = self.wavelength_input.parse::<f64>()
            {
                self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
            }
        });
        if let Some(wl) = self.state.wavelength_nm {
            ui.label(
                egui::RichText::new(format!("  Current: {wl:.2} nm"))
                    .monospace()
                    .weak(),
            );
        }

        ui.add_space(4.0);
        ui.separator();

        // Slit width
        ui.label(egui::RichText::new("Slit").strong());
        ui.horizontal(|ui| {
            ui.label("Width (um):");
            ui.add(egui::TextEdit::singleline(&mut self.slit_width_input).desired_width(60.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                if let Ok(sw) = self.slit_width_input.parse::<f64>() {
                    self.set_parameter(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        "slit_width",
                        &sw.to_string(),
                    );
                } else {
                    self.panel_state.set_error("Invalid slit width value");
                }
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Shutter toggle
        ui.label(egui::RichText::new("Shutter").strong());
        ui.horizontal(|ui| {
            let is_open = self.state.shutter_open.unwrap_or(false);
            if is_open {
                ui.colored_label(egui::Color32::GREEN, "OPEN");
                if ui
                    .add_enabled(!is_busy, egui::Button::new("Close"))
                    .clicked()
                {
                    self.set_shutter(client.as_deref_mut(), runtime, &device_id, false);
                }
            } else {
                ui.colored_label(egui::Color32::YELLOW, "CLOSED");
                if ui
                    .add_enabled(!is_busy, egui::Button::new("Open"))
                    .clicked()
                {
                    self.set_shutter(client.as_deref_mut(), runtime, &device_id, true);
                }
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Filter wheel and flipper (if available)
        let has_filter = self.state.filter_position.is_some();
        let has_flipper = self.state.flipper_position.is_some();
        if has_filter || has_flipper {
            ui.label(egui::RichText::new("Optics").strong());

            if let Some(filter) = self.state.filter_position {
                ui.horizontal(|ui| {
                    ui.label(format!("Filter: position {filter}"));
                    for pos in 1..=6u32 {
                        if ui
                            .add_enabled(
                                !is_busy && filter != pos,
                                egui::Button::new(format!("{pos}")),
                            )
                            .clicked()
                        {
                            self.set_parameter(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                "filter_position",
                                &pos.to_string(),
                            );
                        }
                    }
                });
            }

            if let Some(flipper) = self.state.flipper_position {
                ui.horizontal(|ui| {
                    ui.label("Flipper:");
                    for (pos, label) in [(0, "Direct"), (1, "Side")] {
                        if ui
                            .add_enabled(!is_busy && flipper != pos, egui::Button::new(label))
                            .clicked()
                        {
                            self.set_parameter(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                "flipper_position",
                                &pos.to_string(),
                            );
                        }
                    }
                });
            }

            ui.separator();
        }

        // Refresh + info
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.fetch_state(client, runtime, &device_id);
            }
        });

        ui.collapsing("Device Info", |ui| {
            egui::Grid::new("spectrograph_info")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Device ID:");
                    ui.label(&device_id);
                    ui.end_row();

                    ui.label("Driver:");
                    ui.label(&device.driver_type);
                    ui.end_row();

                    ui.label("Name:");
                    ui.label(&device.name);
                    ui.end_row();
                });
        });

        if self.panel_state.is_busy() {
            ui.ctx().request_repaint();
        }
    }

    fn device_type(&self) -> &'static str {
        "Spectrograph"
    }
}
