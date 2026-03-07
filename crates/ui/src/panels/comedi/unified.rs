//! Unified Comedi DAQ Control Panel.
//!
//! Combines all subsystem panels into a tabbed interface with device status overview.

use crate::runtime::Runtime;
use eframe::egui::{self, Color32, RichText, Ui};
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use client::DaqClient;
use protocol::ni_daq::{DaqStatus, TimingCapabilities};

use super::{
    AnalogInputPanel, AnalogOutputPanel, CounterPanel, DigitalIOPanel, TriggerConfigPanel,
};

/// Active tab in the unified panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComediTab {
    #[default]
    Overview,
    AnalogInput,
    AnalogOutput,
    DigitalIO,
    Counter,
    Trigger,
}

impl ComediTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::AnalogInput => "Analog In",
            Self::AnalogOutput => "Analog Out",
            Self::DigitalIO => "Digital I/O",
            Self::Counter => "Counters",
            Self::Trigger => "Trigger",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Overview => "📊",
            Self::AnalogInput => "📈",
            Self::AnalogOutput => "📉",
            Self::DigitalIO => "🔌",
            Self::Counter => "⏱",
            Self::Trigger => "⚡",
        }
    }
}

/// Device connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl ConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting...",
            Self::Connected => "Connected",
            Self::Error => "Error",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Self::Disconnected => Color32::GRAY,
            Self::Connecting => Color32::YELLOW,
            Self::Connected => Color32::GREEN,
            Self::Error => Color32::RED,
        }
    }
}

/// Unified Comedi Control Panel.
///
/// Provides a tabbed interface to all Comedi subsystem panels with
/// device status overview and quick actions.
pub struct ComediPanel {
    /// Device ID from the registry (e.g. "comedi0")
    device_id: String,
    /// Device path
    device_path: String,
    /// Board name (detected from device via get_daq_status)
    board_name: String,
    /// Driver name
    driver_name: String,
    /// Connection status
    connection_status: ConnectionStatus,
    /// Active tab
    active_tab: ComediTab,
    /// Analog input panel
    ai_panel: AnalogInputPanel,
    /// Analog output panel
    ao_panel: AnalogOutputPanel,
    /// Digital I/O panel
    dio_panel: DigitalIOPanel,
    /// Counter panel
    counter_panel: CounterPanel,
    /// Trigger configuration panel
    trigger_panel: TriggerConfigPanel,
    /// Error log
    error_log: Vec<String>,
    /// Max error log entries
    max_log_entries: usize,
    /// Timing capabilities fetched from server (None until first successful fetch)
    timing_caps: Option<TimingCapabilities>,
    /// Sender for async status results from get_daq_status + get_timing_capabilities
    status_tx: mpsc::Sender<Result<(DaqStatus, Option<TimingCapabilities>), String>>,
    /// Receiver for async status results
    status_rx: mpsc::Receiver<Result<(DaqStatus, Option<TimingCapabilities>), String>>,
    /// Whether a status fetch is in-flight
    status_fetching: bool,
}

impl ComediPanel {
    /// Create a new unified panel for a device.
    ///
    /// `device_id` is the registry key (e.g. `"comedi0"`); the `/dev/` prefix
    /// is derived automatically for display purposes.
    pub fn new(device_id: &str) -> Self {
        let (status_tx, status_rx) = mpsc::channel(4);
        let device_path = if device_id.starts_with("/dev/") {
            device_id.to_string()
        } else {
            format!("/dev/{}", device_id)
        };
        Self {
            device_id: device_id.to_string(),
            device_path,
            board_name: String::new(),
            driver_name: String::new(),
            connection_status: ConnectionStatus::Connecting,
            active_tab: ComediTab::Overview,
            ai_panel: AnalogInputPanel::new(device_id, 16),
            ao_panel: AnalogOutputPanel::new(device_id, 2),
            dio_panel: DigitalIOPanel::new(device_id, 24),
            counter_panel: CounterPanel::new(device_id, 3),
            trigger_panel: TriggerConfigPanel::new(device_id),
            error_log: Vec::new(),
            max_log_entries: 100,
            timing_caps: None,
            status_tx,
            status_rx,
            status_fetching: false,
        }
    }

    /// Main UI entry point.
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        // Drain async status results
        while let Ok(result) = self.status_rx.try_recv() {
            self.status_fetching = false;
            match result {
                Ok((status, timing)) => {
                    status.board_name.clone_into(&mut self.board_name);
                    status.driver_name.clone_into(&mut self.driver_name);
                    self.connection_status = if status.online {
                        ConnectionStatus::Connected
                    } else {
                        ConnectionStatus::Error
                    };
                    if let Some(caps) = timing {
                        self.timing_caps = Some(caps);
                    }
                    self.log_message(&format!(
                        "DAQ status: {} ({}), online={}",
                        status.board_name, status.driver_name, status.online
                    ));
                }
                Err(e) => {
                    self.connection_status = ConnectionStatus::Error;
                    self.log_error(&e);
                }
            }
        }

        // Header with device info and status
        self.render_header(ui);

        ui.separator();

        // Tab bar
        ui.horizontal(|ui| {
            for tab in [
                ComediTab::Overview,
                ComediTab::AnalogInput,
                ComediTab::AnalogOutput,
                ComediTab::DigitalIO,
                ComediTab::Counter,
                ComediTab::Trigger,
            ] {
                let label = format!("{} {}", tab.icon(), tab.label());
                if ui.selectable_label(self.active_tab == tab, label).clicked() {
                    self.active_tab = tab;
                }
            }
        });

        ui.separator();

        // Tab content
        match self.active_tab {
            ComediTab::Overview => self.render_overview(ui, client, runtime),
            ComediTab::AnalogInput => self.ai_panel.ui(ui, client, runtime),
            ComediTab::AnalogOutput => self.ao_panel.ui(ui, client, runtime),
            ComediTab::DigitalIO => self.dio_panel.ui(ui, client, runtime),
            ComediTab::Counter => self.counter_panel.ui(ui, client, runtime),
            ComediTab::Trigger => self.trigger_panel.ui(ui, client, runtime),
        }
    }

    /// Render header with device info.
    fn render_header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Comedi DAQ");

            ui.separator();

            // Device info
            ui.label(format!("Device: {}", self.device_path));
            ui.label(format!("Board: {}", self.board_name));

            ui.separator();

            // Connection status indicator
            let status_text = self.connection_status.label();
            let status_color = self.connection_status.color();
            ui.label(RichText::new(status_text).color(status_color));

            // Connect/disconnect button
            match self.connection_status {
                ConnectionStatus::Disconnected | ConnectionStatus::Error => {
                    if ui.button("Connect").clicked() {
                        self.connection_status = ConnectionStatus::Connecting;
                        self.status_fetching = false; // Allow re-fetch on next overview render
                    }
                }
                ConnectionStatus::Connected => {
                    if ui.button("Disconnect").clicked() {
                        self.connection_status = ConnectionStatus::Disconnected;
                        self.board_name.clear();
                        self.driver_name.clear();
                        self.timing_caps = None;
                        self.status_fetching = false;
                        self.log_message("Disconnected from DAQ device");
                    }
                }
                _ => {}
            }
        });
    }

    /// Fetch DAQ status + timing capabilities from the server (fire-and-forget).
    fn fetch_daq_status(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        if self.status_fetching {
            return;
        }
        self.status_fetching = true;
        self.connection_status = ConnectionStatus::Connecting;

        let tx = self.status_tx.clone();
        let device_id = self.device_id.clone();
        let mut ni_daq = client.ni_daq_client().clone();

        runtime.spawn(async move {
            let status_result = ni_daq
                .get_daq_status(protocol::ni_daq::GetDaqStatusRequest {
                    device_id: device_id.clone(),
                })
                .await;
            match status_result.map(|r| r.into_inner()) {
                Ok(status) => {
                    // Also fetch timing capabilities (best-effort)
                    let timing = ni_daq
                        .get_timing_capabilities(protocol::ni_daq::GetTimingCapabilitiesRequest {
                            device_id: device_id.clone(),
                        })
                        .await
                        .ok()
                        .map(|r| r.into_inner());
                    let _ = tx.send(Ok((status, timing))).await;
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string())).await;
                }
            }
        });
    }

    /// Render overview tab.
    fn render_overview(
        &mut self,
        ui: &mut Ui,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        // Trigger a status fetch on first render or when disconnected
        if !self.status_fetching
            && matches!(
                self.connection_status,
                ConnectionStatus::Disconnected | ConnectionStatus::Connecting
            )
        {
            if let Some(c) = client.as_mut() {
                self.fetch_daq_status(c, runtime);
            }
        }
        ui.columns(2, |columns| {
            // Left column: Device info and subsystem summary
            columns[0].group(|ui| {
                ui.label(RichText::new("Device Information").strong());
                ui.separator();

                egui::Grid::new("device_info_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Path:");
                        ui.label(&self.device_path);
                        ui.end_row();

                        ui.label("Board:");
                        ui.label(&self.board_name);
                        ui.end_row();

                        ui.label("Driver:");
                        ui.label(&self.driver_name);
                        ui.end_row();

                        ui.label("Status:");
                        ui.label(
                            RichText::new(self.connection_status.label())
                                .color(self.connection_status.color()),
                        );
                        ui.end_row();
                    });
            });

            columns[0].add_space(10.0);

            // Subsystem summary
            columns[0].group(|ui| {
                ui.label(RichText::new("Subsystems").strong());
                ui.separator();

                egui::Grid::new("subsystem_grid")
                    .num_columns(3)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Analog Input:");
                        ui.label("16 channels");
                        ui.label("16-bit, 100kS/s");
                        ui.end_row();

                        ui.label("Analog Output:");
                        ui.label("2 channels");
                        ui.label("16-bit");
                        ui.end_row();

                        ui.label("Digital I/O:");
                        ui.label("24 channels");
                        ui.label("TTL/CMOS");
                        ui.end_row();

                        ui.label("Counter/Timer:");
                        ui.label("3 counters");
                        ui.label("24-bit");
                        ui.end_row();
                    });
            });

            // Right column: Quick actions and error log
            columns[1].group(|ui| {
                ui.label(RichText::new("Quick Actions").strong());
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Read All AI").clicked() {
                        self.active_tab = ComediTab::AnalogInput;
                    }
                    if ui.button("Zero All AO").clicked() {
                        self.ao_panel
                            .zero_all_outputs(runtime, client.as_deref().cloned());
                        self.log_message("Zeroing all analog outputs");
                    }
                });

                ui.horizontal(|ui| {
                    if ui.button("Read All DIO").clicked() {
                        self.active_tab = ComediTab::DigitalIO;
                    }
                    if ui.button("Reset Counters").clicked() {
                        self.counter_panel
                            .reset_all_counters(client.as_deref().cloned(), runtime);
                        self.log_message("Resetting all counters");
                    }
                });

                ui.separator();

                if ui.button("Self Test").clicked() {
                    self.log_message("Self-test: use calibrate-comedi.sh for hardware calibration");
                }
            });

            columns[1].add_space(10.0);

            // Error log
            columns[1].group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Event Log").strong());
                    if ui.button("Clear").clicked() {
                        self.error_log.clear();
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        if self.error_log.is_empty() {
                            ui.label(
                                RichText::new("No events logged")
                                    .italics()
                                    .color(Color32::GRAY),
                            );
                        } else {
                            for msg in self.error_log.iter().rev().take(20) {
                                ui.label(RichText::new(msg).small());
                            }
                        }
                    });
            });
        });

        // Capability matrix
        ui.add_space(10.0);
        self.render_capability_matrix(ui);
    }

    /// Render capability matrix showing supported features.
    fn render_capability_matrix(&self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.label(RichText::new("Capability Matrix").strong());
            ui.separator();

            egui::Grid::new("capability_matrix")
                .num_columns(5)
                .spacing([15.0, 4.0])
                .show(ui, |ui| {
                    // Header row
                    ui.label(RichText::new("Subsystem").strong());
                    ui.label(RichText::new("Single").strong());
                    ui.label(RichText::new("Streaming").strong());
                    ui.label(RichText::new("Triggering").strong());
                    ui.label(RichText::new("DMA").strong());
                    ui.end_row();

                    // AI row — show max rate from real timing caps when available
                    ui.label("Analog Input");
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.end_row();

                    // AO row
                    ui.label("Analog Output");
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.end_row();

                    // DIO row
                    ui.label("Digital I/O");
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("—").color(Color32::GRAY));
                    ui.label(RichText::new("—").color(Color32::GRAY));
                    ui.label(RichText::new("—").color(Color32::GRAY));
                    ui.end_row();

                    // Counter row
                    ui.label("Counter/Timer");
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("—").color(Color32::GRAY));
                    ui.label(RichText::new("✓").color(Color32::GREEN));
                    ui.label(RichText::new("—").color(Color32::GRAY));
                    ui.end_row();
                });

            // Show real timing info when available
            if let Some(caps) = &self.timing_caps {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Max AI rate: {:.0} kS/s  ·  Base clock: {:.0} MHz  ·  PFI pins: {}",
                            caps.max_sample_rate_hz / 1000.0,
                            caps.base_clock_hz / 1_000_000.0,
                            caps.pfi_pins.len()
                        ))
                        .small()
                        .color(Color32::GRAY),
                    );
                });
            }
        });
    }

    /// Log a message to the event log.
    pub fn log_message(&mut self, message: &str) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.error_log.push(format!("[{}] {}", timestamp, message));

        // Trim log if too long
        while self.error_log.len() > self.max_log_entries {
            self.error_log.remove(0);
        }
    }

    /// Log an error.
    pub fn log_error(&mut self, error: &str) {
        self.log_message(&format!("ERROR: {}", error));
    }
}
