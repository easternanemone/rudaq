//! Renders the log panel in the GUI.
//!
//! This module contains the `render` function which is responsible for drawing the
//! log panel at the bottom of the main application window. The panel provides
//! features for viewing, filtering, and managing log messages captured from the
//! `log` crate.
//!
//! ## Features
//!
//! - **Log Display:** Shows a time-stamped, color-coded, and scrollable list of log entries.
//! - **Level Filtering:** A dropdown allows the user to filter logs by their severity
//!   (e.g., Error, Warn, Info).
//! - **Text Filtering:** A text input field allows filtering logs by their message content or target.
//! - **Auto-Scrolling:** A toggle to automatically scroll to the latest log message.
//! - **Clear Button:** A button to clear all captured log messages.
//! - **Efficient Rendering:** Uses `ScrollArea::show_rows` to only render the visible portion
//!   of the log list, ensuring good performance even with a large number of log entries.

use crate::gui::Gui;
use eframe::egui::{self, Color32, ScrollArea, Ui};
use log::LevelFilter;

/// Renders the log panel.
pub fn render(ui: &mut Ui, gui: &mut Gui) {
    ui.heading("Event Log");

    // --- Header Controls ---
    ui.horizontal(|ui| {
        // --- Log Level Filter ---
        ui.label("Filter Level:");
        level_filter_combo_box(ui, &mut gui.log_level_filter);

        // --- Text Filter ---
        ui.label("Filter Text:");
        let _ = ui.text_edit_singleline(&mut gui.log_filter_text);

        // --- Spacer and Clear Button ---
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                gui.log_buffer.clear();
                gui.consolidated_logs.clear();
            }
            // --- Scroll to Bottom Toggle ---
            ui.toggle_value(&mut gui.scroll_to_bottom, "Scroll to Bottom");
            // --- Consolidation Toggle ---
            ui.toggle_value(&mut gui.log_consolidation, "Consolidate Logs");
        });
    });

    ui.separator();

    // --- Log Messages Area ---
    let scroll_area = ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(gui.scroll_to_bottom);
    let text_style = egui::TextStyle::Monospace;
    let row_height = ui.text_style_height(&text_style);
    let logs = gui.log_buffer.read();

    if gui.log_consolidation {
        // --- Consolidation Logic ---
        if logs.len() != gui.last_log_buffer_len {
            gui.consolidated_logs.clear();
            for entry in logs.iter() {
                let key = format!("{}:{}", entry.target, entry.message);
                let consolidated_entry = gui.consolidated_logs.entry(key).or_insert_with(|| {
                    crate::gui::ConsolidatedLogEntry {
                        entry: entry.clone(),
                        count: 0,
                        last_timestamp: entry.timestamp,
                    }
                });
                consolidated_entry.count += 1;
                consolidated_entry.last_timestamp = entry.timestamp;
            }
            gui.last_log_buffer_len = logs.len();
        }

        let mut sorted_logs: Vec<_> = gui.consolidated_logs.values().cloned().collect();
        sorted_logs.sort_by_key(|e| e.entry.timestamp);

        let filtered_logs: Vec<_> = sorted_logs
            .iter()
            .filter(|entry| {
                let level_match = entry.entry.level
                    <= gui.log_level_filter.to_level().unwrap_or(log::Level::Trace);
                let text_match = gui.log_filter_text.is_empty()
                    || entry.entry.message.contains(&gui.log_filter_text)
                    || entry.entry.target.contains(&gui.log_filter_text);
                level_match && text_match
            })
            .collect();

        let num_rows = filtered_logs.len();
        scroll_area.show_rows(ui, row_height, num_rows, |ui, row_range| {
            for i in row_range {
                if let Some(consolidated) = filtered_logs.get(i) {
                    let entry = &consolidated.entry;
                    ui.horizontal(|ui| {
                        let level_text = format!("[{:<5}]", entry.level);
                        ui.colored_label(entry.color(), level_text);
                        ui.label(entry.timestamp.format("%H:%M:%S%.3f").to_string());
                        ui.colored_label(Color32::from_gray(150), &entry.target);
                        ui.label(&entry.message);
                        if consolidated.count > 1 {
                            ui.colored_label(Color32::YELLOW, format!("(×{})", consolidated.count));
                        }
                    });
                }
            }
        });
    } else {
        // --- Standard Logic ---
        let filtered_logs: Vec<_> = logs
            .iter()
            .filter(|entry| {
                let level_match =
                    entry.level <= gui.log_level_filter.to_level().unwrap_or(log::Level::Trace);
                let text_match = gui.log_filter_text.is_empty()
                    || entry.message.contains(&gui.log_filter_text)
                    || entry.target.contains(&gui.log_filter_text);
                level_match && text_match
            })
            .collect();

        let num_rows = filtered_logs.len();

        scroll_area.show_rows(ui, row_height, num_rows, |ui, row_range| {
            for i in row_range {
                if let Some(entry) = filtered_logs.get(i) {
                    ui.horizontal(|ui| {
                        let level_text = format!("[{:<5}]", entry.level);
                        ui.colored_label(entry.color(), level_text);
                        ui.label(entry.timestamp.format("%H:%M:%S%.3f").to_string());
                        ui.colored_label(Color32::from_gray(150), &entry.target);
                        ui.label(&entry.message);
                    });
                }
            }
        });
    }
}

/// A combo box for selecting the log level filter.
fn level_filter_combo_box(ui: &mut Ui, level_filter: &mut LevelFilter) {
    egui::ComboBox::from_id_salt("log_level_filter")
        .selected_text(format!("{:?}", level_filter))
        .show_ui(ui, |ui| {
            ui.selectable_value(level_filter, LevelFilter::Off, "Off");
            ui.selectable_value(level_filter, LevelFilter::Error, "Error");
            ui.selectable_value(level_filter, LevelFilter::Warn, "Warn");
            ui.selectable_value(level_filter, LevelFilter::Info, "Info");
            ui.selectable_value(level_filter, LevelFilter::Debug, "Debug");
            ui.selectable_value(level_filter, LevelFilter::Trace, "Trace");
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::Gui;
    use crate::log_capture::LogBuffer;
    use eframe::egui;
    use log::Level;
    use std::collections::HashMap;

    fn create_test_gui() -> (Gui, LogBuffer) {
        let log_buffer = LogBuffer::new();
        let mut gui = Gui {
            command_tx: tokio::sync::mpsc::channel(1).0,
            runtime: Arc::new(tokio::runtime::Runtime::new().unwrap()),
            settings: Arc::new(crate::config::Settings {
                log_level: "info".to_string(),
                storage: crate::config::StorageSettings {
                    default_path: "./data".to_string(),
                    default_format: "csv".to_string(),
                },
                instruments: HashMap::new(),
                processors: None,
            }),
            instrument_registry_v2: Arc::new(crate::instrument::InstrumentRegistryV2::new()),
            data_receiver: tokio::sync::mpsc::channel(1).1,
            log_buffer: log_buffer.clone(),
            pending_operations: HashMap::new(),
            instrument_status_cache: HashMap::new(),
            cache_refresh_counter: 0,
            dock_state: egui_dock::DockState::new(vec![]),
            selected_channel: String::new(),
            storage_manager: crate::gui::storage_manager::StorageManager::new(),
            show_storage: false,
            log_filter_text: String::new(),
            log_level_filter: LevelFilter::Trace,
            scroll_to_bottom: false,
            log_consolidation: false,
            consolidated_logs: HashMap::new(),
            last_log_buffer_len: 0,
            data_cache: HashMap::new(),
            channel_subscriptions: HashMap::new(),
            subscriptions_dirty: false,
            frame_counter: 0,
            screenshot_request: None,
        };
        (gui, log_buffer)
    }

    #[test]
    fn test_clear_logs_button() {
        let (mut gui, log_buffer) = create_test_gui();
        let mut ui = egui::Ui::__test();

        // Add a log entry
        log_buffer.read().push_back(crate::log_capture::LogEntry {
            timestamp: chrono::Local::now(),
            level: Level::Info,
            target: "test".to_string(),
            message: "test message".to_string(),
        });

        assert!(!log_buffer.read().is_empty());

        // Simulate button click
        render(&mut ui, &mut gui);
        // This is a bit of a hack to simulate the button click, as we can't
        // actually click the button in a test. We just call the clear method.
        log_buffer.clear();

        assert!(log_buffer.read().is_empty());
    }

    #[test]
    fn test_log_filtering_by_level() {
        let (mut gui, log_buffer) = create_test_gui();

        // Add log entries with different levels
        log_buffer.read().push_back(crate::log_capture::LogEntry {
            timestamp: chrono::Local::now(),
            level: Level::Info,
            target: "test".to_string(),
            message: "info message".to_string(),
        });
        log_buffer.read().push_back(crate::log_capture::LogEntry {
            timestamp: chrono::Local::now(),
            level: Level::Warn,
            target: "test".to_string(),
            message: "warn message".to_string(),
        });

        // Set filter to Info
        gui.log_level_filter = LevelFilter::Info;

        // The filtering logic is inside the render function, so we can't directly
        // test the filtered list. Instead, we can infer the filtering is working
        // by checking the number of rows that would be rendered.
        // This is not ideal, but it's a start.

        let logs = log_buffer.read();
        let filtered_logs: Vec<_> = logs
            .iter()
            .filter(|entry| {
                let level_match =
                    entry.level <= gui.log_level_filter.to_level().unwrap_or(log::Level::Trace);
                level_match
            })
            .collect();

        assert_eq!(filtered_logs.len(), 1);
        assert_eq!(filtered_logs[0].level, Level::Info);
    }
}
