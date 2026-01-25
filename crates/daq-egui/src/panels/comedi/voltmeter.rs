//! Digital Voltmeter Display Panel for Comedi DAQ devices.
//!
//! Provides DMM-style voltage readout with large numeric display,
//! unit selection, statistics, and bar graph visualization.

use eframe::egui::{self, Color32, RichText, Ui};
use futures::StreamExt;
use std::collections::VecDeque;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;

/// Maximum history for statistics
const MAX_HISTORY: usize = 1000;

/// Reading pushed to the voltmeter
#[derive(Debug, Clone)]
pub struct VoltmeterReading {
    pub channel: u32,
    pub voltage: f64,
    pub timestamp: Option<f64>,
}

impl VoltmeterReading {
    pub fn new(channel: u32, voltage: f64) -> Self {
        Self {
            channel,
            voltage,
            timestamp: None,
        }
    }
}

/// Sender for voltmeter readings
pub type VoltmeterSender = mpsc::Sender<VoltmeterReading>;
/// Receiver for voltmeter readings
pub type VoltmeterReceiver = mpsc::Receiver<VoltmeterReading>;

/// Create channel pair for voltmeter
pub fn voltmeter_channel() -> (VoltmeterSender, VoltmeterReceiver) {
    mpsc::channel(256)
}

/// Display unit for voltage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoltageUnit {
    MicroVolts,
    MilliVolts,
    #[default]
    Volts,
    Auto,
}

impl VoltageUnit {
    pub fn label(&self) -> &'static str {
        match self {
            Self::MicroVolts => "uV",
            Self::MilliVolts => "mV",
            Self::Volts => "V",
            Self::Auto => "Auto",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Auto, Self::Volts, Self::MilliVolts, Self::MicroVolts]
    }

    /// Convert voltage to display value and unit string
    pub fn format(&self, voltage: f64) -> (f64, &'static str) {
        match self {
            Self::MicroVolts => (voltage * 1_000_000.0, "uV"),
            Self::MilliVolts => (voltage * 1000.0, "mV"),
            Self::Volts => (voltage, "V"),
            Self::Auto => {
                let abs = voltage.abs();
                if abs < 0.001 {
                    (voltage * 1_000_000.0, "uV")
                } else if abs < 1.0 {
                    (voltage * 1000.0, "mV")
                } else {
                    (voltage, "V")
                }
            }
        }
    }
}

/// Display mode for the voltmeter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    #[default]
    DC,
    AC,
    ACPlusDC,
    Peak,
    PeakToPeak,
}

impl DisplayMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DC => "DC",
            Self::AC => "AC",
            Self::ACPlusDC => "AC+DC",
            Self::Peak => "Peak",
            Self::PeakToPeak => "Pk-Pk",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::DC,
            Self::AC,
            Self::ACPlusDC,
            Self::Peak,
            Self::PeakToPeak,
        ]
    }
}

/// Per-channel state
#[derive(Debug, Clone)]
struct ChannelState {
    current_voltage: f64,
    history: VecDeque<f64>,
    min: f64,
    max: f64,
    sum: f64,
    sum_sq: f64,
    count: usize,
    last_update: Instant,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            current_voltage: 0.0,
            history: VecDeque::with_capacity(MAX_HISTORY),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            sum: 0.0,
            sum_sq: 0.0,
            count: 0,
            last_update: Instant::now(),
        }
    }
}

impl ChannelState {
    fn push(&mut self, voltage: f64) {
        self.current_voltage = voltage;
        self.history.push_back(voltage);

        // Update running statistics
        self.min = self.min.min(voltage);
        self.max = self.max.max(voltage);
        self.sum += voltage;
        self.sum_sq += voltage * voltage;
        self.count += 1;
        self.last_update = Instant::now();

        // Trim history
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    fn reset_stats(&mut self) {
        self.min = f64::INFINITY;
        self.max = f64::NEG_INFINITY;
        self.sum = 0.0;
        self.sum_sq = 0.0;
        self.count = 0;
        self.history.clear();
    }

    fn mean(&self) -> f64 {
        if self.count > 0 {
            self.sum / self.count as f64
        } else {
            0.0
        }
    }

    fn std_dev(&self) -> f64 {
        if self.count > 1 {
            let mean = self.mean();
            let variance = (self.sum_sq / self.count as f64) - (mean * mean);
            variance.max(0.0).sqrt()
        } else {
            0.0
        }
    }

    fn pk_pk(&self) -> f64 {
        if self.min.is_finite() && self.max.is_finite() {
            self.max - self.min
        } else {
            0.0
        }
    }

    /// Compute AC (RMS of AC component)
    fn ac_rms(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }
        let mean = self.mean();
        let sum_sq: f64 = self.history.iter().map(|v| (v - mean).powi(2)).sum();
        (sum_sq / self.history.len() as f64).sqrt()
    }
}

/// Streaming state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

/// Handle for aborting the streaming task
struct StreamingHandle {
    abort_tx: mpsc::Sender<()>,
}

impl StreamingHandle {
    fn new(abort_tx: mpsc::Sender<()>) -> Self {
        Self { abort_tx }
    }

    async fn abort(&self) {
        let _ = self.abort_tx.send(()).await;
    }
}

/// Digital Voltmeter Display Panel
pub struct VoltmeterPanel {
    /// Start time for relative timestamps
    start_time: Instant,
    /// Per-channel state
    channels: Vec<ChannelState>,
    /// Selected channel
    selected_channel: u32,
    /// Reading receiver
    reading_rx: VoltmeterReceiver,
    /// Reading sender (for cloning)
    reading_tx: VoltmeterSender,
    /// Display unit
    unit: VoltageUnit,
    /// Display mode
    mode: DisplayMode,
    /// Number of decimal places
    decimals: usize,
    /// Show bar graph
    show_bar: bool,
    /// Bar graph range (min, max)
    bar_range: (f64, f64),
    /// Show statistics
    show_stats: bool,
    /// Auto-ranging enabled
    auto_range: bool,
    /// Hold display (freeze reading)
    hold: bool,
    /// Relative mode (subtract reference)
    relative_mode: bool,
    /// Relative reference value
    relative_ref: f64,
    /// Streaming state
    streaming_state: StreamingState,
    streaming_handle: Option<StreamingHandle>,
    streaming_device_id: Option<String>,
    streaming_error: Option<String>,
    /// Sample rate for streaming (Hz)
    stream_sample_rate: f64,
}

impl Default for VoltmeterPanel {
    fn default() -> Self {
        let (tx, rx) = voltmeter_channel();

        Self {
            start_time: Instant::now(),
            channels: (0..16).map(|_| ChannelState::default()).collect(),
            selected_channel: 0,
            reading_rx: rx,
            reading_tx: tx,
            unit: VoltageUnit::Auto,
            mode: DisplayMode::DC,
            decimals: 4,
            show_bar: true,
            bar_range: (-10.0, 10.0),
            show_stats: true,
            auto_range: true,
            hold: false,
            relative_mode: false,
            relative_ref: 0.0,
            streaming_state: StreamingState::Stopped,
            streaming_handle: None,
            streaming_device_id: None,
            streaming_error: None,
            stream_sample_rate: 100.0, // 100 Hz default for voltmeter
        }
    }
}

impl VoltmeterPanel {
    /// Create a new voltmeter panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Get sender for pushing readings
    pub fn get_sender(&self) -> VoltmeterSender {
        self.reading_tx.clone()
    }

    /// Start streaming from a hardware device via gRPC
    pub fn start_streaming(
        &mut self,
        _client: DaqClient,
        runtime: &Runtime,
        device_id: String,
        channel: u32,
        sample_rate_hz: f64,
    ) {
        if self.streaming_state != StreamingState::Stopped {
            self.streaming_error = Some("Already streaming".to_string());
            return;
        }

        self.streaming_state = StreamingState::Starting;
        self.streaming_device_id = Some(device_id.clone());
        self.streaming_error = None;
        self.selected_channel = channel;

        let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);
        let reading_tx = self.reading_tx.clone();

        // Spawn background task to handle streaming
        runtime.spawn(async move {
            use daq_proto::ni_daq::{
                ni_daq_service_client::NiDaqServiceClient, StreamAnalogInputRequest,
            };
            use tonic::transport::Channel;

            let channel_result = Channel::from_static("http://127.0.0.1:50051")
                .connect()
                .await;

            let mut ni_daq_client = match channel_result {
                Ok(channel) => NiDaqServiceClient::new(channel),
                Err(e) => {
                    tracing::error!("Failed to connect NI DAQ client: {}", e);
                    return;
                }
            };

            let request = StreamAnalogInputRequest {
                device_id: device_id.clone(),
                channels: vec![channel],
                sample_rate_hz,
                range_index: 0,
                stop_condition: Some(
                    daq_proto::ni_daq::stream_analog_input_request::StopCondition::Continuous(true),
                ),
                buffer_size: 256,
            };

            let mut stream = match ni_daq_client.stream_analog_input(request).await {
                Ok(response) => response.into_inner(),
                Err(e) => {
                    tracing::error!("Failed to start streaming: {}", e);
                    return;
                }
            };

            tracing::info!(
                "Voltmeter streaming from device={} ch={}",
                device_id,
                channel
            );

            loop {
                tokio::select! {
                    _ = abort_rx.recv() => {
                        tracing::info!("Voltmeter streaming aborted");
                        break;
                    }
                    message = stream.next() => {
                        match message {
                            Some(Ok(data)) => {
                                // Average all voltages in this batch for stability
                                if !data.voltages.is_empty() {
                                    let avg_voltage: f64 = data.voltages.iter().sum::<f64>()
                                        / data.voltages.len() as f64;
                                    let reading = VoltmeterReading::new(channel, avg_voltage);
                                    let _ = reading_tx.try_send(reading);
                                }
                            }
                            Some(Err(e)) => {
                                tracing::error!("Voltmeter stream error: {}", e);
                                break;
                            }
                            None => {
                                tracing::info!("Voltmeter stream ended");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.streaming_handle = Some(StreamingHandle::new(abort_tx));
        self.streaming_state = StreamingState::Running;
    }

    /// Stop streaming
    pub fn stop_streaming(&mut self, runtime: &Runtime) {
        if self.streaming_state != StreamingState::Running {
            return;
        }

        self.streaming_state = StreamingState::Stopping;

        if let Some(handle) = self.streaming_handle.take() {
            runtime.spawn(async move {
                handle.abort().await;
            });
        }

        self.streaming_state = StreamingState::Stopped;
        self.streaming_device_id = None;
    }

    /// Drain pending readings
    fn drain_readings(&mut self) {
        while let Ok(reading) = self.reading_rx.try_recv() {
            if self.hold {
                continue; // Drain but don't update when held
            }

            if let Some(channel) = self.channels.get_mut(reading.channel as usize) {
                channel.push(reading.voltage);
            }
        }
    }

    /// Get the display value based on mode
    fn display_value(&self) -> f64 {
        let channel = &self.channels[self.selected_channel as usize];

        let raw = match self.mode {
            DisplayMode::DC => channel.current_voltage,
            DisplayMode::AC => channel.ac_rms(),
            DisplayMode::ACPlusDC => {
                let dc = channel.mean();
                let ac = channel.ac_rms();
                (dc * dc + ac * ac).sqrt()
            }
            DisplayMode::Peak => channel.max.max(channel.min.abs()),
            DisplayMode::PeakToPeak => channel.pk_pk(),
        };

        if self.relative_mode {
            raw - self.relative_ref
        } else {
            raw
        }
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        self.ui_with_client(ui, None, None, None);
    }

    /// UI with optional client for streaming
    pub fn ui_with_client(
        &mut self,
        ui: &mut Ui,
        client: Option<&mut DaqClient>,
        runtime: Option<&Runtime>,
        device_id: Option<&str>,
    ) {
        // Drain pending readings
        self.drain_readings();

        // Header
        ui.horizontal(|ui| {
            ui.heading("Digital Voltmeter");
            ui.separator();

            // Channel selector
            ui.label("Channel:");
            egui::ComboBox::from_id_salt("vm_channel")
                .selected_text(format!("CH{}", self.selected_channel))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for i in 0..16u32 {
                        ui.selectable_value(&mut self.selected_channel, i, format!("CH{}", i));
                    }
                });

            ui.separator();

            // Streaming controls
            match self.streaming_state {
                StreamingState::Stopped => {
                    if ui.button("▶ Start").clicked() {
                        if let (Some(client), Some(runtime), Some(device_id)) =
                            (client, runtime, device_id)
                        {
                            self.start_streaming(
                                client.clone(),
                                runtime,
                                device_id.to_string(),
                                self.selected_channel,
                                self.stream_sample_rate,
                            );
                        } else {
                            self.streaming_error = Some("No DAQ connection".to_string());
                        }
                    }
                }
                StreamingState::Running => {
                    if ui.button("⏹ Stop").clicked() {
                        if let Some(runtime) = runtime {
                            self.stop_streaming(runtime);
                        }
                    }
                    ui.label(RichText::new("● LIVE").color(Color32::GREEN));
                }
                StreamingState::Starting => {
                    ui.spinner();
                    ui.label("Starting...");
                }
                StreamingState::Stopping => {
                    ui.spinner();
                    ui.label("Stopping...");
                }
            }
        });

        // Show streaming error if present
        if let Some(ref error) = self.streaming_error {
            ui.colored_label(Color32::RED, format!("Error: {}", error));
        }

        ui.separator();

        // Control bar
        ui.horizontal(|ui| {
            // Mode selector
            ui.label("Mode:");
            egui::ComboBox::from_id_salt("vm_mode")
                .selected_text(self.mode.label())
                .width(60.0)
                .show_ui(ui, |ui| {
                    for mode in DisplayMode::all() {
                        ui.selectable_value(&mut self.mode, *mode, mode.label());
                    }
                });

            ui.separator();

            // Unit selector
            ui.label("Unit:");
            egui::ComboBox::from_id_salt("vm_unit")
                .selected_text(self.unit.label())
                .width(60.0)
                .show_ui(ui, |ui| {
                    for unit in VoltageUnit::all() {
                        ui.selectable_value(&mut self.unit, *unit, unit.label());
                    }
                });

            ui.separator();

            // Hold button
            let hold_text = if self.hold { "Release" } else { "Hold" };
            if ui.button(hold_text).clicked() {
                self.hold = !self.hold;
            }

            // Relative mode
            if ui
                .button(if self.relative_mode { "REL ON" } else { "REL" })
                .clicked()
            {
                if self.relative_mode {
                    self.relative_mode = false;
                } else {
                    self.relative_ref = self.display_value();
                    self.relative_mode = true;
                }
            }

            // Reset stats
            if ui.button("Reset Stats").clicked() {
                if let Some(channel) = self.channels.get_mut(self.selected_channel as usize) {
                    channel.reset_stats();
                }
            }
        });

        ui.separator();

        // Main display
        self.render_main_display(ui);

        // Bar graph
        if self.show_bar {
            ui.separator();
            self.render_bar_graph(ui);
        }

        // Statistics
        if self.show_stats {
            ui.separator();
            self.render_statistics(ui);
        }

        ui.separator();

        // View options
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_bar, "Bar Graph");
            ui.checkbox(&mut self.show_stats, "Statistics");

            ui.separator();

            if self.show_bar {
                ui.label("Range:");
                ui.add(
                    egui::DragValue::new(&mut self.bar_range.0)
                        .range(-100.0..=self.bar_range.1)
                        .speed(0.1)
                        .suffix(" V"),
                );
                ui.label("to");
                ui.add(
                    egui::DragValue::new(&mut self.bar_range.1)
                        .range(self.bar_range.0..=100.0)
                        .speed(0.1)
                        .suffix(" V"),
                );
            }
        });

        // Request repaint for live updates
        if !self.hold {
            ui.ctx().request_repaint();
        }
    }

    /// Render the main numeric display
    fn render_main_display(&self, ui: &mut Ui) {
        let voltage = self.display_value();
        let (display_value, unit_str) = self.unit.format(voltage);

        // Large display area
        ui.vertical_centered(|ui| {
            // Mode indicator
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(self.mode.label())
                        .size(14.0)
                        .color(Color32::GRAY),
                );
                if self.hold {
                    ui.label(RichText::new("HOLD").size(14.0).color(Color32::YELLOW));
                }
                if self.relative_mode {
                    ui.label(RichText::new("REL").size(14.0).color(Color32::LIGHT_BLUE));
                }
            });

            // Main value
            let sign = if display_value < 0.0 { "-" } else { " " };
            let abs_value = display_value.abs();

            let value_text = format!("{}{:.decimals$}", sign, abs_value, decimals = self.decimals);

            ui.horizontal(|ui| {
                ui.add_space(ui.available_width() / 4.0);

                // Value
                ui.label(
                    RichText::new(&value_text)
                        .size(48.0)
                        .monospace()
                        .color(if self.hold {
                            Color32::YELLOW
                        } else {
                            Color32::WHITE
                        }),
                );

                // Unit
                ui.label(
                    RichText::new(unit_str)
                        .size(24.0)
                        .color(Color32::LIGHT_GRAY),
                );
            });

            // Channel indicator
            ui.label(
                RichText::new(format!("Channel {}", self.selected_channel))
                    .size(12.0)
                    .color(Color32::GRAY),
            );
        });
    }

    /// Render bar graph
    fn render_bar_graph(&self, ui: &mut Ui) {
        let voltage = self.display_value();
        let (min, max) = self.bar_range;
        let range = max - min;

        // Normalize voltage to 0-1 range
        let normalized = ((voltage - min) / range).clamp(0.0, 1.0);

        ui.horizontal(|ui| {
            ui.label(format!("{:.1}V", min));

            // Bar
            let available_width = ui.available_width() - 80.0;
            let bar_width = (available_width * normalized as f32).max(2.0);

            let (rect, _response) =
                ui.allocate_exact_size(egui::vec2(available_width, 20.0), egui::Sense::hover());

            // Background
            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(40));

            // Fill
            let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(bar_width, 20.0));

            let fill_color = if voltage < 0.0 {
                Color32::from_rgb(100, 150, 255)
            } else {
                Color32::from_rgb(100, 255, 150)
            };

            ui.painter().rect_filled(fill_rect, 2.0, fill_color);

            // Center line (zero)
            if min < 0.0 && max > 0.0 {
                let zero_x = rect.min.x + (available_width * (-min / range) as f32);
                ui.painter().line_segment(
                    [
                        egui::pos2(zero_x, rect.min.y),
                        egui::pos2(zero_x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, Color32::WHITE),
                );
            }

            ui.label(format!("{:.1}V", max));
        });
    }

    /// Render statistics panel
    fn render_statistics(&self, ui: &mut Ui) {
        let channel = &self.channels[self.selected_channel as usize];

        ui.horizontal(|ui| {
            ui.group(|ui| {
                ui.label("Min");
                let (val, unit) = self.unit.format(channel.min);
                ui.label(RichText::new(format!("{:.4} {}", val, unit)).monospace());
            });

            ui.group(|ui| {
                ui.label("Max");
                let (val, unit) = self.unit.format(channel.max);
                ui.label(RichText::new(format!("{:.4} {}", val, unit)).monospace());
            });

            ui.group(|ui| {
                ui.label("Mean");
                let (val, unit) = self.unit.format(channel.mean());
                ui.label(RichText::new(format!("{:.4} {}", val, unit)).monospace());
            });

            ui.group(|ui| {
                ui.label("Std Dev");
                let (val, unit) = self.unit.format(channel.std_dev());
                ui.label(RichText::new(format!("{:.4} {}", val, unit)).monospace());
            });

            ui.group(|ui| {
                ui.label("Samples");
                ui.label(RichText::new(format!("{}", channel.count)).monospace());
            });
        });
    }
}
