//! Digital I/O Control Panel for Comedi DAQ devices.
//!
//! Provides per-pin direction control and state display/control for digital I/O ports.

use eframe::egui::{self, Color32, RichText, Ui};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use daq_client::DaqClient;

use super::DioDirection;

/// Action results from async operations.
#[derive(Debug)]
enum ActionResult {
    PinState { pin: u32, state: bool },
    PortState { port: u32, bits: u32 },
    WriteSuccess { pin: u32, state: bool },
    WriteError { pin: u32, error: String },
}

/// Per-pin configuration.
#[derive(Debug, Clone)]
struct PinConfig {
    direction: DioDirection,
    state: bool,
    label: String,
}

impl Default for PinConfig {
    fn default() -> Self {
        Self {
            direction: DioDirection::Input,
            state: false,
            label: String::new(),
        }
    }
}

/// Digital I/O Control Panel.
pub struct DigitalIOPanel {
    /// Device ID
    device_id: String,
    /// Number of DIO channels (24 for NI PCI-MIO-16XE-10)
    n_channels: u32,
    /// Per-pin configuration
    pins: Vec<PinConfig>,
    /// Auto-refresh for inputs
    auto_refresh: bool,
    /// Refresh interval
    refresh_interval_ms: u32,
    /// Last refresh time
    last_refresh: std::time::Instant,
    /// View mode
    view_mode: ViewMode,
    /// Selected port for port-wide operations
    selected_port: u32,
    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async channels
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
}

/// View mode for DIO panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewMode {
    #[default]
    Grid,
    List,
    Port,
}

impl Default for DigitalIOPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(64);
        let pins: Vec<PinConfig> = (0..24)
            .map(|i| PinConfig {
                label: format!("DIO{}", i),
                ..Default::default()
            })
            .collect();

        Self {
            device_id: String::from("comedi0"),
            n_channels: 24,
            pins,
            auto_refresh: false,
            refresh_interval_ms: 100,
            last_refresh: std::time::Instant::now(),
            view_mode: ViewMode::Grid,
            selected_port: 0,
            status: None,
            error: None,
            action_tx,
            action_rx,
        }
    }
}

impl DigitalIOPanel {
    /// Create a new panel.
    pub fn new(device_id: &str, n_channels: u32) -> Self {
        Self {
            device_id: device_id.to_string(),
            n_channels,
            pins: (0..n_channels)
                .map(|i| PinConfig {
                    label: format!("DIO{}", i),
                    ..Default::default()
                })
                .collect(),
            ..Self::default()
        }
    }

    /// Main UI entry point.
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        self.poll_results();

        // Auto-refresh logic
        if self.auto_refresh {
            let elapsed = self.last_refresh.elapsed();
            if elapsed.as_millis() >= self.refresh_interval_ms as u128 {
                self.read_all_inputs(runtime);
                self.last_refresh = std::time::Instant::now();
            }
            ui.ctx().request_repaint();
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("Digital I/O");
            ui.separator();
            ui.label(format!(
                "Device: {} ({} pins)",
                self.device_id, self.n_channels
            ));
        });

        ui.separator();

        // Status/error
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(Color32::RED));
        }
        if let Some(status) = &self.status {
            ui.label(RichText::new(status).color(Color32::GREEN));
        }

        ui.separator();

        // Control bar - capture button clicks
        let (read_all, all_inputs, all_outputs) = ui
            .horizontal(|ui| {
                // View mode selector
                ui.label("View:");
                ui.selectable_value(&mut self.view_mode, ViewMode::Grid, "Grid");
                ui.selectable_value(&mut self.view_mode, ViewMode::List, "List");
                ui.selectable_value(&mut self.view_mode, ViewMode::Port, "Port");

                ui.separator();

                ui.checkbox(&mut self.auto_refresh, "Auto-refresh");
                if self.auto_refresh {
                    ui.add(
                        egui::DragValue::new(&mut self.refresh_interval_ms)
                            .range(50..=1000)
                            .suffix(" ms"),
                    );
                }

                ui.separator();

                let read = ui.button("Read All").clicked();
                let inputs = ui.button("All Inputs").clicked();
                let outputs = ui.button("All Outputs").clicked();
                (read, inputs, outputs)
            })
            .inner;

        if read_all {
            self.read_all_inputs(runtime);
        }
        if all_inputs {
            self.configure_all_as_inputs();
        }
        if all_outputs {
            self.configure_all_as_outputs();
        }

        ui.separator();

        // Main content based on view mode
        match self.view_mode {
            ViewMode::Grid => self.render_grid_view(ui, runtime),
            ViewMode::List => self.render_list_view(ui, runtime),
            ViewMode::Port => self.render_port_view(ui, runtime),
        }
    }

    /// Render grid view (8x3 grid for 24 pins).
    fn render_grid_view(&mut self, ui: &mut Ui, runtime: &Runtime) {
        // Collect actions to execute after UI rendering
        let mut write_actions: Vec<(u32, bool)> = Vec::new();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("dio_grid")
                .num_columns(8)
                .spacing([4.0, 4.0])
                .show(ui, |ui| {
                    for i in 0..self.pins.len() {
                        // Copy values to avoid borrow conflicts
                        let pin_state = self.pins[i].state;
                        let pin_direction = self.pins[i].direction;
                        let pin_num = i as u32;

                        ui.vertical(|ui| {
                            ui.set_min_width(50.0);

                            // Pin label
                            ui.label(RichText::new(format!("{}", pin_num)).small());

                            // LED indicator / toggle button
                            let color = if pin_state {
                                Color32::GREEN
                            } else {
                                Color32::DARK_GRAY
                            };

                            let response = ui.add(
                                egui::Button::new(
                                    RichText::new(if pin_state { "●" } else { "○" })
                                        .color(color)
                                        .size(16.0),
                                )
                                .min_size(egui::vec2(30.0, 24.0)),
                            );

                            // Click to toggle (only for outputs)
                            if response.clicked() && pin_direction == DioDirection::Output {
                                let new_state = !self.pins[i].state;
                                self.pins[i].state = new_state;
                                write_actions.push((pin_num, new_state));
                            }

                            // Direction indicator
                            let dir_text = match pin_direction {
                                DioDirection::Input => "In",
                                DioDirection::Output => "Out",
                            };
                            let dir_color = match pin_direction {
                                DioDirection::Input => Color32::LIGHT_BLUE,
                                DioDirection::Output => Color32::LIGHT_GREEN,
                            };

                            if ui
                                .add(egui::Button::new(
                                    RichText::new(dir_text).color(dir_color).small(),
                                ))
                                .clicked()
                            {
                                // Toggle direction
                                self.pins[i].direction = match self.pins[i].direction {
                                    DioDirection::Input => DioDirection::Output,
                                    DioDirection::Output => DioDirection::Input,
                                };
                            }
                        });

                        if (i + 1) % 8 == 0 {
                            ui.end_row();
                        }
                    }
                });
        });

        // Execute deferred writes
        for (pin, state) in write_actions {
            self.write_pin(pin, state, runtime);
        }
    }

    /// Render list view.
    fn render_list_view(&mut self, ui: &mut Ui, runtime: &Runtime) {
        // Collect actions to execute after UI rendering
        let mut write_actions: Vec<(u32, bool)> = Vec::new();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for i in 0..self.pins.len() {
                ui.horizontal(|ui| {
                    // Pin number
                    ui.label(format!("DIO{:02}", i));

                    // Direction combo
                    let current_dir = self.pins[i].direction;
                    egui::ComboBox::from_id_salt(format!("dio_dir_{}", i))
                        .selected_text(current_dir.label())
                        .width(60.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.pins[i].direction,
                                DioDirection::Input,
                                "Input",
                            );
                            ui.selectable_value(
                                &mut self.pins[i].direction,
                                DioDirection::Output,
                                "Output",
                            );
                        });

                    // State indicator/control
                    let state = self.pins[i].state;
                    let direction = self.pins[i].direction;
                    let state_text = if state { "HIGH" } else { "LOW" };
                    let state_color = if state { Color32::GREEN } else { Color32::GRAY };

                    if direction == DioDirection::Output {
                        if ui
                            .button(RichText::new(state_text).color(state_color))
                            .clicked()
                        {
                            let new_state = !self.pins[i].state;
                            self.pins[i].state = new_state;
                            write_actions.push((i as u32, new_state));
                        }
                    } else {
                        ui.label(RichText::new(state_text).color(state_color));
                    }

                    // Custom label
                    ui.text_edit_singleline(&mut self.pins[i].label);
                });
            }
        });

        // Execute deferred writes
        for (pin, state) in write_actions {
            self.write_pin(pin, state, runtime);
        }
    }

    /// Render port view (8-bit ports).
    fn render_port_view(&mut self, ui: &mut Ui, runtime: &Runtime) {
        let n_ports = self.n_channels.div_ceil(8);
        let mut write_actions: Vec<(u32, bool)> = Vec::new();

        ui.horizontal(|ui| {
            ui.label("Port:");
            for port in 0..n_ports {
                ui.selectable_value(&mut self.selected_port, port, format!("P{}", port));
            }
        });

        ui.separator();

        let port = self.selected_port;
        let start_pin = port * 8;
        let end_pin = ((port + 1) * 8).min(self.n_channels);

        // Port value display
        let mut port_value: u32 = 0;
        for i in start_pin..end_pin {
            if self.pins[i as usize].state {
                port_value |= 1 << (i - start_pin);
            }
        }

        ui.horizontal(|ui| {
            ui.label("Port Value:");
            ui.label(
                RichText::new(format!("0x{:02X} ({})", port_value, port_value))
                    .monospace()
                    .size(16.0),
            );
        });

        // Binary display
        ui.horizontal(|ui| {
            ui.label("Binary:");
            for i in (start_pin..end_pin).rev() {
                let bit = if self.pins[i as usize].state {
                    "1"
                } else {
                    "0"
                };
                let color = if self.pins[i as usize].state {
                    Color32::GREEN
                } else {
                    Color32::GRAY
                };
                ui.label(RichText::new(bit).color(color).monospace());
            }
        });

        ui.separator();

        // Individual pin controls for this port
        ui.horizontal(|ui| {
            for i in start_pin..end_pin {
                let i_usize = i as usize;
                let pin_state = self.pins[i_usize].state;
                let pin_dir = self.pins[i_usize].direction;

                ui.vertical(|ui| {
                    ui.label(format!("{}", i));

                    let color = if pin_state {
                        Color32::GREEN
                    } else {
                        Color32::DARK_GRAY
                    };

                    if ui
                        .add(
                            egui::Button::new(RichText::new("●").color(color).size(20.0))
                                .min_size(egui::vec2(30.0, 30.0)),
                        )
                        .clicked()
                        && pin_dir == DioDirection::Output
                    {
                        let new_state = !self.pins[i_usize].state;
                        self.pins[i_usize].state = new_state;
                        write_actions.push((i, new_state));
                    }
                });
            }
        });

        ui.separator();

        // Port-wide operations - capture clicks and handle after
        let (set_high, set_low, toggle_all) = ui
            .horizontal(|ui| {
                (
                    ui.button("Set All High").clicked(),
                    ui.button("Set All Low").clicked(),
                    ui.button("Toggle All").clicked(),
                )
            })
            .inner;

        if set_high {
            for i in start_pin..end_pin {
                let i_usize = i as usize;
                if self.pins[i_usize].direction == DioDirection::Output {
                    self.pins[i_usize].state = true;
                    write_actions.push((i, true));
                }
            }
        }
        if set_low {
            for i in start_pin..end_pin {
                let i_usize = i as usize;
                if self.pins[i_usize].direction == DioDirection::Output {
                    self.pins[i_usize].state = false;
                    write_actions.push((i, false));
                }
            }
        }
        if toggle_all {
            for i in start_pin..end_pin {
                let i_usize = i as usize;
                if self.pins[i_usize].direction == DioDirection::Output {
                    let new_state = !self.pins[i_usize].state;
                    self.pins[i_usize].state = new_state;
                    write_actions.push((i, new_state));
                }
            }
        }

        // Execute deferred writes
        for (pin, state) in write_actions {
            self.write_pin(pin, state, runtime);
        }
    }

    // Async operations
    fn write_pin(&self, pin: u32, state: bool, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let _ = tx.send(ActionResult::WriteSuccess { pin, state }).await;
        });
    }

    fn read_all_inputs(&self, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        let n_pins = self.n_channels;
        runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            // Simulate reading - replace with actual gRPC call
            for pin in 0..n_pins {
                let state = (pin % 3) == 0; // Simulated pattern
                let _ = tx.send(ActionResult::PinState { pin, state }).await;
            }
        });
    }

    fn configure_all_as_inputs(&mut self) {
        for pin in &mut self.pins {
            pin.direction = DioDirection::Input;
        }
        self.status = Some("All pins configured as inputs".to_string());
    }

    fn configure_all_as_outputs(&mut self) {
        for pin in &mut self.pins {
            pin.direction = DioDirection::Output;
        }
        self.status = Some("All pins configured as outputs".to_string());
    }

    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            match result {
                ActionResult::PinState { pin, state } => {
                    if let Some(p) = self.pins.get_mut(pin as usize) {
                        p.state = state;
                    }
                }
                ActionResult::PortState { port, bits } => {
                    let start = port * 8;
                    for i in 0..8 {
                        if let Some(p) = self.pins.get_mut((start + i) as usize) {
                            p.state = (bits >> i) & 1 != 0;
                        }
                    }
                }
                ActionResult::WriteSuccess { pin, state } => {
                    self.status = Some(format!(
                        "DIO{}: {}",
                        pin,
                        if state { "HIGH" } else { "LOW" }
                    ));
                    self.error = None;
                }
                ActionResult::WriteError { pin, error } => {
                    self.error = Some(format!("DIO{}: {}", pin, error));
                }
            }
        }
    }
}
