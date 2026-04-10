//! Newport 1830-C Power Meter control panel.
//!
//! This module provides a GUI panel for controlling and monitoring Newport 1830-C
//! optical power meters via gRPC communication with the DAQ daemon.
//!
//! # Features
//!
//! - **Real-time power reading gauge**: Displays power measurements with automatic
//!   unit scaling (W, mW, µW) based on signal level
//! - **Wavelength calibration**: Input field to set the calibration wavelength (nm)
//!   for accurate power measurements at different laser wavelengths
//! - **Auto-refresh**: Configurable automatic polling (default: 500ms interval)
//! - **Unit normalization**: Automatically converts readings from any unit (W, mW, µW, nW)
//!   to milliwatts for consistent internal representation
//!
//! # Unit Handling
//!
//! The Newport 1830-C returns power readings in Watts. The gRPC server includes the
//! measurement units in the response. This panel normalizes all readings to milliwatts
//! internally via [`PowerMeterControlPanel::normalize_power_to_mw`], then dynamically
//! scales the display based on signal magnitude:
//!
//! | Power Level | Display Unit |
//! |-------------|--------------|
//! | ≥ 1000 mW   | W            |
//! | ≥ 1 mW      | mW           |
//! | < 1 mW      | µW           |
//!
//! # Example Flow
//!
//! ```text
//! Newport 1830-C → "5E-9" (5 nW in Watts)
//!                ↓
//! gRPC Server   → ReadValueResponse { value: 5e-9, units: "W" }
//!                ↓
//! GUI Panel     → normalize_power_to_mw(5e-9, "W") = 5e-6 mW
//!                ↓
//! Display       → "5.0000 µW" (scaled for readability)
//! ```

use std::collections::VecDeque;

use crate::runtime::Runtime;
use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints};

use crate::widgets::Gauge;
use crate::widgets::device_controls::{DeviceControlWidget, DevicePanelState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Maximum history samples retained (~2.5 min at 2 Hz)
const HISTORY_MAX: usize = 300;

/// Running statistics for a set of power samples
#[derive(Debug, Clone, Default)]
struct MeterStats {
    mean_mw: f64,
    std_mw: f64,
    min_mw: f64,
    max_mw: f64,
}

/// Power meter state cached from the daemon
#[derive(Debug, Clone, Default)]
struct MeterState {
    power_mw: Option<f64>,
    wavelength_nm: Option<f64>,
    loading: bool,
    /// Running peak (maximum seen since last reset)
    peak_mw: Option<f64>,
    /// Running minimum seen since last reset
    min_mw: Option<f64>,
    /// Ring-buffer history for trend plot
    history: VecDeque<f64>,
    /// Accumulated stats (recomputed on each new sample)
    history_stats: Option<MeterStats>,
}

impl MeterState {
    fn push_sample(&mut self, power_mw: f64) {
        self.power_mw = Some(power_mw);

        // Update peak/min
        self.peak_mw = Some(self.peak_mw.map_or(power_mw, |p| p.max(power_mw)));
        self.min_mw = Some(self.min_mw.map_or(power_mw, |m| m.min(power_mw)));

        // Update history ring buffer
        self.history.push_back(power_mw);
        while self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }

        // Recompute stats
        let n = self.history.len();
        if n > 0 {
            #[allow(clippy::cast_precision_loss)]
            let n_f = n as f64;
            let sum: f64 = self.history.iter().sum();
            let mean = sum / n_f;
            let variance = self
                .history
                .iter()
                .map(|&x| (x - mean).powi(2))
                .sum::<f64>()
                / n_f;
            let hist_min = self.history.iter().copied().fold(f64::INFINITY, f64::min);
            let hist_max = self
                .history
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            self.history_stats = Some(MeterStats {
                mean_mw: mean,
                std_mw: variance.sqrt(),
                min_mw: hist_min,
                max_mw: hist_max,
            });
        }
    }

    fn reset_stats(&mut self) {
        self.peak_mw = None;
        self.min_mw = None;
        self.history.clear();
        self.history_stats = None;
    }
}

/// Async action results
enum ActionResult {
    ReadPower(Result<(f64, String), String>),
    GetWavelength(Result<f64, String>),
    SetWavelength(Result<f64, String>),
}

/// Newport 1830-C Power Meter control panel
pub struct PowerMeterControlPanel {
    /// Common panel state (channels, errors, device_id, etc.)
    panel_state: DevicePanelState<ActionResult>,
    state: MeterState,
    wavelength_input: String,
    /// Whether to show the peak-hold value
    peak_hold_enabled: bool,
    /// Whether to show the min-hold value
    min_hold_enabled: bool,
    /// Whether to show the trend graph
    show_trend: bool,
    /// Whether to show the statistics table
    show_stats: bool,
}

impl Default for PowerMeterControlPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            state: MeterState::default(),
            wavelength_input: "800".to_string(),
            peak_hold_enabled: false,
            min_hold_enabled: false,
            show_trend: false,
            show_stats: false,
        }
    }
}

impl PowerMeterControlPanel {
    /// Auto-refresh interval
    const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

    /// Normalize a power reading to milliwatts (mW).
    ///
    /// The gRPC server returns power values along with their unit string from
    /// the device's configured `measurement_units` metadata. This function
    /// converts any supported unit to milliwatts for consistent internal storage.
    ///
    /// # Supported Units
    ///
    /// | Input Unit | Conversion Factor |
    /// |------------|-------------------|
    /// | W, w       | × 1000            |
    /// | mW, mw     | × 1 (no change)   |
    /// | uW, uw, µW | ÷ 1000            |
    /// | nW, nw     | ÷ 1,000,000       |
    /// | "" (empty) | × 1000 (assume W) |
    /// | other      | × 1 (passthrough) |
    ///
    /// # Arguments
    ///
    /// * `value` - The raw power reading from the device
    /// * `units` - The unit string from `ReadValueResponse.units`
    ///
    /// # Returns
    ///
    /// The power value normalized to milliwatts.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Newport 1830-C returns Watts
    /// let mw = normalize_power_to_mw(5e-9, "W"); // 5 nW → 5e-6 mW
    /// let mw = normalize_power_to_mw(1.5e-3, "W"); // 1.5 mW → 1.5 mW
    /// let mw = normalize_power_to_mw(0.5, "mW"); // Already mW → 0.5 mW
    /// ```
    fn normalize_power_to_mw(value: f64, units: &str) -> f64 {
        match units.trim() {
            "W" | "w" => value * 1000.0,
            "mW" | "mw" => value,
            "uW" | "uw" | "µW" => value / 1000.0,
            "nW" | "nw" => value / 1_000_000.0,
            "" => value * 1000.0,
            _ => value,
        }
    }

    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            self.panel_state.action_completed();

            match result {
                ActionResult::ReadPower(result) => match result {
                    Ok((power, units)) => {
                        let power_mw = Self::normalize_power_to_mw(power, &units);
                        self.state.push_sample(power_mw);
                        self.state.loading = false;
                        self.panel_state.error = None; // Clear any previous error on success
                    }
                    Err(e) => {
                        self.panel_state.set_error(format!("Read failed: {}", e));
                        self.state.loading = false;
                    }
                },
                ActionResult::GetWavelength(result) => {
                    if let Ok(wl) = result {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_input = format!("{:.0}", wl);
                    }
                }
                ActionResult::SetWavelength(result) => match result {
                    Ok(wl) => {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_input = format!("{:.0}", wl);
                        self.panel_state
                            .set_status(format!("Calibration wavelength set to {} nm", wl));
                    }
                    Err(e) => {
                        self.panel_state
                            .set_error(format!("Failed to set wavelength: {}", e));
                    }
                },
            }
        }
    }

    fn read_power(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .read_value(&device_id)
                .await
                .map(|r| (r.value, r.units))
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::ReadPower(result)).await;
        });

        self.panel_state.mark_refreshed();
    }

    fn fetch_wavelength(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else {
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .get_wavelength(&device_id)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::GetWavelength(result)).await;
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
}

impl PowerMeterControlPanel {
    /// Format a milliwatt value with auto-scaling unit label.
    fn format_power(power_mw: f64) -> (f64, &'static str) {
        if power_mw >= 1000.0 {
            (power_mw / 1000.0, "W")
        } else if power_mw >= 1.0 {
            (power_mw, "mW")
        } else {
            (power_mw * 1000.0, "µW")
        }
    }

    /// Render the trend plot (recent sample history).
    fn render_trend(&self, ui: &mut Ui) {
        if self.state.history.is_empty() {
            ui.label(
                egui::RichText::new("No history yet")
                    .italics()
                    .color(egui::Color32::GRAY),
            );
            return;
        }
        let points: Vec<[f64; 2]> = self
            .state
            .history
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                #[allow(clippy::cast_precision_loss)]
                [i as f64, v]
            })
            .collect();
        let plot = Plot::new("power_trend")
            .height(150.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_axes([false, true])
            .label_formatter(|_, pt| {
                let (v, unit) = Self::format_power(pt.y);
                format!("{:.4} {}", v, unit)
            });
        plot.show(ui, |plot_ui| {
            plot_ui.line(
                Line::new("power", PlotPoints::new(points))
                    .color(egui::Color32::from_rgb(80, 200, 120))
                    .width(1.5),
            );
        });
    }

    /// Render the statistics table.
    fn render_stats(&self, ui: &mut Ui) {
        let Some(ref stats) = self.state.history_stats else {
            ui.label(
                egui::RichText::new("Collecting...")
                    .italics()
                    .color(egui::Color32::GRAY),
            );
            return;
        };
        egui::Grid::new("power_stats_grid")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                let rows = [
                    ("Mean", stats.mean_mw),
                    ("σ (std)", stats.std_mw),
                    ("Min", stats.min_mw),
                    ("Max", stats.max_mw),
                ];
                for (label, val_mw) in rows {
                    let (v, unit) = Self::format_power(val_mw);
                    ui.label(label);
                    ui.label(format!("{:.4} {}", v, unit));
                    ui.end_row();
                }
            });
    }
}

impl DeviceControlWidget for PowerMeterControlPanel {
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
            tracing::info!("[PowerMeter] Initial fetch for device={}", device_id);
            self.read_power(client.as_deref_mut(), runtime, &device_id);
            self.fetch_wavelength(client.as_deref_mut(), runtime, &device_id);
        }

        // Auto-refresh logic
        let should_refresh = self.panel_state.should_refresh(Self::REFRESH_INTERVAL);

        if should_refresh && client.is_some() {
            tracing::debug!("[PowerMeter] Auto-refresh read for device={}", device_id);
            self.read_power(client.as_deref_mut(), runtime, &device_id);
        } else if should_refresh && client.is_none() {
            tracing::warn!(
                "[PowerMeter] Auto-refresh skipped: no client for device={}",
                device_id
            );
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("⚡ Power Meter");
            if self.state.loading || self.panel_state.is_busy() {
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

        // Power gauge (large, centered)
        ui.vertical_centered(|ui| {
            #[allow(clippy::cast_possible_truncation)]
            let power = self.state.power_mw.unwrap_or(0.0) as f32;

            // Determine range and units based on power level
            let (value, unit, max_val) = if power >= 1000.0 {
                (power / 1000.0, "W", 5.0)
            } else if power >= 1.0 {
                (power, "mW", 1000.0)
            } else {
                (power * 1000.0, "µW", 1000.0)
            };

            ui.add(
                Gauge::new(value)
                    .range(0.0, max_val)
                    .label("Power")
                    .unit(unit)
                    .size(100.0),
            );

            // Exact value display
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{:.4} mW", self.state.power_mw.unwrap_or(0.0)))
                    .monospace()
                    .size(14.0),
            );
        });

        ui.add_space(8.0);
        ui.separator();

        // Wavelength calibration
        ui.label(egui::RichText::new("Wavelength Calibration").strong());

        ui.horizontal(|ui| {
            ui.label("Wavelength:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.wavelength_input)
                    .desired_width(60.0)
                    .hint_text("nm"),
            );
            ui.label("nm");

            if ui.button("Set").clicked() {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                } else {
                    self.panel_state.error = Some("Invalid wavelength".to_string());
                }
            }

            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(wl) = self.wavelength_input.parse::<f64>()
            {
                self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
            }
        });

        if let Some(wl) = self.state.wavelength_nm {
            ui.label(format!("Current calibration: {} nm", wl));
        }

        ui.add_space(8.0);
        ui.separator();

        // Peak/Min hold display
        if self.peak_hold_enabled
            && let Some(peak) = self.state.peak_mw
        {
            let (v, unit) = Self::format_power(peak);
            ui.horizontal(|ui| {
                ui.label("▲ Peak:");
                ui.label(
                    egui::RichText::new(format!("{:.4} {}", v, unit))
                        .monospace()
                        .color(egui::Color32::from_rgb(255, 180, 50)),
                );
            });
        }
        if self.min_hold_enabled
            && let Some(min) = self.state.min_mw
        {
            let (v, unit) = Self::format_power(min);
            ui.horizontal(|ui| {
                ui.label("▼ Min:");
                ui.label(
                    egui::RichText::new(format!("{:.4} {}", v, unit))
                        .monospace()
                        .color(egui::Color32::from_rgb(100, 180, 255)),
                );
            });
        }

        // Trend graph
        if self.show_trend {
            ui.separator();
            ui.label(egui::RichText::new("Trend").strong());
            self.render_trend(ui);
        }

        // Statistics table
        if self.show_stats {
            ui.separator();
            ui.label(egui::RichText::new("Statistics").strong());
            self.render_stats(ui);
        }

        ui.add_space(8.0);
        ui.separator();

        // Feature checkboxes + Reset
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.peak_hold_enabled, "Peak Hold");
            ui.checkbox(&mut self.min_hold_enabled, "Min Hold");
            ui.checkbox(&mut self.show_trend, "Trend");
            ui.checkbox(&mut self.show_stats, "Stats");
            if ui.button("Reset").clicked() {
                self.state.reset_stats();
            }
        });

        ui.separator();

        // Controls
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.panel_state.auto_refresh, "Auto-refresh");

            if ui.button("🔄 Read Now").clicked() {
                self.read_power(client, runtime, &device_id);
            }
        });

        // Request repaint for auto-refresh
        if self.panel_state.auto_refresh || self.panel_state.is_busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn device_type(&self) -> &'static str {
        "Power Meter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance for floating-point comparisons in power normalization tests.
    /// 1e-12 is sufficient for values produced by simple multiplication chains.
    const TOLERANCE: f64 = 1e-12;

    /// Test unit normalization from Watts to milliwatts.
    ///
    /// The Newport 1830-C returns readings in Watts, which must be
    /// converted to milliwatts for the GUI's internal representation.
    #[test]
    fn test_normalize_watts_to_mw() {
        // Typical Newport 1830-C readings (scientific notation in Watts)
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(5e-9, "W") - 5e-6).abs() < TOLERANCE,
            "5 nW should become 5e-6 mW"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1.5e-3, "W") - 1.5).abs() < TOLERANCE,
            "1.5 mW in Watts should become 1.5 mW"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(0.001, "W") - 1.0).abs() < TOLERANCE,
            "1 mW in Watts should become 1.0 mW"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1.0, "W") - 1000.0).abs() < TOLERANCE,
            "1 W should become 1000 mW"
        );
    }

    /// Test case-insensitive Watts handling.
    #[test]
    fn test_normalize_watts_case_insensitive() {
        let value = 0.005; // 5 mW in Watts
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(value, "W") - 5.0).abs() < TOLERANCE
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(value, "w") - 5.0).abs() < TOLERANCE
        );
    }

    /// Test that milliwatts pass through unchanged.
    #[test]
    fn test_normalize_milliwatts_passthrough() {
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1.5, "mW") - 1.5).abs() < TOLERANCE,
            "mW should pass through unchanged"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(0.001, "mw") - 0.001).abs() < TOLERANCE,
            "lowercase mw should also pass through"
        );
    }

    /// Test microwatt normalization.
    #[test]
    fn test_normalize_microwatts() {
        // 1000 µW = 1 mW
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1000.0, "uW") - 1.0).abs() < TOLERANCE
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1000.0, "µW") - 1.0).abs() < TOLERANCE
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(500.0, "uw") - 0.5).abs() < TOLERANCE
        );
    }

    /// Test nanowatt normalization.
    #[test]
    fn test_normalize_nanowatts() {
        // 1,000,000 nW = 1 mW
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1_000_000.0, "nW") - 1.0).abs()
                < TOLERANCE
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(5000.0, "nw") - 0.005).abs() < TOLERANCE
        );
    }

    /// Test empty units default to Watts (for backwards compatibility).
    #[test]
    fn test_normalize_empty_units_assumes_watts() {
        // Empty string should be treated as Watts (default for Newport)
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(0.001, "") - 1.0).abs() < TOLERANCE,
            "Empty units should assume Watts"
        );
    }

    /// Test unknown units pass through unchanged (safety fallback).
    #[test]
    fn test_normalize_unknown_units_passthrough() {
        // Unknown units should pass through to avoid data corruption
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(42.0, "dBm") - 42.0).abs() < TOLERANCE,
            "Unknown units should pass through unchanged"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(100.0, "arbitrary") - 100.0).abs()
                < TOLERANCE
        );
    }

    /// Test whitespace trimming in unit strings.
    #[test]
    fn test_normalize_trims_whitespace() {
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(0.001, "  W  ") - 1.0).abs() < TOLERANCE,
            "Should trim whitespace from unit string"
        );
        assert!(
            (PowerMeterControlPanel::normalize_power_to_mw(1.0, "\tmW\n") - 1.0).abs() < TOLERANCE
        );
    }

    /// Test the complete data flow from typical Newport readings.
    ///
    /// Newport 1830-C returns scientific notation like "5E-9" which
    /// represents 5 nanowatts. After parsing to f64 (5e-9) and
    /// normalizing with units="W", this should become 5e-6 mW.
    #[test]
    fn test_newport_typical_readings() {
        // These are typical Newport 1830-C readings parsed from strings
        let test_cases = [
            // (parsed_value, expected_mw, description)
            (5e-9_f64, 5e-6_f64, "5 nW → 5e-6 mW"),
            (1.234e-6, 1.234e-3, "1.234 µW → 1.234e-3 mW"),
            (0.75e-9, 0.75e-6, "0.75 nW → 0.75e-6 mW"),
            (1e-3, 1.0, "1 mW → 1 mW"),
            (2.5e-3, 2.5, "2.5 mW → 2.5 mW"),
        ];

        for (input, expected, desc) in test_cases {
            let result = PowerMeterControlPanel::normalize_power_to_mw(input, "W");
            assert!(
                (result - expected).abs() < 1e-15,
                "Failed for {}: got {}, expected {}",
                desc,
                result,
                expected
            );
        }
    }
}
