//! Counter/Timer Control Panel for Comedi DAQ devices.
//!
//! Provides counter mode selection, count display, and pulse generation control.

use eframe::egui::{self, Color32, RichText, Ui};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::widgets::{offline_notice, OfflineContext};
use daq_client::DaqClient;

use super::CounterMode;

/// Action results from async operations.
#[derive(Debug)]
enum ActionResult {
    CountValue { counter: u32, count: u64 },
    ResetSuccess { counter: u32 },
    PulseStopped { counter: u32 },
    Error { counter: u32, error: String },
}

/// Per-counter configuration.
#[derive(Debug, Clone)]
struct CounterConfig {
    mode: CounterMode,
    count: u64,
    gate_source: GateSource,
    // Pulse generation parameters
    frequency: f64,
    duty_cycle: f64,
    // Measurement display
    last_frequency: Option<f64>,
    last_period: Option<f64>,
}

impl Default for CounterConfig {
    fn default() -> Self {
        Self {
            mode: CounterMode::EventCount,
            count: 0,
            gate_source: GateSource::Internal,
            frequency: 1000.0,
            duty_cycle: 50.0,
            last_frequency: None,
            last_period: None,
        }
    }
}

/// Gate source for counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateSource {
    #[default]
    Internal,
    PFI0,
    PFI1,
    RTSI0,
    RTSI1,
}

impl GateSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Internal => "Internal",
            Self::PFI0 => "PFI0",
            Self::PFI1 => "PFI1",
            Self::RTSI0 => "RTSI0",
            Self::RTSI1 => "RTSI1",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Internal,
            Self::PFI0,
            Self::PFI1,
            Self::RTSI0,
            Self::RTSI1,
        ]
    }
}

/// Counter/Timer Control Panel.
pub struct CounterPanel {
    /// Device ID
    device_id: String,
    /// Number of counters (3 for NI PCI-MIO-16XE-10)
    n_counters: u32,
    /// Per-counter configuration
    counters: Vec<CounterConfig>,
    /// Selected counter for detailed view
    selected_counter: usize,
    /// Auto-refresh
    auto_refresh: bool,
    /// Refresh interval
    refresh_interval_ms: u32,
    /// Last refresh time
    last_refresh: std::time::Instant,
    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Async channels
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
}

impl Default for CounterPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);

        Self {
            device_id: String::from("comedi0"),
            n_counters: 3,
            counters: vec![CounterConfig::default(); 3],
            selected_counter: 0,
            auto_refresh: false,
            refresh_interval_ms: 100,
            last_refresh: std::time::Instant::now(),
            status: None,
            error: None,
            action_tx,
            action_rx,
        }
    }
}

impl CounterPanel {
    /// Create a new panel.
    pub fn new(device_id: &str, n_counters: u32) -> Self {
        Self {
            device_id: device_id.to_string(),
            n_counters,
            counters: vec![CounterConfig::default(); n_counters as usize],
            ..Self::default()
        }
    }

    /// Main UI entry point.
    pub fn ui(&mut self, ui: &mut Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        if offline_notice(ui, client.is_none(), OfflineContext::Devices) {
            return;
        }

        self.poll_results();

        // Auto-refresh
        if self.auto_refresh {
            let elapsed = self.last_refresh.elapsed();
            if elapsed.as_millis() >= self.refresh_interval_ms as u128 {
                self.read_all_counters(runtime);
                self.last_refresh = std::time::Instant::now();
            }
            ui.ctx().request_repaint();
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("Counter/Timer");
            ui.separator();
            ui.label(format!(
                "Device: {} ({} counters)",
                self.device_id, self.n_counters
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

        // Control bar - capture button clicks and handle after closure
        let (read_all, reset_all) = ui
            .horizontal(|ui| {
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
                let reset = ui.button("Reset All").clicked();
                (read, reset)
            })
            .inner;

        if read_all {
            self.read_all_counters(runtime);
        }
        if reset_all {
            self.reset_all_counters(runtime);
        }

        ui.separator();

        // Counter tabs
        ui.horizontal(|ui| {
            for i in 0..self.n_counters as usize {
                let count = self.counters[i].count;
                let label = format!("CTR{} ({})", i, count);
                ui.selectable_value(&mut self.selected_counter, i, label);
            }
        });

        ui.separator();

        // Selected counter details
        self.render_counter_details(ui, runtime);
    }

    /// Render details for selected counter.
    fn render_counter_details(&mut self, ui: &mut Ui, runtime: &Runtime) {
        let counter_idx = self.selected_counter;

        // Actions to execute after UI rendering
        let mut read_action = false;
        let mut reset_action = false;
        let mut stop_action = false;

        ui.group(|ui| {
            ui.label(RichText::new(format!("Counter {}", counter_idx)).strong());

            // Count display
            let count = self.counters[counter_idx].count;
            ui.horizontal(|ui| {
                ui.label("Count:");
                ui.label(
                    RichText::new(format!("{}", count))
                        .monospace()
                        .size(24.0)
                        .color(Color32::LIGHT_BLUE),
                );
            });

            ui.separator();

            // Mode selector
            let current_mode = self.counters[counter_idx].mode;
            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_id_salt("counter_mode")
                    .selected_text(current_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in CounterMode::all() {
                            ui.selectable_value(
                                &mut self.counters[counter_idx].mode,
                                *mode,
                                mode.label(),
                            );
                        }
                    });
            });

            // Gate source
            let current_gate = self.counters[counter_idx].gate_source;
            ui.horizontal(|ui| {
                ui.label("Gate:");
                egui::ComboBox::from_id_salt("gate_source")
                    .selected_text(current_gate.label())
                    .show_ui(ui, |ui| {
                        for src in GateSource::all() {
                            ui.selectable_value(
                                &mut self.counters[counter_idx].gate_source,
                                *src,
                                src.label(),
                            );
                        }
                    });
            });

            ui.separator();

            // Mode-specific controls
            let mode = self.counters[counter_idx].mode;
            match mode {
                CounterMode::EventCount => {
                    ui.label("Event Counting Mode");
                    ui.label("Counts rising edges on input signal.");

                    ui.horizontal(|ui| {
                        if ui.button("Read").clicked() {
                            read_action = true;
                        }
                        if ui.button("Reset").clicked() {
                            reset_action = true;
                        }
                    });
                }
                CounterMode::FrequencyMeasurement => {
                    ui.label("Frequency Measurement Mode");
                    if let Some(freq) = self.counters[counter_idx].last_frequency {
                        ui.horizontal(|ui| {
                            ui.label("Frequency:");
                            let (value, unit) = if freq >= 1_000_000.0 {
                                (freq / 1_000_000.0, "MHz")
                            } else if freq >= 1000.0 {
                                (freq / 1000.0, "kHz")
                            } else {
                                (freq, "Hz")
                            };
                            ui.label(
                                RichText::new(format!("{:.3} {}", value, unit))
                                    .size(18.0)
                                    .color(Color32::LIGHT_GREEN),
                            );
                        });
                    } else {
                        ui.label("Waiting for measurement...");
                    }
                }
                CounterMode::PeriodMeasurement => {
                    ui.label("Period Measurement Mode");
                    if let Some(period) = self.counters[counter_idx].last_period {
                        ui.horizontal(|ui| {
                            ui.label("Period:");
                            let (value, unit) = if period < 0.001 {
                                (period * 1_000_000.0, "us")
                            } else if period < 1.0 {
                                (period * 1000.0, "ms")
                            } else {
                                (period, "s")
                            };
                            ui.label(
                                RichText::new(format!("{:.3} {}", value, unit))
                                    .size(18.0)
                                    .color(Color32::LIGHT_GREEN),
                            );
                        });
                    } else {
                        ui.label("Waiting for measurement...");
                    }
                }
                CounterMode::PulseGeneration => {
                    ui.label("Pulse Generation Mode");

                    ui.horizontal(|ui| {
                        ui.label("Frequency:");
                        ui.add(
                            egui::DragValue::new(&mut self.counters[counter_idx].frequency)
                                .range(0.1..=10_000_000.0)
                                .speed(100.0)
                                .suffix(" Hz"),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label("Duty Cycle:");
                        ui.add(
                            egui::Slider::new(
                                &mut self.counters[counter_idx].duty_cycle,
                                1.0..=99.0,
                            )
                            .suffix("%")
                            .clamping(egui::SliderClamping::Always),
                        );
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Start Output").clicked() {
                            self.status = Some("Pulse generation not yet implemented".to_string());
                        }
                        if ui.button("Stop").clicked() {
                            stop_action = true;
                        }
                    });
                }
                CounterMode::QuadratureEncoder => {
                    ui.label("Quadrature Encoder Mode");
                    ui.label("Decodes A/B phase signals from rotary encoder.");
                    let position = self.counters[counter_idx].count as i64;
                    ui.horizontal(|ui| {
                        ui.label("Position:");
                        ui.label(
                            RichText::new(format!("{}", position))
                                .size(18.0)
                                .color(Color32::LIGHT_BLUE),
                        );
                    });
                }
                CounterMode::PulseWidth => {
                    ui.label("Pulse Width Measurement Mode");
                    if let Some(period) = self.counters[counter_idx].last_period {
                        ui.horizontal(|ui| {
                            ui.label("Pulse Width:");
                            let (value, unit) = if period < 0.001 {
                                (period * 1_000_000.0, "us")
                            } else if period < 1.0 {
                                (period * 1000.0, "ms")
                            } else {
                                (period, "s")
                            };
                            ui.label(
                                RichText::new(format!("{:.3} {}", value, unit))
                                    .size(18.0)
                                    .color(Color32::LIGHT_GREEN),
                            );
                        });
                    }
                }
            }
        });

        // Execute deferred actions
        if read_action {
            self.read_counter(counter_idx as u32, runtime);
        }
        if reset_action {
            self.reset_counter(counter_idx as u32, runtime);
        }
        if stop_action {
            self.stop_pulse_generation(counter_idx as u32, runtime);
        }
    }

    // Async operations
    fn stop_pulse_generation(&self, counter: u32, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = tx.send(ActionResult::PulseStopped { counter }).await;
        });
    }

    fn read_counter(&self, counter: u32, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            // Simulated count value
            let count = (counter as u64 + 1) * 1000;
            let _ = tx.send(ActionResult::CountValue { counter, count }).await;
        });
    }

    fn reset_counter(&self, counter: u32, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let _ = tx.send(ActionResult::ResetSuccess { counter }).await;
        });
    }

    fn read_all_counters(&self, runtime: &Runtime) {
        for i in 0..self.n_counters {
            self.read_counter(i, runtime);
        }
    }

    fn reset_all_counters(&mut self, runtime: &Runtime) {
        let tx = self.action_tx.clone();
        let n = self.n_counters;
        for counter in &mut self.counters {
            counter.count = 0;
        }
        runtime.spawn(async move {
            for i in 0..n {
                let _ = tx.send(ActionResult::ResetSuccess { counter: i }).await;
            }
        });
    }

    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            match result {
                ActionResult::CountValue { counter, count } => {
                    if let Some(c) = self.counters.get_mut(counter as usize) {
                        c.count = count;
                    }
                    self.error = None;
                }
                ActionResult::ResetSuccess { counter } => {
                    if let Some(c) = self.counters.get_mut(counter as usize) {
                        c.count = 0;
                    }
                    self.status = Some(format!("CTR{} reset", counter));
                    self.error = None;
                }
                ActionResult::PulseStopped { counter } => {
                    self.status = Some(format!("CTR{} pulse generation stopped", counter));
                    self.error = None;
                }
                ActionResult::Error { counter, error } => {
                    self.error = Some(format!("CTR{}: {}", counter, error));
                }
            }
        }
    }
}
