//! Trigger Configuration Panel for Comedi DAQ devices.
//!
//! Provides configuration for hardware triggers, timing, and synchronization
//! across analog input, analog output, and counter subsystems.

use crate::runtime::Runtime;
use client::DaqClient;
use eframe::egui::{self, Color32, RichText, Ui};
use protocol::ni_daq::ConfigureTriggerRequest;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Trigger source options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TriggerSource {
    #[default]
    Software,
    /// External trigger on PFI line
    PFI0,
    PFI1,
    PFI2,
    PFI3,
    /// RTSI bus trigger
    RTSI0,
    RTSI1,
    /// Internal timer
    InternalClock,
    /// Analog trigger (threshold crossing)
    AnalogTrigger,
}

impl TriggerSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Software => "Software",
            Self::PFI0 => "PFI0",
            Self::PFI1 => "PFI1",
            Self::PFI2 => "PFI2",
            Self::PFI3 => "PFI3",
            Self::RTSI0 => "RTSI0",
            Self::RTSI1 => "RTSI1",
            Self::InternalClock => "Internal Clock",
            Self::AnalogTrigger => "Analog Trigger",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Software,
            Self::PFI0,
            Self::PFI1,
            Self::PFI2,
            Self::PFI3,
            Self::RTSI0,
            Self::RTSI1,
            Self::InternalClock,
            Self::AnalogTrigger,
        ]
    }
}

/// Trigger edge polarity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TriggerPolarity {
    #[default]
    Rising,
    Falling,
    Either,
}

impl TriggerPolarity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rising => "Rising Edge",
            Self::Falling => "Falling Edge",
            Self::Either => "Either Edge",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Rising, Self::Falling, Self::Either]
    }
}

/// Clock source for timed acquisition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClockSource {
    #[default]
    Internal20MHz,
    Internal100kHz,
    ExternalPFI0,
    ExternalRTSI0,
}

impl ClockSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Internal20MHz => "Internal 20MHz",
            Self::Internal100kHz => "Internal 100kHz",
            Self::ExternalPFI0 => "External (PFI0)",
            Self::ExternalRTSI0 => "External (RTSI0)",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Internal20MHz,
            Self::Internal100kHz,
            Self::ExternalPFI0,
            Self::ExternalRTSI0,
        ]
    }
}

/// Trigger configuration for a subsystem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemTriggerConfig {
    /// Whether hardware triggering is enabled
    pub enabled: bool,
    /// Start trigger source
    pub start_source: TriggerSource,
    /// Start trigger polarity
    pub start_polarity: TriggerPolarity,
    /// Convert/sample trigger source (for timed acquisition)
    pub convert_source: TriggerSource,
    /// Sample rate (Hz) when using internal clock
    pub sample_rate: f64,
    /// Number of samples to acquire (0 = continuous)
    pub samples_per_trigger: u32,
    /// Analog trigger level (V) for analog trigger mode
    pub analog_trigger_level: f64,
    /// Analog trigger channel for analog trigger mode
    pub analog_trigger_channel: u32,
    /// Re-trigger mode (auto re-arm after acquisition)
    pub retrigger: bool,
}

impl Default for SubsystemTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_source: TriggerSource::Software,
            start_polarity: TriggerPolarity::Rising,
            convert_source: TriggerSource::InternalClock,
            sample_rate: 10000.0,
            samples_per_trigger: 1000,
            analog_trigger_level: 0.0,
            analog_trigger_channel: 0,
            retrigger: false,
        }
    }
}

/// Async result from apply_configuration
#[derive(Debug)]
enum ApplyResult {
    Ok,
    Err(String),
}

/// Trigger Configuration Panel
pub struct TriggerConfigPanel {
    /// Device ID
    device_id: String,
    /// AI trigger configuration
    ai_config: SubsystemTriggerConfig,
    /// AO trigger configuration
    ao_config: SubsystemTriggerConfig,
    /// Counter trigger configuration
    counter_config: SubsystemTriggerConfig,
    /// Currently selected tab
    selected_tab: TriggerTab,
    /// Clock source
    clock_source: ClockSource,
    /// Master/slave sync mode
    sync_mode: SyncMode,
    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async channel for apply results
    apply_tx: mpsc::Sender<ApplyResult>,
    apply_rx: mpsc::Receiver<ApplyResult>,
}

/// Tab selection for trigger panel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TriggerTab {
    #[default]
    AnalogInput,
    AnalogOutput,
    Counter,
    Timing,
}

impl TriggerTab {
    fn label(self) -> &'static str {
        match self {
            Self::AnalogInput => "Analog Input",
            Self::AnalogOutput => "Analog Output",
            Self::Counter => "Counter",
            Self::Timing => "Timing/Sync",
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::AnalogInput,
            Self::AnalogOutput,
            Self::Counter,
            Self::Timing,
        ]
    }
}

/// Synchronization mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SyncMode {
    #[default]
    Standalone,
    Master,
    Slave,
}

impl SyncMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standalone => "Standalone",
            Self::Master => "Master",
            Self::Slave => "Slave",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Standalone, Self::Master, Self::Slave]
    }
}

impl Default for TriggerConfigPanel {
    fn default() -> Self {
        let (apply_tx, apply_rx) = mpsc::channel(4);
        Self {
            device_id: String::from("comedi0"),
            ai_config: SubsystemTriggerConfig::default(),
            ao_config: SubsystemTriggerConfig::default(),
            counter_config: SubsystemTriggerConfig::default(),
            selected_tab: TriggerTab::AnalogInput,
            clock_source: ClockSource::Internal20MHz,
            sync_mode: SyncMode::Standalone,
            status: None,
            error: None,
            apply_tx,
            apply_rx,
        }
    }
}

impl TriggerConfigPanel {
    /// Create a new trigger configuration panel
    pub fn new(device_id: &str) -> Self {
        Self {
            device_id: device_id.to_string(),
            ..Self::default()
        }
    }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll async apply results
        while let Ok(result) = self.apply_rx.try_recv() {
            match result {
                ApplyResult::Ok => {
                    self.status = Some("Trigger configuration applied".to_string());
                    self.error = None;
                }
                ApplyResult::Err(e) => {
                    self.error = Some(e);
                }
            }
        }

        let client_clone = client.as_deref().cloned();

        // Header
        ui.horizontal(|ui| {
            ui.heading("Trigger Configuration");
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

        // Tab selection
        ui.horizontal(|ui| {
            for tab in TriggerTab::all() {
                if ui
                    .selectable_label(self.selected_tab == *tab, tab.label())
                    .clicked()
                {
                    self.selected_tab = *tab;
                }
            }
        });

        ui.separator();

        // Tab content
        match self.selected_tab {
            TriggerTab::AnalogInput => self.render_subsystem_config(ui, "Analog Input"),
            TriggerTab::AnalogOutput => self.render_ao_config(ui),
            TriggerTab::Counter => self.render_counter_config(ui),
            TriggerTab::Timing => self.render_timing_config(ui),
        }

        ui.separator();

        // Apply button - capture click outside the closure
        let (apply_clicked, reset_clicked) = ui
            .horizontal(|ui| {
                (
                    ui.button("Apply Configuration").clicked(),
                    ui.button("Reset to Defaults").clicked(),
                )
            })
            .inner;

        if apply_clicked {
            self.apply_configuration(client_clone, runtime);
        }
        if reset_clicked {
            self.reset_defaults();
        }
    }

    /// Render subsystem trigger configuration (AI)
    fn render_subsystem_config(&mut self, ui: &mut Ui, _label: &str) {
        let config = &mut self.ai_config;

        ui.group(|ui| {
            ui.checkbox(&mut config.enabled, "Enable Hardware Triggering");

            if !config.enabled {
                ui.label(RichText::new("Software triggering (immediate start)").italics());
                return;
            }

            ui.separator();

            // Start trigger
            ui.label(RichText::new("Start Trigger").strong());
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("ai_start_source")
                    .selected_text(config.start_source.label())
                    .show_ui(ui, |ui| {
                        for src in TriggerSource::all() {
                            ui.selectable_value(&mut config.start_source, *src, src.label());
                        }
                    });

                ui.label("Edge:");
                egui::ComboBox::from_id_salt("ai_start_polarity")
                    .selected_text(config.start_polarity.label())
                    .show_ui(ui, |ui| {
                        for pol in TriggerPolarity::all() {
                            ui.selectable_value(&mut config.start_polarity, *pol, pol.label());
                        }
                    });
            });

            // Analog trigger settings
            if config.start_source == TriggerSource::AnalogTrigger {
                ui.horizontal(|ui| {
                    ui.label("Trigger Channel:");
                    ui.add(egui::DragValue::new(&mut config.analog_trigger_channel).range(0..=15));
                    ui.label("Level:");
                    ui.add(
                        egui::DragValue::new(&mut config.analog_trigger_level)
                            .range(-10.0..=10.0)
                            .speed(0.1)
                            .suffix(" V"),
                    );
                });
            }

            ui.separator();

            // Sample clock / convert trigger
            ui.label(RichText::new("Sample Clock").strong());
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("ai_convert_source")
                    .selected_text(config.convert_source.label())
                    .show_ui(ui, |ui| {
                        for src in TriggerSource::all() {
                            ui.selectable_value(&mut config.convert_source, *src, src.label());
                        }
                    });
            });

            if config.convert_source == TriggerSource::InternalClock {
                ui.horizontal(|ui| {
                    ui.label("Sample Rate:");
                    ui.add(
                        egui::DragValue::new(&mut config.sample_rate)
                            .range(1.0..=100000.0)
                            .speed(100.0)
                            .suffix(" Hz"),
                    );
                });
            }

            ui.separator();

            // Acquisition settings
            ui.label(RichText::new("Acquisition").strong());
            ui.horizontal(|ui| {
                ui.label("Samples per trigger:");
                ui.add(egui::DragValue::new(&mut config.samples_per_trigger).range(0..=1000000));
                if config.samples_per_trigger == 0 {
                    ui.label(RichText::new("(continuous)").italics());
                }
            });

            ui.checkbox(&mut config.retrigger, "Auto re-arm after acquisition");
        });
    }

    /// Render AO trigger configuration
    fn render_ao_config(&mut self, ui: &mut Ui) {
        let config = &mut self.ao_config;

        ui.group(|ui| {
            ui.checkbox(&mut config.enabled, "Enable Hardware Triggering");

            if !config.enabled {
                ui.label(RichText::new("Software triggering (immediate output)").italics());
                return;
            }

            ui.separator();

            // Start trigger
            ui.label(RichText::new("Start Trigger").strong());
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("ao_start_source")
                    .selected_text(config.start_source.label())
                    .show_ui(ui, |ui| {
                        for src in TriggerSource::all() {
                            ui.selectable_value(&mut config.start_source, *src, src.label());
                        }
                    });

                ui.label("Edge:");
                egui::ComboBox::from_id_salt("ao_start_polarity")
                    .selected_text(config.start_polarity.label())
                    .show_ui(ui, |ui| {
                        for pol in TriggerPolarity::all() {
                            ui.selectable_value(&mut config.start_polarity, *pol, pol.label());
                        }
                    });
            });

            ui.separator();

            // Update clock
            ui.label(RichText::new("Update Clock").strong());
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("ao_convert_source")
                    .selected_text(config.convert_source.label())
                    .show_ui(ui, |ui| {
                        for src in TriggerSource::all() {
                            ui.selectable_value(&mut config.convert_source, *src, src.label());
                        }
                    });
            });

            if config.convert_source == TriggerSource::InternalClock {
                ui.horizontal(|ui| {
                    ui.label("Update Rate:");
                    ui.add(
                        egui::DragValue::new(&mut config.sample_rate)
                            .range(1.0..=100000.0)
                            .speed(100.0)
                            .suffix(" Hz"),
                    );
                });
            }

            ui.checkbox(&mut config.retrigger, "Loop waveform output");
        });
    }

    /// Render counter trigger configuration
    fn render_counter_config(&mut self, ui: &mut Ui) {
        let config = &mut self.counter_config;

        ui.group(|ui| {
            ui.checkbox(&mut config.enabled, "Enable External Gate/Trigger");

            if !config.enabled {
                ui.label(RichText::new("Internal gating (always enabled)").italics());
                return;
            }

            ui.separator();

            ui.label(RichText::new("Gate Source").strong());
            ui.horizontal(|ui| {
                ui.label("Source:");
                egui::ComboBox::from_id_salt("ctr_start_source")
                    .selected_text(config.start_source.label())
                    .show_ui(ui, |ui| {
                        for src in TriggerSource::all() {
                            ui.selectable_value(&mut config.start_source, *src, src.label());
                        }
                    });

                ui.label("Polarity:");
                egui::ComboBox::from_id_salt("ctr_start_polarity")
                    .selected_text(config.start_polarity.label())
                    .show_ui(ui, |ui| {
                        for pol in TriggerPolarity::all() {
                            ui.selectable_value(&mut config.start_polarity, *pol, pol.label());
                        }
                    });
            });

            ui.separator();

            ui.label(RichText::new("Gating Mode").strong());
            ui.label("Counter counts only while gate is active.");
        });
    }

    /// Render timing/sync configuration
    fn render_timing_config(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Clock Configuration").strong());

            ui.horizontal(|ui| {
                ui.label("Timebase:");
                egui::ComboBox::from_id_salt("clock_source")
                    .selected_text(self.clock_source.label())
                    .show_ui(ui, |ui| {
                        for src in ClockSource::all() {
                            ui.selectable_value(&mut self.clock_source, *src, src.label());
                        }
                    });
            });

            ui.separator();

            ui.label(RichText::new("Multi-Device Synchronization").strong());

            ui.horizontal(|ui| {
                ui.label("Sync Mode:");
                egui::ComboBox::from_id_salt("sync_mode")
                    .selected_text(self.sync_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in SyncMode::all() {
                            ui.selectable_value(&mut self.sync_mode, *mode, mode.label());
                        }
                    });
            });

            match self.sync_mode {
                SyncMode::Standalone => {
                    ui.label("Device operates independently.");
                }
                SyncMode::Master => {
                    ui.label("Device exports clock and trigger signals via RTSI bus.");
                    ui.label(RichText::new("Connect RTSI cables to slave devices.").italics());
                }
                SyncMode::Slave => {
                    ui.label("Device receives clock and trigger from RTSI bus.");
                    ui.label(RichText::new("Ensure master device is configured.").italics());
                }
            }
        });

        ui.separator();

        // PFI routing
        ui.group(|ui| {
            ui.label(RichText::new("PFI Line Routing").strong());
            ui.label("Configure programmable function interface lines.");

            egui::Grid::new("pfi_routing")
                .num_columns(3)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Line");
                    ui.label("Direction");
                    ui.label("Function");
                    ui.end_row();

                    for i in 0..4 {
                        ui.label(format!("PFI{}", i));
                        ui.label("Input"); // Simplified - would be configurable
                        ui.label(if i == 0 { "AI Start Trigger" } else { "Unused" });
                        ui.end_row();
                    }
                });
        });
    }

    /// Apply the current configuration via gRPC.
    fn apply_configuration(&mut self, client: Option<DaqClient>, runtime: &Runtime) {
        // Map UI trigger source to proto enum (qualified to avoid name clash with local TriggerSource)
        let proto_source = match self.ai_config.start_source {
            TriggerSource::Software | TriggerSource::InternalClock => {
                protocol::ni_daq::TriggerSource::Software as i32
            }
            TriggerSource::PFI0
            | TriggerSource::PFI1
            | TriggerSource::PFI2
            | TriggerSource::PFI3
            | TriggerSource::RTSI0
            | TriggerSource::RTSI1 => protocol::ni_daq::TriggerSource::External as i32,
            TriggerSource::AnalogTrigger => protocol::ni_daq::TriggerSource::Analog as i32,
        };
        let proto_edge = match self.ai_config.start_polarity {
            TriggerPolarity::Rising => protocol::ni_daq::TriggerEdge::Rising as i32,
            TriggerPolarity::Falling => protocol::ni_daq::TriggerEdge::Falling as i32,
            TriggerPolarity::Either => protocol::ni_daq::TriggerEdge::Either as i32,
        };
        let pfi_pin = match self.ai_config.start_source {
            TriggerSource::PFI0 => 0,
            TriggerSource::PFI1 => 1,
            TriggerSource::PFI2 => 2,
            TriggerSource::PFI3 => 3,
            _ => 0,
        };

        let config = protocol::ni_daq::TriggerConfig {
            source: proto_source,
            edge: proto_edge,
            pfi_pin,
            level: self.ai_config.analog_trigger_level,
            hysteresis: 0.0,
            delay_samples: 0,
            retriggerable: self.ai_config.retrigger,
        };

        let req = ConfigureTriggerRequest {
            device_id: self.device_id.clone(),
            config: Some(config),
        };

        let tx = self.apply_tx.clone();
        runtime.spawn(async move {
            if let Some(mut c) = client {
                match c.ni_daq_client().configure_trigger(req).await {
                    Ok(resp) => {
                        let r = resp.into_inner();
                        if r.success {
                            let _ = tx.send(ApplyResult::Ok).await;
                        } else {
                            let _ = tx.send(ApplyResult::Err(r.error_message)).await;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(ApplyResult::Err(e.to_string())).await;
                    }
                }
            } else {
                // Offline: optimistic apply
                let _ = tx.send(ApplyResult::Ok).await;
            }
        });
    }

    /// Reset to default configuration
    fn reset_defaults(&mut self) {
        self.ai_config = SubsystemTriggerConfig::default();
        self.ao_config = SubsystemTriggerConfig::default();
        self.counter_config = SubsystemTriggerConfig::default();
        self.clock_source = ClockSource::Internal20MHz;
        self.sync_mode = SyncMode::Standalone;
        self.status = Some("Reset to defaults".to_string());
        self.error = None;
    }
}
