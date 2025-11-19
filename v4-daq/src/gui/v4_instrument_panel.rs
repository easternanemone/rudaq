//! V4 Instrument Panel for egui GUI
//!
//! Displays real-time V4 power meter data with plots and statistics.
//! Integrates with the V4DataBridge to consume Arrow data.

use super::v4_data_bridge::V4DataBridge;
use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use std::collections::VecDeque;

/// V4 Instrument Panel for displaying real-time power measurements
///
/// Renders:
/// - Live power plot with egui_plot
/// - Instrument status (connected, measurements/sec)
/// - Real-time statistics (min, max, mean, latest)
/// - Configurable time window and update rate
pub struct V4InstrumentPanel {
    /// Instrument ID (e.g., "newport_1830c", "maitai")
    instrument_id: String,

    /// Local plot data cache (extracted from bridge)
    /// Stores [time_offset, power] pairs
    plot_data: VecDeque<[f64; 2]>,

    /// Last timestamp for relative time calculation
    last_timestamp_ns: Option<i64>,

    /// Whether this panel is actively displaying
    is_visible: bool,

    /// Measurement rate (measurements per second)
    measurement_rate: f64,

    /// Frame counter for update rate calculation
    frame_counter: u32,

    /// Last measurement count from bridge (for rate calculation)
    last_measurement_count: usize,
}

impl V4InstrumentPanel {
    /// Create a new V4 instrument panel
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            plot_data: VecDeque::with_capacity(1000),
            last_timestamp_ns: None,
            is_visible: true,
            measurement_rate: 0.0,
            frame_counter: 0,
            last_measurement_count: 0,
        }
    }

    /// Update internal plot data from the bridge
    ///
    /// Extracts measurements from the bridge's ringbuffer and updates
    /// the local plot data cache with relative timestamps.
    fn sync_from_bridge(&mut self, bridge: &V4DataBridge) {
        let measurements = bridge.get_measurements(&self.instrument_id);

        if measurements.is_empty() {
            return;
        }

        // Set last timestamp on first data point
        if self.last_timestamp_ns.is_none() && !measurements.is_empty() {
            self.last_timestamp_ns = Some(measurements[0].timestamp_ns);
        }

        let base_ts = self.last_timestamp_ns.unwrap_or(0);

        // Update plot data with measurements not yet in cache
        self.plot_data.clear();
        for measurement in &measurements {
            let time_offset_s = (measurement.timestamp_ns - base_ts) as f64 / 1_000_000_000.0;
            self.plot_data.push_back([time_offset_s, measurement.power]);
        }

        // Update measurement rate (approximate - samples per second)
        self.frame_counter += 1;
        if self.frame_counter >= 30 {
            // Update rate every 30 frames
            let current_count = measurements.len();
            if current_count > self.last_measurement_count {
                let new_measurements = current_count - self.last_measurement_count;
                // Rough estimate: new measurements / (30 frames / 60 fps)
                self.measurement_rate = (new_measurements as f64 * 60.0) / 30.0;
                self.last_measurement_count = current_count;
            }
            self.frame_counter = 0;
        }
    }

    /// Render the panel UI
    ///
    /// # Arguments
    /// * `ui` - egui context for rendering
    /// * `bridge` - V4DataBridge containing measurement data
    pub fn ui(&mut self, ui: &mut egui::Ui, bridge: &V4DataBridge) {
        // Update plot data from bridge
        self.sync_from_bridge(bridge);

        // Panel header
        ui.heading(format!("V4: {}", self.instrument_id));
        ui.separator();

        // Status and statistics row
        ui.horizontal(|ui| {
            // Connection status
            ui.label("Status:");
            if bridge.get_latest(&self.instrument_id).is_some() {
                ui.colored_label(egui::Color32::GREEN, "Connected");
            } else {
                ui.colored_label(egui::Color32::GRAY, "No data");
            }

            ui.separator();

            // Measurement rate
            ui.label(format!("Rate: {:.1} meas/s", self.measurement_rate));

            ui.separator();

            // Statistics
            if let Some((min, max, mean)) = bridge.get_statistics(&self.instrument_id) {
                ui.label(format!(
                    "Min: {:.3} | Max: {:.3} | Mean: {:.3}",
                    min, max, mean
                ));
            }
        });

        ui.separator();

        // Latest value display
        if let Some(measurement) = bridge.get_latest(&self.instrument_id) {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Latest: {:.6} {} @ {:.1} nm",
                    measurement.power,
                    measurement.unit,
                    measurement.wavelength_nm.unwrap_or(0.0)
                ));
            });
        } else {
            ui.label("Waiting for data...");
        }

        ui.separator();

        // Plot
        render_power_plot(ui, &self.plot_data, &self.instrument_id);
    }
}

/// Render a live power plot with egui_plot
fn render_power_plot(ui: &mut egui::Ui, plot_data: &VecDeque<[f64; 2]>, instrument_id: &str) {
    if plot_data.is_empty() {
        ui.label("No data available");
        return;
    }

    // Create plot line from data
    let line = Line::new(PlotPoints::from_iter(plot_data.iter().copied()));

    Plot::new(format!("v4_power_{}", instrument_id))
        .view_aspect(2.0)
        .x_axis_label("Time (seconds)")
        .y_axis_label("Power")
        .show(ui, |plot_ui| {
            plot_ui.line(line);
        });
}

/// V4 Multi-Instrument Dashboard
///
/// Displays status and quick stats for all instruments connected to the bridge
pub struct V4Dashboard {
    /// List of instrument IDs to monitor
    instruments: Vec<String>,

    /// Expanded state for each instrument panel
    expanded_panels: std::collections::HashMap<String, bool>,
}

impl V4Dashboard {
    /// Create a new V4 dashboard
    pub fn new() -> Self {
        Self {
            instruments: Vec::new(),
            expanded_panels: std::collections::HashMap::new(),
        }
    }

    /// Add an instrument to monitor
    pub fn add_instrument(&mut self, instrument_id: String) {
        if !self.instruments.contains(&instrument_id) {
            self.instruments.push(instrument_id.clone());
            self.expanded_panels.insert(instrument_id, false);
        }
    }

    /// Remove an instrument from monitoring
    pub fn remove_instrument(&mut self, instrument_id: &str) {
        self.instruments.retain(|id| id != instrument_id);
        self.expanded_panels.remove(instrument_id);
    }

    /// Render the dashboard
    pub fn ui(&mut self, ui: &mut egui::Ui, bridge: &V4DataBridge) {
        ui.heading("V4 Instruments");
        ui.separator();

        // Sync instrument list with bridge
        let bridge_instruments = bridge.instruments();
        for inst_id in bridge_instruments {
            self.add_instrument(inst_id);
        }

        // Render each instrument's status card
        for inst_id in self.instruments.clone() {
            let is_expanded = self.expanded_panels.entry(inst_id.clone()).or_insert(false);

            ui.horizontal(|ui| {
                // Expand/collapse button
                if ui.button(if *is_expanded { "-" } else { "+" }).clicked() {
                    *is_expanded = !*is_expanded;
                }

                // Instrument status
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&inst_id);

                        // Connection indicator
                        if bridge.get_latest(&inst_id).is_some() {
                            ui.colored_label(egui::Color32::GREEN, "●");
                        } else {
                            ui.colored_label(egui::Color32::GRAY, "●");
                        }
                    });

                    if *is_expanded {
                        // Show stats when expanded
                        if let Some((min, max, mean)) = bridge.get_statistics(&inst_id) {
                            ui.label(format!("Min: {:.3}", min));
                            ui.label(format!("Max: {:.3}", max));
                            ui.label(format!("Mean: {:.3}", mean));

                            if let Some(m) = bridge.get_latest(&inst_id) {
                                ui.label(format!("Latest: {:.3} {}", m.power, m.unit));
                            }
                        }
                    }
                });
            });

            ui.separator();
        }
    }
}

impl Default for V4Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_creation() {
        let panel = V4InstrumentPanel::new("test_instrument".to_string());
        assert_eq!(panel.instrument_id, "test_instrument");
        assert_eq!(panel.plot_data.len(), 0);
        assert!(panel.is_visible);
    }

    #[test]
    fn test_dashboard_creation() {
        let dashboard = V4Dashboard::new();
        assert_eq!(dashboard.instruments.len(), 0);
    }

    #[test]
    fn test_dashboard_add_instrument() {
        let mut dashboard = V4Dashboard::new();
        dashboard.add_instrument("inst1".to_string());
        dashboard.add_instrument("inst2".to_string());

        assert_eq!(dashboard.instruments.len(), 2);
        assert!(dashboard.instruments.contains(&"inst1".to_string()));
    }

    #[test]
    fn test_dashboard_add_duplicate() {
        let mut dashboard = V4Dashboard::new();
        dashboard.add_instrument("inst1".to_string());
        dashboard.add_instrument("inst1".to_string());

        // Should not add duplicate
        assert_eq!(dashboard.instruments.len(), 1);
    }

    #[test]
    fn test_dashboard_remove() {
        let mut dashboard = V4Dashboard::new();
        dashboard.add_instrument("inst1".to_string());
        dashboard.add_instrument("inst2".to_string());

        dashboard.remove_instrument("inst1");
        assert_eq!(dashboard.instruments.len(), 1);
        assert!(!dashboard.instruments.contains(&"inst1".to_string()));
    }
}
