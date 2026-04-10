//! ESP300 Stage control panel.
//!
//! Provides:
//! - Position display per axis
//! - Jog controls with configurable step size
//! - Home/Stop buttons
//! - Online/offline status and explicit refresh behavior

use crate::layout;
use crate::runtime::Runtime;
use crate::time::Duration;
use egui::Ui;
use std::cell::Cell;

use crate::widgets::device_controls::{
    DeviceControlWidget, DevicePanelState, LatestRequestTracker, action_button, device_info_rows,
    filled_action_button, panel_value_text, parse_f64_input, parse_positive_step_input,
    request_panel_repaint, scoped_widget_id, show_device_info_section,
    show_panel_columns_with_state, show_panel_header, show_panel_messages, show_panel_section,
};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Stage state cached from the daemon.
#[derive(Debug, Clone, Default)]
struct StageState {
    position: Option<f64>,
    moving: bool,
    online: bool,
}

/// Async action results.
enum ActionResult {
    FetchState {
        request_id: u64,
        result: Result<StageState, String>,
    },
    Move(Result<(), String>),
    Stop(Result<(), String>),
}

#[derive(Clone, Copy)]
enum StageUiAction {
    SubmitAbsolute,
    MoveRelative(f64),
    Home,
    Stop,
    Refresh,
}

/// ESP300 Stage control panel.
pub struct StageControlPanel {
    /// Common panel state (channels, errors, device_id, etc.)
    panel_state: DevicePanelState<ActionResult>,
    fetch_request_tracker: LatestRequestTracker,
    refresh_after_command: bool,
    state: StageState,
    position_input: String,
    position_input_editing: bool,
    jog_step: String,
}

impl Default for StageControlPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            fetch_request_tracker: LatestRequestTracker::default(),
            refresh_after_command: false,
            state: StageState::default(),
            position_input: "0.0".to_string(),
            position_input_editing: false,
            jog_step: "1.0".to_string(),
        }
    }
}

impl StageControlPanel {
    const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            match result {
                ActionResult::FetchState { request_id, result } => {
                    self.panel_state.background_task_completed();
                    if !self.fetch_request_tracker.is_current(request_id) {
                        continue;
                    }

                    match result {
                        Ok(state) => {
                            if let Some(pos) = state.position
                                && !self.position_input_editing
                                && !self.state.moving
                            {
                                self.position_input = format!("{pos:.4}");
                            }
                            self.state.position = state.position;
                            self.state.online = state.online;
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
                    self.state.moving = false;
                    match result {
                        Ok(()) => {
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Move completed");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Move failed: {e}"));
                        }
                    }
                }
                ActionResult::Stop(result) => {
                    self.panel_state.action_completed();
                    self.state.moving = false;
                    match result {
                        Ok(()) => {
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Stopped");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Stop failed: {e}"));
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
        let request_id = self.fetch_request_tracker.issue();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .get_device_state(&device_id)
                .await
                .map(|proto| StageState {
                    position: proto.position,
                    moving: false,
                    online: proto.online,
                })
                .map_err(|e| e.to_string());
            let _ = tx
                .send(ActionResult::FetchState { request_id, result })
                .await;
        });
    }

    fn queue_refresh_if_needed(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.refresh_after_command && !self.panel_state.is_refreshing() {
            self.refresh_after_command = false;
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

    fn home(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        self.move_absolute(client, runtime, device_id, 0.0);
    }

    fn try_submit_absolute(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        match parse_f64_input(&self.position_input, "position") {
            Ok(position) if position.is_finite() => {
                self.move_absolute(client, runtime, device_id, position);
            }
            Ok(_) => self
                .panel_state
                .set_error("Invalid position: must be finite"),
            Err(error) => self.panel_state.set_error(error),
        }
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

        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
        }

        self.queue_refresh_if_needed(client.as_mut().map(|c| &mut **c), runtime, &device_id);

        if self.panel_state.should_refresh(Self::REFRESH_INTERVAL) && client.is_some() {
            self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
        }

        let is_busy = self.state.moving || self.panel_state.is_busy();
        let is_refreshing = self.panel_state.is_refreshing();
        let badge = Some(if self.state.online {
            ("Online", layout::colors::SUCCESS)
        } else {
            ("Offline", layout::colors::ERROR)
        });
        let pending_action = Cell::new(None);

        show_panel_header(ui, "Stage", badge, is_busy, is_refreshing);
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
                            ui.label(panel_value_text(format!("{pos:.4}")));
                        } else {
                            ui.label(panel_value_text("---"));
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Absolute Move").strong());
                    ui.horizontal(|ui| {
                        ui.label("Target:");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.position_input)
                                .desired_width(90.0),
                        );
                        panel.position_input_editing = response.has_focus();

                        if ui.add_enabled(!is_busy, action_button("Go")).clicked() {
                            pending_action.set(Some(StageUiAction::SubmitAbsolute));
                        }

                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            pending_action.set(Some(StageUiAction::SubmitAbsolute));
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Jog Controls").strong());

                    let parsed_step = parse_positive_step_input(&panel.jog_step, "step size");
                    let step = parsed_step.as_ref().copied().ok();
                    ui.horizontal(|ui| {
                        ui.label("Step size:");
                        ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.jog_step).desired_width(70.0),
                        );

                        for (label, multiplier) in
                            [("-10", -10.0), ("-1", -1.0), ("+1", 1.0), ("+10", 10.0)]
                        {
                            let enabled = !is_busy && step.is_some();
                            let tooltip = step
                                .map(|value| format!("Jog {:+.4}", value * multiplier))
                                .unwrap_or_else(|| "Enter a positive step size first".to_string());
                            if ui
                                .add_enabled(enabled, action_button(label))
                                .on_hover_text(tooltip)
                                .clicked()
                                && let Some(step) = step
                            {
                                pending_action
                                    .set(Some(StageUiAction::MoveRelative(step * multiplier)));
                            }
                        }
                    });

                    if let Err(error) = parsed_step {
                        ui.colored_label(layout::colors::WARNING, error);
                    }
                });
            },
            |ui, panel| {
                show_panel_section(ui, "Actions", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!is_busy, action_button("Home")).clicked() {
                            pending_action.set(Some(StageUiAction::Home));
                        }

                        let stop_button = filled_action_button("Stop", layout::colors::ERROR);
                        if ui.add(stop_button).clicked() {
                            pending_action.set(Some(StageUiAction::Stop));
                        }

                        if ui
                            .add_enabled(!is_refreshing, action_button("Refresh"))
                            .clicked()
                        {
                            pending_action.set(Some(StageUiAction::Refresh));
                        }
                    });

                    ui.add_space(8.0);
                    let rows = device_info_rows(
                        device,
                        [(
                            "Connection".to_string(),
                            if panel.state.online {
                                "Online".to_string()
                            } else {
                                "Offline".to_string()
                            },
                        )],
                    );
                    show_device_info_section(ui, scoped_widget_id(&device_id, "stage_info"), &rows);
                });
            },
        );

        if let Some(action) = pending_action.get() {
            match action {
                StageUiAction::SubmitAbsolute => {
                    self.try_submit_absolute(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                    );
                }
                StageUiAction::MoveRelative(delta) => {
                    self.move_relative(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        delta,
                    );
                }
                StageUiAction::Home => {
                    self.home(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
                StageUiAction::Stop => {
                    self.stop(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
                StageUiAction::Refresh => {
                    self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
            }
        }

        request_panel_repaint(ui, is_busy || is_refreshing);
    }

    fn device_type(&self) -> &'static str {
        "Stage"
    }
}
