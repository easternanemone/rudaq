//! Analog Input Control Panel for Comedi DAQ devices.
//!
//! Provides channel selection, voltage range configuration, single-shot reads,
//! and hardware-timed streaming via the `StreamAnalogInput` gRPC RPC.

use crate::runtime::Runtime;
use eframe::egui::{self, Color32, RichText, Ui};
use futures::StreamExt;
use protocol::ni_daq::ReadAnalogInputRequest;
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::widgets::{OfflineContext, offline_notice};
use client::DaqClient;

use super::{AnalogReference, NI_VOLTAGE_RANGES};

/// Action results from async operations.
#[derive(Debug)]
enum ActionResult {
    Reading {
        channel: u32,
        voltage: f64,
    },
    ReadingError {
        channel: u32,
        error: String,
    },
    AllReadings {
        voltages: Vec<(u32, f64)>,
    },
    /// The RPC connected successfully — transition from Starting → Running.
    StreamStarted {
        generation: u64,
    },
    /// Batch of interleaved streaming samples, de-interleaved into per-channel voltages.
    StreamBatch {
        generation: u64,
        /// (channel_number, latest_voltage) for each channel in the stream.
        voltages: Vec<(u32, f64)>,
        overflow: bool,
    },
    StreamError {
        generation: u64,
        error: String,
    },
    StreamEnded {
        generation: u64,
    },
}

/// Streaming lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

/// Channel configuration state.
#[derive(Debug, Clone)]
struct ChannelConfig {
    enabled: bool,
    range_index: usize,
    aref: AnalogReference,
    last_reading: Option<f64>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            range_index: 0,
            aref: AnalogReference::Ground,
            last_reading: None,
        }
    }
}

/// Analog Input Control Panel.
///
/// Provides UI for configuring and reading from analog input channels.
/// Supports both single-shot reads and hardware-timed DMA streaming.
pub struct AnalogInputPanel {
    /// Device ID for the Comedi device
    device_id: String,
    /// Number of channels (16 for NI PCI-MIO-16XE-10)
    n_channels: u32,
    /// Per-channel configuration
    channels: HashMap<u32, ChannelConfig>,
    /// Currently selected channel for detailed view
    selected_channel: u32,
    /// Auto-refresh enabled (software-polled fallback)
    auto_refresh: bool,
    /// Refresh interval in milliseconds
    refresh_interval_ms: u32,
    /// Last refresh time
    last_refresh: crate::time::Instant,
    /// Differential mode (uses channel pairs)
    differential_mode: bool,
    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async action sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action receiver
    action_rx: mpsc::Receiver<ActionResult>,
    // -- Streaming state --
    /// Current streaming lifecycle state
    streaming_state: StreamingState,
    /// Abort handle: dropping or sending cancels the background stream task
    streaming_abort_tx: Option<mpsc::Sender<()>>,
    /// Sample rate for streaming (Hz per channel)
    stream_sample_rate: f64,
    /// Ordered list of channel numbers currently being streamed
    streaming_channels: Vec<u32>,
    /// Overflow indicator (set by stream, cleared on next batch)
    stream_overflow: bool,
    /// Monotonically increasing counter to distinguish stream sessions.
    /// Events from a prior generation are silently discarded.
    stream_generation: u64,
}

impl Default for AnalogInputPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(64);
        let mut channels = HashMap::new();
        for i in 0..16 {
            channels.insert(i, ChannelConfig::default());
        }

        Self {
            device_id: String::from("comedi0"),
            n_channels: 16,
            channels,
            selected_channel: 0,
            auto_refresh: false,
            refresh_interval_ms: 100,
            last_refresh: crate::time::Instant::now(),
            differential_mode: false,
            status: None,
            error: None,
            action_tx,
            action_rx,
            streaming_state: StreamingState::Stopped,
            streaming_abort_tx: None,
            stream_sample_rate: 10.0,
            streaming_channels: Vec::new(),
            stream_overflow: false,
            stream_generation: 0,
        }
    }
}

impl AnalogInputPanel {
    /// Create a new panel for a specific device.
    pub fn new(device_id: &str, n_channels: u32) -> Self {
        Self {
            device_id: device_id.to_string(),
            n_channels,
            ..Self::default()
        }
    }

    /// Main UI entry point.
    pub fn ui(&mut self, ui: &mut Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Check for offline mode
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        // Poll async results
        self.poll_results();

        // Auto-refresh logic (only active when NOT streaming)
        if self.auto_refresh && self.streaming_state == StreamingState::Stopped {
            let elapsed = self.last_refresh.elapsed();
            if elapsed.as_millis() >= u128::from(self.refresh_interval_ms) {
                if let Some(c) = client.as_deref() {
                    self.read_all_channels(runtime, c);
                }
                self.last_refresh = crate::time::Instant::now();
            }
            ui.ctx().request_repaint();
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("Analog Input");
            ui.separator();
            ui.label(format!("Device: {}", self.device_id));
            if self.streaming_state == StreamingState::Running {
                ui.separator();
                ui.label(RichText::new("LIVE").color(Color32::GREEN));
                if self.stream_overflow {
                    ui.label(RichText::new("OVF").color(Color32::RED));
                }
            }
        });

        ui.separator();

        // Status/error messages
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
        ui.horizontal(|ui| {
            // Streaming controls
            match self.streaming_state {
                StreamingState::Stopped => {
                    if ui.button("Stream").clicked()
                        && let Some(c) = client.as_mut()
                    {
                        self.start_streaming(c, runtime);
                    }

                    ui.separator();

                    // Auto-refresh toggle (fallback when not streaming)
                    ui.checkbox(&mut self.auto_refresh, "Auto-refresh");
                    if self.auto_refresh {
                        ui.add(
                            egui::DragValue::new(&mut self.refresh_interval_ms)
                                .range(50..=1000)
                                .suffix(" ms")
                                .speed(10),
                        );
                    }
                }
                StreamingState::Running => {
                    if ui.button("Stop").clicked() {
                        self.stop_streaming(runtime);
                    }
                }
                StreamingState::Starting => {
                    ui.spinner();
                    ui.label("Starting stream...");
                }
                StreamingState::Stopping => {
                    ui.spinner();
                    ui.label("Stopping...");
                }
            }

            ui.separator();

            // Sample rate (editable when stopped)
            ui.label("Rate:");
            ui.add_enabled(
                self.streaming_state == StreamingState::Stopped,
                egui::DragValue::new(&mut self.stream_sample_rate)
                    .range(1.0..=100_000.0)
                    .suffix(" Hz")
                    .speed(1.0),
            );

            ui.separator();

            // Mode selection
            if ui
                .selectable_label(!self.differential_mode, "Single-Ended")
                .clicked()
            {
                self.differential_mode = false;
            }
            if ui
                .selectable_label(self.differential_mode, "Differential")
                .clicked()
            {
                self.differential_mode = true;
            }

            ui.separator();

            // Read all button (always available for one-shot)
            if ui
                .add_enabled(
                    self.streaming_state == StreamingState::Stopped,
                    egui::Button::new("Read All"),
                )
                .clicked()
                && let Some(c) = client.as_deref()
            {
                self.read_all_channels(runtime, c);
            }
        });

        ui.separator();

        // Main content: channel grid + detail panel
        let client_clone = client.map(|c| c.clone());
        ui.columns(2, |columns| {
            // Left: Channel grid
            self.render_channel_grid(&mut columns[0], client_clone.clone(), runtime);

            // Right: Selected channel details
            self.render_channel_details(&mut columns[1], client_clone, runtime);
        });

        // Request repaint when streaming for live updates
        if self.streaming_state == StreamingState::Running {
            ui.ctx().request_repaint();
        }
    }

    // =========================================================================
    // Streaming
    // =========================================================================

    /// Start hardware-timed streaming for all enabled channels.
    #[allow(clippy::cast_possible_truncation)] // range_index is always < 14
    fn start_streaming(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.streaming_state != StreamingState::Stopped {
            return;
        }

        // Collect enabled channels in sorted order
        let mut stream_channels: Vec<u32> = self
            .channels
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(ch, _)| *ch)
            .collect();
        stream_channels.sort_unstable();

        if stream_channels.is_empty() {
            self.error = Some("No channels enabled".to_string());
            return;
        }

        // Use range from the first enabled channel (RPC accepts a single range_index
        // shared by all channels in the stream).
        let range_index = stream_channels
            .first()
            .and_then(|ch| self.channels.get(ch))
            .map(|c| c.range_index as u32)
            .unwrap_or(0);

        self.stream_generation += 1;
        let generation = self.stream_generation;

        self.streaming_state = StreamingState::Starting;
        self.streaming_channels.clone_from(&stream_channels);
        self.stream_overflow = false;
        self.error = None;
        self.status = Some(format!(
            "Starting stream: {} ch @ {:.0} Hz",
            stream_channels.len(),
            self.stream_sample_rate
        ));

        let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);
        self.streaming_abort_tx = Some(abort_tx);

        let tx = self.action_tx.clone();
        let device_id = self.device_id.clone();
        let sample_rate = self.stream_sample_rate;
        let channels_for_task = stream_channels;

        let mut ni_daq_client = client.ni_daq_streaming_client().clone();

        runtime.spawn(async move {
            use protocol::ni_daq::StreamAnalogInputRequest;

            let request = StreamAnalogInputRequest {
                device_id: device_id.clone(),
                channels: channels_for_task.clone(),
                sample_rate_hz: sample_rate,
                range_index,
                stop_condition: Some(
                    protocol::ni_daq::stream_analog_input_request::StopCondition::Continuous(true),
                ),
                buffer_size: 256,
            };

            let mut stream = match ni_daq_client.stream_analog_input(request).await {
                Ok(response) => {
                    let _ = tx.send(ActionResult::StreamStarted { generation }).await;
                    response.into_inner()
                }
                Err(e) => {
                    let _ = tx
                        .send(ActionResult::StreamError {
                            generation,
                            error: format!("Failed to start stream: {}", e),
                        })
                        .await;
                    return;
                }
            };

            loop {
                tokio::select! {
                    _ = abort_rx.recv() => {
                        break;
                    }
                    message = stream.next() => {
                        match message {
                            Some(Ok(data)) => {
                                let n_ch = data.n_channels.max(1) as usize;
                                let voltages = de_interleave_latest(
                                    &data.voltages,
                                    n_ch,
                                    &channels_for_task,
                                );
                                let _ = tx.try_send(ActionResult::StreamBatch {
                                    generation,
                                    voltages,
                                    overflow: data.overflow,
                                });
                            }
                            Some(Err(e)) => {
                                let _ = tx.send(ActionResult::StreamError {
                                    generation,
                                    error: format!("Stream error: {}", e),
                                }).await;
                                break;
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
            }

            let _ = tx.send(ActionResult::StreamEnded { generation }).await;
        });
    }

    /// Stop the active stream.
    ///
    /// Sets state to `Stopping` and sends the abort signal. The background task
    /// will emit `StreamEnded` when it finishes, which transitions to `Stopped`.
    fn stop_streaming(&mut self, runtime: &Runtime) {
        if !matches!(
            self.streaming_state,
            StreamingState::Running | StreamingState::Starting
        ) {
            return;
        }
        self.streaming_state = StreamingState::Stopping;
        self.streaming_channels.clear();
        self.status = Some("Stopping stream...".to_string());

        if let Some(abort_tx) = self.streaming_abort_tx.take() {
            runtime.spawn(async move {
                let _ = abort_tx.send(()).await;
            });
        }
    }

    // =========================================================================
    // Rendering
    // =========================================================================

    /// Render the channel selection grid.
    fn render_channel_grid(&mut self, ui: &mut Ui, client: Option<DaqClient>, runtime: &Runtime) {
        ui.group(|ui| {
            ui.label(RichText::new("Channels").strong());
            ui.separator();

            // 4x4 grid for 16 channels (or 8 pairs in differential mode)
            let channels_to_show = if self.differential_mode {
                self.n_channels / 2
            } else {
                self.n_channels
            };

            egui::Grid::new("ai_channel_grid")
                .num_columns(4)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    for i in 0..channels_to_show {
                        let channel = if self.differential_mode { i * 2 } else { i };
                        let config = self.channels.get(&channel).cloned().unwrap_or_default();

                        // Channel button with reading
                        let label = if self.differential_mode {
                            format!("D{}", i)
                        } else {
                            format!("CH{}", i)
                        };

                        let reading_text = config
                            .last_reading
                            .map(|v| format!("{:.3}V", v))
                            .unwrap_or_else(|| "---".to_string());

                        let is_selected = self.selected_channel == channel;
                        let button_text = format!("{}\n{}", label, reading_text);

                        let response = ui.selectable_label(is_selected, button_text);

                        if response.clicked() {
                            self.selected_channel = channel;
                        }

                        // Context menu for quick read
                        response.context_menu(|ui| {
                            if ui.button("Read Now").clicked() {
                                self.read_channel(channel, client.clone(), runtime);
                                ui.close();
                            }
                        });

                        // End row every 4 channels
                        if (i + 1) % 4 == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    }

    /// Render details for the selected channel.
    fn render_channel_details(
        &mut self,
        ui: &mut Ui,
        client: Option<DaqClient>,
        runtime: &Runtime,
    ) {
        ui.group(|ui| {
            let channel = self.selected_channel;
            let config = self.channels.entry(channel).or_default();

            ui.label(RichText::new(format!("Channel {} Configuration", channel)).strong());
            ui.separator();

            // Enable checkbox
            ui.checkbox(&mut config.enabled, "Enabled");

            ui.separator();

            // Voltage range selector
            ui.horizontal(|ui| {
                ui.label("Range:");
                egui::ComboBox::from_id_salt("ai_range_select")
                    .selected_text(NI_VOLTAGE_RANGES[config.range_index].label())
                    .show_ui(ui, |ui| {
                        for (idx, range) in NI_VOLTAGE_RANGES.iter().enumerate() {
                            ui.selectable_value(&mut config.range_index, idx, range.label());
                        }
                    });
            });

            // Analog reference selector
            ui.horizontal(|ui| {
                ui.label("Reference:");
                egui::ComboBox::from_id_salt("ai_aref_select")
                    .selected_text(config.aref.label())
                    .show_ui(ui, |ui| {
                        for aref in AnalogReference::all() {
                            ui.selectable_value(&mut config.aref, *aref, aref.label());
                        }
                    });
            });

            ui.separator();

            // Current reading display
            ui.horizontal(|ui| {
                ui.label("Reading:");
                let reading_text = config
                    .last_reading
                    .map(|v| format!("{:.6} V", v))
                    .unwrap_or_else(|| "No reading".to_string());

                ui.label(
                    RichText::new(reading_text)
                        .size(18.0)
                        .color(Color32::LIGHT_BLUE),
                );
            });

            ui.separator();

            // Read button
            if ui.button("Read Channel").clicked() {
                self.read_channel(channel, client, runtime);
            }
        });
    }

    // =========================================================================
    // Single-shot reads
    // =========================================================================

    /// Read a single channel via NiDaqService.ReadAnalogInput RPC.
    fn read_channel(&self, channel: u32, client: Option<DaqClient>, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        let device_id = self.device_id.clone();
        #[allow(clippy::cast_possible_truncation)] // range_index is always < 14
        let range_index = self
            .channels
            .get(&channel)
            .map(|c| c.range_index as u32)
            .unwrap_or(0);

        runtime.spawn(async move {
            if let Some(mut c) = client {
                let req = ReadAnalogInputRequest {
                    device_id,
                    channel,
                    range_index,
                };
                match c.ni_daq_client().read_analog_input(req).await {
                    Ok(resp) => {
                        let r = resp.into_inner();
                        if r.success {
                            let _ = tx
                                .send(ActionResult::Reading {
                                    channel,
                                    voltage: r.voltage,
                                })
                                .await;
                        } else {
                            let _ = tx
                                .send(ActionResult::ReadingError {
                                    channel,
                                    error: r.error_message,
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(ActionResult::ReadingError {
                                channel,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }
            }
        });
    }

    /// Read all enabled channels via NiDaqService.ReadAnalogInput RPC.
    #[allow(clippy::cast_possible_truncation)] // range_index is always < 14
    fn read_all_channels(&self, runtime: &Runtime, client: &DaqClient) {
        let tx = self.action_tx.clone();
        let channels: Vec<(u32, u32)> = self
            .channels
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(ch, c)| (*ch, c.range_index as u32))
            .collect();

        let client = client.clone();
        let device_id = self.device_id.clone();

        runtime.spawn(async move {
            use futures::future::join_all;

            let futures = channels.iter().map(|&(ch, range_index)| {
                let mut client = client.clone();
                let device_id = device_id.clone();
                async move {
                    let req = ReadAnalogInputRequest {
                        device_id,
                        channel: ch,
                        range_index,
                    };
                    match client.ni_daq_client().read_analog_input(req).await {
                        Ok(resp) => {
                            let r = resp.into_inner();
                            if r.success {
                                Some((ch, r.voltage))
                            } else {
                                None
                            }
                        }
                        Err(_) => None,
                    }
                }
            });

            let results = join_all(futures).await;
            let voltages: Vec<(u32, f64)> = results.into_iter().flatten().collect();

            if !voltages.is_empty() {
                let _ = tx.send(ActionResult::AllReadings { voltages }).await;
            }
        });
    }

    // =========================================================================
    // Result polling
    // =========================================================================

    /// Poll for async results.
    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            match result {
                ActionResult::Reading { channel, voltage } => {
                    if let Some(config) = self.channels.get_mut(&channel) {
                        config.last_reading = Some(voltage);
                    }
                    self.error = None;
                }
                ActionResult::ReadingError { channel, error } => {
                    self.error = Some(format!("CH{}: {}", channel, error));
                }
                ActionResult::AllReadings { voltages } => {
                    for (channel, voltage) in voltages {
                        if let Some(config) = self.channels.get_mut(&channel) {
                            config.last_reading = Some(voltage);
                        }
                    }
                    self.error = None;
                }
                ActionResult::StreamStarted { generation } => {
                    if generation == self.stream_generation {
                        self.streaming_state = StreamingState::Running;
                        self.status = Some("Streaming".to_string());
                    }
                }
                ActionResult::StreamBatch {
                    generation,
                    voltages,
                    overflow,
                } => {
                    if generation == self.stream_generation {
                        self.stream_overflow = overflow;
                        for (channel, voltage) in voltages {
                            if let Some(config) = self.channels.get_mut(&channel) {
                                config.last_reading = Some(voltage);
                            }
                        }
                        self.error = None;
                    }
                }
                ActionResult::StreamError { generation, error } => {
                    if generation == self.stream_generation {
                        self.error = Some(error);
                        self.streaming_state = StreamingState::Stopped;
                        self.streaming_abort_tx = None;
                        self.streaming_channels.clear();
                    }
                }
                ActionResult::StreamEnded { generation } => {
                    if generation == self.stream_generation
                        && self.streaming_state != StreamingState::Stopped
                    {
                        self.streaming_state = StreamingState::Stopped;
                        self.streaming_abort_tx = None;
                        self.streaming_channels.clear();
                        self.status = Some("Stream ended".to_string());
                    }
                }
            }
        }
    }
}

/// Extract the latest voltage per channel from interleaved stream data.
///
/// The server sends interleaved data: `[ch0_s0, ch1_s0, ch0_s1, ch1_s1, ...]`.
/// We only need the most recent sample per channel for UI display, so we grab
/// the last complete scan (last `n_channels` values) from the batch.
fn de_interleave_latest(
    interleaved: &[f64],
    n_channels: usize,
    channel_numbers: &[u32],
) -> Vec<(u32, f64)> {
    if interleaved.is_empty() || n_channels == 0 {
        return Vec::new();
    }

    // Find the start of the last complete scan. Only consider fully
    // complete scans; if there are fewer samples than channels, return
    // no data to avoid misattributing voltages to channels.
    let total = interleaved.len();
    let n_scans = total / n_channels;
    if n_scans == 0 {
        return Vec::new();
    }
    let last_scan_start = (n_scans - 1) * n_channels;

    channel_numbers
        .iter()
        .enumerate()
        .filter_map(|(idx, &ch)| {
            let pos = last_scan_start + idx;
            interleaved.get(pos).map(|&v| (ch, v))
        })
        .collect()
}
