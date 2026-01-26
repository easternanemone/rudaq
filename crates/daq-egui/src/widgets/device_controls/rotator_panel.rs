//! ELL14 Rotator control panel.
//!
//! Provides:
//! - Position display with degree formatting
//! - Jog buttons: -90, -10, -1, +1, +10, +90
//! - Home button
//! - Direct position input

use std::sync::atomic::{AtomicU64, Ordering};

use egui::Ui;
use tokio::runtime::Runtime;

use crate::client::DaqClient;
use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use daq_proto::daq::DeviceInfo;

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

/// ELL14 Rotator control panel
pub struct RotatorControlPanel {
    /// Unique instance ID for diagnostic logging (identifies duplicate panels)
    panel_instance_id: u64,
    /// Common panel state (channels, errors, device_id, etc.)
    panel_state: DevicePanelState<ActionResult>,
    state: RotatorState,
    position_input: String,
    /// Status refresh request in flight - does NOT disable controls
    refresh_in_flight: bool,
    /// Last command time for debouncing rapid clicks
    last_command_time: Option<std::time::Instant>,
}

impl Default for RotatorControlPanel {
    fn default() -> Self {
        let panel_instance_id = PANEL_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(panel_instance_id, "RotatorControlPanel instance created");
        Self {
            panel_instance_id,
            panel_state: DevicePanelState::new(),
            state: RotatorState::default(),
            position_input: "0.0".to_string(),
            refresh_in_flight: false,
            last_command_time: None,
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
                    // Status refresh complete - does not affect user action count
                    self.refresh_in_flight = false;
                    match result {
                        Ok(state) => {
                            self.state = state;
                            if let Some(pos) = self.state.position_deg {
                                self.position_input = format!("{:.2}", pos);
                            }
                            self.panel_state.error = None;
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Failed to fetch state: {}", e));
                        }
                    }
                }
                ActionResult::Move(result) => {
                    // User action complete
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.panel_state.set_status("Move completed");
                            self.state.moving = false;
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Move failed: {}", e));
                            self.state.moving = false;
                        }
                    }
                }
                ActionResult::Home(result) => {
                    // User action complete
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.panel_state.set_status("Home completed");
                            self.state.moving = false;
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Home failed: {}", e));
                            self.state.moving = false;
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

        // Track refresh separately - doesn't disable controls
        self.refresh_in_flight = true;
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client.get_device_state(&device_id).await;
            let state_result = result
                .map(|proto| RotatorState {
                    position_deg: proto.position,
                    moving: false, // TODO: Get from proto when available
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::FetchState(state_result)).await;
        });

        self.panel_state.mark_refreshed();
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

        // Debounce rapid clicks
        if !self.can_send_command() {
            tracing::debug!(
                panel_instance_id = self.panel_instance_id,
                device_id,
                position,
                elapsed_ms = self
                    .last_command_time
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0),
                "Move absolute command debounced - too soon after last command"
            );
            return;
        }

        self.state.moving = true;
        self.panel_state.action_started();
        self.last_command_time = Some(std::time::Instant::now());
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

        // Debounce rapid clicks
        if !self.can_send_command() {
            tracing::debug!(
                panel_instance_id = self.panel_instance_id,
                device_id,
                delta,
                elapsed_ms = self
                    .last_command_time
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0),
                "Jog command debounced - too soon after last command"
            );
            return;
        }

        self.state.moving = true;
        self.panel_state.action_started();
        self.last_command_time = Some(std::time::Instant::now());
        tracing::info!(
            panel_instance_id = self.panel_instance_id,
            device_id,
            delta,
            actions_in_flight = self.panel_state.actions_in_flight,
            "Jog command sent"
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

    fn home(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        // Debounce rapid clicks
        if !self.can_send_command() {
            tracing::debug!(
                panel_instance_id = self.panel_instance_id,
                device_id,
                elapsed_ms = self
                    .last_command_time
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0),
                "Home command debounced - too soon after last command"
            );
            return;
        }

        self.state.moving = true;
        self.panel_state.action_started();
        self.last_command_time = Some(std::time::Instant::now());
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

        // Initial fetch
        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Auto-refresh logic - only refresh if no refresh already in flight
        let should_refresh = self.panel_state.should_refresh(Self::REFRESH_INTERVAL)
            && !self.refresh_in_flight;

        if should_refresh && client.is_some() {
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("🔄 Rotator");
            if self.state.moving || self.panel_state.is_busy() {
                ui.spinner();
                ui.label("Moving...");
            } else if self.refresh_in_flight {
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

        // Current position display (large)
        ui.vertical_centered(|ui| {
            if let Some(pos) = self.state.position_deg {
                ui.label(
                    egui::RichText::new(format!("{:.2}°", pos))
                        .monospace()
                        .size(32.0),
                );
            } else {
                ui.label(egui::RichText::new("---°").monospace().size(32.0));
            }
        });

        ui.add_space(8.0);

        // Jog buttons row - only disable during user actions (move/home), NOT during status refresh
        let is_busy = self.state.moving || self.panel_state.is_busy();

        ui.horizontal(|ui| {
            ui.label("Jog:");

            if ui
                .add_enabled(!is_busy, egui::Button::new("-90°"))
                .clicked()
            {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -90.0);
            }
            if ui
                .add_enabled(!is_busy, egui::Button::new("-10°"))
                .clicked()
            {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -10.0);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("-1°")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, -1.0);
            }
            if ui.add_enabled(!is_busy, egui::Button::new("+1°")).clicked() {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, 1.0);
            }
            if ui
                .add_enabled(!is_busy, egui::Button::new("+10°"))
                .clicked()
            {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, 10.0);
            }
            if ui
                .add_enabled(!is_busy, egui::Button::new("+90°"))
                .clicked()
            {
                self.move_relative(client.as_deref_mut(), runtime, &device_id, 90.0);
            }
        });

        ui.add_space(4.0);

        // Direct position input
        ui.horizontal(|ui| {
            ui.label("Move to:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.position_input)
                    .desired_width(60.0)
                    .hint_text("deg"),
            );
            ui.label("°");

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
        ui.separator();

        // Quick position buttons
        ui.horizontal(|ui| {
            ui.label("Quick positions:");
            for angle in [0.0, 45.0, 90.0, 180.0, 270.0] {
                if ui
                    .add_enabled(!is_busy, egui::Button::new(format!("{}°", angle)))
                    .clicked()
                {
                    self.move_absolute(client.as_deref_mut(), runtime, &device_id, angle);
                }
            }
        });

        ui.add_space(8.0);

        // Action buttons
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!is_busy, egui::Button::new("🏠 Home"))
                .clicked()
            {
                self.home(client.as_deref_mut(), runtime, &device_id);
            }

            ui.checkbox(&mut self.panel_state.auto_refresh, "Auto-refresh");

            if ui.button("🔄 Refresh").clicked() {
                self.fetch_state(client, runtime, &device_id);
            }
        });

        // Request repaint for auto-refresh or while busy
        if self.panel_state.auto_refresh || self.panel_state.is_busy() || self.refresh_in_flight {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
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
            !panel.refresh_in_flight,
            "refresh_in_flight should be false on creation"
        );

        // Auto-refresh should be enabled by default
        assert!(
            panel.panel_state.auto_refresh,
            "Auto-refresh should be enabled by default"
        );

        // No error or status messages initially
        assert!(panel.panel_state.error.is_none(), "Error should be None on creation");
        assert!(panel.panel_state.status.is_none(), "Status should be None on creation");

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
    fn test_can_send_command_after_debounce() {
        let mut panel = RotatorControlPanel::default();

        // Set last_command_time to a time in the past (well beyond debounce period)
        panel.last_command_time =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(500));

        // Should allow command since debounce period (250ms) has elapsed
        assert!(
            panel.can_send_command(),
            "Should be able to send command after debounce period has elapsed"
        );
    }

    /// Test that can_send_command returns false during the debounce period.
    /// This verifies the debounce logic correctly blocks rapid commands.
    #[test]
    fn test_can_send_command_during_debounce() {
        let mut panel = RotatorControlPanel::default();

        // Set last_command_time to now (within debounce period)
        panel.last_command_time = Some(std::time::Instant::now());

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

    /// Test that refresh_in_flight is tracked separately from actions_in_flight.
    /// This is critical because status refreshes should NOT disable user controls.
    #[test]
    fn test_refresh_in_flight_separate_from_actions() {
        let mut panel = RotatorControlPanel::default();

        // Initially both false/zero
        assert!(!panel.refresh_in_flight);
        assert_eq!(panel.panel_state.actions_in_flight, 0);

        // Setting refresh_in_flight should not affect actions_in_flight
        panel.refresh_in_flight = true;
        assert!(panel.refresh_in_flight);
        assert_eq!(
            panel.panel_state.actions_in_flight, 0,
            "refresh should not affect actions count"
        );

        // Setting actions_in_flight should not affect refresh_in_flight
        panel.panel_state.actions_in_flight = 1;
        assert!(
            panel.refresh_in_flight,
            "actions should not affect refresh flag"
        );
        assert_eq!(panel.panel_state.actions_in_flight, 1);

        // Clear refresh but keep action
        panel.refresh_in_flight = false;
        assert!(!panel.refresh_in_flight);
        assert_eq!(
            panel.panel_state.actions_in_flight, 1,
            "clearing refresh should not affect actions"
        );
    }
}
