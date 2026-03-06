//! Counter Value Display Panel for Comedi DAQ devices.
//!
//! Provides large numeric display of counter values with rate calculation,
//! frequency display, and totalizer features.

use crate::time::Instant;
use eframe::egui::{self, Color32, RichText, Ui};
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// Maximum history for rate calculation
const RATE_HISTORY_SIZE: usize = 50;

/// Counter update message
#[derive(Debug, Clone)]
pub struct CounterUpdate {
    pub counter: u32,
    pub count: u64,
    pub timestamp: Option<f64>,
}

impl CounterUpdate {
    pub fn new(counter: u32, count: u64) -> Self {
        Self {
            counter,
            count,
            timestamp: None,
        }
    }
}

/// Sender for counter updates
pub type CounterDisplaySender = mpsc::Sender<CounterUpdate>;
/// Receiver for counter updates
pub type CounterDisplayReceiver = mpsc::Receiver<CounterUpdate>;

/// Create channel pair for counter display
pub fn counter_display_channel() -> (CounterDisplaySender, CounterDisplayReceiver) {
    mpsc::channel(256)
}

/// Display format for counter values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CounterDisplayFormat {
    #[default]
    Decimal,
    Hexadecimal,
    Binary,
    Frequency,
    Period,
}

impl CounterDisplayFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Decimal => "Decimal",
            Self::Hexadecimal => "Hex",
            Self::Binary => "Binary",
            Self::Frequency => "Frequency",
            Self::Period => "Period",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Decimal,
            Self::Hexadecimal,
            Self::Binary,
            Self::Frequency,
            Self::Period,
        ]
    }
}

/// Per-counter state
#[derive(Debug, Clone)]
struct CounterState {
    count: u64,
    last_count: u64,
    last_update: Instant,
    rate_history: VecDeque<(Instant, u64)>,
    total: u64,
    start_count: u64,
    #[allow(dead_code)] // bd-opn6: used when arm/disarm gRPC is wired (Phase 4)
    running: bool,
    overflow_count: u64,
}

impl Default for CounterState {
    fn default() -> Self {
        Self {
            count: 0,
            last_count: 0,
            last_update: Instant::now(),
            rate_history: VecDeque::with_capacity(RATE_HISTORY_SIZE),
            total: 0,
            start_count: 0,
            running: true,
            overflow_count: 0,
        }
    }
}

impl CounterState {
    fn update(&mut self, new_count: u64) {
        let now = Instant::now();

        // Detect overflow (count wrapped around)
        if new_count < self.count && self.count > 0x80000000 {
            self.overflow_count += 1;
        }

        // Calculate delta
        let delta = if new_count >= self.last_count {
            new_count - self.last_count
        } else {
            // Overflow occurred
            (u64::MAX - self.last_count) + new_count + 1
        };

        self.total += delta;
        self.last_count = self.count;
        self.count = new_count;
        self.last_update = now;

        // Add to rate history
        self.rate_history.push_back((now, new_count));
        while self.rate_history.len() > RATE_HISTORY_SIZE {
            self.rate_history.pop_front();
        }
    }

    fn reset(&mut self) {
        self.start_count = self.count;
        self.total = 0;
        self.overflow_count = 0;
        self.rate_history.clear();
    }

    /// Calculate rate (counts per second)
    fn rate(&self) -> f64 {
        if self.rate_history.len() < 2 {
            return 0.0;
        }

        let (Some(first), Some(last)) = (self.rate_history.front(), self.rate_history.back())
        else {
            return 0.0;
        };

        let dt = last.0.duration_since(first.0).as_secs_f64();
        if dt < 0.001 {
            return 0.0;
        }

        let delta_count = if last.1 >= first.1 {
            last.1 - first.1
        } else {
            // Overflow
            (u64::MAX - first.1) + last.1 + 1
        };

        #[allow(clippy::cast_precision_loss)]
        let rate = delta_count as f64 / dt;
        rate
    }

    /// Time since last update in seconds
    fn age_secs(&self) -> f64 {
        self.last_update.elapsed().as_secs_f64()
    }
}

/// Counter Value Display Panel
pub struct CounterDisplayPanel {
    #[allow(dead_code)] // bd-opn6: used for elapsed-time display in Phase 4
    /// Start time
    start_time: Instant,
    /// Counter states
    counters: Vec<CounterState>,
    /// Number of counters
    n_counters: u32,
    /// Update receiver
    update_rx: CounterDisplayReceiver,
    /// Update sender (for cloning)
    update_tx: CounterDisplaySender,
    /// Display format
    format: CounterDisplayFormat,
    /// Selected counter for large display
    selected_counter: u32,
    /// Show all counters
    show_all: bool,
    /// Show rate
    show_rate: bool,
    /// Show totalizer
    show_total: bool,
    /// Frozen
    frozen: bool,
}

impl Default for CounterDisplayPanel {
    fn default() -> Self {
        let (tx, rx) = counter_display_channel();

        Self {
            start_time: Instant::now(),
            counters: (0..8).map(|_| CounterState::default()).collect(),
            n_counters: 3,
            update_rx: rx,
            update_tx: tx,
            format: CounterDisplayFormat::Decimal,
            selected_counter: 0,
            show_all: true,
            show_rate: true,
            show_total: true,
            frozen: false,
        }
    }
}

impl CounterDisplayPanel {
    /// Create a new counter display panel
    pub fn new(n_counters: u32) -> Self {
        Self {
            n_counters: n_counters.min(8),
            ..Self::default()
        }
    }

    /// Get sender for pushing updates
    pub fn get_sender(&self) -> CounterDisplaySender {
        self.update_tx.clone()
    }

    /// Drain pending updates
    fn drain_updates(&mut self) {
        while let Ok(update) = self.update_rx.try_recv() {
            if self.frozen {
                continue;
            }

            if let Some(counter) = self.counters.get_mut(update.counter as usize) {
                counter.update(update.count);
            }
        }
    }

    /// Format a count value
    fn format_count(&self, count: u64) -> String {
        match self.format {
            CounterDisplayFormat::Decimal => {
                // Add thousands separators
                let s = count.to_string();
                let mut result = String::new();
                for (i, c) in s.chars().rev().enumerate() {
                    if i > 0 && i % 3 == 0 {
                        result.insert(0, ',');
                    }
                    result.insert(0, c);
                }
                result
            }
            CounterDisplayFormat::Hexadecimal => format!("0x{:08X}", count),
            CounterDisplayFormat::Binary => {
                if count <= 0xFFFF {
                    format!("0b{:016b}", count)
                } else {
                    format!("0b{:032b}", count & 0xFFFFFFFF)
                }
            }
            CounterDisplayFormat::Frequency | CounterDisplayFormat::Period => {
                // These use rate, not raw count
                let rate = self.counters[self.selected_counter as usize].rate();
                if self.format == CounterDisplayFormat::Frequency {
                    Self::format_frequency(rate)
                } else if rate > 0.0 {
                    Self::format_period(1.0 / rate)
                } else {
                    "---".to_string()
                }
            }
        }
    }

    /// Format frequency with appropriate unit
    fn format_frequency(hz: f64) -> String {
        if hz >= 1_000_000.0 {
            format!("{:.3} MHz", hz / 1_000_000.0)
        } else if hz >= 1000.0 {
            format!("{:.3} kHz", hz / 1000.0)
        } else {
            format!("{:.3} Hz", hz)
        }
    }

    /// Format period with appropriate unit
    fn format_period(secs: f64) -> String {
        if secs < 0.000001 {
            format!("{:.3} ns", secs * 1_000_000_000.0)
        } else if secs < 0.001 {
            format!("{:.3} us", secs * 1_000_000.0)
        } else if secs < 1.0 {
            format!("{:.3} ms", secs * 1000.0)
        } else {
            format!("{:.3} s", secs)
        }
    }

    /// Format rate with appropriate unit
    fn format_rate(rate: f64) -> String {
        if rate >= 1_000_000.0 {
            format!("{:.2} M/s", rate / 1_000_000.0)
        } else if rate >= 1000.0 {
            format!("{:.2} k/s", rate / 1000.0)
        } else {
            format!("{:.2} /s", rate)
        }
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        // Drain updates
        self.drain_updates();

        // Header
        ui.horizontal(|ui| {
            ui.heading("Counter Display");
            ui.separator();
            ui.label(format!("{} counters", self.n_counters));

            ui.separator();

            // Freeze toggle
            let freeze_text = if self.frozen { "Unfreeze" } else { "Freeze" };
            if ui.button(freeze_text).clicked() {
                self.frozen = !self.frozen;
            }

            // Reset selected counter
            if ui.button("Reset").clicked() {
                if let Some(counter) = self.counters.get_mut(self.selected_counter as usize) {
                    counter.reset();
                }
            }

            // Reset all
            if ui.button("Reset All").clicked() {
                for counter in &mut self.counters {
                    counter.reset();
                }
            }
        });

        ui.separator();

        // Control bar
        ui.horizontal(|ui| {
            // Format selector
            ui.label("Format:");
            egui::ComboBox::from_id_salt("ctr_format")
                .selected_text(self.format.label())
                .width(80.0)
                .show_ui(ui, |ui| {
                    for fmt in CounterDisplayFormat::all() {
                        ui.selectable_value(&mut self.format, *fmt, fmt.label());
                    }
                });

            ui.separator();

            // Counter selector
            ui.label("Counter:");
            for i in 0..self.n_counters {
                if ui
                    .selectable_label(self.selected_counter == i, format!("CTR{}", i))
                    .clicked()
                {
                    self.selected_counter = i;
                }
            }

            ui.separator();

            ui.checkbox(&mut self.show_rate, "Rate");
            ui.checkbox(&mut self.show_total, "Total");
            ui.checkbox(&mut self.show_all, "All Counters");
        });

        ui.separator();

        // Main display
        self.render_main_display(ui);

        if self.show_all {
            ui.separator();
            self.render_all_counters(ui);
        }

        // Request repaint
        if !self.frozen {
            ui.ctx().request_repaint();
        }
    }

    /// Render the main large display
    fn render_main_display(&self, ui: &mut Ui) {
        let counter = &self.counters[self.selected_counter as usize];

        ui.vertical_centered(|ui| {
            // Counter label
            ui.label(
                RichText::new(format!("Counter {}", self.selected_counter))
                    .size(16.0)
                    .color(Color32::GRAY),
            );

            // Main value
            let display_value = self.format_count(counter.count);
            let text_size = if display_value.len() > 20 { 32.0 } else { 48.0 };

            ui.label(
                RichText::new(&display_value)
                    .size(text_size)
                    .monospace()
                    .color(if self.frozen {
                        Color32::YELLOW
                    } else {
                        Color32::WHITE
                    }),
            );

            // Rate display
            if self.show_rate {
                let rate = counter.rate();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Rate:").color(Color32::GRAY));
                    ui.label(
                        RichText::new(Self::format_rate(rate))
                            .size(20.0)
                            .monospace()
                            .color(Color32::LIGHT_BLUE),
                    );
                });
            }

            // Total display
            if self.show_total {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Total:").color(Color32::GRAY));
                    ui.label(
                        RichText::new(format!("{}", counter.total))
                            .size(16.0)
                            .monospace()
                            .color(Color32::LIGHT_GREEN),
                    );

                    if counter.overflow_count > 0 {
                        ui.label(
                            RichText::new(format!("({} overflows)", counter.overflow_count))
                                .size(12.0)
                                .color(Color32::YELLOW),
                        );
                    }
                });
            }

            // Age indicator
            let age = counter.age_secs();
            let age_text = if age < 1.0 {
                format!("{:.0} ms ago", age * 1000.0)
            } else {
                format!("{:.1} s ago", age)
            };
            ui.label(RichText::new(age_text).size(10.0).color(if age > 1.0 {
                Color32::YELLOW
            } else {
                Color32::GRAY
            }));
        });
    }

    /// Render all counters summary
    fn render_all_counters(&self, ui: &mut Ui) {
        egui::Grid::new("all_counters")
            .num_columns(5)
            .striped(true)
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                // Header
                ui.label(RichText::new("Counter").strong());
                ui.label(RichText::new("Count").strong());
                ui.label(RichText::new("Rate").strong());
                ui.label(RichText::new("Total").strong());
                ui.label(RichText::new("Age").strong());
                ui.end_row();

                for i in 0..self.n_counters as usize {
                    let counter = &self.counters[i];
                    #[allow(clippy::cast_possible_truncation)]
                    let is_selected = self.selected_counter == i as u32;

                    // Name
                    let name_text = RichText::new(format!("CTR{}", i));
                    ui.label(if is_selected {
                        name_text.strong().color(Color32::LIGHT_BLUE)
                    } else {
                        name_text
                    });

                    // Count
                    ui.label(RichText::new(counter.count.to_string()).monospace());

                    // Rate
                    ui.label(RichText::new(Self::format_rate(counter.rate())).monospace());

                    // Total
                    ui.label(RichText::new(counter.total.to_string()).monospace());

                    // Age
                    let age = counter.age_secs();
                    let age_text = if age < 1.0 {
                        format!("{:.0}ms", age * 1000.0)
                    } else {
                        format!("{:.1}s", age)
                    };
                    ui.label(RichText::new(age_text).color(if age > 1.0 {
                        Color32::YELLOW
                    } else {
                        Color32::GRAY
                    }));

                    ui.end_row();
                }
            });
    }
}
