//! Dover SmartStage control panel with Trigger-On-Position (TOP) support.
//!
//! Extends basic stage controls (position, jog, home, stop) with:
//! - Velocity control
//! - TOP configuration (start, end, increment, pulse width, bidirectional)
//! - TOP enable/disable

use std::cell::Cell;

use crate::runtime::Runtime;
use egui::Ui;

use crate::widgets::device_controls::{
    DeviceControlWidget, DevicePanelState, action_button, device_info_rows, panel_hint_text,
    panel_value_text, request_panel_repaint, scoped_widget_id, show_device_info_section,
    show_panel_columns_with_state, show_panel_header, show_panel_messages, show_panel_section,
};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// TOP (Trigger-On-Position) configuration parameters.
#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
enum DoverUiAction {
    MoveAbsolute(f64),
    MoveRelative(f64),
    Home,
    Stop,
    Refresh,
    SetVelocity(f64),
    EnableTop(TopConfig),
    DisableTop,
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
            match result {
                ActionResult::FetchState(result) => {
                    self.panel_state.background_task_completed();
                    match result {
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
                    }
                }
                ActionResult::Move(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.moving = false;
                            self.panel_state.request_refresh_after_command();
                            self.panel_state.set_status("Move completed");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Move failed: {e}"));
                            self.state.moving = false;
                        }
                    }
                }
                ActionResult::Stop(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.moving = false;
                            self.panel_state.request_refresh_after_command();
                            self.panel_state.set_status("Stopped");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Stop failed: {e}"));
                        }
                    }
                }
                ActionResult::SetParameter(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(msg) => {
                            self.panel_state.request_refresh_after_command();
                            self.panel_state.set_status(msg);
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Set failed: {e}"));
                        }
                    }
                }
                ActionResult::EnableTop(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.top_enabled = true;
                            self.panel_state.request_refresh_after_command();
                            self.panel_state.set_status("TOP enabled");
                        }
                        Err(e) => {
                            self.panel_state
                                .set_error(format!("Enable TOP failed: {e}"));
                        }
                    }
                }
                ActionResult::DisableTop(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.top_enabled = false;
                            self.panel_state.request_refresh_after_command();
                            self.panel_state.set_status("TOP disabled");
                        }
                        Err(e) => {
                            self.panel_state
                                .set_error(format!("Disable TOP failed: {e}"));
                        }
                    }
                }
            }
        }
    }

    fn fetch_state(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            return;
        };

        self.panel_state.mark_refreshed();
        self.panel_state.background_task_started();
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

    fn queue_refresh_if_needed(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.panel_state.consume_refresh_after_command() {
            self.fetch_state(client, runtime, device_id);
        }
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

        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
        }

        self.queue_refresh_if_needed(client.as_mut().map(|c| &mut **c), runtime, &device_id);

        if self
            .panel_state
            .should_refresh(std::time::Duration::from_millis(500))
            && client.is_some()
        {
            self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
        }

        let is_busy = self.state.moving || self.panel_state.is_busy();
        let is_refreshing = self.panel_state.is_refreshing();
        let badge = if self.state.top_enabled {
            Some(("TOP", egui::Color32::LIGHT_GREEN))
        } else {
            None
        };
        let pending_action = Cell::new(None);

        show_panel_header(ui, "Dover Stage", badge, is_busy, is_refreshing);
        show_panel_messages(
            ui,
            self.panel_state.error.as_deref(),
            self.panel_state.status.as_deref(),
        );
        ui.add_space(8.0);

        show_panel_columns_with_state(
            ui,
            self,
            |ui, panel| {
                show_panel_section(ui, "Motion", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Position:");
                        if let Some(pos) = panel.state.position {
                            ui.label(panel_value_text(format!("{pos:.4} mm")));
                        } else {
                            ui.label(panel_value_text("--- mm"));
                        }

                        if panel.state.online {
                            ui.colored_label(egui::Color32::GREEN, "Online");
                        } else {
                            ui.colored_label(egui::Color32::RED, "Offline");
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Absolute Move").strong());
                    ui.horizontal(|ui| {
                        ui.label("Target:");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.position_input)
                                .desired_width(60.0),
                        );
                        ui.label("mm");

                        if ui.add_enabled(!is_busy, action_button("Go")).clicked() {
                            if let Ok(pos) = panel.position_input.parse::<f64>() {
                                pending_action.set(Some(DoverUiAction::MoveAbsolute(pos)));
                            } else {
                                panel.panel_state.set_error("Invalid position value");
                            }
                        }

                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            if let Ok(pos) = panel.position_input.parse::<f64>() {
                                pending_action.set(Some(DoverUiAction::MoveAbsolute(pos)));
                            } else {
                                panel.panel_state.set_error("Invalid position value");
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Jog Controls").strong());
                    ui.horizontal(|ui| {
                        ui.label("Step:");
                        ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.jog_step).desired_width(50.0),
                        );
                        ui.label("mm");

                        let step: f64 = panel.jog_step.parse().unwrap_or(0.1);

                        if ui.add_enabled(!is_busy, action_button("<<")).clicked() {
                            pending_action.set(Some(DoverUiAction::MoveRelative(-step * 10.0)));
                        }
                        if ui.add_enabled(!is_busy, action_button("<")).clicked() {
                            pending_action.set(Some(DoverUiAction::MoveRelative(-step)));
                        }
                        if ui.add_enabled(!is_busy, action_button(">")).clicked() {
                            pending_action.set(Some(DoverUiAction::MoveRelative(step)));
                        }
                        if ui.add_enabled(!is_busy, action_button(">>")).clicked() {
                            pending_action.set(Some(DoverUiAction::MoveRelative(step * 10.0)));
                        }
                    });
                });

                show_panel_section(ui, "Velocity", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("mm/s:");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.velocity_input)
                                .desired_width(60.0),
                        );

                        if ui.add_enabled(!is_busy, action_button("Set")).clicked() {
                            if let Ok(vel) = panel.velocity_input.parse::<f64>() {
                                pending_action.set(Some(DoverUiAction::SetVelocity(vel)));
                            } else {
                                panel.panel_state.set_error("Invalid velocity value");
                            }
                        }

                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            if let Ok(vel) = panel.velocity_input.parse::<f64>() {
                                pending_action.set(Some(DoverUiAction::SetVelocity(vel)));
                            } else {
                                panel.panel_state.set_error("Invalid velocity value");
                            }
                        }
                    });

                    if let Some(vel) = panel.state.velocity {
                        ui.label(panel_hint_text(format!("Current: {vel:.2} mm/s")));
                    }
                });
            },
            |ui, panel| {
                show_panel_section(ui, "Actions", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!is_busy, action_button("Home")).clicked() {
                            pending_action.set(Some(DoverUiAction::Home));
                        }

                        if ui
                            .add(
                                egui::Button::new("Stop")
                                    .fill(egui::Color32::from_rgb(180, 60, 60)),
                            )
                            .clicked()
                        {
                            pending_action.set(Some(DoverUiAction::Stop));
                        }

                        if ui
                            .add_enabled(!is_refreshing, action_button("Refresh"))
                            .clicked()
                        {
                            pending_action.set(Some(DoverUiAction::Refresh));
                        }
                    });
                });

                show_panel_section(ui, "Trigger-On-Position (TOP)", |ui| {
                    egui::Grid::new(scoped_widget_id(&device_id, "top_config"))
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Start (mm):");
                            ui.add(
                                egui::TextEdit::singleline(&mut panel.top_start)
                                    .desired_width(60.0),
                            );
                            ui.end_row();

                            ui.label("End (mm):");
                            ui.add(
                                egui::TextEdit::singleline(&mut panel.top_end).desired_width(60.0),
                            );
                            ui.end_row();

                            ui.label("Increment (mm):");
                            ui.add(
                                egui::TextEdit::singleline(&mut panel.top_increment)
                                    .desired_width(60.0),
                            );
                            ui.end_row();

                            ui.label("Pulse width (ns):");
                            ui.add(
                                egui::TextEdit::singleline(&mut panel.top_pulse_width_ns)
                                    .desired_width(60.0),
                            );
                            ui.end_row();

                            ui.label("Bidirectional:");
                            ui.checkbox(&mut panel.top_bidirectional, "");
                            ui.end_row();
                        });

                    if let (Ok(start), Ok(end), Ok(inc)) = (
                        panel.top_start.parse::<f64>(),
                        panel.top_end.parse::<f64>(),
                        panel.top_increment.parse::<f64>(),
                    ) && inc > 0.0
                    {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let n_triggers = ((end - start) / inc) as u32;
                        ui.label(panel_hint_text(format!("{n_triggers} triggers expected")));
                    }

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if panel.state.top_enabled {
                            if ui
                                .add_enabled(
                                    !is_busy,
                                    egui::Button::new("Disable TOP")
                                        .fill(egui::Color32::from_rgb(180, 60, 60)),
                                )
                                .clicked()
                            {
                                pending_action.set(Some(DoverUiAction::DisableTop));
                            }
                        } else if ui
                            .add_enabled(
                                !is_busy,
                                egui::Button::new("Enable TOP")
                                    .fill(egui::Color32::from_rgb(60, 140, 60)),
                            )
                            .clicked()
                        {
                            pending_action.set(Some(DoverUiAction::EnableTop(TopConfig {
                                start: panel.top_start.parse::<f64>().unwrap_or(0.0),
                                end: panel.top_end.parse::<f64>().unwrap_or(20.0),
                                increment: panel.top_increment.parse::<f64>().unwrap_or(0.1),
                                pulse_width_ns: panel
                                    .top_pulse_width_ns
                                    .parse::<u32>()
                                    .unwrap_or(1000),
                                bidirectional: panel.top_bidirectional,
                            })));
                        }
                    });
                });

                show_panel_section(ui, "Device Info", |ui| {
                    let rows = device_info_rows(device, []);
                    show_device_info_section(ui, scoped_widget_id(&device_id, "dover_info"), &rows);
                });
            },
        );

        if let Some(action) = pending_action.get() {
            match action {
                DoverUiAction::MoveAbsolute(position) => {
                    self.move_absolute(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        position,
                    );
                }
                DoverUiAction::MoveRelative(delta) => {
                    self.move_relative(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        delta,
                    );
                }
                DoverUiAction::Home => {
                    self.move_absolute(client.as_mut().map(|c| &mut **c), runtime, &device_id, 0.0);
                }
                DoverUiAction::Stop => {
                    self.stop(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
                DoverUiAction::Refresh => {
                    self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
                DoverUiAction::SetVelocity(velocity) => {
                    self.set_velocity(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        velocity,
                    );
                }
                DoverUiAction::EnableTop(top) => {
                    self.enable_top(client.as_mut().map(|c| &mut **c), runtime, &device_id, top);
                }
                DoverUiAction::DisableTop => {
                    self.disable_top(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
            }
        }

        request_panel_repaint(ui, is_busy || is_refreshing);
    }

    fn device_type(&self) -> &'static str {
        "DoverStage"
    }
}
