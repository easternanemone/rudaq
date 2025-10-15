//! Interactive instrument control panels
//!
//! This module provides interactive GUI controls for each instrument type
//! that can be docked in the main window.

use crate::{app::DaqApp, core::InstrumentCommand};
use egui::{Ui, Color32, Slider};
use log::error;

/// MaiTai laser control panel
pub struct MaiTaiControlPanel {
    pub instrument_id: String,
    pub target_wavelength: f64,
    pub shutter_open: bool,
    pub laser_on: bool,
    pub current_wavelength: f64,
    pub current_power: f64,
}

impl MaiTaiControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            target_wavelength: 800.0,
            shutter_open: false,
            laser_on: false,
            current_wavelength: 800.0,
            current_power: 0.0,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp) {
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
                } else {
                    self.current_wavelength = self.target_wavelength;
                }
            }
        });

        ui.add_space(10.0);

        // Current wavelength display
        ui.horizontal(|ui| {
            ui.label("Actual Wavelength:");
            ui.colored_label(Color32::GREEN, format!("{:.1} nm", self.current_wavelength));
        });

        ui.add_space(10.0);

        // Power display
        ui.horizontal(|ui| {
            ui.label("Output Power:");
            ui.colored_label(Color32::YELLOW, format!("{:.2} W", self.current_power));
        });

        ui.add_space(10.0);

        // Visual wavelength indicator
        ui.group(|ui| {
            ui.label("Tuning Range");
            ui.add(Slider::new(&mut self.target_wavelength, 700.0..=1000.0)
                .text("nm")
                .show_value(false));

            // Show current wavelength marker
            ui.label(format!("▼ {:.0} nm", self.current_wavelength));
        });

        ui.add_space(15.0);

        // Control buttons
        ui.horizontal(|ui| {
            // Shutter button
            let shutter_text = if self.shutter_open { "Close Shutter" } else { "Open Shutter" };
            let shutter_color = if self.shutter_open { Color32::GREEN } else { Color32::GRAY };
            if ui.button(egui::RichText::new(shutter_text).color(shutter_color)).clicked() {
                let new_state = !self.shutter_open;
                let cmd = InstrumentCommand::SetParameter(
                    "shutter".to_string(),
                    if new_state { "open" } else { "close" }.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle shutter: {}", e);
                } else {
                    self.shutter_open = new_state;
                }
            }

            ui.add_space(10.0);

            // Laser ON/OFF button
            let laser_text = if self.laser_on { "Laser ON" } else { "Laser OFF" };
            let laser_color = if self.laser_on { Color32::RED } else { Color32::DARK_GRAY };
            if ui.button(egui::RichText::new(laser_text).color(laser_color).size(16.0)).clicked() {
                let new_state = !self.laser_on;
                let cmd = InstrumentCommand::SetParameter(
                    "laser".to_string(),
                    if new_state { "on" } else { "off" }.to_string()
                );
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle laser: {}", e);
                } else {
                    self.laser_on = new_state;
                }
            }
        });

        ui.add_space(10.0);

        // System status
        ui.separator();
        let status_text = if self.laser_on {
            "System Status: ACTIVE"
        } else {
            "System Status: Ready to turn on"
        };
        let status_color = if self.laser_on { Color32::RED } else { Color32::LIGHT_GREEN };
        ui.colored_label(status_color, status_text);
    }
}

/// Newport 1830-C power meter control panel
pub struct Newport1830CControlPanel {
    pub instrument_id: String,
    pub wavelength: f64,
    pub range: i32,
    pub units: i32,
    pub current_power: f64,
}

impl Newport1830CControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            wavelength: 1550.0,
            range: 0, // autorange
            units: 0, // Watts
            current_power: 0.0,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp) {
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

        // Power reading display
        ui.group(|ui| {
            ui.vertical_centered(|ui| {
                ui.label("Current Reading");
                ui.heading(egui::RichText::new(format!("{:.3e}", self.current_power))
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
    pub positions: Vec<f64>,
    pub target_positions: Vec<f64>,
}

impl ElliptecControlPanel {
    pub fn new(instrument_id: String, device_addresses: Vec<u8>) -> Self {
        let num_devices = device_addresses.len();
        Self {
            instrument_id,
            device_addresses,
            positions: vec![0.0; num_devices],
            target_positions: vec![0.0; num_devices],
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp) {
        ui.heading("Elliptec Rotation Mounts");
        ui.separator();

        for (idx, &addr) in self.device_addresses.iter().enumerate() {
            ui.group(|ui| {
                ui.label(format!("Device {} (Address {})", idx, addr));

                ui.horizontal(|ui| {
                    ui.label("Position:");
                    ui.colored_label(Color32::GREEN, format!("{:.2}°", self.positions[idx]));
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
                        } else {
                            self.positions[idx] = self.target_positions[idx];
                        }
                    }
                });

                // Angle slider
                ui.add(Slider::new(&mut self.target_positions[idx], 0.0..=360.0)
                    .text("°"));

                // Preset buttons
                ui.horizontal(|ui| {
                    if ui.button("0°").clicked() {
                        self.target_positions[idx] = 0.0;
                    }
                    if ui.button("45°").clicked() {
                        self.target_positions[idx] = 45.0;
                    }
                    if ui.button("90°").clicked() {
                        self.target_positions[idx] = 90.0;
                    }
                    if ui.button("180°").clicked() {
                        self.target_positions[idx] = 180.0;
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
            } else {
                for pos in &mut self.positions {
                    *pos = 0.0;
                }
                for target in &mut self.target_positions {
                    *target = 0.0;
                }
            }
        }
    }
}

/// ESP300 motion controller panel
pub struct ESP300ControlPanel {
    pub instrument_id: String,
    pub num_axes: usize,
    pub positions: Vec<f64>,
    pub target_positions: Vec<f64>,
    pub velocities: Vec<f64>,
}

impl ESP300ControlPanel {
    pub fn new(instrument_id: String, num_axes: usize) -> Self {
        Self {
            instrument_id,
            num_axes,
            positions: vec![0.0; num_axes],
            target_positions: vec![0.0; num_axes],
            velocities: vec![5.0; num_axes],
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp) {
        ui.heading("ESP300 Motion Controller");
        ui.separator();

        for axis in 0..self.num_axes {
            ui.group(|ui| {
                ui.label(format!("Axis {} Control", axis + 1));

                ui.horizontal(|ui| {
                    ui.label("Position:");
                    ui.colored_label(Color32::GREEN, format!("{:.3} mm", self.positions[axis]));
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
                        } else {
                            self.positions[axis] = self.target_positions[axis];
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
                        self.target_positions[axis] -= 10.0;
                    }
                    if ui.button("◀ -1").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "-1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                        self.target_positions[axis] -= 1.0;
                    }
                    if ui.button("◀ -0.1").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "-0.1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                        self.target_positions[axis] -= 0.1;
                    }
                    if ui.button("+0.1 ▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "0.1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                        self.target_positions[axis] += 0.1;
                    }
                    if ui.button("+1 ▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "1".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                        self.target_positions[axis] += 1.0;
                    }
                    if ui.button("+10 ▶▶").clicked() {
                        let cmd = InstrumentCommand::Execute(
                            "move_relative".to_string(),
                            vec![(axis + 1).to_string(), "10".to_string()]
                        );
                        let _ = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd));
                        self.target_positions[axis] += 10.0;
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
            } else {
                for pos in &mut self.positions {
                    *pos = 0.0;
                }
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
    pub acquiring: bool,
}

impl PVCAMControlPanel {
    pub fn new(instrument_id: String) -> Self {
        Self {
            instrument_id,
            exposure_ms: 100.0,
            gain: 1,
            binning: (1, 1),
            acquiring: false,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, app: &DaqApp) {
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

        // Acquisition controls
        ui.horizontal(|ui| {
            let button_text = if self.acquiring { "Stop Acquisition" } else { "Start Acquisition" };
            let button_color = if self.acquiring { Color32::RED } else { Color32::GREEN };

            if ui.button(egui::RichText::new(button_text).color(button_color)).clicked() {
                let new_state = !self.acquiring;
                let cmd = if new_state {
                    InstrumentCommand::Execute("start_acquisition".to_string(), vec![])
                } else {
                    InstrumentCommand::Execute("stop_acquisition".to_string(), vec![])
                };
                if let Err(e) = app.with_inner(|inner| inner.send_instrument_command(&self.instrument_id, cmd)) {
                    error!("Failed to toggle PVCAM acquisition: {}", e);
                } else {
                    self.acquiring = new_state;
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
        let status_text = if self.acquiring { "ACQUIRING" } else { "IDLE" };
        let status_color = if self.acquiring { Color32::GREEN } else { Color32::GRAY };
        ui.colored_label(status_color, status_text);
    }
}
