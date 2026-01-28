//! ESP300 Stage control panel.
//!
//! Provides:
//! - Position display per axis
//! - Jog controls with configurable step size
//! - Home/Stop buttons
//! - Velocity display

use egui::Ui;
use tokio::runtime::Runtime;

use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use daq_client::DaqClient;
use protocol::daq::DeviceInfo;

/// Stage state cached from the daemon
#[derive(Debug, Clone, Default)]
struct StageState {
    position: Option<f64>,
    moving: bool,
    online: bool,
}

/// Async action results
enum ActionResult {
    FetchState(Result<StageState, String>),
    Move(Result<(), String>),
    Stop(Result<(), String>),
}

/// ESP300 Stage control panel
pub struct StageControlPanel {
    /// Common panel state (channels, errors, device_id, etc.)
    panel_state: DevicePanelState<ActionResult>,
    state: StageState,
    position_input: String,
    jog_step: String,
}

impl Default for StageControlPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            state: StageState::default(),
            position_input: "0.0".to_string(),
            jog_step: "1.0".to_string(),
        }
    }
}

impl StageControlPanel {
    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            self.panel_state.action_completed();

            match result {
                ActionResult::FetchState(result) => match result {
                    Ok(state) => {
                        self.state = state;
                        if let Some(pos) = self.state.position {
                            self.position_input = format!("{:.4}", pos);
                        }
                        self.panel_state.error = None;
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Failed to fetch state: {}", e));
                    }
                },
                ActionResult::Move(result) => match result {
                    Ok(()) => {
                        self.panel_state.set_status("Move completed");
                        self.state.moving = false;
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Move failed: {}", e));
                        self.state.moving = false;
                    }
                },
                ActionResult::Stop(result) => match result {
                    Ok(()) => {
                        self.panel_state.set_status("Stopped");
                        self.state.moving = false;
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Stop failed: {}", e));
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
            let result = client.get_device_state(&device_id).await;
            let state_result = result
                .map(|proto| StageState {
                    position: proto.position,
                    moving: false,
                    online: proto.online,
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::FetchState(state_result)).await;
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
            // Use execute_device_command to send stop
            let result = client
                .execute_device_command(&device_id, "stop", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Stop(result)).await;
        });
    }

    fn home(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        // Home by moving to 0
        self.move_absolute(client, runtime, device_id, 0.0);
    }
}

impl DeviceControlWidget for StageControlPanel {
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
            ui.heading("📍 Stage");
            if self.state.moving || self.panel_state.is_busy() {
                ui.spinner();
                ui.label("Moving...");
            }
        });

        if let Some(ref err) = self.panel_state.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.panel_state.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        // Current position display
        ui.horizontal(|ui| {
            ui.label("Position:");
            if let Some(pos) = self.state.position {
                ui.label(
                    egui::RichText::new(format!("{:.4}", pos))
                        .monospace()
                        .strong()
                        .size(18.0),
                );
            } else {
                ui.label(egui::RichText::new("---").monospace().size(18.0));
            }

            // Online indicator
            if self.state.online {
                ui.colored_label(egui::Color32::GREEN, "● Online");
            } else {
                ui.colored_label(egui::Color32::RED, "● Offline");
            }
        });

        ui.add_space(8.0);
        ui.separator();

        let is_busy = self.state.moving || self.panel_state.is_busy();

        // Move to absolute position
        ui.label(egui::RichText::new("Absolute Move").strong());

        ui.horizontal(|ui| {
            ui.label("Target:");
            let response =
                ui.add(egui::TextEdit::singleline(&mut self.position_input).desired_width(80.0));

            if ui.add_enabled(!is_busy, egui::Button::new("Go")).clicked() {
                if let Ok(pos) = self.position_input.parse::<f64>() {
                    self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
                } else {
                    self.panel_state.error = Some("Invalid position value".to_string());
                }
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !is_busy {
                if let Ok(pos) = self.position_input.parse::<f64>() {
                    self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
                }
            }
        });

        ui.add_space(8.0);

        // Jog controls
        ui.label(egui::RichText::new("Jog Controls").strong());

        ui.horizontal(|ui| {
            ui.label("Step size:");
            ui.add(egui::TextEdit::singleline(&mut self.jog_step).desired_width(60.0));

            let step: f64 = self.jog_step.parse().unwrap_or(1.0);

            if ui.add_enabled(!is_busy, egui::Button::new("◀◀")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -step * 10.0);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("◀")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -step);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("▶")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, step);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("▶▶")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, step * 10.0);
            }
        });

        ui.add_space(8.0);
        ui.separator();

        // Action buttons
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!is_busy, egui::Button::new("🏠 Home"))
                .clicked()
            {
                self.home(client.as_deref_mut(), runtime, &device_id);
            }

            // Stop button - always enabled
            if ui
                .add(egui::Button::new("🛑 Stop").fill(egui::Color32::from_rgb(180, 60, 60)))
                .clicked()
            {
                self.stop(client.as_deref_mut(), runtime, &device_id);
            }

            if ui.button("🔄 Refresh").clicked() {
                self.fetch_state(client, runtime, &device_id);
            }
        });

        // Device info
        ui.collapsing("▶ Device Info", |ui| {
            egui::Grid::new("stage_info")
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

        // Request repaint while moving
        if self.state.moving || self.panel_state.is_busy() {
            ui.ctx().request_repaint();
        }
    }

    fn device_type(&self) -> &'static str {
        "Stage"
    }
}
