//! Analog Input Control Panel for Comedi DAQ devices.
//!
//! Provides channel selection, voltage range configuration, and real-time
//! voltage readout for analog input subsystems.

use crate::runtime::Runtime;
use eframe::egui::{self, Color32, RichText, Ui};
use protocol::ni_daq::ReadAnalogInputRequest;
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use client::DaqClient;

use super::{AnalogReference, NI_VOLTAGE_RANGES};

/// Action results from async operations.
#[derive(Debug)]
enum ActionResult {
    Reading { channel: u32, voltage: f64 },
    ReadingError { channel: u32, error: String },
    AllReadings { voltages: Vec<(u32, f64)> },
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
pub struct AnalogInputPanel {
    /// Device ID for the Comedi device
    device_id: String,
    /// Number of channels (16 for NI PCI-MIO-16XE-10)
    n_channels: u32,
    /// Per-channel configuration
    channels: HashMap<u32, ChannelConfig>,
    /// Currently selected channel for detailed view
    selected_channel: u32,
    /// Auto-refresh enabled
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
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Check for offline mode
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        // Poll async results
        self.poll_results();

        // Auto-refresh logic
        if self.auto_refresh {
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
            // Auto-refresh toggle
            ui.checkbox(&mut self.auto_refresh, "Auto-refresh");

            if self.auto_refresh {
                ui.add(
                    egui::DragValue::new(&mut self.refresh_interval_ms)
                        .range(50..=1000)
                        .suffix(" ms")
                        .speed(10),
                );
            }

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

            // Read all button
            if ui.button("Read All").clicked() {
                if let Some(c) = client.as_deref() {
                    self.read_all_channels(runtime, c);
                }
            }
        });

        ui.separator();

        // Main content: channel grid + detail panel
        let client_clone = client.as_deref().cloned();
        ui.columns(2, |columns| {
            // Left: Channel grid
            self.render_channel_grid(&mut columns[0], client_clone.clone(), runtime);

            // Right: Selected channel details
            self.render_channel_details(&mut columns[1], client_clone, runtime);
        });
    }

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
            }
        }
    }
}
