//! Config-driven UI rendering for device control panels
//!
//! Renders device control panels based on UiConfig from TOML files

use daq_hardware::config::schema::{ControlPanelConfig, ControlSection, PresetValue};
use eframe::egui;
use protocol::daq::DeviceInfo;

/// Render a control panel based on configuration
pub fn render_config_panel(ui: &mut egui::Ui, _device: &DeviceInfo, config: &ControlPanelConfig) {
    // Show header if configured
    if config.show_header {
        ui.heading("Device Controls");
        ui.separator();
    }

    // Render sections based on layout
    match config.layout {
        daq_hardware::config::schema::PanelLayout::Vertical => {
            render_sections_vertical(ui, &config.sections);
        }
        daq_hardware::config::schema::PanelLayout::Horizontal => {
            ui.horizontal(|ui| {
                render_sections_vertical(ui, &config.sections);
            });
        }
        daq_hardware::config::schema::PanelLayout::Grid => {
            // TODO: Implement grid layout with configurable columns
            render_sections_vertical(ui, &config.sections);
        }
    }
}

/// Render sections vertically (stacked)
fn render_sections_vertical(ui: &mut egui::Ui, sections: &[ControlSection]) {
    for section in sections {
        render_section(ui, section);
    }
}

/// Render a single control section
fn render_section(ui: &mut egui::Ui, section: &ControlSection) {
    match section {
        ControlSection::Motion(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label("Motion controls coming soon");
                // TODO: Implement motion controls (position display, jog buttons, etc.)
            });
        }
        ControlSection::PresetButtons(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                render_preset_buttons(ui, &cfg.presets, cfg.vertical);
            });
        }
        ControlSection::CustomAction(cfg) => {
            ui.group(|ui| {
                if ui.button(&cfg.label).clicked() {
                    tracing::info!("Custom action clicked: {}", cfg.command);
                    // TODO: Execute command
                }
            });
        }
        ControlSection::Camera(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label("Camera controls coming soon");
                // TODO: Implement camera controls
            });
        }
        ControlSection::Shutter(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label("Shutter controls coming soon");
                // TODO: Implement shutter toggle/buttons
            });
        }
        ControlSection::Wavelength(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label("Wavelength controls coming soon");
                // TODO: Implement wavelength slider and presets
            });
        }
        ControlSection::Parameter(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label(format!("Parameter: {}", cfg.parameter));
                // TODO: Implement parameter display/edit
            });
        }
        ControlSection::StatusDisplay(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label(format!("Status params: {:?}", cfg.parameters));
                // TODO: Implement status display
            });
        }
        ControlSection::Sensor(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label("Sensor reading display coming soon");
                // TODO: Implement sensor reading display
            });
        }
        ControlSection::Separator(cfg) => {
            if cfg.visible {
                ui.separator();
            } else {
                ui.add_space(cfg.height as f32);
            }
        }
        ControlSection::Custom(cfg) => {
            ui.group(|ui| {
                ui.label(&cfg.label);
                ui.label(format!("Custom widget: {}", cfg.widget));
                // TODO: Support custom widgets via plugin system
            });
        }
    }
}

/// Render preset buttons
fn render_preset_buttons(ui: &mut egui::Ui, presets: &[PresetValue], vertical: bool) {
    let render_button = |ui: &mut egui::Ui, preset: &PresetValue| {
        let (label, value) = match preset {
            PresetValue::Number(v) => (format!("{:.1}", v), *v),
            PresetValue::Labeled { label, value } => (label.clone(), *value),
        };

        if ui.button(&label).clicked() {
            tracing::info!("Preset clicked: {} = {}", label, value);
            // TODO: Send move command to device
        }
    };

    if vertical {
        for preset in presets {
            render_button(ui, preset);
        }
    } else {
        ui.horizontal(|ui| {
            for preset in presets {
                render_button(ui, preset);
            }
        });
    }
}
