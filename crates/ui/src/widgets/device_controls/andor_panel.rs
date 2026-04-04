//! Andor iStar gated camera control panel.
//!
//! Provides:
//! - Temperature display (sensor readback)
//! - Exposure time control
//! - Trigger mode selector (Internal / External)
//! - DDG (Digital Delay Generator) delay and width
//! - MCP gain slider
//! - Arm / Disarm controls
//! - Acquisition status indicator

use crate::runtime::Runtime;
use egui::Ui;

use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Andor camera state cached from the daemon.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // `online` reserved for future online/offline indicator
struct AndorCameraState {
    temperature: Option<f64>,
    exposure_s: Option<f64>,
    trigger_mode: Option<String>,
    ddg_delay_ps: Option<i64>,
    ddg_width_ps: Option<i64>,
    mcp_gain: Option<i32>,
    armed: bool,
    cooling: bool,
    online: bool,
}

/// Async action results for the Andor camera panel.
enum ActionResult {
    FetchState(Result<AndorCameraState, String>),
    SetParameter(Result<String, String>),
    Arm(Result<(), String>),
    Disarm(Result<(), String>),
}

/// Andor iStar gated camera control panel.
pub struct AndorCameraPanel {
    panel_state: DevicePanelState<ActionResult>,
    state: AndorCameraState,
    exposure_input: String,
    ddg_delay_input: String,
    ddg_width_input: String,
    mcp_gain_input: f32,
    trigger_mode_idx: usize,
}

const TRIGGER_MODES: [&str; 2] = ["Internal", "External"];

impl Default for AndorCameraPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            state: AndorCameraState::default(),
            exposure_input: "0.01".to_string(),
            ddg_delay_input: "1300000".to_string(),
            ddg_width_input: "10000000".to_string(),
            mcp_gain_input: 0.0,
            trigger_mode_idx: 0,
        }
    }
}

impl AndorCameraPanel {
    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            self.panel_state.action_completed();

            match result {
                ActionResult::FetchState(result) => match result {
                    Ok(state) => {
                        if let Some(exp) = state.exposure_s {
                            self.exposure_input = format!("{:.4}", exp);
                        }
                        if let Some(delay) = state.ddg_delay_ps {
                            self.ddg_delay_input = delay.to_string();
                        }
                        if let Some(width) = state.ddg_width_ps {
                            self.ddg_width_input = width.to_string();
                        }
                        if let Some(gain) = state.mcp_gain {
                            #[allow(clippy::cast_precision_loss)]
                            {
                                self.mcp_gain_input = gain as f32;
                            }
                        }
                        if let Some(ref mode) = state.trigger_mode {
                            self.trigger_mode_idx = TRIGGER_MODES
                                .iter()
                                .position(|m| m.eq_ignore_ascii_case(mode))
                                .unwrap_or(0);
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
                ActionResult::Arm(result) => match result {
                    Ok(()) => {
                        self.state.armed = true;
                        self.panel_state.set_status("Camera armed");
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Arm failed: {e}"));
                    }
                },
                ActionResult::Disarm(result) => match result {
                    Ok(()) => {
                        self.state.armed = false;
                        self.panel_state.set_status("Camera disarmed");
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Disarm failed: {e}"));
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
            let mut state = AndorCameraState {
                online: true,
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

            state.temperature = get_param(&mut client, &device_id, "temperature")
                .await
                .and_then(|v| v.parse::<f64>().ok());
            state.exposure_s = get_param(&mut client, &device_id, "exposure")
                .await
                .and_then(|v| v.parse::<f64>().ok());
            state.trigger_mode = get_param(&mut client, &device_id, "trigger_mode").await;
            state.ddg_delay_ps = get_param(&mut client, &device_id, "ddg_delay")
                .await
                .and_then(|v| v.parse::<i64>().ok());
            state.ddg_width_ps = get_param(&mut client, &device_id, "ddg_width")
                .await
                .and_then(|v| v.parse::<i64>().ok());
            state.mcp_gain = get_param(&mut client, &device_id, "mcp_gain")
                .await
                .and_then(|v| v.parse::<i32>().ok());
            state.armed = get_param(&mut client, &device_id, "armed")
                .await
                .map(|v| v == "true")
                .unwrap_or(false);
            state.cooling = get_param(&mut client, &device_id, "cooling")
                .await
                .map(|v| v == "true")
                .unwrap_or(false);

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

    fn arm(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "arm", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Arm(result)).await;
        });
    }

    fn disarm(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "disarm", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Disarm(result)).await;
        });
    }
}

impl DeviceControlWidget for AndorCameraPanel {
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

        // Auto-refresh every 2 seconds
        if self
            .panel_state
            .should_refresh(std::time::Duration::from_secs(2))
        {
            self.panel_state.mark_refreshed();
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("Andor iStar");
            if self.state.armed {
                ui.colored_label(egui::Color32::GREEN, "ARMED");
            }
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

        // Temperature
        ui.horizontal(|ui| {
            ui.label("Temperature:");
            if let Some(temp) = self.state.temperature {
                let color = if temp <= -10.0 {
                    egui::Color32::LIGHT_BLUE
                } else {
                    egui::Color32::YELLOW
                };
                ui.colored_label(color, format!("{temp:.1} C"));
            } else {
                ui.label("---");
            }
            if self.state.cooling {
                ui.colored_label(egui::Color32::LIGHT_BLUE, "(cooling)");
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Exposure
        ui.label(egui::RichText::new("Exposure").strong());
        ui.horizontal(|ui| {
            ui.label("Time (s):");
            let response =
                ui.add(egui::TextEdit::singleline(&mut self.exposure_input).desired_width(80.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                if let Ok(val) = self.exposure_input.parse::<f64>() {
                    self.set_parameter(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        "exposure",
                        &val.to_string(),
                    );
                } else {
                    self.panel_state.set_error("Invalid exposure value");
                }
            }

            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !is_busy
                && let Ok(val) = self.exposure_input.parse::<f64>()
            {
                self.set_parameter(
                    client.as_deref_mut(),
                    runtime,
                    &device_id,
                    "exposure",
                    &val.to_string(),
                );
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Trigger Mode
        ui.label(egui::RichText::new("Trigger Mode").strong());
        ui.horizontal(|ui| {
            let prev = self.trigger_mode_idx;
            egui::ComboBox::from_id_salt("trigger_mode")
                .selected_text(TRIGGER_MODES[self.trigger_mode_idx])
                .show_ui(ui, |ui| {
                    for (i, mode) in TRIGGER_MODES.iter().enumerate() {
                        ui.selectable_value(&mut self.trigger_mode_idx, i, *mode);
                    }
                });
            if self.trigger_mode_idx != prev && !is_busy {
                self.set_parameter(
                    client.as_deref_mut(),
                    runtime,
                    &device_id,
                    "trigger_mode",
                    TRIGGER_MODES[self.trigger_mode_idx],
                );
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // DDG Controls
        ui.label(egui::RichText::new("DDG (Gate Timing)").strong());

        ui.horizontal(|ui| {
            ui.label("Delay (ps):");
            ui.add(egui::TextEdit::singleline(&mut self.ddg_delay_input).desired_width(100.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                if let Ok(val) = self.ddg_delay_input.parse::<i64>() {
                    self.set_parameter(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        "ddg_delay",
                        &val.to_string(),
                    );
                } else {
                    self.panel_state.set_error("Invalid DDG delay value");
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Width (ps):");
            ui.add(egui::TextEdit::singleline(&mut self.ddg_width_input).desired_width(100.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                if let Ok(val) = self.ddg_width_input.parse::<i64>() {
                    self.set_parameter(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        "ddg_width",
                        &val.to_string(),
                    );
                } else {
                    self.panel_state.set_error("Invalid DDG width value");
                }
            }
        });

        // Show human-readable DDG values
        if let (Some(delay), Some(width)) = (self.state.ddg_delay_ps, self.state.ddg_width_ps) {
            #[allow(clippy::cast_precision_loss)]
            let delay_us = delay as f64 / 1_000_000.0;
            #[allow(clippy::cast_precision_loss)]
            let width_us = width as f64 / 1_000_000.0;
            ui.label(
                egui::RichText::new(format!("  {delay_us:.1} us delay, {width_us:.1} us gate"))
                    .weak(),
            );
        }

        ui.add_space(4.0);
        ui.separator();

        // MCP Gain
        ui.label(egui::RichText::new("MCP Gain").strong());
        ui.horizontal(|ui| {
            let prev = self.mcp_gain_input;
            ui.add(egui::Slider::new(&mut self.mcp_gain_input, 0.0..=4095.0).text(""));
            // Send on slider release (drag_stopped)
            if (self.mcp_gain_input - prev).abs() > f32::EPSILON
                && !ui.input(|i| i.pointer.any_down())
                && !is_busy
            {
                #[allow(clippy::cast_possible_truncation)]
                let gain = self.mcp_gain_input as i32;
                self.set_parameter(
                    client.as_deref_mut(),
                    runtime,
                    &device_id,
                    "mcp_gain",
                    &gain.to_string(),
                );
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Arm / Disarm
        ui.horizontal(|ui| {
            if self.state.armed {
                if ui
                    .add_enabled(!is_busy, egui::Button::new("Disarm"))
                    .clicked()
                {
                    self.disarm(client.as_deref_mut(), runtime, &device_id);
                }
            } else if ui
                .add_enabled(
                    !is_busy,
                    egui::Button::new("Arm").fill(egui::Color32::from_rgb(60, 140, 60)),
                )
                .clicked()
            {
                self.arm(client.as_deref_mut(), runtime, &device_id);
            }

            if ui.button("Refresh").clicked() {
                self.fetch_state(client, runtime, &device_id);
            }
        });

        // Device info collapsible
        ui.collapsing("Device Info", |ui| {
            egui::Grid::new("andor_info")
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
        "AndorCamera"
    }
}
