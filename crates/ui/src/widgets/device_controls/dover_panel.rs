//! Dover SmartStage control panel with Trigger-On-Position (TOP) support.
//!
//! Extends basic stage controls (position, jog, home, stop) with:
//! - Velocity control
//! - TOP configuration (start, end, increment, pulse width, bidirectional)
//! - TOP enable/disable

use crate::runtime::Runtime;
use egui::Ui;

use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// TOP (Trigger-On-Position) configuration parameters.
struct TopConfig {
    start: f64,
    end: f64,
    increment: f64,
    pulse_width_ns: u32,
    bidirectional: bool,
}

/// Dover stage state cached from the daemon.
#[derive(Debug, Clone, Default)]
struct DoverStageState {
    position: Option<f64>,
    velocity: Option<f64>,
    moving: bool,
    top_enabled: bool,
    online: bool,
}

/// Async action results for the Dover stage panel.
enum ActionResult {
    FetchState(Result<DoverStageState, String>),
    Move(Result<(), String>),
    Stop(Result<(), String>),
    SetParameter(Result<String, String>),
    EnableTop(Result<(), String>),
    DisableTop(Result<(), String>),
}

/// Dover SmartStage control panel with TOP support.
pub struct DoverStagePanel {
    panel_state: DevicePanelState<ActionResult>,
    state: DoverStageState,
    position_input: String,
    jog_step: String,
    velocity_input: String,
    // TOP configuration
    top_start: String,
    top_end: String,
    top_increment: String,
    top_pulse_width_ns: String,
    top_bidirectional: bool,
}

impl Default for DoverStagePanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            state: DoverStageState::default(),
            position_input: "0.0".to_string(),
            jog_step: "0.1".to_string(),
            velocity_input: "1.0".to_string(),
            top_start: "0.0".to_string(),
            top_end: "20.0".to_string(),
            top_increment: "0.1".to_string(),
            top_pulse_width_ns: "1000".to_string(),
            top_bidirectional: false,
        }
    }
}

impl DoverStagePanel {
    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            self.panel_state.action_completed();

            match result {
                ActionResult::FetchState(result) => match result {
                    Ok(state) => {
                        if let Some(pos) = state.position {
                            self.position_input = format!("{pos:.4}");
                        }
                        if let Some(vel) = state.velocity {
                            self.velocity_input = format!("{vel:.2}");
                        }
                        self.state = state;
                        self.panel_state.error = None;
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Failed to fetch state: {e}"));
                    }
                },
                ActionResult::Move(result) => match result {
                    Ok(()) => {
                        self.panel_state.set_status("Move completed");
                        self.state.moving = false;
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Move failed: {e}"));
                        self.state.moving = false;
                    }
                },
                ActionResult::Stop(result) => match result {
                    Ok(()) => {
                        self.panel_state.set_status("Stopped");
                        self.state.moving = false;
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Stop failed: {e}"));
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
                ActionResult::EnableTop(result) => match result {
                    Ok(()) => {
                        self.state.top_enabled = true;
                        self.panel_state.set_status("TOP enabled");
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Enable TOP failed: {e}"));
                    }
                },
                ActionResult::DisableTop(result) => match result {
                    Ok(()) => {
                        self.state.top_enabled = false;
                        self.panel_state.set_status("TOP disabled");
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Disable TOP failed: {e}"));
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
            // Fetch position/online from device state
            let dev_state = client.get_device_state(&device_id).await;

            let state_result = dev_state
                .map(|proto| DoverStageState {
                    position: proto.position,
                    moving: false,
                    online: proto.online,
                    ..Default::default()
                })
                .map_err(|e| e.to_string());

            // Supplement with parameter values for velocity/TOP if available
            if let Ok(mut state) = state_result {
                if let Ok(pv) = client.get_parameter(&device_id, "velocity").await {
                    state.velocity = pv.value.parse::<f64>().ok();
                }
                if let Ok(pv) = client.get_parameter(&device_id, "top_enabled").await {
                    state.top_enabled = pv.value == "true";
                }
                let _ = tx.send(ActionResult::FetchState(Ok(state))).await;
            } else {
                let _ = tx.send(ActionResult::FetchState(state_result)).await;
            }
        });
    }

    fn move_absolute(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        position: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.state.moving = true;
        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .move_absolute(&device_id, position)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Move(result)).await;
        });
    }

    fn move_relative(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        delta: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.state.moving = true;
        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .move_relative(&device_id, delta)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Move(result)).await;
        });
    }

    fn stop(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "stop", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Stop(result)).await;
        });
    }

    fn set_velocity(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        velocity: f64,
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
                .set_parameter(&device_id, "velocity", &velocity.to_string())
                .await
                .map(|_| format!("Velocity set to {velocity:.2} mm/s"))
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetParameter(result)).await;
        });
    }

    fn enable_top(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        top: TopConfig,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        let args = format!(
            "{},{},{},{},{}",
            top.start, top.end, top.increment, top.pulse_width_ns, top.bidirectional
        );

        runtime.spawn(async move {
            let result = client
                .execute_device_command(&device_id, "enable_top", &args)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::EnableTop(result)).await;
        });
    }

    fn disable_top(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "disable_top", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::DisableTop(result)).await;
        });
    }
}

impl DeviceControlWidget for DoverStagePanel {
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

        // Header
        ui.horizontal(|ui| {
            ui.heading("Dover Stage");
            if self.state.moving || self.panel_state.is_busy() {
                ui.spinner();
                ui.label("Moving...");
            }
            if self.state.top_enabled {
                ui.colored_label(egui::Color32::LIGHT_GREEN, "TOP");
            }
        });

        if let Some(ref err) = self.panel_state.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.panel_state.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        // Current position
        ui.horizontal(|ui| {
            ui.label("Position:");
            if let Some(pos) = self.state.position {
                ui.label(
                    egui::RichText::new(format!("{pos:.4} mm"))
                        .monospace()
                        .strong()
                        .size(18.0),
                );
            } else {
                ui.label(egui::RichText::new("---").monospace().size(18.0));
            }

            if self.state.online {
                ui.colored_label(egui::Color32::GREEN, "Online");
            } else {
                ui.colored_label(egui::Color32::RED, "Offline");
            }
        });

        ui.add_space(4.0);
        ui.separator();

        let is_busy = self.state.moving || self.panel_state.is_busy();

        // Absolute move
        ui.label(egui::RichText::new("Absolute Move").strong());
        ui.horizontal(|ui| {
            ui.label("Target (mm):");
            let response =
                ui.add(egui::TextEdit::singleline(&mut self.position_input).desired_width(80.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Go")).clicked() {
                if let Ok(pos) = self.position_input.parse::<f64>() {
                    self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
                } else {
                    self.panel_state.set_error("Invalid position value");
                }
            }

            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !is_busy
                && let Ok(pos) = self.position_input.parse::<f64>()
            {
                self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
            }
        });

        ui.add_space(4.0);

        // Jog controls
        ui.label(egui::RichText::new("Jog Controls").strong());
        ui.horizontal(|ui| {
            ui.label("Step (mm):");
            ui.add(egui::TextEdit::singleline(&mut self.jog_step).desired_width(60.0));

            let step: f64 = self.jog_step.parse().unwrap_or(0.1);

            if ui.add_enabled(!is_busy, egui::Button::new("<<")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -step * 10.0);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("<")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -step);
            }
            if ui.add_enabled(!is_busy, egui::Button::new(">")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, step);
            }
            if ui.add_enabled(!is_busy, egui::Button::new(">>")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, step * 10.0);
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Velocity
        ui.label(egui::RichText::new("Velocity").strong());
        ui.horizontal(|ui| {
            ui.label("mm/s:");
            ui.add(egui::TextEdit::singleline(&mut self.velocity_input).desired_width(60.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                if let Ok(vel) = self.velocity_input.parse::<f64>() {
                    self.set_velocity(client.as_deref_mut(), runtime, &device_id, vel);
                } else {
                    self.panel_state.set_error("Invalid velocity value");
                }
            }
        });
        if let Some(vel) = self.state.velocity {
            ui.label(egui::RichText::new(format!("  Current: {vel:.2} mm/s")).weak());
        }

        ui.add_space(4.0);
        ui.separator();

        // TOP Configuration (collapsible)
        ui.collapsing(
            egui::RichText::new("Trigger-On-Position (TOP)").strong(),
            |ui| {
                egui::Grid::new("top_config")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Start (mm):");
                        ui.add(egui::TextEdit::singleline(&mut self.top_start).desired_width(60.0));
                        ui.end_row();

                        ui.label("End (mm):");
                        ui.add(egui::TextEdit::singleline(&mut self.top_end).desired_width(60.0));
                        ui.end_row();

                        ui.label("Increment (mm):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.top_increment).desired_width(60.0),
                        );
                        ui.end_row();

                        ui.label("Pulse width (ns):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.top_pulse_width_ns)
                                .desired_width(60.0),
                        );
                        ui.end_row();

                        ui.label("Bidirectional:");
                        ui.checkbox(&mut self.top_bidirectional, "");
                        ui.end_row();
                    });

                // Show computed trigger count
                if let (Ok(start), Ok(end), Ok(inc)) = (
                    self.top_start.parse::<f64>(),
                    self.top_end.parse::<f64>(),
                    self.top_increment.parse::<f64>(),
                ) && inc > 0.0
                {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let n_triggers = ((end - start) / inc) as u32;
                    ui.label(
                        egui::RichText::new(format!("  {n_triggers} triggers expected")).weak(),
                    );
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if self.state.top_enabled {
                        if ui
                            .add_enabled(
                                !is_busy,
                                egui::Button::new("Disable TOP")
                                    .fill(egui::Color32::from_rgb(180, 60, 60)),
                            )
                            .clicked()
                        {
                            self.disable_top(client.as_deref_mut(), runtime, &device_id);
                        }
                    } else if ui
                        .add_enabled(
                            !is_busy,
                            egui::Button::new("Enable TOP")
                                .fill(egui::Color32::from_rgb(60, 140, 60)),
                        )
                        .clicked()
                    {
                        self.enable_top(
                            client.as_deref_mut(),
                            runtime,
                            &device_id,
                            TopConfig {
                                start: self.top_start.parse::<f64>().unwrap_or(0.0),
                                end: self.top_end.parse::<f64>().unwrap_or(20.0),
                                increment: self.top_increment.parse::<f64>().unwrap_or(0.1),
                                pulse_width_ns: self
                                    .top_pulse_width_ns
                                    .parse::<u32>()
                                    .unwrap_or(1000),
                                bidirectional: self.top_bidirectional,
                            },
                        );
                    }
                });
            },
        );

        ui.add_space(4.0);
        ui.separator();

        // Action buttons
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!is_busy, egui::Button::new("Home"))
                .clicked()
            {
                self.move_absolute(client.as_deref_mut(), runtime, &device_id, 0.0);
            }

            if ui
                .add(egui::Button::new("Stop").fill(egui::Color32::from_rgb(180, 60, 60)))
                .clicked()
            {
                self.stop(client.as_deref_mut(), runtime, &device_id);
            }

            if ui.button("Refresh").clicked() {
                self.fetch_state(client, runtime, &device_id);
            }
        });

        // Device info
        ui.collapsing("Device Info", |ui| {
            egui::Grid::new("dover_info")
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

        if self.state.moving || self.panel_state.is_busy() {
            ui.ctx().request_repaint();
        }
    }

    fn device_type(&self) -> &'static str {
        "DoverStage"
    }
}
