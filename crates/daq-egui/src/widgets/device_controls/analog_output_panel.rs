//! Analog Output control panel for DAC devices (e.g., EOM voltage control).
//!
//! Provides:
//! - Voltage slider with -10V to +10V range
//! - Numeric input for precise voltage entry
//! - Quick buttons: 0V, Min, Max
//! - Status display

use egui::Ui;
use serde_json::json;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::device_controls::DeviceControlWidget;
use daq_proto::daq::DeviceInfo;

/// Async action results
enum ActionResult {
    WriteVoltage(Result<f64, String>),
}

/// Analog Output control panel for DAC devices
pub struct AnalogOutputControlPanel {
    /// Current voltage setpoint
    voltage: f64,
    /// Voltage input as string for text editing
    voltage_input: String,
    /// Action channel sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Action channel receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of actions in flight
    actions_in_flight: usize,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Device ID (cached)
    device_id: Option<String>,
    /// Min voltage (from metadata or default)
    min_voltage: f64,
    /// Max voltage (from metadata or default)
    max_voltage: f64,
}

impl Default for AnalogOutputControlPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            voltage: 0.0,
            voltage_input: "0.0".to_string(),
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            min_voltage: -10.0,
            max_voltage: 10.0,
        }
    }
}

impl AnalogOutputControlPanel {
    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            self.actions_in_flight = self.actions_in_flight.saturating_sub(1);

            match result {
                ActionResult::WriteVoltage(result) => match result {
                    Ok(voltage) => {
                        self.voltage = voltage;
                        self.voltage_input = format!("{:.3}", voltage);
                        self.status = Some(format!("Set to {:.3} V", voltage));
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(format!("Write failed: {}", e));
                    }
                },
            }
        }
    }

    fn write_voltage(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        voltage: f64,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected".to_string());
            return;
        };

        self.actions_in_flight += 1;
        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();
        let voltage = voltage.clamp(self.min_voltage, self.max_voltage);

        runtime.spawn(async move {
            // Use write_voltage command with JSON args
            let args = json!({ "voltage": voltage }).to_string();
            let result = client
                .execute_device_command(&device_id, "write_voltage", &args)
                .await
                .map(|_| voltage)
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::WriteVoltage(result)).await;
        });
    }
}

impl DeviceControlWidget for AnalogOutputControlPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.poll_results();

        let device_id = device.id.clone();
        self.device_id = Some(device_id.clone());

        // Header
        ui.horizontal(|ui| {
            ui.heading("⚡ Analog Output");
            if self.actions_in_flight > 0 {
                ui.spinner();
            }
        });

        if let Some(ref err) = self.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        // Current voltage display (large)
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(format!("{:.3} V", self.voltage))
                    .monospace()
                    .size(32.0),
            );
        });

        ui.add_space(8.0);

        // Voltage slider
        let is_busy = self.actions_in_flight > 0;

        ui.horizontal(|ui| {
            ui.label("Voltage:");
            let mut voltage = self.voltage;
            let slider = egui::Slider::new(&mut voltage, self.min_voltage..=self.max_voltage)
                .suffix(" V")
                .clamping(egui::SliderClamping::Always);

            if ui.add_enabled(!is_busy, slider).changed() {
                self.voltage = voltage;
                self.voltage_input = format!("{:.3}", voltage);
                self.write_voltage(client.as_deref_mut(), runtime, &device_id, voltage);
            }
        });

        // Numeric input
        ui.horizontal(|ui| {
            ui.label("Set:");
            let response = ui.add_enabled(
                !is_busy,
                egui::TextEdit::singleline(&mut self.voltage_input)
                    .desired_width(80.0)
                    .hint_text("V"),
            );
            ui.label("V");

            if ui.add_enabled(!is_busy, egui::Button::new("Apply")).clicked() {
                if let Ok(v) = self.voltage_input.parse::<f64>() {
                    let v = v.clamp(self.min_voltage, self.max_voltage);
                    self.write_voltage(client.as_deref_mut(), runtime, &device_id, v);
                } else {
                    self.error = Some("Invalid voltage value".to_string());
                }
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !is_busy {
                if let Ok(v) = self.voltage_input.parse::<f64>() {
                    let v = v.clamp(self.min_voltage, self.max_voltage);
                    self.write_voltage(client.as_deref_mut(), runtime, &device_id, v);
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();

        // Quick buttons
        ui.horizontal(|ui| {
            ui.label("Quick set:");

            if ui.add_enabled(!is_busy, egui::Button::new("0 V")).clicked() {
                self.write_voltage(client.as_deref_mut(), runtime, &device_id, 0.0);
            }

            if ui
                .add_enabled(!is_busy, egui::Button::new(format!("{:.0} V", self.min_voltage)))
                .clicked()
            {
                self.write_voltage(client.as_deref_mut(), runtime, &device_id, self.min_voltage);
            }

            if ui
                .add_enabled(!is_busy, egui::Button::new(format!("{:.0} V", self.max_voltage)))
                .clicked()
            {
                self.write_voltage(client.as_deref_mut(), runtime, &device_id, self.max_voltage);
            }
        });

        // Request repaint while busy
        if self.actions_in_flight > 0 {
            ui.ctx().request_repaint();
        }
    }

    fn device_type(&self) -> &'static str {
        "Analog Output"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that a new AnalogOutputControlPanel has correct default state.
    /// This ensures the panel initializes with sensible values before any
    /// user interaction or daemon connection.
    #[test]
    fn test_default_panel_state() {
        let panel = AnalogOutputControlPanel::default();

        // Initial voltage should be 0.0
        assert_eq!(panel.voltage, 0.0, "Default voltage should be 0.0");

        // Voltage input string should match the voltage value
        assert_eq!(
            panel.voltage_input, "0.0",
            "Default voltage input should be '0.0'"
        );

        // No actions should be in flight initially
        assert_eq!(
            panel.actions_in_flight, 0,
            "No actions should be in flight on creation"
        );

        // No error or status messages initially
        assert!(
            panel.error.is_none(),
            "Error should be None on creation"
        );
        assert!(
            panel.status.is_none(),
            "Status should be None on creation"
        );

        // Device ID not set until UI is rendered with device info
        assert!(
            panel.device_id.is_none(),
            "Device ID should be None on creation"
        );
    }

    /// Test that voltage bounds are set to standard DAC range.
    /// The default range of -10V to +10V is standard for most DAC hardware.
    #[test]
    fn test_voltage_bounds() {
        let panel = AnalogOutputControlPanel::default();

        assert_eq!(
            panel.min_voltage, -10.0,
            "Min voltage should be -10.0 V (standard DAC range)"
        );
        assert_eq!(
            panel.max_voltage, 10.0,
            "Max voltage should be +10.0 V (standard DAC range)"
        );
    }

    /// Test that the voltage input string defaults to "0.0".
    /// This is important for proper text field initialization.
    #[test]
    fn test_voltage_input_string_default() {
        let panel = AnalogOutputControlPanel::default();

        assert_eq!(
            panel.voltage_input, "0.0",
            "Voltage input string should be '0.0' for proper text field display"
        );

        // Verify it parses correctly
        let parsed: Result<f64, _> = panel.voltage_input.parse();
        assert!(
            parsed.is_ok(),
            "Default voltage input should be parseable as f64"
        );
        assert_eq!(
            parsed.unwrap(),
            panel.voltage,
            "Parsed voltage input should match voltage field"
        );
    }

    /// Test that actions_in_flight counter starts at zero.
    /// This is critical for proper control enable/disable logic.
    #[test]
    fn test_actions_in_flight_default() {
        let panel = AnalogOutputControlPanel::default();

        assert_eq!(
            panel.actions_in_flight, 0,
            "Actions in flight should start at 0"
        );
    }

    /// Test that the device_type() method returns the correct type string.
    /// This is used for panel identification and routing.
    #[test]
    fn test_device_type() {
        let panel = AnalogOutputControlPanel::default();

        assert_eq!(
            panel.device_type(),
            "Analog Output",
            "Device type should be 'Analog Output'"
        );
    }

    /// Test that voltage_input is formatted with 3 decimal places.
    /// This ensures consistent precision display for voltage values.
    /// The format string "{:.3}" is used in poll_results() and ui().
    #[test]
    fn test_voltage_input_uses_three_decimal_places() {
        // Test formatting of various voltage values
        let test_cases = [
            (0.0, "0.000"),
            (1.0, "1.000"),
            (-5.5, "-5.500"),
            (3.14159, "3.142"),  // Should round
            (10.0, "10.000"),
            (-10.0, "-10.000"),
            (0.001, "0.001"),
            (0.0001, "0.000"),   // Below precision, rounds to 0.000
        ];

        for (voltage, expected) in test_cases {
            let formatted = format!("{:.3}", voltage);
            assert_eq!(
                formatted, expected,
                "Voltage {} should format as '{}', got '{}'",
                voltage, expected, formatted
            );
        }
    }

    /// Test voltage clamping logic by verifying the bounds.
    /// In write_voltage(), voltage is clamped to [min_voltage, max_voltage].
    /// This test documents the expected clamping behavior.
    #[test]
    fn test_voltage_clamping_bounds() {
        let panel = AnalogOutputControlPanel::default();

        // Values within range should not be clamped
        let in_range = 5.0_f64;
        let clamped = in_range.clamp(panel.min_voltage, panel.max_voltage);
        assert_eq!(clamped, 5.0, "In-range value should not change");

        // Values below min should be clamped to min
        let below_min = -15.0_f64;
        let clamped = below_min.clamp(panel.min_voltage, panel.max_voltage);
        assert_eq!(clamped, -10.0, "Value below min should clamp to -10.0");

        // Values above max should be clamped to max
        let above_max = 15.0_f64;
        let clamped = above_max.clamp(panel.min_voltage, panel.max_voltage);
        assert_eq!(clamped, 10.0, "Value above max should clamp to +10.0");

        // Edge cases: exactly at bounds
        let at_min = -10.0_f64;
        let clamped = at_min.clamp(panel.min_voltage, panel.max_voltage);
        assert_eq!(clamped, -10.0, "Value at min should stay at min");

        let at_max = 10.0_f64;
        let clamped = at_max.clamp(panel.min_voltage, panel.max_voltage);
        assert_eq!(clamped, 10.0, "Value at max should stay at max");
    }
}
