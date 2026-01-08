//! Logging & Status Panel for scientific experiment monitoring
//!
//! Provides real-time log viewing with:
//! - Log level filtering (Error, Warn, Info, Debug, Trace)
//! - Timestamped entries with source identification
//! - Status indicators for connection, system, and experiment state
//! - Log export to text file
//! - Auto-scroll with pause capability
//! - Text search filtering

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Instant;

use eframe::egui;

/// Maximum number of log entries to keep in memory
const MAX_LOG_ENTRIES: usize = 10_000;

/// Case-insensitive ASCII substring search without allocation (bd-tjwm.4)
///
/// Returns true if `haystack` contains `needle` (case-insensitive).
/// Only handles ASCII characters; non-ASCII bytes are compared exactly.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }

    let needle_bytes = needle.as_bytes();
    let haystack_bytes = haystack.as_bytes();

    'outer: for start in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        for (i, &nb) in needle_bytes.iter().enumerate() {
            let hb = haystack_bytes[start + i];
            if !hb.eq_ignore_ascii_case(&nb) {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

/// Log severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl LogLevel {
    /// Get display label for the level
    pub fn label(&self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    /// Get short label for compact display (for compact log formats)
    #[allow(dead_code)]
    pub fn short_label(&self) -> &'static str {
        match self {
            Self::Error => "E",
            Self::Warn => "W",
            Self::Info => "I",
            Self::Debug => "D",
            Self::Trace => "T",
        }
    }

    /// Get color for the level
    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Error => egui::Color32::from_rgb(255, 100, 100), // Red
            Self::Warn => egui::Color32::from_rgb(255, 200, 100),  // Orange/Yellow
            Self::Info => egui::Color32::from_rgb(100, 200, 255),  // Light Blue
            Self::Debug => egui::Color32::from_rgb(180, 180, 180), // Gray
            Self::Trace => egui::Color32::from_rgb(140, 140, 140), // Dark Gray
        }
    }

    /// All log levels for iteration
    pub fn all() -> &'static [LogLevel] {
        &[
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ]
    }
}

/// Log category for filtering by subsystem
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LogCategory {
    /// Show all log entries
    #[default]
    All,
    /// Daemon connection and health checks
    Connection,
    /// Device operations, hardware control
    Devices,
    /// Frame streaming, image viewer
    Streaming,
    /// Scan operations
    Scans,
    /// Data storage operations
    Storage,
    /// Script execution
    Scripts,
    /// Module system
    Modules,
    /// General system/GUI events
    System,
}

impl LogCategory {
    /// Get display label for the category
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Connection => "Connection",
            Self::Devices => "Devices",
            Self::Streaming => "Streaming",
            Self::Scans => "Scans",
            Self::Storage => "Storage",
            Self::Scripts => "Scripts",
            Self::Modules => "Modules",
            Self::System => "System",
        }
    }

    /// Get color for the category (for visual distinction)
    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::All => egui::Color32::WHITE,
            Self::Connection => egui::Color32::from_rgb(100, 200, 255),  // Blue
            Self::Devices => egui::Color32::from_rgb(255, 180, 100),     // Orange
            Self::Streaming => egui::Color32::from_rgb(180, 100, 255),   // Purple
            Self::Scans => egui::Color32::from_rgb(100, 255, 180),       // Cyan
            Self::Storage => egui::Color32::from_rgb(255, 255, 100),     // Yellow
            Self::Scripts => egui::Color32::from_rgb(255, 100, 180),     // Pink
            Self::Modules => egui::Color32::from_rgb(180, 255, 100),     // Lime
            Self::System => egui::Color32::from_rgb(180, 180, 180),      // Gray
        }
    }

    /// All categories for iteration (excluding All)
    pub fn all_categories() -> &'static [LogCategory] {
        &[
            LogCategory::All,
            LogCategory::Connection,
            LogCategory::Devices,
            LogCategory::Streaming,
            LogCategory::Scans,
            LogCategory::Storage,
            LogCategory::Scripts,
            LogCategory::Modules,
            LogCategory::System,
        ]
    }

    /// Determine category from log source name
    pub fn from_source(source: &str) -> Self {
        let source_lower = source.to_lowercase();

        // Connection-related
        if source_lower.contains("connection")
            || source_lower.contains("reconnect")
            || source_lower.contains("health")
        {
            return Self::Connection;
        }

        // Streaming-related (image viewer, signal plotter)
        if source_lower.contains("image_viewer")
            || source_lower.contains("signal_plotter")
            || source_lower.contains("stream")
            || source_lower.contains("frame")
        {
            return Self::Streaming;
        }

        // Device-related
        if source_lower.contains("device")
            || source_lower.contains("hardware")
            || source_lower.contains("instrument")
            || source_lower.contains("driver")
        {
            return Self::Devices;
        }

        // Scan-related
        if source_lower.contains("scan") {
            return Self::Scans;
        }

        // Storage-related
        if source_lower.contains("storage")
            || source_lower.contains("hdf5")
            || source_lower.contains("ring_buffer")
        {
            return Self::Storage;
        }

        // Script-related
        if source_lower.contains("script") || source_lower.contains("rhai") {
            return Self::Scripts;
        }

        // Module-related
        if source_lower.contains("module") {
            return Self::Modules;
        }

        // Default to System
        Self::System
    }
}

/// A single log entry
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Entry ID for stable UI identification (for future row virtualization)
    #[allow(dead_code)]
    pub id: u64,
    /// Timestamp relative to panel start
    pub timestamp_secs: f64,
    /// Severity level
    pub level: LogLevel,
    /// Log category (subsystem)
    pub category: LogCategory,
    /// Source module/component
    pub source: String,
    /// Log message
    pub message: String,
}

impl LogEntry {
    /// Create a new log entry (auto-assigns category from source)
    pub fn new(id: u64, timestamp_secs: f64, level: LogLevel, source: &str, message: &str) -> Self {
        let category = LogCategory::from_source(source);
        Self {
            id,
            timestamp_secs,
            level,
            category,
            source: source.to_string(),
            message: message.to_string(),
        }
    }

    /// Format timestamp as HH:MM:SS.mmm
    pub fn formatted_timestamp(&self) -> String {
        let total_secs = self.timestamp_secs;
        let hours = (total_secs / 3600.0) as u32;
        let mins = ((total_secs % 3600.0) / 60.0) as u32;
        let secs = (total_secs % 60.0) as u32;
        let millis = ((total_secs % 1.0) * 1000.0) as u32;
        format!("{:02}:{:02}:{:02}.{:03}", hours, mins, secs, millis)
    }

    /// Format for export
    pub fn to_export_line(&self) -> String {
        format!(
            "[{}] {} [{}] [{}] {}",
            self.formatted_timestamp(),
            self.level.label(),
            self.category.label(),
            self.source,
            self.message
        )
    }
}

/// Connection status indicator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Connection diagnostics for the logging panel (bd-j3xz.3.3).
///
/// Captures connection health metrics for UI display.
#[derive(Debug, Clone, Default)]
pub struct ConnectionDiagnostics {
    /// RTT of last successful health check in milliseconds.
    pub last_rtt_ms: Option<f64>,
    /// Total number of errors since connection established.
    pub total_errors: u32,
    /// Seconds since last error occurred (None if no errors).
    pub secs_since_last_error: Option<f64>,
    /// The last error message.
    pub last_error_message: Option<String>,
    /// Seconds since last successful health check.
    pub secs_since_last_success: Option<f64>,
    /// Number of consecutive health check failures.
    pub consecutive_failures: u32,
}

impl ConnectionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting...",
            Self::Connected => "Connected",
            Self::Error => "Error",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Disconnected => egui::Color32::GRAY,
            Self::Connecting => egui::Color32::YELLOW,
            Self::Connected => egui::Color32::GREEN,
            Self::Error => egui::Color32::RED,
        }
    }
}

/// System status indicator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)] // All variants defined for completeness
pub enum SystemStatus {
    #[default]
    Idle,
    Busy,
    Warning,
    Error,
}

impl SystemStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Busy => "Busy",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Idle => egui::Color32::from_rgb(100, 200, 100),
            Self::Busy => egui::Color32::from_rgb(100, 150, 255),
            Self::Warning => egui::Color32::YELLOW,
            Self::Error => egui::Color32::RED,
        }
    }
}

/// Experiment execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)] // All variants defined for completeness
pub enum ExperimentStatus {
    #[default]
    None,
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}

impl ExperimentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "No Experiment",
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::None => egui::Color32::GRAY,
            Self::Queued => egui::Color32::from_rgb(180, 180, 100),
            Self::Running => egui::Color32::from_rgb(100, 200, 100),
            Self::Paused => egui::Color32::YELLOW,
            Self::Completed => egui::Color32::from_rgb(100, 255, 100),
            Self::Failed => egui::Color32::RED,
        }
    }
}

/// Logging & Status Panel state
pub struct LoggingPanel {
    /// Log entries (newest at end)
    entries: VecDeque<LogEntry>,
    /// Next entry ID
    next_id: u64,
    /// Panel start time for timestamps
    start_time: Instant,

    // Filter settings
    /// Minimum level to display
    pub min_level: LogLevel,
    /// Selected category filter
    pub selected_category: LogCategory,
    /// Text search filter
    pub search_filter: String,
    /// Level toggles (which levels to show)
    pub level_enabled: [bool; 5],
    /// Show category column
    pub show_category: bool,

    // Display settings
    /// Auto-scroll to bottom
    pub auto_scroll: bool,
    /// Scroll position frozen
    pub scroll_paused: bool,
    /// Show source column
    pub show_source: bool,
    /// Show level column
    pub show_level: bool,

    // Export
    /// Export file path
    pub export_path: String,
    /// Export status message
    pub export_status: Option<(String, bool)>,
    /// Export in progress flag (bd-tjwm.8)
    export_in_progress: bool,
    /// Export result receiver (bd-tjwm.8)
    export_rx: Option<mpsc::Receiver<Result<(usize, String), String>>>,

    // Status indicators
    pub connection_status: ConnectionStatus,
    pub system_status: SystemStatus,
    pub experiment_status: ExperimentStatus,
    /// Optional experiment name
    pub experiment_name: Option<String>,
    /// Optional progress (0.0-1.0)
    pub experiment_progress: Option<f32>,
    /// Connection diagnostics (bd-j3xz.3.3)
    pub connection_diagnostics: ConnectionDiagnostics,
    /// Whether to show the diagnostics panel (bd-j3xz.3.3)
    show_diagnostics: bool,
}

impl Default for LoggingPanel {
    fn default() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            next_id: 0,
            start_time: Instant::now(),
            min_level: LogLevel::Debug, // Default to Debug to show streaming events
            selected_category: LogCategory::All,
            search_filter: String::new(),
            level_enabled: [true; 5], // All levels enabled
            show_category: true,
            auto_scroll: true,
            scroll_paused: false,
            show_source: true,
            show_level: true,
            export_path: String::from("logs/session.log"),
            export_status: None,
            export_in_progress: false,
            export_rx: None,
            connection_status: ConnectionStatus::default(),
            system_status: SystemStatus::default(),
            experiment_status: ExperimentStatus::default(),
            experiment_name: None,
            experiment_progress: None,
            connection_diagnostics: ConnectionDiagnostics::default(),
            show_diagnostics: false,
        }
    }
}

impl LoggingPanel {
    /// Create a new logging panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a log entry
    pub fn log(&mut self, level: LogLevel, source: &str, message: &str) {
        let timestamp = self.start_time.elapsed().as_secs_f64();
        let entry = LogEntry::new(self.next_id, timestamp, level, source, message);
        self.next_id += 1;

        self.entries.push_back(entry);

        // Trim if over capacity
        while self.entries.len() > MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// Convenience methods for each log level
    pub fn error(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Error, source, message);
    }

    #[allow(dead_code)]
    pub fn warn(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Warn, source, message);
    }

    pub fn info(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Info, source, message);
    }

    #[allow(dead_code)]
    pub fn debug(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Debug, source, message);
    }

    #[allow(dead_code)]
    pub fn trace(&mut self, source: &str, message: &str) {
        self.log(LogLevel::Trace, source, message);
    }

    /// Clear all log entries
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get number of entries (for external queries)
    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Get filtered entries
    ///
    /// Uses allocation-free case-insensitive search (bd-tjwm.4)
    fn filtered_entries(&self) -> Vec<&LogEntry> {
        let search = &self.search_filter;

        self.entries
            .iter()
            .filter(|e| {
                // Category filter (All shows everything)
                if self.selected_category != LogCategory::All
                    && e.category != self.selected_category
                {
                    return false;
                }

                // Level filter
                let level_idx = e.level as usize;
                if !self.level_enabled[level_idx] {
                    return false;
                }

                // Min level filter
                if e.level > self.min_level {
                    return false;
                }

                // Search filter (allocation-free case-insensitive)
                if !search.is_empty() {
                    let matches = contains_ignore_ascii_case(&e.message, search)
                        || contains_ignore_ascii_case(&e.source, search);
                    if !matches {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Generate export content
    fn generate_export(&self) -> String {
        let filtered = self.filtered_entries();
        let mut output = String::with_capacity(filtered.len() * 100);

        output.push_str("# rust-daq Log Export\n");
        output.push_str(&format!("# Entries: {}\n", filtered.len()));
        output.push_str(&format!(
            "# Filter: category={}, level>={}, search='{}'\n",
            self.selected_category.label(),
            self.min_level.label(),
            self.search_filter
        ));
        output.push_str("#\n");

        for entry in filtered {
            output.push_str(&entry.to_export_line());
            output.push('\n');
        }

        output
    }

    /// Export logs to file (synchronous version for internal use)
    fn export_to_file_sync(path: &str, content: String) -> Result<(usize, String), String> {
        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
        }

        let line_count = content.lines().count().saturating_sub(4); // Subtract header lines
        std::fs::write(path, content).map_err(|e| format!("Failed to write file: {}", e))?;

        Ok((line_count, path.to_string()))
    }

    /// Start async export (bd-tjwm.8: non-blocking)
    fn start_async_export(&mut self) {
        if self.export_in_progress {
            return;
        }

        let path = self.export_path.clone();
        let content = self.generate_export();
        let (tx, rx) = mpsc::channel();

        self.export_in_progress = true;
        self.export_status = Some(("Exporting...".to_string(), true));
        self.export_rx = Some(rx);

        std::thread::spawn(move || {
            let result = Self::export_to_file_sync(&path, content);
            let _ = tx.send(result);
        });
    }

    /// Poll for async export result
    fn poll_export_result(&mut self) {
        if let Some(rx) = &self.export_rx {
            match rx.try_recv() {
                Ok(Ok((count, path))) => {
                    self.export_status =
                        Some((format!("Exported {} entries to {}", count, path), true));
                    self.export_in_progress = false;
                    self.export_rx = None;
                }
                Ok(Err(e)) => {
                    self.export_status = Some((e, false));
                    self.export_in_progress = false;
                    self.export_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still in progress
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.export_status = Some(("Export thread crashed".to_string(), false));
                    self.export_in_progress = false;
                    self.export_rx = None;
                }
            }
        }
    }

    /// Render the status bar at the top
    fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Connection status with diagnostics toggle (bd-j3xz.3.3)
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let color = self.connection_status.color();
                    ui.colored_label(color, "●");
                    ui.label(self.connection_status.label());
                    // Show RTT if available
                    if let Some(rtt) = self.connection_diagnostics.last_rtt_ms {
                        ui.label(
                            egui::RichText::new(format!("{:.0}ms", rtt))
                                .small()
                                .color(egui::Color32::from_gray(160)),
                        );
                    }
                    // Diagnostics toggle button
                    if ui
                        .small_button(if self.show_diagnostics { "▼" } else { "▶" })
                        .on_hover_text("Toggle connection diagnostics")
                        .clicked()
                    {
                        self.show_diagnostics = !self.show_diagnostics;
                    }
                });
            });

            ui.separator();

            // System status
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let color = self.system_status.color();
                    ui.colored_label(color, "●");
                    ui.label(format!("System: {}", self.system_status.label()));
                });
            });

            ui.separator();

            // Experiment status
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let color = self.experiment_status.color();
                    ui.colored_label(color, "●");
                    if let Some(name) = &self.experiment_name {
                        ui.label(format!("{}: {}", name, self.experiment_status.label()));
                    } else {
                        ui.label(self.experiment_status.label());
                    }
                    if let Some(progress) = self.experiment_progress {
                        ui.add(egui::ProgressBar::new(progress).desired_width(60.0));
                    }
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} entries", self.entries.len()));
            });
        });

        // Show expanded diagnostics panel (bd-j3xz.3.3)
        if self.show_diagnostics {
            self.show_diagnostics_panel(ui);
        }
    }

    /// Render the connection diagnostics panel (bd-j3xz.3.3)
    fn show_diagnostics_panel(&self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Connection Diagnostics")
                        .strong()
                        .small(),
                );
            });

            ui.horizontal(|ui| {
                // RTT
                ui.label("RTT:");
                if let Some(rtt) = self.connection_diagnostics.last_rtt_ms {
                    let color = if rtt < 50.0 {
                        egui::Color32::GREEN
                    } else if rtt < 200.0 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::RED
                    };
                    ui.colored_label(color, format!("{:.1}ms", rtt));
                } else {
                    ui.label("--");
                }

                ui.separator();

                // Last success
                ui.label("Last OK:");
                if let Some(secs) = self.connection_diagnostics.secs_since_last_success {
                    ui.label(format!("{:.0}s ago", secs));
                } else {
                    ui.label("--");
                }

                ui.separator();

                // Total errors
                ui.label("Errors:");
                let error_color = if self.connection_diagnostics.total_errors == 0 {
                    egui::Color32::GREEN
                } else if self.connection_diagnostics.total_errors < 5 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::RED
                };
                ui.colored_label(
                    error_color,
                    format!("{}", self.connection_diagnostics.total_errors),
                );

                ui.separator();

                // Consecutive failures
                ui.label("Consecutive:");
                let consec_color = if self.connection_diagnostics.consecutive_failures == 0 {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::RED
                };
                ui.colored_label(
                    consec_color,
                    format!("{}", self.connection_diagnostics.consecutive_failures),
                );
            });

            // Show last error if any
            if let Some(ref msg) = self.connection_diagnostics.last_error_message {
                ui.horizontal(|ui| {
                    ui.label("Last error:");
                    if let Some(secs) = self.connection_diagnostics.secs_since_last_error {
                        ui.label(
                            egui::RichText::new(format!("({:.0}s ago)", secs))
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }
                    ui.colored_label(egui::Color32::from_rgb(255, 150, 150), msg);
                });
            }
        });
    }

    /// Render the filter controls
    fn show_filter_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Category dropdown (most important filter)
            ui.label("Category:");
            let cat_color = self.selected_category.color();
            egui::ComboBox::from_id_salt("log_category")
                .selected_text(
                    egui::RichText::new(self.selected_category.label())
                        .color(cat_color),
                )
                .show_ui(ui, |ui| {
                    for category in LogCategory::all_categories() {
                        let color = category.color();
                        ui.selectable_value(
                            &mut self.selected_category,
                            *category,
                            egui::RichText::new(category.label()).color(color),
                        );
                    }
                });

            ui.separator();

            ui.label("Level:");
            for level in LogLevel::all() {
                let idx = *level as usize;
                let color = if self.level_enabled[idx] {
                    level.color()
                } else {
                    egui::Color32::DARK_GRAY
                };

                if ui
                    .add(egui::Button::new(level.label()).fill(color.gamma_multiply(0.3)))
                    .clicked()
                {
                    self.level_enabled[idx] = !self.level_enabled[idx];
                }
            }

            ui.separator();

            ui.label("Min:");
            egui::ComboBox::from_id_salt("min_level")
                .selected_text(self.min_level.label())
                .show_ui(ui, |ui| {
                    for level in LogLevel::all() {
                        ui.selectable_value(&mut self.min_level, *level, level.label());
                    }
                });

            ui.separator();

            ui.label("Search:");
            ui.add(
                egui::TextEdit::singleline(&mut self.search_filter)
                    .desired_width(150.0)
                    .hint_text("Filter..."),
            );

            if !self.search_filter.is_empty() && ui.small_button("✕").clicked() {
                self.search_filter.clear();
            }
        });
    }

    /// Render the log entries table
    fn show_log_table(&mut self, ui: &mut egui::Ui) {
        let filtered = self.filtered_entries();
        let text_height = egui::TextStyle::Body
            .resolve(ui.style())
            .size
            .max(ui.spacing().interact_size.y);

        ui.push_id("log_table", |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(self.auto_scroll && !self.scroll_paused)
                .show_rows(ui, text_height, filtered.len(), |ui, row_range| {
                    for idx in row_range {
                        if let Some(entry) = filtered.get(idx) {
                            ui.horizontal(|ui| {
                                // Timestamp
                                ui.monospace(entry.formatted_timestamp());

                                // Level (colored)
                                if self.show_level {
                                    ui.colored_label(entry.level.color(), entry.level.label());
                                }

                                // Category (colored badge)
                                if self.show_category {
                                    ui.label(
                                        egui::RichText::new(format!("[{}]", entry.category.label()))
                                            .color(entry.category.color())
                                            .small(),
                                    );
                                }

                                // Source
                                if self.show_source && !entry.source.is_empty() {
                                    ui.label(
                                        egui::RichText::new(format!("[{}]", entry.source))
                                            .color(egui::Color32::from_gray(140)),
                                    );
                                }

                                // Message
                                ui.label(&entry.message);
                            });
                        }
                    }
                });
        });
    }

    /// Render bottom controls
    fn show_bottom_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Auto-scroll toggle
            if ui
                .selectable_label(self.auto_scroll, "Auto-scroll")
                .clicked()
            {
                self.auto_scroll = !self.auto_scroll;
            }

            // Pause/Resume
            if self.auto_scroll
                && ui
                    .button(if self.scroll_paused {
                        "▶ Resume"
                    } else {
                        "⏸ Pause"
                    })
                    .clicked()
            {
                self.scroll_paused = !self.scroll_paused;
            }

            ui.separator();

            // Column toggles
            ui.checkbox(&mut self.show_level, "Level");
            ui.checkbox(&mut self.show_category, "Category");
            ui.checkbox(&mut self.show_source, "Source");

            ui.separator();

            // Clear button
            if ui.button("Clear").clicked() {
                self.clear();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Export button (bd-tjwm.8: non-blocking export)
                let export_enabled = !self.export_in_progress;
                if ui
                    .add_enabled(export_enabled, egui::Button::new("Export"))
                    .clicked()
                {
                    self.start_async_export();
                }

                // Export path
                ui.add(
                    egui::TextEdit::singleline(&mut self.export_path)
                        .desired_width(180.0)
                        .hint_text("Export path..."),
                );

                // Show export status
                if let Some((msg, success)) = &self.export_status {
                    let color = if *success {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    };
                    ui.colored_label(color, msg);
                }
            });
        });
    }

    /// Main UI render method
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Poll for async export completion (bd-tjwm.8)
        self.poll_export_result();

        ui.heading("Logging & Status");
        ui.separator();

        // Status bar
        self.show_status_bar(ui);
        ui.add_space(4.0);

        // Filter controls
        self.show_filter_controls(ui);
        ui.add_space(4.0);
        ui.separator();

        // Log table (takes remaining space)
        let available = ui.available_height() - 30.0; // Reserve space for bottom controls
        ui.allocate_ui(
            egui::vec2(ui.available_width(), available.max(100.0)),
            |ui| {
                self.show_log_table(ui);
            },
        );

        ui.separator();

        // Bottom controls
        self.show_bottom_controls(ui);
    }

    /// Demo mode: add sample log entries for testing
    #[allow(dead_code)]
    pub fn add_demo_entries(&mut self) {
        self.info("System", "rust-daq logging panel initialized");
        self.info(
            "Connection",
            "Attempting to connect to daemon at 127.0.0.1:50051",
        );
        self.warn("Connection", "Connection timeout, retrying...");
        self.info("Connection", "Connected to daemon successfully");
        self.debug("Hardware", "Discovered mock_stage (MockStage v1.0)");
        self.debug("Hardware", "Discovered mock_camera (MockCamera 640x480)");
        self.debug("Hardware", "Discovered mock_power_meter (MockPowerMeter)");
        self.info("System", "3 devices initialized");
        self.trace("EventLoop", "Frame rendered in 2.3ms");
        self.info("Experiment", "Starting GridScan experiment");
        self.debug("Experiment", "Moving stage to position 0.0");
        self.debug("PowerMeter", "Reading: 1.23 mW");
        self.debug("Experiment", "Moving stage to position 1.0");
        self.debug("PowerMeter", "Reading: 1.45 mW");
        self.warn(
            "PowerMeter",
            "Reading fluctuation detected: 0.15 mW variance",
        );
        self.debug("Experiment", "Moving stage to position 2.0");
        self.error("Stage", "Motion controller timeout at position 2.0");
        self.warn("Experiment", "Retrying motion command...");
        self.info("Experiment", "Motion recovered, continuing scan");
        self.info("Experiment", "GridScan completed: 10 points in 45.2s");
        self.info("Storage", "Data saved to output/scan_2025-12-21_001.h5");
    }
}
