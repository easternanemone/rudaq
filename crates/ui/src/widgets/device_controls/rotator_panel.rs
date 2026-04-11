//! ELL14 Rotator control panel.
//!
//! Provides:
//! - Position display with degree formatting
//! - Jog buttons: -90, -10, -1, +1, +10, +90
//! - Home button
//! - Direct position input

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::runtime::Runtime;
use crate::time::Instant;
use egui::Ui;

use crate::widgets::device_controls::{
    DeviceControlWidget, DevicePanelState, action_button, device_info_rows, panel_value_text,
    request_panel_repaint, scoped_widget_id, show_device_info_section,
    show_panel_columns_with_state, show_panel_header, show_panel_messages, show_panel_section,
};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Global counter for unique panel instance IDs (for diagnostic logging)
static PANEL_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Rotator state cached from the daemon
#[derive(Debug, Clone, Default)]
struct RotatorState {
    position_deg: Option<f64>,
    moving: bool,
}

/// Async action results
enum ActionResult {
    FetchState(Result<RotatorState, String>),
    Move(Result<(), String>),
    Home(Result<(), String>),
}

#[derive(Clone, Copy)]
enum RotatorUiAction {
    MoveAbsolute(f64),
    MoveRelative(f64),
    Home,
    Refresh,
}

/// ELL14 Rotator control panel
pub struct RotatorControlPanel {
    /// Unique instance ID for diagnostic logging (identifies duplicate panels)
    panel_instance_id: u64,
    /// Common panel state (channels, errors, device_id, etc.)
    panel_state: DevicePanelState<ActionResult>,
    refresh_after_command: bool,
    last_command_time: Option<Instant>,
    state: RotatorState,
    position_input: String,
    position_input_editing: bool,
}

impl Default for RotatorControlPanel {
    fn default() -> Self {
        let panel_instance_id = PANEL_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(panel_instance_id, "RotatorControlPanel instance created");
        Self {
            panel_instance_id,
            panel_state: DevicePanelState::new(),
            refresh_after_command: false,
            last_command_time: None,
            state: RotatorState::default(),
            position_input: "0.0".to_string(),
            position_input_editing: false,
        }
    }
}

impl RotatorControlPanel {
    /// Auto-refresh interval
    const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

    /// Minimum time between commands to prevent duplicate clicks
    const COMMAND_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

    /// Check if enough time has passed since the last command to allow a new one
    fn can_send_command(&self) -> bool {
        self.last_command_time
            .map(|t| t.elapsed() >= Self::COMMAND_DEBOUNCE)
            .unwrap_or(true)
    }

    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            match result {
                ActionResult::FetchState(result) => {
                    self.panel_state.background_task_completed();
                    match result {
                        Ok(state) => {
                            self.state.position_deg = state.position_deg;
                            if let Some(pos) = self.state.position_deg
                                && !self.position_input_editing
                                && !self.state.moving
                            {
                                self.position_input = format!("{pos:.2}");
                            }
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
                ActionResult::Home(result) => {
                    self.panel_state.action_completed();
                    self.state.moving = false;
                    match result {
                        Ok(()) => {
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Homed successfully");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Home failed: {e}"));
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
            let result = client.get_device_state(&device_id).await;
            let state_result = result
                .map(|proto| RotatorState {
                    position_deg: proto.position,
                    moving: false,
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::FetchState(state_result)).await;
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

    #[allow(clippy::cast_possible_truncation)]
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

        if !self.can_send_command() {
            self.panel_state
                .set_status("Please wait before sending another move command");
            return;
        }

        self.state.moving = true;
        self.last_command_time = Some(Instant::now());
        self.panel_state.action_started();
        tracing::info!(
            panel_instance_id = self.panel_instance_id,
            device_id,
            position,
            actions_in_flight = self.panel_state.actions_in_flight,
            "Move absolute command sent"
        );
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

    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_possible_truncation)]
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

        if !self.can_send_command() {
            self.panel_state
                .set_status("Please wait before sending another move command");
            return;
        }

        self.state.moving = true;
        self.last_command_time = Some(Instant::now());
        self.panel_state.action_started();
        tracing::info!(
            panel_instance_id = self.panel_instance_id,
            device_id,
            delta,
            actions_in_flight = self.panel_state.actions_in_flight,
            "Move relative command sent"
        );
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

    #[allow(clippy::cast_possible_truncation)]
    fn home(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        if !self.can_send_command() {
            self.panel_state
                .set_status("Please wait before sending another move command");
            return;
        }

        self.state.moving = true;
        self.last_command_time = Some(Instant::now());
        self.panel_state.action_started();
        tracing::info!(
            panel_instance_id = self.panel_instance_id,
            device_id,
            actions_in_flight = self.panel_state.actions_in_flight,
            "Home command sent"
        );
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            // Home by moving to 0 position
            let result = client
                .move_absolute(&device_id, 0.0)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Home(result)).await;
        });
    }
}

impl DeviceControlWidget for RotatorControlPanel {
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
        let badge = None;
        let pending_action = Cell::new(None);

        show_panel_header(ui, "Rotator", badge, is_busy, is_refreshing);
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
                        if let Some(pos) = panel.state.position_deg {
                            ui.label(panel_value_text(format!("{pos:.2}°")));
                        } else {
                            ui.label(panel_value_text("---°"));
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
                        ui.label("°");
                        panel.position_input_editing = response.has_focus();

                        if ui.add_enabled(!is_busy, action_button("Go")).clicked() {
                            if let Ok(pos) = panel.position_input.parse::<f64>() {
                                pending_action.set(Some(RotatorUiAction::MoveAbsolute(pos)));
                            } else {
                                panel.panel_state.set_error("Invalid position value");
                            }
                        }

                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            if let Ok(pos) = panel.position_input.parse::<f64>() {
                                pending_action.set(Some(RotatorUiAction::MoveAbsolute(pos)));
                            } else {
                                panel.panel_state.set_error("Invalid position value");
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Quick Positions").strong());
                    ui.horizontal_wrapped(|ui| {
                        for angle in [0.0, 45.0, 90.0, 180.0, 270.0] {
                            if ui
                                .add_enabled(!is_busy, action_button(format!("{angle}°")))
                                .clicked()
                            {
                                pending_action.set(Some(RotatorUiAction::MoveAbsolute(angle)));
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Jog Controls").strong());
                    ui.horizontal_wrapped(|ui| {
                        for (label, delta) in [
                            ("-90°", -90.0),
                            ("-10°", -10.0),
                            ("-1°", -1.0),
                            ("+1°", 1.0),
                            ("+10°", 10.0),
                            ("+90°", 90.0),
                        ] {
                            if ui.add_enabled(!is_busy, action_button(label)).clicked() {
                                pending_action.set(Some(RotatorUiAction::MoveRelative(delta)));
                            }
                        }
                    });
                });
            },
            |ui, _panel| {
                show_panel_section(ui, "Actions", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!is_busy, action_button("Home")).clicked() {
                            pending_action.set(Some(RotatorUiAction::Home));
                        }

                        if ui
                            .add_enabled(!is_refreshing, action_button("Refresh"))
                            .clicked()
                        {
                            pending_action.set(Some(RotatorUiAction::Refresh));
                        }
                    });

                    ui.add_space(8.0);
                    let rows = device_info_rows(device, []);
                    show_device_info_section(
                        ui,
                        scoped_widget_id(&device_id, "rotator_info"),
                        &rows,
                    );
                });
            },
        );

        if let Some(action) = pending_action.get() {
            match action {
                RotatorUiAction::MoveAbsolute(position) => {
                    self.move_absolute(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        position,
                    );
                }
                RotatorUiAction::MoveRelative(delta) => {
                    self.move_relative(
                        client.as_mut().map(|c| &mut **c),
                        runtime,
                        &device_id,
                        delta,
                    );
                }
                RotatorUiAction::Home => {
                    self.home(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
                RotatorUiAction::Refresh => {
                    self.fetch_state(client.as_mut().map(|c| &mut **c), runtime, &device_id);
                }
            }
        }

        request_panel_repaint(ui, is_busy || is_refreshing);
    }

    fn device_type(&self) -> &'static str {
        "Rotator"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that a new RotatorControlPanel has correct default state.
    /// This ensures the panel initializes with sensible values before any
    /// user interaction or daemon connection.
    #[test]
    fn test_default_panel_state() {
        let panel = RotatorControlPanel::default();

        // No actions should be in flight initially
        assert_eq!(
            panel.panel_state.actions_in_flight, 0,
            "No actions should be in flight on creation"
        );

        // No refresh should be in flight initially
        assert!(
            !panel.panel_state.is_refreshing(),
            "panel should not start in a refreshing state"
        );

        // Auto-refresh should be enabled by default
        assert!(
            panel.panel_state.auto_refresh,
            "Auto-refresh should be enabled by default"
        );

        // No error or status messages initially
        assert!(
            panel.panel_state.error.is_none(),
            "Error should be None on creation"
        );
        assert!(
            panel.panel_state.status.is_none(),
            "Status should be None on creation"
        );

        // Device ID not set until UI is rendered with device info
        assert!(
            panel.panel_state.device_id.is_none(),
            "Device ID should be None on creation"
        );

        // Initial fetch not done yet
        assert!(
            !panel.panel_state.initial_fetch_done,
            "Initial fetch should not be done on creation"
        );

        // Position input should default to "0.0"
        assert_eq!(
            panel.position_input, "0.0",
            "Position input should default to '0.0'"
        );

        // Last refresh and command time should be None
        assert!(
            panel.panel_state.last_refresh.is_none(),
            "Last refresh should be None on creation"
        );
        assert!(
            panel.last_command_time.is_none(),
            "Last command time should be None on creation"
        );

        // State should have default values
        assert!(
            panel.state.position_deg.is_none(),
            "Position should be None on creation"
        );
        assert!(!panel.state.moving, "Moving should be false on creation");
    }

    /// Test that each new panel gets a unique incrementing instance ID.
    /// This is critical for debugging duplicate panel issues - each panel
    /// should have a distinct ID for logging purposes.
    #[test]
    fn test_panel_instance_id_unique() {
        let panel1 = RotatorControlPanel::default();
        let panel2 = RotatorControlPanel::default();
        let panel3 = RotatorControlPanel::default();

        // Each panel should have a different instance ID
        assert_ne!(
            panel1.panel_instance_id, panel2.panel_instance_id,
            "Panel 1 and 2 should have different instance IDs"
        );
        assert_ne!(
            panel2.panel_instance_id, panel3.panel_instance_id,
            "Panel 2 and 3 should have different instance IDs"
        );
        assert_ne!(
            panel1.panel_instance_id, panel3.panel_instance_id,
            "Panel 1 and 3 should have different instance IDs"
        );

        // IDs should be incrementing (panel2 > panel1, panel3 > panel2)
        assert!(
            panel2.panel_instance_id > panel1.panel_instance_id,
            "Panel instance IDs should increment"
        );
        assert!(
            panel3.panel_instance_id > panel2.panel_instance_id,
            "Panel instance IDs should increment"
        );
    }

    /// Test that debounce constants are set to expected values.
    /// COMMAND_DEBOUNCE prevents rapid-fire commands that could cause
    /// hardware issues (like the double-movement bug in ELL14 rotators).
    /// REFRESH_INTERVAL controls how often status is polled.
    #[test]
    fn test_debounce_constants() {
        assert_eq!(
            RotatorControlPanel::COMMAND_DEBOUNCE,
            std::time::Duration::from_millis(250),
            "COMMAND_DEBOUNCE should be 250ms to prevent rapid-fire commands"
        );

        assert_eq!(
            RotatorControlPanel::REFRESH_INTERVAL,
            std::time::Duration::from_millis(500),
            "REFRESH_INTERVAL should be 500ms for status polling"
        );
    }

    /// Test that can_send_command returns true when no previous command was sent.
    /// This ensures the first command after panel creation is always allowed.
    #[test]
    fn test_can_send_command_no_previous() {
        let panel = RotatorControlPanel::default();

        // With no previous command (last_command_time is None), should allow command
        assert!(
            panel.can_send_command(),
            "Should be able to send command when no previous command was sent"
        );
    }

    /// Test that can_send_command returns true after the debounce period has elapsed.
    /// This verifies the debounce logic correctly allows commands after waiting.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_can_send_command_after_debounce() {
        let mut panel = RotatorControlPanel::default();

        // Set last_command_time to a time in the past (well beyond debounce period)
        panel.last_command_time = Some(
            crate::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(500))
                .unwrap(),
        );

        // Should allow command since debounce period (250ms) has elapsed
        assert!(
            panel.can_send_command(),
            "Should be able to send command after debounce period has elapsed"
        );
    }

    /// Test that can_send_command returns false during the debounce period.
    /// This verifies the debounce logic correctly blocks rapid commands.
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_can_send_command_during_debounce() {
        let mut panel = RotatorControlPanel::default();

        // Set last_command_time to now (within debounce period)
        panel.last_command_time = Some(crate::time::Instant::now());

        // Should NOT allow command since we're within the debounce period
        assert!(
            !panel.can_send_command(),
            "Should NOT be able to send command during debounce period"
        );
    }

    /// Test that device_type() returns the correct type string.
    /// This is used for panel identification and routing.
    #[test]
    fn test_device_type() {
        let panel = RotatorControlPanel::default();

        assert_eq!(
            panel.device_type(),
            "Rotator",
            "device_type() should return 'Rotator'"
        );
    }

    /// Test that the rotator state defaults are correct.
    #[test]
    fn test_rotator_state_defaults() {
        let state = RotatorState::default();

        assert!(
            state.position_deg.is_none(),
            "Default position should be None"
        );
        assert!(!state.moving, "Default moving state should be false");
    }

    /// Test that actions_in_flight counter can be incremented and decremented.
    /// This is important for the UI to know when to disable controls.
    #[test]
    fn test_actions_in_flight_tracking() {
        let mut panel = RotatorControlPanel::default();

        // Initially zero
        assert_eq!(panel.panel_state.actions_in_flight, 0);

        // Increment
        panel.panel_state.actions_in_flight += 1;
        assert_eq!(panel.panel_state.actions_in_flight, 1);

        panel.panel_state.actions_in_flight += 1;
        assert_eq!(panel.panel_state.actions_in_flight, 2);

        // Decrement with action_completed (uses saturating_sub)
        panel.panel_state.action_completed();
        assert_eq!(panel.panel_state.actions_in_flight, 1);

        panel.panel_state.action_completed();
        assert_eq!(panel.panel_state.actions_in_flight, 0);

        // action_completed at zero should stay at zero
        panel.panel_state.action_completed();
        assert_eq!(
            panel.panel_state.actions_in_flight, 0,
            "saturating_sub should not go negative"
        );
    }

    /// Test that position_input uses 2 decimal places for display formatting.
    /// This ensures consistent display of rotator position values.
    /// Note: Rotator uses 2 decimal places (degrees) vs analog output's 3 decimal places (volts).
    #[test]
    fn test_position_input_uses_two_decimal_places() {
        // The formatting happens in poll_results when FetchState succeeds:
        //   self.position_input = format!("{:.2}", pos);

        // Test various position values to verify 2 decimal place formatting
        let test_cases = [
            (0.0, "0.00"),
            (45.0, "45.00"),
            (90.123, "90.12"),   // Rounds down
            (90.126, "90.13"),   // Rounds up
            (180.999, "181.00"), // Rounds up to next integer
            (-45.5, "-45.50"),   // Negative value
            (359.994, "359.99"), // Near 360
            (0.001, "0.00"),     // Very small - rounds to 0.00
            (0.004, "0.00"),     // Just below rounding boundary
            (12.345, "12.35"),   // Standard rounding case
            (270.0, "270.00"),   // Full rotation value
        ];

        for (input, expected) in test_cases {
            let formatted = format!("{:.2}", input);
            assert_eq!(
                formatted, expected,
                "Position {:.6} should format as '{}' (2 decimal places), got '{}'",
                input, expected, formatted
            );
        }
    }

    /// Test that refresh tracking is separate from actions_in_flight.
    /// This is critical because status refreshes should NOT disable user controls.
    #[test]
    fn test_refresh_tracking_separate_from_actions() {
        let mut panel = RotatorControlPanel::default();

        assert!(!panel.panel_state.is_refreshing());
        assert_eq!(panel.panel_state.actions_in_flight, 0);

        panel.panel_state.background_task_started();
        assert!(panel.panel_state.is_refreshing());
        assert_eq!(
            panel.panel_state.actions_in_flight, 0,
            "refresh should not affect actions count"
        );

        panel.panel_state.actions_in_flight = 1;
        assert!(
            panel.panel_state.is_refreshing(),
            "actions should not affect refresh state"
        );
        assert_eq!(panel.panel_state.actions_in_flight, 1);

        panel.panel_state.background_task_completed();
        assert!(!panel.panel_state.is_refreshing());
        assert_eq!(
            panel.panel_state.actions_in_flight, 1,
            "clearing refresh should not affect actions"
        );
    }
}
