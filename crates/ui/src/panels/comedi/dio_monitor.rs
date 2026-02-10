//! DIO State Monitor Panel for Comedi DAQ devices.
//!
//! Provides LED-style visualization of digital I/O pin states
//! with activity indicators, pulse detection, and edge counting.

use eframe::egui::{self, Color32, RichText, Ui};
use std::collections::VecDeque;
use std::time::Instant;
use tokio::sync::mpsc;

/// Maximum history for edge detection
const MAX_HISTORY: usize = 100;

/// DIO state update
#[derive(Debug, Clone)]
pub struct DioStateUpdate {
    pub pin: u32,
    pub state: bool,
    pub timestamp: Option<f64>,
}

impl DioStateUpdate {
    pub fn new(pin: u32, state: bool) -> Self {
        Self {
            pin,
            state,
            timestamp: None,
        }
    }
}

/// Sender for DIO state updates
pub type DioMonitorSender = mpsc::Sender<DioStateUpdate>;
/// Receiver for DIO state updates
pub type DioMonitorReceiver = mpsc::Receiver<DioStateUpdate>;

/// Create channel pair for DIO monitor
pub fn dio_monitor_channel() -> (DioMonitorSender, DioMonitorReceiver) {
    mpsc::channel(512)
}

/// Pin state with edge detection
#[derive(Debug, Clone)]
struct PinState {
    state: bool,
    last_change: Instant,
    rising_edges: u64,
    falling_edges: u64,
    history: VecDeque<(Instant, bool)>,
    pulse_active: bool,
    last_pulse_width_ms: Option<f64>,
}

impl Default for PinState {
    fn default() -> Self {
        Self {
            state: false,
            last_change: Instant::now(),
            rising_edges: 0,
            falling_edges: 0,
            history: VecDeque::with_capacity(MAX_HISTORY),
            pulse_active: false,
            last_pulse_width_ms: None,
        }
    }
}

impl PinState {
    fn update(&mut self, new_state: bool) {
        let now = Instant::now();

        if new_state != self.state {
            // Edge detected
            if new_state {
                self.rising_edges += 1;
                self.pulse_active = true;
            } else {
                self.falling_edges += 1;
                if self.pulse_active {
                    self.last_pulse_width_ms =
                        Some(self.last_change.elapsed().as_secs_f64() * 1000.0);
                    self.pulse_active = false;
                }
            }
            self.last_change = now;
        }

        self.state = new_state;
        self.history.push_back((now, new_state));

        // Trim history
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    fn reset_counters(&mut self) {
        self.rising_edges = 0;
        self.falling_edges = 0;
    }

    /// Time since last change in ms
    fn ms_since_change(&self) -> f64 {
        self.last_change.elapsed().as_secs_f64() * 1000.0
    }

    /// Check if recently active (changed within threshold)
    fn is_active(&self, threshold_ms: f64) -> bool {
        self.ms_since_change() < threshold_ms
    }
}

/// Display mode for the monitor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorDisplayMode {
    #[default]
    LEDs,
    Table,
    Timing,
}

impl MonitorDisplayMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::LEDs => "LED View",
            Self::Table => "Table View",
            Self::Timing => "Timing View",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::LEDs, Self::Table, Self::Timing]
    }
}

/// DIO State Monitor Panel
pub struct DioMonitorPanel {
    /// Start time
    start_time: Instant,
    /// Pin states (up to 32 pins)
    pins: Vec<PinState>,
    /// Number of pins to display
    n_pins: u32,
    /// State receiver
    state_rx: DioMonitorReceiver,
    /// State sender (for cloning)
    state_tx: DioMonitorSender,
    /// Display mode
    display_mode: MonitorDisplayMode,
    /// Activity threshold (ms)
    activity_threshold_ms: f64,
    /// Show edge counts
    show_edge_counts: bool,
    /// Show pulse widths
    show_pulse_widths: bool,
    /// Frozen (stop updates)
    frozen: bool,
    /// Pin labels (optional custom names)
    pin_labels: Vec<String>,
    /// LED size
    led_size: f32,
    /// Active (high) color
    color_high: Color32,
    /// Inactive (low) color
    color_low: Color32,
    /// Activity indicator color
    color_activity: Color32,
}

impl Default for DioMonitorPanel {
    fn default() -> Self {
        let (tx, rx) = dio_monitor_channel();

        Self {
            start_time: Instant::now(),
            pins: (0..32).map(|_| PinState::default()).collect(),
            n_pins: 24,
            state_rx: rx,
            state_tx: tx,
            display_mode: MonitorDisplayMode::LEDs,
            activity_threshold_ms: 100.0,
            show_edge_counts: true,
            show_pulse_widths: true,
            frozen: false,
            pin_labels: (0..32).map(|i| format!("DIO{}", i)).collect(),
            led_size: 24.0,
            color_high: Color32::from_rgb(50, 255, 100),
            color_low: Color32::from_gray(60),
            color_activity: Color32::from_rgb(255, 200, 50),
        }
    }
}

impl DioMonitorPanel {
    /// Create a new DIO monitor panel
    pub fn new(n_pins: u32) -> Self {
        Self {
            n_pins: n_pins.min(32),
            ..Self::default()
        }
    }

    /// Get sender for pushing state updates
    pub fn get_sender(&self) -> DioMonitorSender {
        self.state_tx.clone()
    }

    /// Drain pending state updates
    fn drain_updates(&mut self) {
        while let Ok(update) = self.state_rx.try_recv() {
            if self.frozen {
                continue;
            }

            if let Some(pin) = self.pins.get_mut(update.pin as usize) {
                pin.update(update.state);
            }
        }
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        // Drain updates
        self.drain_updates();

        // Header
        ui.horizontal(|ui| {
            ui.heading("DIO State Monitor");
            ui.separator();
            ui.label(format!("{} pins", self.n_pins));

            ui.separator();

            // Freeze toggle
            let freeze_text = if self.frozen { "Unfreeze" } else { "Freeze" };
            if ui.button(freeze_text).clicked() {
                self.frozen = !self.frozen;
            }

            // Reset counters
            if ui.button("Reset Counters").clicked() {
                for pin in &mut self.pins {
                    pin.reset_counters();
                }
            }
        });

        ui.separator();

        // Display mode selector
        ui.horizontal(|ui| {
            ui.label("View:");
            for mode in MonitorDisplayMode::all() {
                if ui
                    .selectable_label(self.display_mode == *mode, mode.label())
                    .clicked()
                {
                    self.display_mode = *mode;
                }
            }

            ui.separator();

            ui.checkbox(&mut self.show_edge_counts, "Edge Counts");
            ui.checkbox(&mut self.show_pulse_widths, "Pulse Widths");
        });

        ui.separator();

        // Main display
        match self.display_mode {
            MonitorDisplayMode::LEDs => self.render_led_view(ui),
            MonitorDisplayMode::Table => self.render_table_view(ui),
            MonitorDisplayMode::Timing => self.render_timing_view(ui),
        }

        // Request repaint for activity animation
        if !self.frozen {
            ui.ctx().request_repaint();
        }
    }

    /// Render LED-style view
    fn render_led_view(&mut self, ui: &mut Ui) {
        let cols = 8;
        let rows = self.n_pins.div_ceil(cols);

        egui::Grid::new("dio_led_grid")
            .num_columns(cols as usize)
            .spacing([12.0, 12.0])
            .show(ui, |ui| {
                for row in 0..rows {
                    for col in 0..cols {
                        let pin_idx = row * cols + col;
                        if pin_idx >= self.n_pins {
                            ui.label(""); // Empty cell
                            continue;
                        }

                        let pin = &self.pins[pin_idx as usize];
                        let is_active = pin.is_active(self.activity_threshold_ms);

                        ui.vertical(|ui| {
                            // LED circle
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(self.led_size, self.led_size),
                                egui::Sense::hover(),
                            );

                            let center = rect.center();
                            let radius = self.led_size / 2.0 - 2.0;

                            // LED color
                            let led_color = if pin.state {
                                self.color_high
                            } else {
                                self.color_low
                            };

                            // Draw LED
                            ui.painter().circle_filled(center, radius, led_color);

                            // Activity ring
                            if is_active {
                                ui.painter().circle_stroke(
                                    center,
                                    radius + 2.0,
                                    egui::Stroke::new(2.0, self.color_activity),
                                );
                            }

                            // Outer ring
                            ui.painter().circle_stroke(
                                center,
                                radius,
                                egui::Stroke::new(1.0, Color32::from_gray(100)),
                            );

                            // Label
                            ui.label(
                                RichText::new(&self.pin_labels[pin_idx as usize])
                                    .size(10.0)
                                    .color(Color32::GRAY),
                            );

                            // Tooltip with details
                            response.on_hover_ui(|ui| {
                                ui.label(format!("Pin: {}", self.pin_labels[pin_idx as usize]));
                                ui.label(format!(
                                    "State: {}",
                                    if pin.state { "HIGH" } else { "LOW" }
                                ));
                                ui.label(format!("Rising edges: {}", pin.rising_edges));
                                ui.label(format!("Falling edges: {}", pin.falling_edges));
                                if let Some(pw) = pin.last_pulse_width_ms {
                                    ui.label(format!("Last pulse: {:.2} ms", pw));
                                }
                            });
                        });
                    }
                    ui.end_row();
                }
            });
    }

    /// Render table view
    fn render_table_view(&self, ui: &mut Ui) {
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                egui::Grid::new("dio_table")
                    .num_columns(if self.show_edge_counts { 6 } else { 3 })
                    .striped(true)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        // Header
                        ui.label(RichText::new("Pin").strong());
                        ui.label(RichText::new("State").strong());
                        ui.label(RichText::new("Last Change").strong());
                        if self.show_edge_counts {
                            ui.label(RichText::new("Rising").strong());
                            ui.label(RichText::new("Falling").strong());
                        }
                        if self.show_pulse_widths {
                            ui.label(RichText::new("Pulse Width").strong());
                        }
                        ui.end_row();

                        // Data rows
                        for i in 0..self.n_pins as usize {
                            let pin = &self.pins[i];

                            ui.label(&self.pin_labels[i]);

                            // State with color
                            let state_text = if pin.state { "HIGH" } else { "LOW" };
                            let state_color = if pin.state {
                                self.color_high
                            } else {
                                Color32::GRAY
                            };
                            ui.label(RichText::new(state_text).color(state_color));

                            // Last change
                            let ms = pin.ms_since_change();
                            let change_text = if ms < 1000.0 {
                                format!("{:.0} ms ago", ms)
                            } else if ms < 60000.0 {
                                format!("{:.1} s ago", ms / 1000.0)
                            } else {
                                format!("{:.1} min ago", ms / 60000.0)
                            };
                            ui.label(change_text);

                            if self.show_edge_counts {
                                ui.label(pin.rising_edges.to_string());
                                ui.label(pin.falling_edges.to_string());
                            }

                            if self.show_pulse_widths {
                                if let Some(pw) = pin.last_pulse_width_ms {
                                    ui.label(format!("{:.2} ms", pw));
                                } else {
                                    ui.label("-");
                                }
                            }

                            ui.end_row();
                        }
                    });
            });
    }

    /// Render timing diagram view
    fn render_timing_view(&self, ui: &mut Ui) {
        let now = Instant::now();
        let time_window = 1.0; // 1 second window

        ui.group(|ui| {
            ui.label(RichText::new("Timing Diagram (1s window)").strong());

            let available_width = ui.available_width() - 80.0;
            let row_height = 20.0;

            for i in 0..self.n_pins.min(8) as usize {
                // Show first 8 pins
                let pin = &self.pins[i];

                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.pin_labels[i]).monospace().size(10.0));

                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(available_width, row_height),
                        egui::Sense::hover(),
                    );

                    // Background
                    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(30));

                    // Draw waveform from history
                    let mut prev_x = rect.min.x;
                    let mut prev_state = false;

                    for (time, state) in &pin.history {
                        let age = now.duration_since(*time).as_secs_f64();
                        if age > time_window {
                            prev_state = *state;
                            continue;
                        }

                        let x = rect.max.x - (age / time_window * available_width as f64) as f32;

                        // Draw horizontal line at previous level
                        let prev_y = if prev_state {
                            rect.min.y + 4.0
                        } else {
                            rect.max.y - 4.0
                        };
                        let curr_y = if *state {
                            rect.min.y + 4.0
                        } else {
                            rect.max.y - 4.0
                        };

                        if prev_x < x {
                            ui.painter().line_segment(
                                [egui::pos2(prev_x, prev_y), egui::pos2(x, prev_y)],
                                egui::Stroke::new(1.5, self.color_high),
                            );

                            // Vertical transition
                            if prev_state != *state {
                                ui.painter().line_segment(
                                    [egui::pos2(x, prev_y), egui::pos2(x, curr_y)],
                                    egui::Stroke::new(1.5, self.color_high),
                                );
                            }
                        }

                        prev_x = x;
                        prev_state = *state;
                    }

                    // Draw line to current time
                    let curr_y = if pin.state {
                        rect.min.y + 4.0
                    } else {
                        rect.max.y - 4.0
                    };
                    ui.painter().line_segment(
                        [egui::pos2(prev_x, curr_y), egui::pos2(rect.max.x, curr_y)],
                        egui::Stroke::new(1.5, self.color_high),
                    );
                });
            }

            if self.n_pins > 8 {
                ui.label(
                    RichText::new(format!("(showing 8 of {} pins)", self.n_pins))
                        .size(10.0)
                        .color(Color32::GRAY),
                );
            }
        });
    }
}
