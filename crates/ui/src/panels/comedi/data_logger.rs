//! Data Logger View Panel for Comedi DAQ devices.
//!
//! Provides scrolling tabular display of acquired data with timestamps,
//! filtering, and export capabilities.

use crate::time::Instant;
use eframe::egui::{self, Color32, RichText, Ui};
use std::collections::VecDeque;
use std::fmt::Write;
use tokio::sync::mpsc;

/// Maximum rows in the log buffer
const MAX_LOG_ROWS: usize = 10_000;

/// A logged data point
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: f64,
    pub channel: String,
    pub value: f64,
    pub unit: String,
}

impl LogEntry {
    pub fn new(
        timestamp: f64,
        channel: impl Into<String>,
        value: f64,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            timestamp,
            channel: channel.into(),
            value,
            unit: unit.into(),
        }
    }

    pub fn voltage(timestamp: f64, channel: u32, voltage: f64) -> Self {
        Self {
            timestamp,
            channel: format!("AI{}", channel),
            value: voltage,
            unit: "V".to_string(),
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn counter(timestamp: f64, counter: u32, count: u64) -> Self {
        Self {
            timestamp,
            channel: format!("CTR{}", counter),
            value: count as f64,
            unit: "counts".to_string(),
        }
    }

    pub fn digital(timestamp: f64, pin: u32, state: bool) -> Self {
        Self {
            timestamp,
            channel: format!("DIO{}", pin),
            value: if state { 1.0 } else { 0.0 },
            unit: String::new(),
        }
    }
}

/// Sender for log entries
pub type DataLoggerSender = mpsc::Sender<LogEntry>;
/// Receiver for log entries
pub type DataLoggerReceiver = mpsc::Receiver<LogEntry>;

/// Create channel pair for data logger
pub fn data_logger_channel() -> (DataLoggerSender, DataLoggerReceiver) {
    mpsc::channel(4096)
}

/// Column configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFormat {
    Timestamp,
    Channel,
    Value,
    Unit,
}

/// Data Logger View Panel
pub struct DataLoggerPanel {
    /// Start time for relative timestamps
    start_time: Instant,
    /// Log buffer
    log_buffer: VecDeque<LogEntry>,
    /// Entry receiver
    entry_rx: DataLoggerReceiver,
    /// Entry sender (for cloning)
    entry_tx: DataLoggerSender,
    /// Auto-scroll enabled
    auto_scroll: bool,
    /// Logging enabled
    logging_enabled: bool,
    /// Channel filter (empty = all)
    channel_filter: String,
    /// Show absolute timestamps
    absolute_timestamps: bool,
    /// Decimal places for values
    decimals: usize,
    /// Rows per page
    rows_per_page: usize,
    /// Current page
    current_page: usize,
    /// Total entries received
    total_entries: u64,
    /// Entries dropped (buffer overflow)
    dropped_entries: u64,
    /// Statistics
    stats: LogStatistics,
    /// Show statistics panel
    show_stats: bool,
    /// Export path
    export_path: String,
    /// Export status message
    export_status: Option<String>,
}

/// Statistics for logged data
#[derive(Debug, Clone)]
struct LogStatistics {
    entry_rate: f64,
    last_rate_calc: Instant,
    entries_since_calc: u64,
}

impl Default for LogStatistics {
    fn default() -> Self {
        Self {
            entry_rate: 0.0,
            last_rate_calc: Instant::now(),
            entries_since_calc: 0,
        }
    }
}

impl LogStatistics {
    #[allow(clippy::cast_precision_loss)]
    fn update(&mut self) {
        self.entries_since_calc += 1;

        let elapsed = self.last_rate_calc.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.entry_rate = self.entries_since_calc as f64 / elapsed;
            self.entries_since_calc = 0;
            self.last_rate_calc = Instant::now();
        }
    }
}

impl Default for DataLoggerPanel {
    fn default() -> Self {
        let (tx, rx) = data_logger_channel();

        Self {
            start_time: Instant::now(),
            log_buffer: VecDeque::with_capacity(MAX_LOG_ROWS),
            entry_rx: rx,
            entry_tx: tx,
            auto_scroll: true,
            logging_enabled: true,
            channel_filter: String::new(),
            absolute_timestamps: false,
            decimals: 4,
            rows_per_page: 100,
            current_page: 0,
            total_entries: 0,
            dropped_entries: 0,
            stats: LogStatistics::default(),
            show_stats: true,
            export_path: String::from("data_log.csv"),
            export_status: None,
        }
    }
}

impl DataLoggerPanel {
    /// Create a new data logger panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Get sender for pushing log entries
    pub fn get_sender(&self) -> DataLoggerSender {
        self.entry_tx.clone()
    }

    /// Drain pending log entries
    fn drain_entries(&mut self) {
        while let Ok(entry) = self.entry_rx.try_recv() {
            if !self.logging_enabled {
                continue;
            }

            self.total_entries += 1;
            self.stats.update();

            // Add to buffer
            self.log_buffer.push_back(entry);

            // Trim buffer if too large
            while self.log_buffer.len() > MAX_LOG_ROWS {
                self.log_buffer.pop_front();
                self.dropped_entries += 1;
            }
        }

        // Auto-scroll to last page
        if self.auto_scroll && !self.log_buffer.is_empty() {
            let filtered = self.filtered_entries();
            if !filtered.is_empty() {
                self.current_page = (filtered.len().saturating_sub(1)) / self.rows_per_page;
            }
        }
    }

    /// Get filtered entries
    fn filtered_entries(&self) -> Vec<&LogEntry> {
        if self.channel_filter.is_empty() {
            self.log_buffer.iter().collect()
        } else {
            let filter_lower = self.channel_filter.to_lowercase();
            self.log_buffer
                .iter()
                .filter(|e| e.channel.to_lowercase().contains(&filter_lower))
                .collect()
        }
    }

    /// Clear the log buffer
    pub fn clear(&mut self) {
        self.log_buffer.clear();
        self.total_entries = 0;
        self.dropped_entries = 0;
        self.current_page = 0;
    }

    /// Export log to CSV
    fn export_csv(&self) -> Result<String, String> {
        let mut csv = String::from("Timestamp,Channel,Value,Unit\n");

        for entry in &self.log_buffer {
            writeln!(
                csv,
                "{:.6},{},{:.decimals$},{}",
                entry.timestamp,
                entry.channel,
                entry.value,
                entry.unit,
                decimals = self.decimals
            )
            .unwrap();
        }

        Ok(csv)
    }

    /// Format timestamp for display
    fn format_timestamp(&self, ts: f64) -> String {
        if self.absolute_timestamps {
            // Format as absolute time (would need actual wall clock)
            format!("{:.3}s", ts)
        } else {
            // Relative to start
            if ts < 60.0 {
                format!("{:.3}s", ts)
            } else if ts < 3600.0 {
                let mins = (ts / 60.0).floor();
                let secs = ts % 60.0;
                format!("{:.0}m {:.1}s", mins, secs)
            } else {
                let hours = (ts / 3600.0).floor();
                let mins = ((ts % 3600.0) / 60.0).floor();
                format!("{:.0}h {:.0}m", hours, mins)
            }
        }
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        // Drain pending entries
        self.drain_entries();

        // Header
        ui.horizontal(|ui| {
            ui.heading("Data Logger");
            ui.separator();

            // Logging toggle
            let log_text = if self.logging_enabled {
                "Logging"
            } else {
                "Paused"
            };
            if ui
                .selectable_label(self.logging_enabled, log_text)
                .clicked()
            {
                self.logging_enabled = !self.logging_enabled;
            }

            ui.separator();

            // Entry count
            ui.label(format!(
                "{} entries ({} dropped)",
                self.total_entries, self.dropped_entries
            ));

            if self.stats.entry_rate > 0.0 {
                ui.label(format!("| {:.1}/s", self.stats.entry_rate));
            }
        });

        ui.separator();

        // Control bar
        ui.horizontal(|ui| {
            // Channel filter
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.channel_filter)
                    .hint_text("Channel name...")
                    .desired_width(100.0),
            );

            if ui.button("Clear Filter").clicked() {
                self.channel_filter.clear();
            }

            ui.separator();

            // Auto-scroll
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");

            ui.separator();

            // Clear
            if ui.button("Clear Log").clicked() {
                self.clear();
            }

            // Export
            if ui.button("Export CSV").clicked() {
                match self.export_csv() {
                    Ok(csv) => {
                        // In a real app, would write to file
                        self.export_status = Some(format!(
                            "Generated {} bytes ({} rows)",
                            csv.len(),
                            self.log_buffer.len()
                        ));
                    }
                    Err(e) => {
                        self.export_status = Some(format!("Export error: {}", e));
                    }
                }
            }
        });

        // Export status
        if let Some(status) = &self.export_status {
            ui.label(RichText::new(status).color(Color32::GREEN));
        }

        ui.separator();

        // Data table
        self.render_table(ui);

        // Pagination
        ui.separator();
        self.render_pagination(ui);

        // Statistics
        if self.show_stats {
            ui.separator();
            self.render_statistics(ui);
        }

        // Request repaint for live updates
        if self.logging_enabled {
            ui.ctx().request_repaint();
        }
    }

    /// Render the data table
    fn render_table(&self, ui: &mut Ui) {
        let filtered = self.filtered_entries();
        let total_rows = filtered.len();
        let total_pages = (total_rows.saturating_sub(1)) / self.rows_per_page + 1;
        let current_page = self.current_page.min(total_pages.saturating_sub(1));

        let start_row = current_page * self.rows_per_page;
        let end_row = (start_row + self.rows_per_page).min(total_rows);

        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                egui::Grid::new("data_log_table")
                    .num_columns(4)
                    .striped(true)
                    .spacing([20.0, 2.0])
                    .show(ui, |ui| {
                        // Header
                        ui.label(RichText::new("Timestamp").strong());
                        ui.label(RichText::new("Channel").strong());
                        ui.label(RichText::new("Value").strong());
                        ui.label(RichText::new("Unit").strong());
                        ui.end_row();

                        // Data rows
                        for entry in filtered.iter().skip(start_row).take(end_row - start_row) {
                            ui.label(
                                RichText::new(self.format_timestamp(entry.timestamp))
                                    .monospace()
                                    .size(11.0),
                            );
                            ui.label(
                                RichText::new(&entry.channel)
                                    .monospace()
                                    .color(Self::channel_color(&entry.channel)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{:.decimals$}",
                                    entry.value,
                                    decimals = self.decimals
                                ))
                                .monospace(),
                            );
                            ui.label(RichText::new(&entry.unit).size(11.0));
                            ui.end_row();
                        }
                    });
            });
    }

    /// Get color for channel name
    fn channel_color(channel: &str) -> Color32 {
        if channel.starts_with("AI") {
            Color32::from_rgb(100, 200, 255)
        } else if channel.starts_with("AO") {
            Color32::from_rgb(255, 200, 100)
        } else if channel.starts_with("DIO") {
            Color32::from_rgb(100, 255, 150)
        } else if channel.starts_with("CTR") {
            Color32::from_rgb(255, 150, 200)
        } else {
            Color32::GRAY
        }
    }

    /// Render pagination controls
    fn render_pagination(&mut self, ui: &mut Ui) {
        let filtered = self.filtered_entries();
        let total_rows = filtered.len();
        let total_pages = (total_rows.saturating_sub(1)) / self.rows_per_page + 1;

        ui.horizontal(|ui| {
            // Previous page
            if ui
                .add_enabled(self.current_page > 0, egui::Button::new("<< First"))
                .clicked()
            {
                self.current_page = 0;
                self.auto_scroll = false;
            }

            if ui
                .add_enabled(self.current_page > 0, egui::Button::new("< Prev"))
                .clicked()
            {
                self.current_page = self.current_page.saturating_sub(1);
                self.auto_scroll = false;
            }

            // Page indicator
            ui.label(format!(
                "Page {} of {} ({} rows)",
                self.current_page + 1,
                total_pages,
                total_rows
            ));

            // Next page
            if ui
                .add_enabled(
                    self.current_page < total_pages.saturating_sub(1),
                    egui::Button::new("Next >"),
                )
                .clicked()
            {
                self.current_page += 1;
                self.auto_scroll = false;
            }

            if ui
                .add_enabled(
                    self.current_page < total_pages.saturating_sub(1),
                    egui::Button::new("Last >>"),
                )
                .clicked()
            {
                self.current_page = total_pages.saturating_sub(1);
                self.auto_scroll = true;
            }

            ui.separator();

            // Rows per page
            ui.label("Rows/page:");
            egui::ComboBox::from_id_salt("rows_per_page")
                .selected_text(format!("{}", self.rows_per_page))
                .width(60.0)
                .show_ui(ui, |ui| {
                    for n in [25, 50, 100, 250, 500] {
                        ui.selectable_value(&mut self.rows_per_page, n, format!("{}", n));
                    }
                });
        });
    }

    /// Render statistics panel
    fn render_statistics(&self, ui: &mut Ui) {
        let filtered = self.filtered_entries();

        // Calculate basic stats per channel
        let mut channel_stats: std::collections::HashMap<String, (f64, f64, f64, usize)> =
            std::collections::HashMap::new();

        for entry in &filtered {
            let stat = channel_stats.entry(entry.channel.clone()).or_insert((
                f64::INFINITY,
                f64::NEG_INFINITY,
                0.0,
                0,
            ));

            stat.0 = stat.0.min(entry.value);
            stat.1 = stat.1.max(entry.value);
            stat.2 += entry.value;
            stat.3 += 1;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Statistics").strong());
            ui.checkbox(&mut self.show_stats.clone(), "Show"); // Note: needs mutable
        });

        if channel_stats.is_empty() {
            ui.label("No data");
            return;
        }

        egui::Grid::new("log_stats")
            .num_columns(5)
            .striped(true)
            .spacing([15.0, 2.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Channel").strong());
                ui.label(RichText::new("Count").strong());
                ui.label(RichText::new("Min").strong());
                ui.label(RichText::new("Max").strong());
                ui.label(RichText::new("Mean").strong());
                ui.end_row();

                let mut channels: Vec<_> = channel_stats.iter().collect();
                channels.sort_by(|a, b| a.0.cmp(b.0));

                for (channel, (min, max, sum, count)) in channels {
                    #[allow(clippy::cast_precision_loss)]
                    let mean = sum / *count as f64;

                    ui.label(RichText::new(channel).color(Self::channel_color(channel)));
                    ui.label(format!("{}", count));
                    ui.label(format!("{:.4}", min));
                    ui.label(format!("{:.4}", max));
                    ui.label(format!("{:.4}", mean));
                    ui.end_row();
                }
            });
    }
}
