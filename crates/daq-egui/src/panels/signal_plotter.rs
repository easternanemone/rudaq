//! Signal Plotter Panel - lightweight 1D signal visualization
//!
//! For micro-visualizations (5-10 second scope) providing quick feedback.
//! Primary data visualization uses Rerun viewer.
//!
//! ## Async Integration Pattern
//!
//! This panel uses message-passing for thread-safe async updates:
//! - `ObservableUpdateSender` is passed to background Tokio tasks
//! - `SignalPlotterPanel` stores a receiver and drains it each frame
//! - No mutable borrows cross async boundaries

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Instant;

/// Maximum history depth (points)
const MAX_HISTORY: usize = 500;

/// Maximum queued observable updates (prevents memory growth under high-rate producers)
const MAX_QUEUED_UPDATES: usize = 1000;

/// Default time window to display (seconds)
const DEFAULT_TIME_WINDOW: f64 = 10.0;

/// Maximum history to keep for scrollback (seconds)
const MAX_HISTORY_SECONDS: f64 = 120.0;

/// Available time window presets
const TIME_WINDOW_OPTIONS: &[f64] = &[1.0, 5.0, 10.0, 30.0, 60.0];

/// Observable update message for async integration
///
/// This struct is sent from background Tokio tasks to the UI thread
/// via mpsc channel, avoiding mutable borrows across async boundaries.
#[derive(Debug, Clone)]
pub struct ObservableUpdate {
    pub device_id: String,
    pub observable_name: String,
    pub value: f64,
    /// Timestamp in seconds (for future time-sync feature)
    #[allow(dead_code)]
    pub timestamp_secs: f64,
}

impl ObservableUpdate {
    pub fn new(
        device_id: impl Into<String>,
        observable_name: impl Into<String>,
        value: f64,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            observable_name: observable_name.into(),
            value,
            timestamp_secs: 0.0, // Will be set relative to trace start time
        }
    }
}

/// Sender handle for pushing observable updates from async tasks
///
/// Clone this and pass to background Tokio tasks. Thread-safe.
/// Uses SyncSender with bounded capacity to prevent memory exhaustion.
pub type ObservableUpdateSender = mpsc::SyncSender<ObservableUpdate>;

/// Receiver handle stored in SignalPlotterPanel
pub type ObservableUpdateReceiver = mpsc::Receiver<ObservableUpdate>;

/// Create a new bounded channel pair for observable updates
///
/// Uses a bounded channel to prevent memory exhaustion under high-rate producers.
/// When the queue is full, `try_send` will drop updates rather than blocking.
pub fn observable_channel() -> (ObservableUpdateSender, ObservableUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_UPDATES)
}

/// Preset colors for new traces
const TRACE_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(255, 100, 100), // Red
    egui::Color32::from_rgb(100, 200, 100), // Green
    egui::Color32::from_rgb(100, 150, 255), // Blue
    egui::Color32::from_rgb(255, 200, 100), // Orange
    egui::Color32::from_rgb(200, 100, 255), // Purple
    egui::Color32::from_rgb(100, 255, 255), // Cyan
    egui::Color32::from_rgb(255, 100, 200), // Pink
    egui::Color32::from_rgb(200, 200, 100), // Yellow
];

/// A single signal trace
pub struct SignalTrace {
    pub label: String,
    pub device_id: String,
    pub observable_name: String,
    pub color: egui::Color32,
    pub visible: bool,
    pub points: VecDeque<(f64, f64)>, // (time_offset, value)
    pub start_time: Instant,
}

impl SignalTrace {
    /// Create a new trace with current time as start (convenience constructor)
    #[allow(dead_code)]
    pub fn new(label: &str, device_id: &str, observable_name: &str, color: egui::Color32) -> Self {
        Self::with_start_time(label, device_id, observable_name, color, Instant::now())
    }

    /// Create a new trace with a specific start time (for shared baseline)
    pub fn with_start_time(
        label: &str,
        device_id: &str,
        observable_name: &str,
        color: egui::Color32,
        start_time: Instant,
    ) -> Self {
        Self {
            label: label.to_string(),
            device_id: device_id.to_string(),
            observable_name: observable_name.to_string(),
            color,
            visible: true,
            points: VecDeque::with_capacity(MAX_HISTORY),
            start_time,
        }
    }

    /// Add a new data point
    pub fn push(&mut self, value: f64) {
        let time = self.start_time.elapsed().as_secs_f64();
        self.points.push_back((time, value));

        // Trim old points beyond maximum history (keep more for scrollback)
        let cutoff = time - MAX_HISTORY_SECONDS;
        while self
            .points
            .front()
            .map(|(t, _)| *t < cutoff)
            .unwrap_or(false)
        {
            self.points.pop_front();
        }

        // Also enforce max history count
        while self.points.len() > MAX_HISTORY {
            self.points.pop_front();
        }
    }

    /// Get current time offset (for external time queries)
    #[allow(dead_code)]
    pub fn current_time(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Get last value
    pub fn last_value(&self) -> Option<f64> {
        self.points.back().map(|(_, v)| *v)
    }

    /// Compute statistics for points within a time range
    pub fn statistics_for_range(&self, t_start: f64, t_end: f64) -> TraceStatistics {
        let values: Vec<f64> = self
            .points
            .iter()
            .filter(|(t, _)| *t >= t_start && *t <= t_end)
            .map(|(_, v)| *v)
            .collect();

        if values.is_empty() {
            return TraceStatistics::default();
        }

        let n = values.len();
        let sum: f64 = values.iter().sum();
        let mean = sum / n as f64;

        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        let variance: f64 = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = variance.sqrt();

        TraceStatistics {
            count: n,
            min,
            max,
            mean,
            std_dev,
        }
    }
}

/// Statistics for a trace over a time window
#[derive(Debug, Clone, Default)]
pub struct TraceStatistics {
    /// Number of samples
    pub count: usize,
    /// Minimum value
    pub min: f64,
    /// Maximum value
    pub max: f64,
    /// Mean value
    pub mean: f64,
    /// Standard deviation
    pub std_dev: f64,
}

/// Signal Plotter Panel state
pub struct SignalPlotterPanel {
    /// Shared time baseline for all traces (ensures comparable timestamps)
    panel_start_time: Instant,
    /// Active traces
    traces: Vec<SignalTrace>,
    /// Y-axis range (None = autoscale)
    y_range: Option<(f64, f64)>,
    /// Y-axis min input (text field for manual entry)
    y_min_input: String,
    /// Y-axis max input (text field for manual entry)
    y_max_input: String,
    /// Y-axis is locked to manual range
    y_axis_locked: bool,
    /// Time window to display (seconds)
    time_window: f64,
    /// Frozen mode (stop scrolling, allow examination of historical data)
    frozen: bool,
    /// Time offset when frozen (allows scrollback)
    frozen_time_offset: f64,
    /// Show legend
    show_legend: bool,
    /// Show statistics panel
    show_statistics: bool,
    /// Show trace manager panel
    show_trace_manager: bool,
    /// Paused (stop updating)
    paused: bool,
    /// Receiver for async observable updates
    update_rx: Option<ObservableUpdateReceiver>,
    /// Sender clone for spawning new subscriptions
    update_tx: Option<ObservableUpdateSender>,
    /// New trace device ID input
    new_trace_device: String,
    /// New trace observable name input
    new_trace_observable: String,
    /// New trace label input
    new_trace_label: String,
    /// New trace color index
    new_trace_color_idx: usize,
    /// Export file path
    export_path: String,
    /// Last export status message
    export_status: Option<(String, bool)>, // (message, is_error)
}

impl Default for SignalPlotterPanel {
    fn default() -> Self {
        let (tx, rx) = observable_channel();
        Self {
            panel_start_time: Instant::now(),
            traces: Vec::new(),
            y_range: None,
            y_min_input: String::new(),
            y_max_input: String::new(),
            y_axis_locked: false,
            time_window: DEFAULT_TIME_WINDOW,
            frozen: false,
            frozen_time_offset: 0.0,
            show_legend: true,
            show_statistics: false,
            show_trace_manager: false,
            paused: false,
            update_rx: Some(rx),
            update_tx: Some(tx),
            new_trace_device: String::new(),
            new_trace_observable: String::new(),
            new_trace_label: String::new(),
            new_trace_color_idx: 0,
            export_path: String::from("signal_data.csv"),
            export_status: None,
        }
    }
}

impl SignalPlotterPanel {
    /// Create a new panel with a dedicated update channel
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a clone of the update sender for async tasks
    ///
    /// Pass this to background Tokio tasks that need to push updates.
    pub fn get_sender(&self) -> Option<ObservableUpdateSender> {
        self.update_tx.clone()
    }

    /// Drain pending updates from async tasks
    ///
    /// Call this at the start of each frame to process updates.
    /// Non-blocking: returns immediately if no updates available.
    pub fn drain_updates(&mut self) {
        // Collect all pending updates first to avoid borrow issues
        let updates: Vec<ObservableUpdate> = if let Some(rx) = &self.update_rx {
            std::iter::from_fn(|| rx.try_recv().ok()).collect()
        } else {
            return;
        };

        if self.paused {
            // Drained but not processed - avoid backpressure
            return;
        }

        // Now process collected updates
        for update in updates {
            self.push_observable(&update.device_id, &update.observable_name, update.value);
        }
    }
}

impl SignalPlotterPanel {
    /// Add a new trace (uses shared time baseline for consistent timestamps)
    pub fn add_trace(
        &mut self,
        label: &str,
        device_id: &str,
        observable_name: &str,
        color: egui::Color32,
    ) {
        self.traces.push(SignalTrace::with_start_time(
            label,
            device_id,
            observable_name,
            color,
            self.panel_start_time,
        ));
    }

    /// Remove a trace by label (public API for external control)
    #[allow(dead_code)]
    pub fn remove_trace(&mut self, label: &str) {
        self.traces.retain(|t| t.label != label);
    }

    /// Push value to a trace by label (public API for external control)
    #[allow(dead_code)]
    pub fn push_value(&mut self, label: &str, value: f64) {
        if self.paused {
            return;
        }
        if let Some(trace) = self.traces.iter_mut().find(|t| t.label == label) {
            trace.push(value);
        }
    }

    /// Push value by device_id and observable_name
    pub fn push_observable(&mut self, device_id: &str, observable_name: &str, value: f64) {
        if self.paused {
            return;
        }
        if let Some(trace) = self
            .traces
            .iter_mut()
            .find(|t| t.device_id == device_id && t.observable_name == observable_name)
        {
            trace.push(value);
        }
    }

    /// Export all visible traces to CSV
    fn export_to_csv(&mut self, path: std::path::PathBuf) {
        use crate::export::{signal::SignalTraceData, SignalExportOptions};

        // Collect visible traces
        let traces: Vec<SignalTraceData> = self
            .traces
            .iter()
            .filter(|t| t.visible)
            .map(|t| {
                SignalTraceData::from_deque(
                    t.label.clone(),
                    t.device_id.clone(),
                    t.observable_name.clone(),
                    &t.points,
                )
            })
            .collect();

        if traces.is_empty() {
            self.export_status = Some(("No visible traces to export".to_string(), true));
            return;
        }

        // Export with default options
        let options = SignalExportOptions::default();
        match crate::export::signal::export_signal_traces(&path, &traces, &options) {
            Ok(_) => {
                let filename = path.file_name().unwrap_or_default().to_string_lossy();
                self.export_status = Some((
                    format!("✓ Exported {} traces to {}", traces.len(), filename),
                    false,
                ));
            }
            Err(e) => {
                self.export_status = Some((format!("Export failed: {}", e), true));
            }
        }
    }

    /// Render the signal plotter
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Drain any pending async updates first
        self.drain_updates();

        // Toolbar
        ui.horizontal(|ui| {
            ui.heading("Signal Scope");

            ui.separator();

            let label = if self.paused {
                "▶ Resume"
            } else {
                "⏸ Pause"
            };
            ui.toggle_value(&mut self.paused, label);

            ui.toggle_value(&mut self.show_legend, "Legend");
            ui.toggle_value(&mut self.show_statistics, "Stats");
            ui.toggle_value(&mut self.show_trace_manager, "Traces");

            if ui.button("Clear").clicked() {
                // Reset panel baseline so new traces align with cleared traces
                self.panel_start_time = Instant::now();
                for trace in &mut self.traces {
                    trace.points.clear();
                    trace.start_time = self.panel_start_time;
                }
            }
        });

        // Y-axis controls row
        ui.horizontal(|ui| {
            ui.label("Y-axis:");

            // Autoscale button
            if ui.selectable_label(!self.y_axis_locked, "Auto").clicked() {
                self.y_axis_locked = false;
                self.y_range = None;
            }

            ui.separator();

            // Manual range inputs
            ui.label("Min:");
            let min_response = ui.add(
                egui::TextEdit::singleline(&mut self.y_min_input)
                    .desired_width(60.0)
                    .hint_text("auto"),
            );

            ui.label("Max:");
            let max_response = ui.add(
                egui::TextEdit::singleline(&mut self.y_max_input)
                    .desired_width(60.0)
                    .hint_text("auto"),
            );

            // Lock/Apply button
            let lock_label = if self.y_axis_locked {
                "🔒 Locked"
            } else {
                "🔓 Apply"
            };
            if ui.button(lock_label).clicked()
                || min_response.lost_focus()
                || max_response.lost_focus()
            {
                // Try to parse min/max values
                let y_min = self.y_min_input.trim().parse::<f64>().ok();
                let y_max = self.y_max_input.trim().parse::<f64>().ok();

                if let (Some(min), Some(max)) = (y_min, y_max) {
                    if min < max {
                        self.y_range = Some((min, max));
                        self.y_axis_locked = true;
                    }
                } else if self.y_axis_locked {
                    // Toggle unlock if already locked
                    self.y_axis_locked = false;
                    self.y_range = None;
                }
            }

            // Reset button
            if ui.button("Reset").clicked() {
                self.y_min_input.clear();
                self.y_max_input.clear();
                self.y_axis_locked = false;
                self.y_range = None;
            }
        });

        // Time window controls row
        ui.horizontal(|ui| {
            ui.label("Time:");

            // Time window dropdown
            egui::ComboBox::from_id_salt("time_window_select")
                .selected_text(format!("{}s", self.time_window as i32))
                .width(60.0)
                .show_ui(ui, |ui| {
                    for &window in TIME_WINDOW_OPTIONS {
                        let label = format!("{}s", window as i32);
                        if ui
                            .selectable_label((self.time_window - window).abs() < 0.01, label)
                            .clicked()
                        {
                            self.time_window = window;
                        }
                    }
                });

            ui.separator();

            // Freeze toggle
            let freeze_label = if self.frozen {
                "🔒 Frozen"
            } else {
                "❄ Freeze"
            };
            if ui.selectable_label(self.frozen, freeze_label).clicked() {
                self.frozen = !self.frozen;
                if self.frozen {
                    // Capture current time offset when freezing
                    self.frozen_time_offset = 0.0;
                }
            }

            // Scroll controls when frozen
            if self.frozen {
                let current_time = self.current_time();
                let max_scroll = (current_time - self.time_window).max(0.0);

                ui.separator();
                ui.label("Scroll:");

                // Scroll left (earlier)
                if ui.button("◀").clicked() && self.frozen_time_offset < max_scroll {
                    self.frozen_time_offset =
                        (self.frozen_time_offset + self.time_window / 4.0).min(max_scroll);
                }

                // Scroll slider
                let mut scroll_pos = self.frozen_time_offset;
                if ui
                    .add(
                        egui::Slider::new(&mut scroll_pos, 0.0..=max_scroll.max(0.01))
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always),
                    )
                    .changed()
                {
                    self.frozen_time_offset = scroll_pos;
                }

                // Scroll right (later)
                if ui.button("▶").clicked() && self.frozen_time_offset > 0.0 {
                    self.frozen_time_offset =
                        (self.frozen_time_offset - self.time_window / 4.0).max(0.0);
                }

                // Jump to live
                if ui.button("⏵ Live").clicked() {
                    self.frozen = false;
                    self.frozen_time_offset = 0.0;
                }
            }
        });

        // Export controls row
        ui.horizontal(|ui| {
            ui.label("Export:");

            // Export button
            if ui.button("📁 Export to CSV").clicked() {
                // Open file dialog
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("signal_data.csv")
                    .add_filter("CSV Files", &["csv"])
                    .add_filter("All Files", &["*"])
                    .save_file()
                {
                    self.export_to_csv(path);
                }
            }

            // Show export status
            if let Some((msg, is_error)) = &self.export_status {
                let color = if *is_error {
                    egui::Color32::RED
                } else {
                    egui::Color32::GREEN
                };
                ui.colored_label(color, msg);

                // Auto-clear after 5 seconds
                if ui.ctx().input(|i| i.time) > 0.0 {
                    // Clear on next frame (simple timeout)
                    if ui.button("✕").clicked() {
                        self.export_status = None;
                    }
                }
            }
        });

        ui.separator();

        // Current values display
        let visible_traces: Vec<_> = self.traces.iter().filter(|t| t.visible).collect();
        if !visible_traces.is_empty() {
            ui.horizontal(|ui| {
                for trace in &visible_traces {
                    if let Some(value) = trace.last_value() {
                        ui.colored_label(trace.color, format!("{}: {:.4}", trace.label, value));
                        ui.separator();
                    }
                }
            });
        }

        // Plot
        let current_time = self.current_time();

        let mut plot = Plot::new("signal_scope")
            .height(200.0)
            .show_axes(true)
            .show_grid(true)
            .x_axis_label("Time (s)")
            .y_axis_label("Value");

        if self.show_legend {
            plot = plot.legend(egui_plot::Legend::default());
        }

        plot.show(ui, |plot_ui| {
            // Calculate x-axis bounds based on time window and frozen state
            let (x_min, x_max) = if self.frozen {
                // When frozen, show window at offset from current time
                let end = current_time - self.frozen_time_offset;
                let start = (end - self.time_window).max(0.0);
                (start, end)
            } else {
                // Live mode: show last time_window seconds
                let x_max = current_time;
                let x_min = (current_time - self.time_window).max(0.0);
                (x_min, x_max)
            };

            // Set plot bounds with optional Y-axis range
            let (y_min, y_max) = if let Some((min, max)) = self.y_range {
                (min, max)
            } else {
                (f64::NEG_INFINITY, f64::INFINITY)
            };

            plot_ui.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
                [x_min, y_min],
                [x_max, y_max],
            ));

            for trace in &self.traces {
                if !trace.visible {
                    continue;
                }

                let points: PlotPoints = trace.points.iter().map(|(t, v)| [*t, *v]).collect();

                let line = Line::new(&trace.label, points)
                    .color(trace.color)
                    .width(2.0);

                plot_ui.line(line);
            }
        });

        // Statistics panel
        if self.show_statistics && !self.traces.is_empty() {
            ui.separator();

            // Calculate visible time range
            let current_time = self.current_time();
            let (t_start, t_end) = if self.frozen {
                let end = current_time - self.frozen_time_offset;
                let start = (end - self.time_window).max(0.0);
                (start, end)
            } else {
                let t_end = current_time;
                let t_start = (current_time - self.time_window).max(0.0);
                (t_start, t_end)
            };

            egui::Grid::new("trace_stats_grid")
                .num_columns(6)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    // Header
                    ui.strong("Trace");
                    ui.strong("Count");
                    ui.strong("Min");
                    ui.strong("Max");
                    ui.strong("Mean");
                    ui.strong("Std Dev");
                    ui.end_row();

                    // Per-trace statistics (only visible traces)
                    for trace in self.traces.iter().filter(|t| t.visible) {
                        let stats = trace.statistics_for_range(t_start, t_end);

                        ui.colored_label(trace.color, &trace.label);
                        ui.label(format!("{}", stats.count));
                        ui.label(if stats.count > 0 {
                            format!("{:.4}", stats.min)
                        } else {
                            "-".to_string()
                        });
                        ui.label(if stats.count > 0 {
                            format!("{:.4}", stats.max)
                        } else {
                            "-".to_string()
                        });
                        ui.label(if stats.count > 0 {
                            format!("{:.4}", stats.mean)
                        } else {
                            "-".to_string()
                        });
                        ui.label(if stats.count > 0 {
                            format!("{:.4}", stats.std_dev)
                        } else {
                            "-".to_string()
                        });
                        ui.end_row();
                    }
                });
        }

        // Trace manager panel
        if self.show_trace_manager {
            ui.separator();
            ui.collapsing("Trace Manager", |ui| {
                // Existing traces list
                if !self.traces.is_empty() {
                    ui.label("Active Traces:");

                    let mut trace_to_remove: Option<usize> = None;

                    egui::Grid::new("trace_manager_grid")
                        .num_columns(5)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            // Header
                            ui.strong("Visible");
                            ui.strong("Color");
                            ui.strong("Label");
                            ui.strong("Device/Observable");
                            ui.strong("");
                            ui.end_row();

                            for (idx, trace) in self.traces.iter_mut().enumerate() {
                                // Visibility toggle
                                let vis_icon = if trace.visible { "👁" } else { "👁‍🗨" };
                                if ui
                                    .button(vis_icon)
                                    .on_hover_text(if trace.visible {
                                        "Hide trace"
                                    } else {
                                        "Show trace"
                                    })
                                    .clicked()
                                {
                                    trace.visible = !trace.visible;
                                }

                                // Color button (opens color picker)
                                ui.color_edit_button_srgba(&mut trace.color);

                                // Label (editable)
                                ui.add(
                                    egui::TextEdit::singleline(&mut trace.label)
                                        .desired_width(80.0),
                                );

                                // Device/Observable info
                                ui.label(format!("{}/{}", trace.device_id, trace.observable_name));

                                // Remove button
                                if ui.button("✖").on_hover_text("Remove trace").clicked() {
                                    trace_to_remove = Some(idx);
                                }

                                ui.end_row();
                            }
                        });

                    // Remove trace if requested
                    if let Some(idx) = trace_to_remove {
                        self.traces.remove(idx);
                    }

                    ui.separator();
                }

                // Add new trace section
                ui.label("Add New Trace:");

                ui.horizontal(|ui| {
                    ui.label("Device:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_trace_device)
                            .desired_width(100.0)
                            .hint_text("device_id"),
                    );

                    ui.label("Observable:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_trace_observable)
                            .desired_width(100.0)
                            .hint_text("observable"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Label:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_trace_label)
                            .desired_width(100.0)
                            .hint_text("trace name"),
                    );

                    ui.label("Color:");
                    // Color preset buttons
                    for (idx, &color) in TRACE_COLORS.iter().enumerate() {
                        let is_selected = self.new_trace_color_idx == idx;
                        let btn = egui::Button::new("  ").fill(color).stroke(if is_selected {
                            egui::Stroke::new(2.0, egui::Color32::WHITE)
                        } else {
                            egui::Stroke::NONE
                        });
                        if ui.add(btn).clicked() {
                            self.new_trace_color_idx = idx;
                        }
                    }
                });

                ui.horizontal(|ui| {
                    let can_add =
                        !self.new_trace_device.is_empty() && !self.new_trace_observable.is_empty();

                    if ui
                        .add_enabled(can_add, egui::Button::new("➕ Add Trace"))
                        .clicked()
                    {
                        // Clone strings to avoid borrow conflict
                        let device = self.new_trace_device.clone();
                        let observable = self.new_trace_observable.clone();
                        let label = if self.new_trace_label.is_empty() {
                            format!("{}/{}", device, observable)
                        } else {
                            self.new_trace_label.clone()
                        };

                        let color = TRACE_COLORS[self.new_trace_color_idx % TRACE_COLORS.len()];

                        self.add_trace(&label, &device, &observable, color);

                        // Clear inputs and advance color
                        self.new_trace_device.clear();
                        self.new_trace_observable.clear();
                        self.new_trace_label.clear();
                        self.new_trace_color_idx =
                            (self.new_trace_color_idx + 1) % TRACE_COLORS.len();
                    }
                });

                // Export section
                ui.separator();
                ui.label("Export Data:");

                ui.horizontal(|ui| {
                    ui.label("File:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.export_path)
                            .desired_width(200.0)
                            .hint_text("path/to/file.csv"),
                    );

                    let can_export = !self.traces.is_empty() && !self.export_path.is_empty();

                    if ui
                        .add_enabled(can_export, egui::Button::new("📁 Export CSV"))
                        .clicked()
                    {
                        let path = self.export_path.clone();
                        match self.export_to_csv(&path) {
                            Ok(()) => {
                                let point_count: usize =
                                    self.traces.iter().map(|t| t.points.len()).sum();
                                self.export_status = Some((
                                    format!(
                                        "Exported {} traces ({} points) to {}",
                                        self.traces.len(),
                                        point_count,
                                        path
                                    ),
                                    false,
                                ));
                            }
                            Err(e) => {
                                self.export_status = Some((e, true));
                            }
                        }
                    }
                });

                // Show export status
                if let Some((message, is_error)) = &self.export_status {
                    let color = if *is_error {
                        egui::Color32::from_rgb(255, 100, 100)
                    } else {
                        egui::Color32::from_rgb(100, 200, 100)
                    };
                    ui.colored_label(color, message);
                }
            });
        } else if self.traces.is_empty() {
            ui.label("No traces. Click 'Traces' to add observables.");
        }
    }

    /// Get trace count (public API for external queries)
    #[allow(dead_code)]
    pub fn trace_count(&self) -> usize {
        self.traces.len()
    }

    /// Get current time from panel baseline
    pub fn current_time(&self) -> f64 {
        self.panel_start_time.elapsed().as_secs_f64()
    }

    /// Generate CSV content from all traces
    ///
    /// Format: timestamp, trace1_value, trace2_value, ...
    /// Uses NaN for missing values when traces have different timestamps
    pub fn generate_csv(&self) -> String {
        if self.traces.is_empty() {
            return String::from("# No data\n");
        }

        let mut csv = String::new();

        // Header with trace names
        csv.push_str("timestamp");
        for trace in &self.traces {
            csv.push(',');
            // Escape label if it contains special characters
            if trace.label.contains(',') || trace.label.contains('"') {
                csv.push('"');
                csv.push_str(&trace.label.replace('"', "\"\""));
                csv.push('"');
            } else {
                csv.push_str(&trace.label);
            }
        }
        csv.push('\n');

        // Collect all unique timestamps and sort them
        let mut all_timestamps: Vec<f64> = self
            .traces
            .iter()
            .flat_map(|t| t.points.iter().map(|(ts, _)| *ts))
            .collect();
        all_timestamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        all_timestamps.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

        // For each timestamp, find values from each trace
        for ts in &all_timestamps {
            csv.push_str(&format!("{:.6}", ts));

            for trace in &self.traces {
                csv.push(',');
                // Find value at this timestamp (exact match within tolerance)
                if let Some((_, val)) = trace.points.iter().find(|(t, _)| (*t - ts).abs() < 1e-9) {
                    csv.push_str(&format!("{:.6}", val));
                }
                // If no value, leave empty (which represents NaN in CSV conventions)
            }
            csv.push('\n');
        }

        csv
    }

    /// Export trace data to CSV file
    pub fn export_to_csv(&mut self, path: &str) -> Result<(), String> {
        use std::io::Write;

        let csv_content = self.generate_csv();

        let file =
            std::fs::File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;

        let mut writer = std::io::BufWriter::new(file);
        writer
            .write_all(csv_content.as_bytes())
            .map_err(|e| format!("Failed to write data: {}", e))?;

        writer
            .flush()
            .map_err(|e| format!("Failed to flush buffer: {}", e))?;

        Ok(())
    }
}
