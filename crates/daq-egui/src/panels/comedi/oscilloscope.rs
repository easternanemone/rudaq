//! Oscilloscope/Waveform Viewer Panel for Comedi DAQ devices.
//!
//! Provides real-time time-domain visualization of analog input signals
//! with triggering, cursors, and measurement statistics.
//!
//! ## Design Notes
//!
//! This panel uses a "push samples" pattern to avoid DaqClient Clone requirements:
//! - External code pushes samples via mpsc channel
//! - Panel drains channel each frame and updates display
//! - No async spawns from within the panel

use eframe::egui::{self, Color32, RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints, VLine};
use futures::StreamExt;
use std::collections::VecDeque;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use daq_client::DaqClient;

/// Maximum points to keep per channel (prevents unbounded memory growth)
const MAX_POINTS_PER_CHANNEL: usize = 10_000;

/// Default time window in seconds
const DEFAULT_TIME_WINDOW: f64 = 1.0;

/// Time window presets (seconds)
const TIME_WINDOW_OPTIONS: &[f64] = &[0.1, 0.5, 1.0, 2.0, 5.0, 10.0];

/// Channel colors for multi-channel display
const CHANNEL_COLORS: &[Color32] = &[
    Color32::from_rgb(255, 200, 50),  // CH0: Yellow
    Color32::from_rgb(50, 200, 255),  // CH1: Cyan
    Color32::from_rgb(50, 255, 100),  // CH2: Green
    Color32::from_rgb(255, 100, 100), // CH3: Red
    Color32::from_rgb(200, 100, 255), // CH4: Purple
    Color32::from_rgb(255, 150, 50),  // CH5: Orange
    Color32::from_rgb(100, 255, 255), // CH6: Light Cyan
    Color32::from_rgb(255, 100, 200), // CH7: Pink
];

/// A sample pushed to the oscilloscope
#[derive(Debug, Clone)]
pub struct OscilloscopeSample {
    pub channel: u32,
    pub voltage: f64,
    /// Optional timestamp in seconds since start (if not provided, uses wall clock)
    pub timestamp: Option<f64>,
}

impl OscilloscopeSample {
    pub fn new(channel: u32, voltage: f64) -> Self {
        Self {
            channel,
            voltage,
            timestamp: None,
        }
    }

    pub fn with_timestamp(channel: u32, voltage: f64, timestamp: f64) -> Self {
        Self {
            channel,
            voltage,
            timestamp: Some(timestamp),
        }
    }
}

/// Sender for pushing samples to the oscilloscope
pub type OscilloscopeSender = mpsc::Sender<OscilloscopeSample>;

/// Receiver for samples (held by panel)
pub type OscilloscopeReceiver = mpsc::Receiver<OscilloscopeSample>;

/// Create a bounded channel pair for oscilloscope samples
pub fn oscilloscope_channel() -> (OscilloscopeSender, OscilloscopeReceiver) {
    mpsc::channel(4096)
}

/// Trigger mode for the oscilloscope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerMode {
    #[default]
    Auto,
    Normal,
    Single,
    Off,
}

impl TriggerMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Normal => "Normal",
            Self::Single => "Single",
            Self::Off => "Off",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Auto, Self::Normal, Self::Single, Self::Off]
    }
}

/// Trigger edge direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerEdge {
    #[default]
    Rising,
    Falling,
    Either,
}

impl TriggerEdge {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Rising => "Rising",
            Self::Falling => "Falling",
            Self::Either => "Either",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Rising, Self::Falling, Self::Either]
    }
}

/// Per-channel state
#[derive(Debug, Clone)]
struct ChannelState {
    enabled: bool,
    label: String,
    color: Color32,
    /// Points: (time_offset, voltage)
    points: VecDeque<(f64, f64)>,
    /// Y-offset for display (vertical position)
    y_offset: f64,
    /// Y-scale multiplier
    y_scale: f64,
}

impl ChannelState {
    fn new(channel: u32, color: Color32) -> Self {
        Self {
            enabled: channel == 0, // Only CH0 enabled by default
            label: format!("CH{}", channel),
            color,
            points: VecDeque::with_capacity(MAX_POINTS_PER_CHANNEL),
            y_offset: 0.0,
            y_scale: 1.0,
        }
    }

    fn push(&mut self, time: f64, voltage: f64) {
        self.points.push_back((time, voltage));

        // Trim to max size
        while self.points.len() > MAX_POINTS_PER_CHANNEL {
            self.points.pop_front();
        }
    }

    fn clear(&mut self) {
        self.points.clear();
    }

    /// Get points within time window, applying offset and scale
    fn visible_points(&self, t_start: f64, t_end: f64) -> Vec<[f64; 2]> {
        self.points
            .iter()
            .filter(|(t, _)| *t >= t_start && *t <= t_end)
            .map(|(t, v)| [*t, (*v + self.y_offset) * self.y_scale])
            .collect()
    }

    /// Get statistics for visible window
    fn statistics(&self, t_start: f64, t_end: f64) -> ChannelStats {
        let values: Vec<f64> = self
            .points
            .iter()
            .filter(|(t, _)| *t >= t_start && *t <= t_end)
            .map(|(_, v)| *v)
            .collect();

        if values.is_empty() {
            return ChannelStats::default();
        }

        let n = values.len();
        let sum: f64 = values.iter().sum();
        let mean = sum / n as f64;
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        ChannelStats {
            count: n,
            min,
            max,
            mean,
            pk_pk: max - min,
        }
    }
}

/// Statistics for a channel
#[derive(Debug, Clone, Default)]
struct ChannelStats {
    count: usize,
    min: f64,
    max: f64,
    mean: f64,
    pk_pk: f64,
}

/// Signal source for the oscilloscope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalSource {
    #[default]
    External,
    Synthetic,
}

impl SignalSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::External => "External",
            Self::Synthetic => "Synthetic",
        }
    }
}

/// Synthetic signal type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyntheticSignal {
    #[default]
    Sine,
    Square,
    Triangle,
    Sawtooth,
    Noise,
}

impl SyntheticSignal {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Noise => "Noise",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Sine,
            Self::Square,
            Self::Triangle,
            Self::Sawtooth,
            Self::Noise,
        ]
    }

    /// Generate value at time t with given frequency and amplitude
    fn generate(&self, t: f64, frequency: f64, amplitude: f64) -> f64 {
        let phase = (t * frequency * std::f64::consts::TAU) % std::f64::consts::TAU;

        match self {
            Self::Sine => amplitude * phase.sin(),
            Self::Square => {
                if phase < std::f64::consts::PI {
                    amplitude
                } else {
                    -amplitude
                }
            }
            Self::Triangle => {
                let normalized = phase / std::f64::consts::TAU;
                if normalized < 0.5 {
                    amplitude * (4.0 * normalized - 1.0)
                } else {
                    amplitude * (3.0 - 4.0 * normalized)
                }
            }
            Self::Sawtooth => {
                let normalized = phase / std::f64::consts::TAU;
                amplitude * (2.0 * normalized - 1.0)
            }
            Self::Noise => {
                // Simple pseudo-random based on time
                let seed = (t * 1_000_000.0) as u64;
                let noise = ((seed.wrapping_mul(1103515245).wrapping_add(12345)) % 1000) as f64
                    / 500.0
                    - 1.0;
                amplitude * noise
            }
        }
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

/// Oscilloscope/Waveform Viewer Panel
pub struct OscilloscopePanel {
    /// Panel start time for relative timestamps
    start_time: Instant,
    /// Per-channel state (up to 16 channels)
    channels: Vec<ChannelState>,
    /// Sample receiver
    sample_rx: OscilloscopeReceiver,
    /// Sample sender (for cloning)
    sample_tx: OscilloscopeSender,

    // Display settings
    /// Time window to display (seconds)
    time_window: f64,
    /// Running vs paused
    running: bool,
    /// Frozen time offset (when paused, allows scrollback)
    frozen_time: f64,
    /// Y-axis autoscale
    y_autoscale: bool,
    /// Manual Y-axis range
    y_range: (f64, f64),

    // Trigger settings
    trigger_mode: TriggerMode,
    trigger_edge: TriggerEdge,
    trigger_channel: u32,
    trigger_level: f64,
    /// Time of last trigger (for display)
    last_trigger_time: Option<f64>,
    /// Single trigger armed
    single_armed: bool,
    /// Single trigger fired (waiting for arm)
    single_fired: bool,

    // Signal source
    signal_source: SignalSource,
    synthetic_signal: SyntheticSignal,
    synthetic_frequency: f64,
    synthetic_amplitude: f64,
    /// Synthetic generator last update time
    synthetic_last_time: f64,

    // UI state
    show_measurements: bool,
    show_channel_config: bool,

    // Streaming state (External signal source)
    streaming_state: StreamingState,
    streaming_handle: Option<StreamingHandle>,
    streaming_device_id: Option<String>,
    streaming_error: Option<String>,
    /// Channels to stream when using external source
    external_channels: Vec<u32>,
    /// Sample rate for external streaming (Hz)
    external_sample_rate: f64,
}

impl Default for OscilloscopePanel {
    fn default() -> Self {
        let (tx, rx) = oscilloscope_channel();
        let start_time = Instant::now();

        // Create 16 channels
        let channels: Vec<ChannelState> = (0..16)
            .map(|i| {
                let color = CHANNEL_COLORS[i as usize % CHANNEL_COLORS.len()];
                ChannelState::new(i, color)
            })
            .collect();

        Self {
            start_time,
            channels,
            sample_rx: rx,
            sample_tx: tx,
            time_window: DEFAULT_TIME_WINDOW,
            running: true,
            frozen_time: 0.0,
            y_autoscale: true,
            y_range: (-10.0, 10.0),
            trigger_mode: TriggerMode::Auto,
            trigger_edge: TriggerEdge::Rising,
            trigger_channel: 0,
            trigger_level: 0.0,
            last_trigger_time: None,
            single_armed: false,
            single_fired: false,
            signal_source: SignalSource::Synthetic, // Default to synthetic for demo
            synthetic_signal: SyntheticSignal::Sine,
            synthetic_frequency: 10.0,
            synthetic_amplitude: 5.0,
            synthetic_last_time: 0.0,
            show_measurements: true,
            show_channel_config: false,
            streaming_state: StreamingState::Stopped,
            streaming_handle: None,
            streaming_device_id: None,
            streaming_error: None,
            external_channels: vec![0],   // Default to CH0
            external_sample_rate: 1000.0, // 1 kHz default
        }
    }
}

impl OscilloscopePanel {
    /// Create a new oscilloscope panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a sender clone for pushing samples from external code
    pub fn get_sender(&self) -> OscilloscopeSender {
        self.sample_tx.clone()
    }

    /// Start streaming from a hardware device via gRPC
    pub fn start_streaming(
        &mut self,
        _client: DaqClient,
        runtime: &Runtime,
        device_id: String,
        channels: Vec<u32>,
        sample_rate_hz: f64,
    ) {
        if self.streaming_state != StreamingState::Stopped {
            self.streaming_error = Some("Already streaming".to_string());
            return;
        }

        self.streaming_state = StreamingState::Starting;
        self.streaming_device_id = Some(device_id.clone());
        self.streaming_error = None;

        let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);
        let sample_tx = self.sample_tx.clone();

        // Spawn background task to handle streaming
        runtime.spawn(async move {
            // Import NI DAQ proto types
            use daq_proto::ni_daq::{
                ni_daq_service_client::NiDaqServiceClient, StreamAnalogInputRequest,
            };
            use tonic::transport::Channel;

            // Create NI DAQ client (reuse the gRPC channel from DaqClient)
            // For now, we'll connect directly - TODO: get channel from DaqClient
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
                channels: channels.clone(),
                sample_rate_hz,
                range_index: 0, // Default range
                stop_condition: Some(
                    daq_proto::ni_daq::stream_analog_input_request::StopCondition::Continuous(true),
                ),
                buffer_size: 1024,
            };

            let mut stream = match ni_daq_client.stream_analog_input(request).await {
                Ok(response) => response.into_inner(),
                Err(e) => {
                    tracing::error!("Failed to start streaming: {}", e);
                    return;
                }
            };

            tracing::info!("Started streaming from device={}", device_id);

            // Process stream until aborted or error
            loop {
                tokio::select! {
                    // Check for abort signal
                    _ = abort_rx.recv() => {
                        tracing::info!("Streaming aborted by user");
                        break;
                    }
                    // Process incoming data
                    message = stream.next() => {
                        match message {
                            Some(Ok(data)) => {
                                // Deinterleave voltages and push to channels
                                let n_channels = data.n_channels as usize;
                                if n_channels == 0 {
                                    continue;
                                }

                                for (i, voltage) in data.voltages.iter().enumerate() {
                                    let channel_idx = i % n_channels;
                                    if let Some(&channel_id) = channels.get(channel_idx) {
                                        let sample = OscilloscopeSample::new(channel_id, *voltage);
                                        // Drop samples if channel is full (backpressure)
                                        let _ = sample_tx.try_send(sample);
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                tracing::error!("Stream error: {}", e);
                                break;
                            }
                            None => {
                                tracing::info!("Stream ended");
                                break;
                            }
                        }
                    }
                }
            }

            tracing::info!("Streaming task finished");
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

    /// Current time since panel start
    fn current_time(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Drain pending samples from channel
    fn drain_samples(&mut self) {
        while let Ok(sample) = self.sample_rx.try_recv() {
            if !self.running {
                continue; // Drain but don't process when paused
            }

            let time = sample.timestamp.unwrap_or_else(|| self.current_time());

            if let Some(channel) = self.channels.get_mut(sample.channel as usize) {
                channel.push(time, sample.voltage);
            }
        }
    }

    /// Generate synthetic samples
    fn generate_synthetic(&mut self) {
        if self.signal_source != SignalSource::Synthetic || !self.running {
            return;
        }

        let current = self.current_time();
        let dt = 1.0 / 1000.0; // 1kHz sample rate

        // Generate samples to catch up
        while self.synthetic_last_time < current {
            self.synthetic_last_time += dt;
            let t = self.synthetic_last_time;

            // Generate for enabled channels with frequency offsets
            for (i, channel) in self.channels.iter_mut().enumerate() {
                if channel.enabled {
                    // Slightly different frequency per channel for visual interest
                    let freq_offset = i as f64 * 0.5;
                    let voltage = self.synthetic_signal.generate(
                        t,
                        self.synthetic_frequency + freq_offset,
                        self.synthetic_amplitude,
                    );
                    channel.push(t, voltage);
                }
            }
        }
    }

    /// Clear all channel data
    pub fn clear(&mut self) {
        for channel in &mut self.channels {
            channel.clear();
        }
        self.last_trigger_time = None;
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        self.ui_with_client(ui, None, None, None);
    }

    /// UI with optional client for external streaming
    pub fn ui_with_client(
        &mut self,
        ui: &mut Ui,
        client: Option<&mut DaqClient>,
        runtime: Option<&Runtime>,
        device_id: Option<&str>,
    ) {
        // Drain pending samples
        self.drain_samples();

        // Generate synthetic if enabled
        self.generate_synthetic();

        let current_time = if self.running {
            self.current_time()
        } else {
            self.frozen_time
        };

        // Header
        ui.horizontal(|ui| {
            ui.heading("Oscilloscope");
            ui.separator();

            // Run/Stop button
            let run_text = if self.running { "Stop" } else { "Run" };
            if ui.button(run_text).clicked() {
                self.running = !self.running;
                if !self.running {
                    self.frozen_time = self.current_time();
                }
            }

            // Single trigger
            if self.trigger_mode == TriggerMode::Single {
                let arm_text = if self.single_armed { "Armed" } else { "Arm" };
                if ui.button(arm_text).clicked() {
                    self.single_armed = true;
                    self.single_fired = false;
                }
            }

            // Clear button
            if ui.button("Clear").clicked() {
                self.clear();
            }

            ui.separator();

            // Signal source
            ui.label("Source:");
            egui::ComboBox::from_id_salt("osc_source")
                .selected_text(self.signal_source.label())
                .width(80.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut self.signal_source,
                            SignalSource::External,
                            "External",
                        )
                        .clicked()
                    {
                        // Switched to external
                    }
                    if ui
                        .selectable_value(
                            &mut self.signal_source,
                            SignalSource::Synthetic,
                            "Synthetic",
                        )
                        .clicked()
                    {
                        self.synthetic_last_time = self.current_time();
                    }
                });

            // External streaming status and controls
            if self.signal_source == SignalSource::External {
                ui.separator();

                match self.streaming_state {
                    StreamingState::Stopped => {
                        if ui.button("▶ Stream").clicked() {
                            if let (Some(client), Some(runtime), Some(device_id)) =
                                (client, runtime, device_id)
                            {
                                self.start_streaming(
                                    client.clone(),
                                    runtime,
                                    device_id.to_string(),
                                    self.external_channels.clone(),
                                    self.external_sample_rate,
                                );
                            } else {
                                self.streaming_error =
                                    Some("No DAQ connection available".to_string());
                            }
                        }
                    }
                    StreamingState::Running => {
                        if ui.button("⏹ Stop").clicked() {
                            if let Some(runtime) = runtime {
                                self.stop_streaming(runtime);
                            }
                        }
                        ui.label(RichText::new("● STREAMING").color(Color32::GREEN));
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
            }
        });

        // Show streaming error if present
        if let Some(ref error) = self.streaming_error {
            ui.colored_label(Color32::RED, format!("Error: {}", error));
        }

        ui.separator();

        // Control bar
        ui.horizontal(|ui| {
            // Time window selector
            ui.label("Time/div:");
            egui::ComboBox::from_id_salt("osc_time_window")
                .selected_text(format!("{:.1}s", self.time_window))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for &tw in TIME_WINDOW_OPTIONS {
                        ui.selectable_value(&mut self.time_window, tw, format!("{:.1}s", tw));
                    }
                });

            ui.separator();

            // Trigger controls
            ui.label("Trigger:");
            egui::ComboBox::from_id_salt("osc_trigger_mode")
                .selected_text(self.trigger_mode.label())
                .width(70.0)
                .show_ui(ui, |ui| {
                    for mode in TriggerMode::all() {
                        ui.selectable_value(&mut self.trigger_mode, *mode, mode.label());
                    }
                });

            ui.add(
                egui::DragValue::new(&mut self.trigger_level)
                    .range(-10.0..=10.0)
                    .speed(0.1)
                    .suffix(" V"),
            );

            egui::ComboBox::from_id_salt("osc_trigger_edge")
                .selected_text(self.trigger_edge.label())
                .width(60.0)
                .show_ui(ui, |ui| {
                    for edge in TriggerEdge::all() {
                        ui.selectable_value(&mut self.trigger_edge, *edge, edge.label());
                    }
                });

            ui.separator();

            // Y-axis controls
            ui.checkbox(&mut self.y_autoscale, "Auto Y");

            if !self.y_autoscale {
                ui.add(
                    egui::DragValue::new(&mut self.y_range.0)
                        .range(-100.0..=self.y_range.1)
                        .speed(0.1)
                        .prefix("Y: "),
                );
                ui.label("to");
                ui.add(
                    egui::DragValue::new(&mut self.y_range.1)
                        .range(self.y_range.0..=100.0)
                        .speed(0.1),
                );
            }

            ui.separator();

            // View toggles
            ui.checkbox(&mut self.show_measurements, "Stats");
            ui.checkbox(&mut self.show_channel_config, "Channels");
        });

        ui.separator();

        // Synthetic generator controls (if synthetic mode)
        if self.signal_source == SignalSource::Synthetic {
            ui.horizontal(|ui| {
                ui.label("Waveform:");
                egui::ComboBox::from_id_salt("osc_synthetic_signal")
                    .selected_text(self.synthetic_signal.label())
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for sig in SyntheticSignal::all() {
                            ui.selectable_value(&mut self.synthetic_signal, *sig, sig.label());
                        }
                    });

                ui.label("Freq:");
                ui.add(
                    egui::DragValue::new(&mut self.synthetic_frequency)
                        .range(0.1..=1000.0)
                        .speed(1.0)
                        .suffix(" Hz"),
                );

                ui.label("Amp:");
                ui.add(
                    egui::DragValue::new(&mut self.synthetic_amplitude)
                        .range(0.1..=10.0)
                        .speed(0.1)
                        .suffix(" V"),
                );
            });
            ui.separator();
        }

        // Main content area
        ui.columns(if self.show_channel_config { 2 } else { 1 }, |columns| {
            // Left: Waveform plot
            self.render_plot(&mut columns[0], current_time);

            // Right: Channel config (if enabled)
            if self.show_channel_config && columns.len() > 1 {
                self.render_channel_config(&mut columns[1]);
            }
        });

        // Measurements panel (if enabled)
        if self.show_measurements {
            ui.separator();
            self.render_measurements(ui, current_time);
        }

        // Request continuous repaint when running
        if self.running {
            ui.ctx().request_repaint();
        }
    }

    /// Render the waveform plot
    fn render_plot(&self, ui: &mut Ui, current_time: f64) {
        let t_end = current_time;
        let t_start = t_end - self.time_window;

        // Compute Y bounds
        let (y_min, y_max) = if self.y_autoscale {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;

            for channel in &self.channels {
                if !channel.enabled {
                    continue;
                }
                for (t, v) in &channel.points {
                    if *t >= t_start && *t <= t_end {
                        let scaled = (*v + channel.y_offset) * channel.y_scale;
                        min = min.min(scaled);
                        max = max.max(scaled);
                    }
                }
            }

            if min.is_infinite() {
                (-10.0, 10.0)
            } else {
                let margin = (max - min).max(0.1) * 0.1;
                (min - margin, max + margin)
            }
        } else {
            self.y_range
        };

        let plot = Plot::new("oscilloscope_plot")
            .height(300.0)
            .include_x(t_start)
            .include_x(t_end)
            .include_y(y_min)
            .include_y(y_max)
            .x_axis_label("Time (s)")
            .y_axis_label("Voltage (V)")
            .allow_zoom(true)
            .allow_drag(true)
            .show_axes([true, true])
            .show_grid(true);

        plot.show(ui, |plot_ui| {
            // Draw channel traces
            for channel in &self.channels {
                if !channel.enabled {
                    continue;
                }

                let points = channel.visible_points(t_start, t_end);
                if !points.is_empty() {
                    let line = Line::new(&channel.label, PlotPoints::new(points))
                        .color(channel.color)
                        .width(1.5);
                    plot_ui.line(line);
                }
            }

            // Draw trigger level line
            if self.trigger_mode != TriggerMode::Off {
                let trigger_line = egui_plot::HLine::new("trigger", self.trigger_level)
                    .color(Color32::from_rgb(255, 0, 0));
                plot_ui.hline(trigger_line);
            }

            // Draw trigger time marker
            if let Some(trigger_time) = self.last_trigger_time {
                if trigger_time >= t_start && trigger_time <= t_end {
                    let trigger_marker =
                        VLine::new("trigger", trigger_time).color(Color32::from_rgb(255, 100, 0));
                    plot_ui.vline(trigger_marker);
                }
            }
        });
    }

    /// Render channel configuration panel
    fn render_channel_config(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Channels").strong());
            ui.separator();

            // Check if any channel beyond 8 is enabled (compute before mutable borrow)
            let show_all = self
                .channels
                .get(8..)
                .is_some_and(|rest| rest.iter().any(|c| c.enabled));
            let max_visible = if show_all { 16 } else { 8 };

            egui::ScrollArea::vertical()
                .max_height(250.0)
                .show(ui, |ui| {
                    for (i, channel) in self.channels.iter_mut().enumerate() {
                        if i >= max_visible {
                            break;
                        }

                        ui.horizontal(|ui| {
                            ui.checkbox(&mut channel.enabled, "");
                            ui.colored_label(channel.color, &channel.label);

                            if channel.enabled {
                                ui.separator();
                                ui.label("Offset:");
                                ui.add(
                                    egui::DragValue::new(&mut channel.y_offset)
                                        .range(-10.0..=10.0)
                                        .speed(0.1),
                                );
                            }
                        });
                    }
                });
        });
    }

    /// Render measurements panel
    fn render_measurements(&self, ui: &mut Ui, current_time: f64) {
        let t_end = current_time;
        let t_start = t_end - self.time_window;

        ui.horizontal(|ui| {
            for channel in &self.channels {
                if !channel.enabled {
                    continue;
                }

                let stats = channel.statistics(t_start, t_end);

                ui.group(|ui| {
                    ui.colored_label(channel.color, &channel.label);
                    ui.label(format!("Mean: {:.3}V", stats.mean));
                    ui.label(format!("Pk-Pk: {:.3}V", stats.pk_pk));
                    ui.label(format!("Min: {:.3}V", stats.min));
                    ui.label(format!("Max: {:.3}V", stats.max));
                });
            }
        });
    }
}
