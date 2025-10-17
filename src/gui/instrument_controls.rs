//! Interactive instrument control panels.
//!
//! This module provides interactive GUI panels for controlling various hardware instruments.
//! Each panel is a separate struct that manages its own state and provides a `ui` method
//! to render the controls using `egui`. These panels are designed to be instantiated and
//! displayed within tabs in the main `egui_dock` area.
//!
//! ## Design
//!
//! - **Stateful Panels:** Each control panel (e.g., `MaiTaiControlPanel`, `PVCAMControlPanel`)
//!   is a struct that holds the UI state (like target values, toggles) and relevant data
//!   received from the instrument.
//! - **`DaqApp` Handle:** Each `ui` method takes a reference to `DaqApp`, which acts as a
//!   bridge to the application's core logic. This allows the UI to send commands to the
//!   instruments.
//! - **Command-Based Interaction:** User interactions (e.g., clicking a button, changing a slider)
//!   are translated into `InstrumentCommand` enums. These commands are sent to the corresponding
//!   instrument task via a channel managed by the `DaqApp`.
//! - **Error Handling:** Errors that occur when sending commands are logged to the central
//!   log panel using the `log::error!` macro.
//! - **Immediate Feedback (UI State):** To provide a responsive user experience, the local UI state
//!   (e.g., a display of the current position) is often updated immediately, assuming the command
//!   will be successful. The actual instrument state is updated asynchronously.

use crate::{app::DaqApp, core::InstrumentCommand};
use egui::{Ui, Color32, Slider};
use log::error;

use std::collections::HashMap;
use crate::core::DataPoint;

/// MaiTai laser control panel
pub struct MaiTaiControlPanel {
    pub instrument_id: String,
    pub target_wavelength: f64,
}

impl MaiTaiControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            target_wavelength: 800.0,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp, data_cache: &HashMap<String, DataPoint>) {
        ui.heading("MaiTai Laser Control");
        ui.separator();

        // Wavelength control
        ui.horizontal(|ui| {
            ui.label("Set Wavelength (nm):");
            ui.add(egui::DragValue::new(&mut self.target_wavelength)
                .speed(1.0)
                .range(700.0..=1000.0));
            if ui.button("Set").clicked() {
                let cmd = InstrumentCommand::SetParameter(
                    "wavelength".to_string(),
                    self.target_wavelength.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to set wavelength: {}", e);
                }
            }
        });

        ui.add_space(10.0);

        // Get current state from cache
        let current_wavelength = data_cache.get(&format!("{}:wavelength", self.instrument_id))
            .map_or(0.0, |dp| dp.value);
        let current_power = data_cache.get(&format!("{}:power", self.instrument_id))
            .map_or(0.0, |dp| dp.value);
        let shutter_open = data_cache.get(&format!("{}:shutter", self.instrument_id))
            .map_or(false, |dp| dp.value > 0.0);
        let laser_on = data_cache.get(&format!("{}:laser", self.instrument_id))
            .map_or(false, |dp| dp.value > 0.0);


        // Current wavelength display
        ui.horizontal(|ui| {
            ui.label("Actual Wavelength:");
            ui.colored_label(Color32::GREEN, format!("{:.1} nm", current_wavelength));
        });

        ui.add_space(10.0);

        // Power display
        ui.horizontal(|ui| {
            ui.label("Output Power:");
            ui.colored_label(Color32::YELLOW, format!("{:.2} W", current_power));
        });

        ui.add_space(10.0);

        // Visual wavelength indicator
        ui.group(|ui| {
            ui.label("Tuning Range");
            ui.add(Slider::new(&mut self.target_wavelength, 700.0..=1000.0)
                .text("nm")
                .show_value(false));

            // Show current wavelength marker
            ui.label(format!("▼ {:.0} nm", current_wavelength));
        });

        ui.add_space(15.0);

        // Control buttons
        ui.horizontal(|ui| {
            // Shutter button
            let shutter_text = if shutter_open { "Close Shutter" } else { "Open Shutter" };
            let shutter_color = if shutter_open { Color32::GREEN } else { Color32::GRAY };
            if ui.button(egui::RichText::new(shutter_text).color(shutter_color)).clicked() {
                let new_state = !shutter_open;
                let cmd = InstrumentCommand::SetParameter(
                    "shutter".to_string(),
                    if new_state { "open" } else { "close" }.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle shutter: {}", e);
                }
            }

            ui.add_space(10.0);

            // Laser ON/OFF button
            let laser_text = if laser_on { "Laser ON" } else { "Laser OFF" };
            let laser_color = if laser_on { Color32::RED } else { Color32::DARK_GRAY };
            if ui.button(egui::RichText::new(laser_text).color(laser_color).size(16.0)).clicked() {
                let new_state = !laser_on;
                let cmd = InstrumentCommand::SetParameter(
                    "laser".to_string(),
                    if new_state { "on" } else { "off" }.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle laser: {}", e);
                }
            }
        });

        ui.add_space(10.0);

        // System status
        ui.separator();
        let status_text = if laser_on {
            "System Status: ACTIVE"
        } else {
            "System Status: Ready to turn on"
        };
        let status_color = if laser_on { Color32::RED } else { Color32::LIGHT_GREEN };
        ui.colored_label(status_color, status_text);
    }
}

/// Newport 1830-C power meter control panel
pub struct Newport1830CControlPanel {
    pub instrument_id: String,
    pub wavelength: f64,
    pub range: i32,
    pub units: i32,
}

impl Newport1830CControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            wavelength: 1550.0,
            range: 0, // autorange
            units: 0, // Watts
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp, data_cache: &HashMap<String, DataPoint>) {
        ui.heading("Newport 1830-C Power Meter");
        ui.separator();

        // Wavelength setting
        ui.horizontal(|ui| {
            ui.label("Wavelength (nm):");
            ui.add(egui::DragValue::new(&mut self.wavelength)
                .speed(1.0)
                .range(400.0..=1800.0));
            if ui.button("Set").clicked() {
                let cmd = InstrumentCommand::SetParameter(
                    "wavelength".to_string(),
                    self.wavelength.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to set Newport wavelength: {}", e);
                }
            }
        });

        ui.add_space(10.0);

        // Range selection
        ui.horizontal(|ui| {
            ui.label("Range:");
            let range_text = if self.range == 0 {
                "Auto".to_string()
            } else {
                format!("Range {}", self.range)
            };
            egui::ComboBox::from_id_salt("range_combo")
                .selected_text(range_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.range, 0, "Auto");
                    for i in 1..=7 {
                        ui.selectable_value(&mut self.range, i, format!("Range {}", i));
                    }
                });
        });

        ui.add_space(10.0);

        // Units selection
        ui.horizontal(|ui| {
            ui.label("Units:");
            egui::ComboBox::from_id_salt("units_combo")
                .selected_text(match self.units {
                    0 => "Watts",
                    1 => "dBm",
                    2 => "dB",
                    3 => "REL",
                    _ => "Unknown",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.units, 0, "Watts");
                    ui.selectable_value(&mut self.units, 1, "dBm");
                    ui.selectable_value(&mut self.units, 2, "dB");
                    ui.selectable_value(&mut self.units, 3, "REL");
                });
        });

        ui.add_space(15.0);

        // Get current state from cache
        let current_power = data_cache.get(&format!("{}:power", self.instrument_id))
            .map_or(0.0, |dp| dp.value);

        // Power reading display
        ui.group(|ui| {
            ui.vertical_centered(|ui| {
                ui.label("Current Reading");
                ui.heading(egui::RichText::new(format!("{:.3e}", current_power))
                    .color(Color32::LIGHT_GREEN)
                    .size(24.0));
                ui.label(match self.units {
                    0 => "W",
                    1 => "dBm",
                    2 => "dB",
                    3 => "REL",
                    _ => "",
                });
            });
        });

        ui.add_space(10.0);

        // Zero button
        if ui.button("Zero / Dark Current").clicked() {
            let cmd = InstrumentCommand::Execute("zero".to_string(), vec![]);
            if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                error!("Failed to zero Newport 1830-C: {}", e);
            }
        }
    }
}

/// Elliptec rotation mount control panel
pub struct ElliptecControlPanel {
    pub instrument_id: String,
    pub device_addresses: Vec<u8>,
    pub target_positions: Vec<f64>,
}

impl ElliptecControlPanel {
    pub fn new(instrument_id: String, device_addresses: Vec<u8>) -> Self {
        let num_devices = device_addresses.len();
        Self {
            instrument_id,
            device_addresses,
            target_positions: vec![0.0; num_devices],
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp, data_cache: &HashMap<String, DataPoint>) {
        ui.heading("Elliptec Rotation Mounts");
        ui.separator();

        for (idx, &addr) in self.device_addresses.iter().enumerate() {
            ui.group(|ui| {
                ui.label(format!("Device {} (Address {})", idx, addr));

                let position = data_cache.get(&format!("{}:device{}_position", self.instrument_id, addr))
                    .map_or(0.0, |dp| dp.value);

                ui.horizontal(|ui| {
                    ui.label("Position:");
                    ui.colored_label(Color32::GREEN, format!("{:.2}°", position));
                });

                ui.horizontal(|ui| {
                    ui.label("Target:");
                    ui.add(egui::DragValue::new(&mut self.target_positions[idx])
                        .speed(1.0)
                        .range(0.0..=360.0)
                        .suffix("°"));

                    if ui.button("Move").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", addr),
                            self.target_positions[idx].to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move Elliptec device {}: {}", addr, e);
                        }
                    }
                });

                // Angle slider
                ui.add(Slider::new(&mut self.target_positions[idx], 0.0..=360.0)
                    .text("°"));

                // Preset buttons
                ui.horizontal(|ui| {
                    if ui.button("0°").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", addr),
                            "0".to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move Elliptec device {}: {}", addr, e);
                        }
                    }
                    if ui.button("45°").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", addr),
                            "45".to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move Elliptec device {}: {}", addr, e);
                        }
                    }
                    if ui.button("90°").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", addr),
                            "90".to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move Elliptec device {}: {}", addr, e);
                        }
                    }
                    if ui.button("180°").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", addr),
                            "180".to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move Elliptec device {}: {}", addr, e);
                        }
                    }
                });
            });

            ui.add_space(10.0);
        }

        // Home all button
        if ui.button("Home All Devices").clicked() {
            let cmd = InstrumentCommand::Execute("home".to_string(), vec![]);
            if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                error!("Failed to home Elliptec devices: {}", e);
            }
        }
    }
}

/// ESP300 motion controller panel
pub struct ESP300ControlPanel {
    pub instrument_id: String,
    pub num_axes: usize,
    pub target_positions: Vec<f64>,
    pub velocities: Vec<f64>,
}

impl ESP300ControlPanel {
    pub fn new(instrument_id: String, num_axes: usize) -> Self {
        Self {
            instrument_id,
            num_axes,
            target_positions: vec![0.0; num_axes],
            velocities: vec![5.0; num_axes],
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp, data_cache: &HashMap<String, DataPoint>) {
        ui.heading("ESP300 Motion Controller");
        ui.separator();

        for axis in 0..self.num_axes {
            ui.group(|ui| {
                ui.label(format!("Axis {} Control", axis + 1));

                let position = data_cache.get(&format!("{}:axis{}_position", self.instrument_id, axis + 1))
                    .map_or(0.0, |dp| dp.value);

                ui.horizontal(|ui| {
                    ui.label("Position:");
                    ui.colored_label(Color32::GREEN, format!("{:.3} mm", position));
                });

                ui.horizontal(|ui| {
                    ui.label("Target:");
                    ui.add(egui::DragValue::new(&mut self.target_positions[axis])
                        .speed(0.1)
                        .suffix(" mm"));

                    if ui.button("Move Abs").clicked() {
                        let cmd = InstrumentCommand::SetParameter(
                            format!("{}:position", axis + 1),
                            self.target_positions[axis].to_string()
                        );
                        if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                            error!("Failed to move ESP300 axis {}: {}", axis + 1, e);
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Velocity:");
                    ui.add(egui::DragValue::new(&mut self.velocities[axis])
                        .speed(0.1)
                        .range(0.1..=100.0)
                        .suffix(" mm/s"));
                });

                // Jog buttons
                ui.horizontal(|ui| {
                    if ui.button("◀◀ -10").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "-10".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                    if ui.button("◀ -1").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "-1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                    if ui.button("◀ -0.1").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "-0.1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                    if ui.button("+0.1 ▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "0.1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                    if ui.button("+1 ▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                    if ui.button("+10 ▶▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "10".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                    }
                });

                if ui.button("Stop").clicked() {
                    let cmd = InstrumentCommand::Execute(
                        "stop".to_string(),
                        vec![(axis + 1).to_string()]
                    );
                    let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                }
            });

            ui.add_space(10.0);
        }

        // Home all axes
        if ui.button("Home All Axes").clicked() {
            let cmd = InstrumentCommand::Execute("home".to_string(), vec![]);
            if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                error!("Failed to home ESP300 axes: {}", e);
            }
        }
    }
}

/// PVCAM camera control panel
pub struct PVCAMControlPanel {
    pub instrument_id: String,
    pub exposure_ms: f64,
    pub gain: i32,
    pub binning: (i32, i32),
}

impl PVCAMControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            exposure_ms: 100.0,
            gain: 1,
            binning: (1, 1),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp, data_cache: &HashMap<String, DataPoint>) {
        ui.heading("PVCAM Camera Control");
        ui.separator();

        // Exposure control
        ui.horizontal(|ui| {
            ui.label("Exposure:");
            ui.add(egui::DragValue::new(&mut self.exposure_ms)
                .speed(1.0)
                .range(1.0..=10000.0)
                .suffix(" ms"));
        });

        // Gain control
        ui.horizontal(|ui| {
            ui.label("Gain:");
            ui.add(Slider::new(&mut self.gain, 1..=16));
        });

        // Binning control
        ui.horizontal(|ui| {
            ui.label("Binning:");
            egui::ComboBox::from_id_salt("binning_combo")
                .selected_text(format!("{}x{}", self.binning.0, self.binning.1))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.binning, (1, 1), "1x1");
                    ui.selectable_value(&mut self.binning, (2, 2), "2x2");
                    ui.selectable_value(&mut self.binning, (4, 4), "4x4");
                });
        });

        ui.add_space(15.0);

        // Get current state from cache
        let acquiring = data_cache.get(&format!("{}:acquiring", self.instrument_id))
            .map_or(false, |dp| dp.value > 0.0);

        // Acquisition controls
        ui.horizontal(|ui| {
            let button_text = if acquiring { "Stop Acquisition" } else { "Start Acquisition" };
            let button_color = if acquiring { Color32::RED } else { Color32::GREEN };

            if ui.button(egui::RichText::new(button_text).color(button_color)).clicked() {
                let new_state = !acquiring;
                let cmd = if new_state {
                    InstrumentCommand::Execute("start_acquisition".to_string(), vec![])
                } else {
                    InstrumentCommand::Execute("stop_acquisition".to_string(), vec![])
                };
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle PVCAM acquisition: {}", e);
                }
            }

            if ui.button("Snap").clicked() {
                let cmd = InstrumentCommand::Execute("snap".to_string(), vec![]);
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to snap PVCAM frame: {}", e);
                }
            }
        });

        ui.add_space(10.0);

        // Status
        let status_text = if acquiring { "ACQUIRING" } else { "IDLE" };
        let status_color = if acquiring { Color32::GREEN } else { Color32::GRAY };
        ui.colored_label(status_color, status_text);
    }
}
