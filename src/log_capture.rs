//! A custom log collector for capturing application logs for display in the GUI.

use chrono::{DateTime, Local};
use egui::Color32;
use log::{Level, Log, Metadata, Record};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_LOG_ENTRIES: usize = 1000;

/// Represents a single log entry.
#[derive(Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub level: Level,
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// Returns a color corresponding to the log level for GUI display.
    pub fn color(&self) -> Color32 {
        match self.level {
            Level::Error => Color32::from_rgb(255, 100, 100), // Light Red
            Level::Warn => Color32::from_rgb(255, 255, 100),  // Yellow
            Level::Info => Color32::from_rgb(100, 200, 255),  // Light Blue
            Level::Debug => Color32::from_rgb(150, 150, 150), // Gray
            Level::Trace => Color32::from_rgb(200, 150, 255), // Light Purple
        }
    }
}

/// A thread-safe, fixed-capacity log buffer.
#[derive(Clone)]
pub struct LogBuffer(Arc<Mutex<VecDeque<LogEntry>>>);

impl LogBuffer {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(
            MAX_LOG_ENTRIES,
        ))))
    }

    pub fn read(&self) -> std::sync::MutexGuard<'_, VecDeque<LogEntry>> {
        self.0.lock().unwrap()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// A simple logger that captures logs into a `LogBuffer`.
pub struct LogCollector {
    buffer: LogBuffer,
}

impl LogCollector {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }

    /// Returns a reference to the internal log buffer.
    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }
}

impl Log for LogCollector {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true // Capture all levels, filtering will be done in the GUI
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let mut buffer = self.buffer.0.lock().unwrap();

        if buffer.len() >= MAX_LOG_ENTRIES {
            buffer.pop_front();
        }

        buffer.push_back(LogEntry {
            timestamp: Local::now(),
            level: record.level(),
            target: record.target().to_string(),
            message: format!("{}", record.args()),
        });
    }

    fn flush(&self) {}
}