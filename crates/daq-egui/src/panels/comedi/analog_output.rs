//! Analog Output Control Panel for Comedi DAQ devices.
//!
//! Provides DAC control with voltage sliders and optional waveform generation.

use eframe::egui::{self, Color32, RichText, Ui};
use serde_json::json;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use daq_client::DaqClient;

use super::NI_VOLTAGE_RANGES;

/// Waveform types for function generator mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaveformType {
    #[default]
    DC,
    Sine,
    Square,
    Triangle,
    Sawtooth,
}

impl WaveformType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DC => "DC",
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::DC,
            Self::Sine,
            Self::Square,
            Self::Triangle,
            Self::Sawtooth,
        ]
    }
}

/// Action results from async operations.
#[derive(Debug)]
enum ActionResult {
    WriteSuccess { channel: u32, voltage: f64 },
    WriteError { channel: u32, error: String },
    ZeroAllSuccess,
}

/// Per-channel output state.
#[derive(Debug, Clone)]
struct ChannelState {
    /// Current voltage setpoint
    voltage: f64,
    /// Voltage range index
    range_index: usize,
    /// Whether output is active
    output_enabled: bool,
    /// Waveform type (for function generator mode)
    waveform: WaveformType,
    /// Waveform frequency in Hz
    frequency: f64,
    /// Waveform amplitude (peak-to-peak)
    amplitude: f64,
    /// Waveform DC offset
    offset: f64,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            voltage: 0.0,
            range_index: 0,
            output_enabled: false,
            waveform: WaveformType::DC,
            frequency: 1.0,
            amplitude: 1.0,
            offset: 0.0,
        }
    }
}

/// Analog Output Control Panel.
pub struct AnalogOutputPanel {
    /// Device ID
    device_id: String,
    /// Number of channels (2 for NI PCI-MIO-16XE-10)
    n_channels: u32,
    /// Per-channel state
    channels: Vec<ChannelState>,
    /// Selected channel for waveform config
    selected_channel: usize,
    /// Function generator mode enabled
    funcgen_mode: bool,
    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async action sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action receiver
    action_rx: mpsc::Receiver<ActionResult>,
}

impl Default for AnalogOutputPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);

        Self {
            device_id: String::from("comedi0"),
            n_channels: 2,
            channels: vec![ChannelState::default(); 2],
            selected_channel: 0,
            funcgen_mode: false,
            status: None,
            error: None,
            action_tx,
            action_rx,
        }
    }
}

impl AnalogOutputPanel {
    /// Create a new panel for a specific device.
    pub fn new(device_id: &str, n_channels: u32) -> Self {
        Self {
            device_id: device_id.to_string(),
            n_channels,
            channels: vec![ChannelState::default(); n_channels as usize],
            ..Self::default()
        }
    }

    /// Main UI entry point.
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        self.poll_results();

        // Header
        ui.horizontal(|ui| {
            ui.heading("Analog Output");
            ui.separator();
            ui.label(format!("Device: {}", self.device_id));
        });

        ui.separator();

        // Status/error
        if let Some(error) = &self.error {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Error:").color(Color32::RED));
                ui.label(RichText::new(error).color(Color32::RED));
            });
        }
        if let Some(status) = &self.status {
            ui.label(RichText::new(status).color(Color32::GREEN));
        }

        ui.separator();

        // Control bar
        let zero_clicked = ui
            .horizontal(|ui| {
                ui.checkbox(&mut self.funcgen_mode, "Function Generator Mode");
                ui.separator();
                ui.button("Zero All Outputs").clicked()
            })
            .inner;

        if zero_clicked {
            self.zero_all_outputs(runtime, client.as_deref().cloned());
        }

        ui.separator();

        // Channel controls
        for ch in 0..self.n_channels as usize {
            self.render_channel_control(ui, ch, runtime, client.as_deref().cloned());
            ui.separator();
        }

        // Waveform config (if in funcgen mode)
        if self.funcgen_mode {
            ui.separator();
            self.render_waveform_config(ui);
        }
    }

    /// Render control for a single channel.
    fn render_channel_control(
        &mut self,
        ui: &mut Ui,
        ch: usize,
        runtime: &Runtime,
        client: Option<DaqClient>,
    ) {
        let range = NI_VOLTAGE_RANGES[self.channels[ch].range_index];

        // Collect actions to perform after UI rendering to avoid borrow conflicts
        let mut actions: Vec<(u32, f64)> = Vec::new();

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Channel {}", ch)).strong());

                // Enable toggle
                let was_enabled = self.channels[ch].output_enabled;
                ui.checkbox(&mut self.channels[ch].output_enabled, "Output");

                // If just enabled, queue write of current voltage
                if self.channels[ch].output_enabled && !was_enabled {
                    actions.push((ch as u32, self.channels[ch].voltage));
                }
            });

            ui.horizontal(|ui| {
                // Voltage slider
                ui.label("Voltage:");

                let mut voltage = self.channels[ch].voltage;
                let slider = egui::Slider::new(&mut voltage, range.min..=range.max)
                    .suffix(" V")
                    .clamping(egui::SliderClamping::Always);

                if ui.add(slider).changed() {
                    self.channels[ch].voltage = voltage;
                    if self.channels[ch].output_enabled {
                        actions.push((ch as u32, voltage));
                    }
                }

                // Numeric input
                let mut voltage_drag = self.channels[ch].voltage;
                if ui
                    .add(
                        egui::DragValue::new(&mut voltage_drag)
                            .range(range.min..=range.max)
                            .speed(0.01)
                            .suffix(" V"),
                    )
                    .changed()
                {
                    self.channels[ch].voltage = voltage_drag;
                }
            });

            ui.horizontal(|ui| {
                // Range selector
                ui.label("Range:");
                egui::ComboBox::from_id_salt(format!("ao_range_{}", ch))
                    .selected_text(range.label())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (idx, r) in NI_VOLTAGE_RANGES.iter().enumerate() {
                            if ui
                                .selectable_value(
                                    &mut self.channels[ch].range_index,
                                    idx,
                                    r.label(),
                                )
                                .clicked()
                            {
                                // Clamp voltage to new range
                                self.channels[ch].voltage =
                                    self.channels[ch].voltage.clamp(r.min, r.max);
                            }
                        }
                    });

                // Quick voltage buttons
                ui.separator();
                if ui.button("0V").clicked() {
                    self.channels[ch].voltage = 0.0;
                    if self.channels[ch].output_enabled {
                        actions.push((ch as u32, 0.0));
                    }
                }
                if ui.button("Max").clicked() {
                    self.channels[ch].voltage = range.max;
                    if self.channels[ch].output_enabled {
                        actions.push((ch as u32, range.max));
                    }
                }
                if ui.button("Min").clicked() {
                    self.channels[ch].voltage = range.min;
                    if self.channels[ch].output_enabled {
                        actions.push((ch as u32, range.min));
                    }
                }
            });
        });

        // Execute queued voltage writes after UI is done
        for (channel, voltage) in actions {
            self.write_voltage(channel, voltage, runtime, client.clone());
        }
    }

    /// Render waveform configuration panel.
    fn render_waveform_config(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Waveform Generator").strong());

            ui.horizontal(|ui| {
                ui.label("Channel:");
                for ch in 0..self.n_channels as usize {
                    ui.selectable_value(&mut self.selected_channel, ch, format!("CH{}", ch));
                }
            });

            let state = &mut self.channels[self.selected_channel];

            ui.horizontal(|ui| {
                ui.label("Waveform:");
                egui::ComboBox::from_id_salt("waveform_type")
                    .selected_text(state.waveform.label())
                    .show_ui(ui, |ui| {
                        for wf in WaveformType::all() {
                            ui.selectable_value(&mut state.waveform, *wf, wf.label());
                        }
                    });
            });

            if state.waveform != WaveformType::DC {
                ui.horizontal(|ui| {
                    ui.label("Frequency:");
                    ui.add(
                        egui::DragValue::new(&mut state.frequency)
                            .range(0.1..=10000.0)
                            .speed(1.0)
                            .suffix(" Hz"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Amplitude:");
                    ui.add(
                        egui::DragValue::new(&mut state.amplitude)
                            .range(0.0..=20.0)
                            .speed(0.1)
                            .suffix(" Vpp"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Offset:");
                    ui.add(
                        egui::DragValue::new(&mut state.offset)
                            .range(-10.0..=10.0)
                            .speed(0.1)
                            .suffix(" V"),
                    );
                });

                ui.horizontal(|ui| {
                    if ui.button("Start Waveform").clicked() {
                        // TODO: Start waveform generation
                        self.status = Some("Waveform generation not yet implemented".to_string());
                    }
                    if ui.button("Stop").clicked() {
                        // TODO: Stop waveform generation
                    }
                });
            }
        });
    }

    /// Write voltage to a channel.
    fn write_voltage(
        &self,
        channel: u32,
        voltage: f64,
        runtime: &Runtime,
        client: Option<DaqClient>,
    ) {
        let tx = self.action_tx.clone();
        let device_id = self.device_id.clone();

        runtime.spawn(async move {
            if let Some(mut client) = client {
                let args = json!({
                    "channel": channel,
                    "voltage": voltage
                })
                .to_string();

                match client
                    .execute_device_command(&device_id, "write_voltage", &args)
                    .await
                {
                    Ok(_) => {
                        let _ = tx
                            .send(ActionResult::WriteSuccess { channel, voltage })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(ActionResult::WriteError {
                                channel,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }
            } else {
                // Simulation
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                let _ = tx
                    .send(ActionResult::WriteSuccess { channel, voltage })
                    .await;
            }
        });
    }

    /// Zero all outputs.
    fn zero_all_outputs(&mut self, runtime: &Runtime, client: Option<DaqClient>) {
        for ch in 0..self.n_channels as usize {
            self.channels[ch].voltage = 0.0;
            self.write_voltage(ch as u32, 0.0, runtime, client.clone());
        }

        let tx = self.action_tx.clone();
        runtime.spawn(async move {
            let _ = tx.send(ActionResult::ZeroAllSuccess).await;
        });
    }

    /// Poll for async results.
    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            match result {
                ActionResult::WriteSuccess { channel, voltage } => {
                    self.status = Some(format!("CH{}: Set to {:.3}V", channel, voltage));
                    self.error = None;
                }
                ActionResult::WriteError { channel, error } => {
                    self.error = Some(format!("CH{}: {}", channel, error));
                }
                ActionResult::ZeroAllSuccess => {
                    self.status = Some("All outputs set to 0V".to_string());
                    self.error = None;
                }
            }
        }
    }
}
