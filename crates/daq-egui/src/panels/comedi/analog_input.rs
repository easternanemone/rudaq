//! Analog Input Control Panel for Comedi DAQ devices.
//!
//! Provides channel selection, voltage range configuration, and real-time
//! voltage readout for analog input subsystems.

use eframe::egui::{self, Color32, RichText, Ui};
use std::collections::HashMap;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::{offline_notice, OfflineContext};

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
    last_refresh: std::time::Instant,
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
            last_refresh: std::time::Instant::now(),
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
        let mut panel = Self::default();
        panel.device_id = device_id.to_string();
        panel.n_channels = n_channels;
        panel
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
            if elapsed.as_millis() >= self.refresh_interval_ms as u128 {
                if let Some(c) = client.as_deref() {
                    self.read_all_channels(runtime, c);
                }
                self.last_refresh = std::time::Instant::now();
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
        ui.columns(2, |columns| {
            // Left: Channel grid
            self.render_channel_grid(&mut columns[0], runtime);

            // Right: Selected channel details
            self.render_channel_details(&mut columns[1], runtime);
        });
    }

    /// Render the channel selection grid.
    fn render_channel_grid(&mut self, ui: &mut Ui, runtime: &Runtime) {
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
                                self.read_channel(channel, runtime);
                                ui.close_menu();
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
    fn render_channel_details(&mut self, ui: &mut Ui, runtime: &Runtime) {
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
                self.read_channel(channel, runtime);
            }
        });
    }

    /// Read a single channel.
    fn read_channel(&self, channel: u32, runtime: &Runtime) {
        let tx = self.action_tx.clone();

        runtime.spawn(async move {
            // TODO: Implement actual gRPC call
            // For now, simulate a reading
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            // Simulated voltage (replace with actual read)
            let voltage = (channel as f64 * 0.1) + rand_voltage();

            let _ = tx.send(ActionResult::Reading { channel, voltage }).await;
        });
    }

    /// Read all enabled channels.
    fn read_all_channels(&self, runtime: &Runtime, client: &DaqClient) {
        let tx = self.action_tx.clone();
        let channels: Vec<u32> = self
            .channels
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(ch, _)| *ch)
            .collect();

        let client = client.clone();
        let device_id_base = self.device_id.clone();

        runtime.spawn(async move {
            use futures::future::join_all;

            // Create futures for concurrent execution
            let futures = channels.iter().map(|&ch| {
                let mut client = client.clone();
                let device_id = format!("{}/ai{}", device_id_base, ch);
                async move {
                    match client.read_value(&device_id).await {
                        Ok(response) if response.success => Some((ch, response.value)),
                        _ => None,
                    }
                }
            });

            // Wait for all reads to complete
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

/// Generate a small random voltage offset for simulation.
fn rand_voltage() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (nanos % 1000) as f64 / 10000.0 - 0.05
}
