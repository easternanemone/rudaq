//! Centralized settings window for the GUI.

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::theme::ThemePreference;

/// Application settings that can be configured by the user.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    /// Connection settings
    pub connection: ConnectionSettings,
    /// Appearance settings
    pub appearance: AppearanceSettings,
    /// Logging settings
    pub logging: LoggingSettings,
    /// Storage settings
    pub storage: StorageSettings,
}

/// Connection settings for daemon communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSettings {
    /// Daemon address (hostname:port or URL)
    pub daemon_address: String,
    /// Enable automatic reconnection on disconnect
    pub auto_reconnect: bool,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            daemon_address: "localhost:50051".to_string(),
            auto_reconnect: true,
            timeout_secs: 10,
        }
    }
}

/// Appearance settings for theme and UI scaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// Theme preference (Light/Dark/System)
    pub theme: ThemePreference,
    /// Font size multiplier (1.0 = default)
    pub font_scale: f32,
    /// UI scale multiplier (1.0 = default)
    pub ui_scale: f32,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            font_scale: 1.0,
            ui_scale: 1.0,
        }
    }
}

/// Logging settings for log level and file output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingSettings {
    /// Log level (trace, debug, info, warn, error)
    pub level: LogLevel,
    /// Log file path (empty = no file logging)
    pub log_file_path: String,
    /// Maximum log file size in MB (0 = unlimited)
    pub max_log_size_mb: u32,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            log_file_path: String::new(),
            max_log_size_mb: 100,
        }
    }
}

/// Log level enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "Trace",
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
        }
    }
}

/// Storage settings for data output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSettings {
    /// Default save directory
    pub default_save_dir: String,
    /// Preferred file format (HDF5, CSV, etc.)
    pub file_format: FileFormat,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            default_save_dir: {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join("daq_data")
                        .to_string_lossy()
                        .to_string()
                }
                #[cfg(target_arch = "wasm32")]
                {
                    "daq_data".to_string()
                }
            },
            file_format: FileFormat::Hdf5,
        }
    }
}

/// File format enum for data output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileFormat {
    Hdf5,
    Csv,
    Json,
}

impl FileFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hdf5 => "HDF5",
            Self::Csv => "CSV",
            Self::Json => "JSON",
        }
    }
}

/// Settings window state.
pub struct SettingsWindow {
    /// Whether the settings window is open
    pub open: bool,
    /// Working copy of settings (for Apply/Cancel)
    working_settings: AppSettings,
    /// Selected section tab
    selected_section: SettingsSection,
}

impl Default for SettingsWindow {
    fn default() -> Self {
        Self {
            open: false,
            working_settings: AppSettings::default(),
            selected_section: SettingsSection::Connection,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Connection,
    Appearance,
    Logging,
    Storage,
    Calibration,
    Shortcuts,
}

impl SettingsSection {
    fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Appearance => "Appearance",
            Self::Logging => "Logging",
            Self::Storage => "Storage",
            Self::Calibration => "Calibration",
            Self::Shortcuts => "Shortcuts",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Connection => crate::icons::PLUGS,
            Self::Appearance => crate::icons::PALETTE,
            Self::Logging => crate::icons::LIST_BULLETS,
            Self::Storage => crate::icons::DATABASE,
            Self::Calibration => crate::icons::RULER,
            Self::Shortcuts => crate::icons::KEYBOARD,
        }
    }
}

impl SettingsWindow {
    /// Show the settings window.
    pub fn show(&mut self, ctx: &egui::Context, current_settings: &mut AppSettings) -> bool {
        if !self.open {
            return false;
        }

        let mut should_apply = false;
        let mut should_close = false;

        // Copy current settings to working copy when opening
        if self.working_settings.connection.daemon_address
            != current_settings.connection.daemon_address
        {
            self.working_settings = current_settings.clone();
        }

        // Use a local copy of open state to avoid borrow conflicts
        let mut open = self.open;

        egui::Window::new(format!("{} Settings", crate::icons::action::SETTINGS))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(700.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                // Use horizontal layout with sidebar
                egui::SidePanel::left("settings_sidebar")
                    .resizable(false)
                    .exact_width(150.0)
                    .show_inside(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.heading("Sections");
                            ui.separator();

                            // Section buttons
                            for section in [
                                SettingsSection::Connection,
                                SettingsSection::Appearance,
                                SettingsSection::Logging,
                                SettingsSection::Storage,
                                SettingsSection::Calibration,
                                SettingsSection::Shortcuts,
                            ] {
                                let is_selected = self.selected_section == section;
                                let response = ui.selectable_label(
                                    is_selected,
                                    format!("{} {}", section.icon(), section.label()),
                                );
                                if response.clicked() {
                                    self.selected_section = section;
                                }
                            }
                        });
                    });

                egui::CentralPanel::default().show_inside(ui, |ui| {
                    ui.heading(self.selected_section.label());
                    ui.separator();

                    egui::ScrollArea::vertical().show(ui, |ui| match self.selected_section {
                        SettingsSection::Connection => {
                            self.show_connection_settings(ui);
                        }
                        SettingsSection::Appearance => {
                            self.show_appearance_settings(ui, ctx);
                        }
                        SettingsSection::Logging => {
                            self.show_logging_settings(ui);
                        }
                        SettingsSection::Storage => {
                            self.show_storage_settings(ui);
                        }
                        SettingsSection::Calibration => {
                            self.show_calibration_settings(ui);
                        }
                        SettingsSection::Shortcuts => {
                            self.show_shortcuts_settings(ui);
                        }
                    });

                    ui.separator();

                    // Bottom action buttons
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            should_apply = true;
                        }
                        if ui.button("OK").clicked() {
                            should_apply = true;
                            should_close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Reset to Defaults").clicked() {
                                self.working_settings = AppSettings::default();
                            }
                        });
                    });
                });
            });

        // Update open state from local copy (window X button handling)
        self.open = open;

        if should_apply {
            *current_settings = self.working_settings.clone();
        }

        if should_close {
            self.open = false;
        }

        should_apply
    }

    fn show_connection_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("connection_grid")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Daemon Address:");
                ui.text_edit_singleline(&mut self.working_settings.connection.daemon_address);
                ui.end_row();

                ui.label("Auto-reconnect:");
                ui.checkbox(&mut self.working_settings.connection.auto_reconnect, "");
                ui.end_row();

                ui.label("Timeout (seconds):");
                ui.add(
                    egui::DragValue::new(&mut self.working_settings.connection.timeout_secs)
                        .speed(1.0)
                        .range(1..=60),
                );
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.separator();
        ui.label(
            egui::RichText::new("Note: Connection changes require reconnect")
                .small()
                .weak(),
        );
    }

    fn show_appearance_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Grid::new("appearance_grid")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Theme:");
                egui::ComboBox::from_id_salt("theme_combo")
                    .selected_text(self.working_settings.appearance.theme.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.working_settings.appearance.theme,
                            ThemePreference::Light,
                            "Light",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.appearance.theme,
                            ThemePreference::Dark,
                            "Dark",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.appearance.theme,
                            ThemePreference::System,
                            "System",
                        );
                    });
                ui.end_row();

                ui.label("Font Scale:");
                ui.add(
                    egui::Slider::new(&mut self.working_settings.appearance.font_scale, 0.8..=2.0)
                        .step_by(0.1),
                );
                ui.end_row();

                ui.label("UI Scale:");
                ui.add(
                    egui::Slider::new(&mut self.working_settings.appearance.ui_scale, 0.8..=2.0)
                        .step_by(0.1),
                );
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Preview:").weak());
        ui.label(format!("Current zoom: {:.0}%", ctx.zoom_factor() * 100.0));
    }

    fn show_logging_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("logging_grid")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Log Level:");
                egui::ComboBox::from_id_salt("log_level_combo")
                    .selected_text(self.working_settings.logging.level.as_str())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.working_settings.logging.level,
                            LogLevel::Trace,
                            "Trace",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.logging.level,
                            LogLevel::Debug,
                            "Debug",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.logging.level,
                            LogLevel::Info,
                            "Info",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.logging.level,
                            LogLevel::Warn,
                            "Warn",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.logging.level,
                            LogLevel::Error,
                            "Error",
                        );
                    });
                ui.end_row();

                ui.label("Log File Path:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.working_settings.logging.log_file_path);
                    // TODO(bd-wev5): integrate native file picker (rfd crate)
                    if ui
                        .add_enabled(false, egui::Button::new("Browse..."))
                        .on_disabled_hover_text(
                            "File picker not yet available — enter the path manually",
                        )
                        .clicked()
                    {}
                });
                ui.end_row();

                ui.label("Max Log Size (MB):");
                ui.add(
                    egui::DragValue::new(&mut self.working_settings.logging.max_log_size_mb)
                        .speed(10.0)
                        .range(0..=1000),
                );
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Tip: Set path to empty to disable file logging")
                .small()
                .weak(),
        );
    }

    fn show_storage_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("storage_grid")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Default Save Directory:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.working_settings.storage.default_save_dir);
                    // TODO(bd-wev5): integrate native directory picker (rfd crate)
                    if ui
                        .add_enabled(false, egui::Button::new("Browse..."))
                        .on_disabled_hover_text(
                            "Directory picker not yet available — enter the path manually",
                        )
                        .clicked()
                    {}
                });
                ui.end_row();

                ui.label("File Format:");
                egui::ComboBox::from_id_salt("file_format_combo")
                    .selected_text(self.working_settings.storage.file_format.as_str())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.working_settings.storage.file_format,
                            FileFormat::Hdf5,
                            "HDF5",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.storage.file_format,
                            FileFormat::Csv,
                            "CSV",
                        );
                        ui.selectable_value(
                            &mut self.working_settings.storage.file_format,
                            FileFormat::Json,
                            "JSON",
                        );
                    });
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Note: HDF5 format recommended for large datasets")
                .small()
                .weak(),
        );
    }

    fn show_calibration_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "Calibration file management is not yet available. \
                 Use the CLI `rust-daq-daemon calibrate` command for offline calibration.",
            )
            .weak(),
        );
    }

    fn show_shortcuts_settings(&mut self, ui: &mut egui::Ui) {
        ui.label("Keyboard shortcuts:");
        ui.add_space(10.0);

        egui::Grid::new("shortcuts_grid")
            .num_columns(2)
            .spacing([40.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label("Settings");
                ui.monospace("Ctrl+,");
                ui.end_row();

                ui.label("Quit");
                ui.monospace("Ctrl+Q");
                ui.end_row();

                ui.label("Connect/Disconnect");
                ui.monospace("Ctrl+K");
                ui.end_row();

                ui.label("Reset Layout");
                ui.monospace("Ctrl+Shift+R");
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.separator();
        ui.label(
            egui::RichText::new("Note: Keyboard shortcuts are not currently customizable")
                .small()
                .weak(),
        );
    }

    /// Open the settings window.
    pub fn open(&mut self) {
        self.open = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert_eq!(settings.connection.daemon_address, "localhost:50051");
        assert!(settings.connection.auto_reconnect);
        assert!((settings.appearance.font_scale - 1.0).abs() < f32::EPSILON);
        assert_eq!(settings.logging.level, LogLevel::Info);
    }

    #[test]
    fn test_log_level_as_str() {
        assert_eq!(LogLevel::Trace.as_str(), "Trace");
        assert_eq!(LogLevel::Debug.as_str(), "Debug");
        assert_eq!(LogLevel::Info.as_str(), "Info");
        assert_eq!(LogLevel::Warn.as_str(), "Warn");
        assert_eq!(LogLevel::Error.as_str(), "Error");
    }

    #[test]
    fn test_file_format_as_str() {
        assert_eq!(FileFormat::Hdf5.as_str(), "HDF5");
        assert_eq!(FileFormat::Csv.as_str(), "CSV");
        assert_eq!(FileFormat::Json.as_str(), "JSON");
    }

    #[test]
    fn test_settings_serialization() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.connection.daemon_address,
            settings.connection.daemon_address
        );
    }

    #[test]
    fn test_settings_window_default() {
        let window = SettingsWindow::default();
        assert!(!window.open);
        assert_eq!(window.selected_section, SettingsSection::Connection);
    }
}
