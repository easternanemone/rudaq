//! Newport 1830-C Power Meter control panel.
//!
//! Provides:
//! - Real-time power reading gauge
//! - Wavelength calibration input
//! - Units display

use egui::Ui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client::DaqClient;
use crate::widgets::device_controls::DeviceControlWidget;
use crate::widgets::Gauge;
use daq_proto::daq::DeviceInfo;

/// Power meter state cached from the daemon
#[derive(Debug, Clone, Default)]
struct MeterState {
    power_mw: Option<f64>,
    wavelength_nm: Option<f64>,
    loading: bool,
}

/// Async action results
enum ActionResult {
    ReadPower(Result<f64, String>),
    GetWavelength(Result<f64, String>),
    SetWavelength(Result<f64, String>),
}

/// Newport 1830-C Power Meter control panel
pub struct PowerMeterControlPanel {
    state: MeterState,
    wavelength_input: String,
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
    actions_in_flight: usize,
    error: Option<String>,
    status: Option<String>,
    device_id: Option<String>,
    initial_fetch_done: bool,
    /// Auto-refresh enabled
    auto_refresh: bool,
    /// Last refresh time for auto-refresh
    last_refresh: Option<std::time::Instant>,
}

impl Default for PowerMeterControlPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            state: MeterState::default(),
            wavelength_input: "800".to_string(),
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            initial_fetch_done: false,
            auto_refresh: true,
            last_refresh: None,
        }
    }
}

impl PowerMeterControlPanel {
    /// Auto-refresh interval
    const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            self.actions_in_flight = self.actions_in_flight.saturating_sub(1);

            match result {
                ActionResult::ReadPower(result) => match result {
                    Ok(power) => {
                        self.state.power_mw = Some(power);
                        self.state.loading = false;
                    }
                    Err(e) => {
                        self.error = Some(format!("Read failed: {}", e));
                        self.state.loading = false;
                    }
                },
                ActionResult::GetWavelength(result) => {
                    if let Ok(wl) = result {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_input = format!("{:.0}", wl);
                    }
                }
                ActionResult::SetWavelength(result) => match result {
                    Ok(wl) => {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_input = format!("{:.0}", wl);
                        self.status = Some(format!("Calibration wavelength set to {} nm", wl));
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to set wavelength: {}", e));
                    }
                },
            }
        }
    }

    fn read_power(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            return;
        };

        self.actions_in_flight += 1;
        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .read_value(&device_id)
                .await
                .map(|r| r.value)
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::ReadPower(result)).await;
        });

        self.last_refresh = Some(std::time::Instant::now());
    }

    fn fetch_wavelength(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else {
            return;
        };

        self.actions_in_flight += 1;
        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .get_wavelength(&device_id)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::GetWavelength(result)).await;
        });
    }

    fn set_wavelength(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        wavelength_nm: f64,
    ) {
        let Some(client) = client else {
            self.error = Some("Not connected".to_string());
            return;
        };

        self.actions_in_flight += 1;
        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_wavelength(&device_id, wavelength_nm)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetWavelength(result)).await;
        });
    }
}

impl DeviceControlWidget for PowerMeterControlPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.poll_results();

        let device_id = device.id.clone();
        self.device_id = Some(device_id.clone());

        // Initial fetch
        if !self.initial_fetch_done && client.is_some() {
            self.initial_fetch_done = true;
            tracing::info!("[PowerMeter] Initial fetch for device={}", device_id);
            self.read_power(client.as_deref_mut(), runtime, &device_id);
            self.fetch_wavelength(client.as_deref_mut(), runtime, &device_id);
        }

        // Auto-refresh logic
        let should_refresh = self.auto_refresh
            && self.actions_in_flight == 0
            && self
                .last_refresh
                .map(|t| t.elapsed() >= Self::REFRESH_INTERVAL)
                .unwrap_or(true);

        if should_refresh && client.is_some() {
            tracing::debug!("[PowerMeter] Auto-refresh read for device={}", device_id);
            self.read_power(client.as_deref_mut(), runtime, &device_id);
        } else if should_refresh && client.is_none() {
            tracing::warn!("[PowerMeter] Auto-refresh skipped: no client for device={}", device_id);
        }

        // Header
        ui.horizontal(|ui| {
            ui.heading("⚡ Power Meter");
            if self.state.loading || self.actions_in_flight > 0 {
                ui.spinner();
            }
        });

        if let Some(ref err) = self.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        // Power gauge (large, centered)
        ui.vertical_centered(|ui| {
            let power = self.state.power_mw.unwrap_or(0.0) as f32;

            // Determine range and units based on power level
            let (value, unit, max_val) = if power >= 1000.0 {
                (power / 1000.0, "W", 5.0)
            } else if power >= 1.0 {
                (power, "mW", 1000.0)
            } else {
                (power * 1000.0, "µW", 1000.0)
            };

            ui.add(
                Gauge::new(value)
                    .range(0.0, max_val)
                    .label("Power")
                    .unit(unit)
                    .size(100.0),
            );

            // Exact value display
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{:.4} mW", self.state.power_mw.unwrap_or(0.0)))
                    .monospace()
                    .size(14.0),
            );
        });

        ui.add_space(8.0);
        ui.separator();

        // Wavelength calibration
        ui.label(egui::RichText::new("Wavelength Calibration").strong());

        ui.horizontal(|ui| {
            ui.label("λ:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.wavelength_input)
                    .desired_width(60.0)
                    .hint_text("nm"),
            );
            ui.label("nm");

            if ui.button("Set").clicked() {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                } else {
                    self.error = Some("Invalid wavelength".to_string());
                }
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                }
            }
        });

        if let Some(wl) = self.state.wavelength_nm {
            ui.label(format!("Current calibration: {} nm", wl));
        }

        ui.add_space(8.0);
        ui.separator();

        // Controls
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_refresh, "Auto-refresh");

            if ui.button("🔄 Read Now").clicked() {
                self.read_power(client.as_deref_mut(), runtime, &device_id);
            }
        });

        // Request repaint for auto-refresh
        if self.auto_refresh || self.actions_in_flight > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn device_type(&self) -> &'static str {
        "Power Meter"
    }
}
