//! MaiTai Ti:Sapphire laser control panel.
//!
//! Provides controls for:
//! - Emission toggle (laser on/off)
//! - Shutter toggle (open/close)
//! - Wavelength control (690-1040nm)
//! - Power display gauge

use egui::Ui;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tracing;

use crate::client::DaqClient;
use crate::widgets::device_controls::DeviceControlWidget;
use crate::widgets::Gauge;
use daq_proto::daq::DeviceInfo;

/// Polling interval for state updates (1 second)
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// MaiTai laser state cached from the daemon
#[derive(Debug, Clone, Default)]
struct LaserState {
    emission_enabled: Option<bool>,
    shutter_open: Option<bool>,
    wavelength_nm: Option<f64>,
    power_mw: Option<f64>,
    /// True if a fetch is in progress
    loading: bool,
}

/// Async action results for the MaiTai panel
enum ActionResult {
    FetchState {
        emission: Option<bool>,
        shutter: Option<bool>,
        wavelength: Option<f64>,
        power: Option<f64>,
    },
    SetEmission(Result<bool, String>),
    SetShutter(Result<bool, String>),
    SetWavelength(Result<f64, String>),
}

/// MaiTai Ti:Sapphire laser control panel
pub struct MaiTaiControlPanel {
    /// Cached laser state
    state: LaserState,
    /// Wavelength slider value (for live dragging)
    wavelength_slider: f64,
    /// Wavelength text input
    wavelength_input: String,
    /// Whether wavelength slider is being dragged
    wavelength_dragging: bool,
    /// Async action channel
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
    /// Actions in flight counter
    actions_in_flight: usize,
    /// Error message to display
    error: Option<String>,
    /// Status message to display
    status: Option<String>,
    /// Device ID for tracking (set on first render)
    device_id: Option<String>,
    /// Initial state fetch done
    initial_fetch_done: bool,
    /// Last time we polled for state updates
    last_poll_time: Instant,
}

impl Default for MaiTaiControlPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            state: LaserState::default(),
            wavelength_slider: 800.0,
            wavelength_input: "800".to_string(),
            wavelength_dragging: false,
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            initial_fetch_done: false,
            last_poll_time: Instant::now(),
        }
    }
}

impl MaiTaiControlPanel {
    /// Poll for async results
    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            self.actions_in_flight = self.actions_in_flight.saturating_sub(1);

            match result {
                ActionResult::FetchState {
                    emission,
                    shutter,
                    wavelength,
                    power,
                } => {
                    self.state.emission_enabled = emission;
                    self.state.shutter_open = shutter;
                    if let Some(wl) = wavelength {
                        self.state.wavelength_nm = Some(wl);
                        if !self.wavelength_dragging {
                            self.wavelength_slider = wl;
                            self.wavelength_input = format!("{:.1}", wl);
                        }
                    }
                    self.state.power_mw = power;
                    self.state.loading = false;
                }
                ActionResult::SetEmission(result) => match result {
                    Ok(enabled) => {
                        self.state.emission_enabled = Some(enabled);
                        self.status = Some(if enabled {
                            "Emission ON".to_string()
                        } else {
                            "Emission OFF".to_string()
                        });
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to set emission: {}", e));
                    }
                },
                ActionResult::SetShutter(result) => match result {
                    Ok(open) => {
                        self.state.shutter_open = Some(open);
                        self.status = Some(if open {
                            "Shutter OPEN".to_string()
                        } else {
                            "Shutter CLOSED".to_string()
                        });
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to set shutter: {}", e));
                    }
                },
                ActionResult::SetWavelength(result) => match result {
                    Ok(wl) => {
                        self.state.wavelength_nm = Some(wl);
                        self.wavelength_slider = wl;
                        self.wavelength_input = format!("{:.1}", wl);
                        self.status = Some(format!("Wavelength set to {:.1} nm", wl));
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to set wavelength: {}", e));
                    }
                },
            }
        }
    }

    /// Fetch current laser state from daemon
    fn fetch_state(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else {
            return;
        };

        self.state.loading = true;
        self.actions_in_flight += 1;

        let client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            // IMPORTANT: Query sequentially, NOT in parallel!
            // Serial devices can only handle one command at a time.
            // Parallel queries cause response interleaving bugs.
            let mut client = client;

            let emission = client.get_emission(&device_id).await.ok();
            let shutter = client.get_shutter(&device_id).await.ok();
            let wavelength = client.get_wavelength(&device_id).await.ok();
            let power = client.read_value(&device_id).await.ok().map(|r| r.value);

            let _ = tx
                .send(ActionResult::FetchState {
                    emission,
                    shutter,
                    wavelength,
                    power,
                })
                .await;
        });
    }

    /// Set emission state
    fn set_emission(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        enabled: bool,
    ) {
        tracing::info!("[GUI] set_emission called: device={}, enabled={}, client_present={}", device_id, enabled, client.is_some());
        let Some(client) = client else {
            tracing::warn!("[GUI] set_emission: NO CLIENT - cannot send RPC!");
            self.error = Some("Not connected".to_string());
            return;
        };
        tracing::info!("[GUI] set_emission: spawning async task to call RPC");

        self.actions_in_flight += 1;
        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_emission(&device_id, enabled)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetEmission(result)).await;
        });
    }

    /// Set shutter state
    fn set_shutter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        open: bool,
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
                .set_shutter(&device_id, open)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetShutter(result)).await;
        });
    }

    /// Set wavelength
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

impl DeviceControlWidget for MaiTaiControlPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        // Poll for async results
        self.poll_results();

        // Track device ID
        let device_id = device.id.clone();
        self.device_id = Some(device_id.clone());

        // Initial state fetch
        if !self.initial_fetch_done && client.is_some() {
            self.initial_fetch_done = true;
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Periodic state polling (every POLL_INTERVAL)
        if client.is_some()
            && self.actions_in_flight == 0
            && self.last_poll_time.elapsed() >= POLL_INTERVAL
        {
            self.last_poll_time = Instant::now();
            self.fetch_state(client.as_deref_mut(), runtime, &device_id);
        }

        // Request continuous repaint for polling
        ui.ctx().request_repaint_after(POLL_INTERVAL);

        // Header with device name
        ui.horizontal(|ui| {
            ui.heading("🔴 MaiTai Ti:Sapphire Laser");
            if self.state.loading || self.actions_in_flight > 0 {
                ui.spinner();
            }
        });

        // Error/status messages
        if let Some(ref err) = self.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        ui.separator();

        // Main control area - two columns layout
        ui.columns(2, |cols| {
            // Left column: Power gauge
            cols[0].vertical_centered(|ui| {
                // Note: MaiTai read:pow? returns WATTS, not milliwatts
                let power = self.state.power_mw.unwrap_or(0.0) as f32;
                ui.add(
                    Gauge::new(power)
                        .range(0.0, 5.0) // 0-5 Watts typical range
                        .label("Power")
                        .unit(" W")
                        .size(80.0),
                );
            });

            // Right column: Controls
            cols[1].vertical(|ui| {
                // Emission control
                ui.horizontal(|ui| {
                    ui.label("Emission:");
                    let is_on = self.state.emission_enabled.unwrap_or(false);

                    // Single button that toggles state
                    let button_text = if is_on { "🟢 ON" } else { "⚫ OFF" };
                    let button = egui::Button::new(button_text)
                        .min_size(egui::vec2(80.0, 24.0));

                    if ui.add(button).clicked() {
                        let new_state = !is_on;
                        tracing::info!("[GUI] Emission button clicked! Setting to {}", new_state);
                        self.set_emission(client.as_deref_mut(), runtime, &device_id, new_state);
                    }
                });

                ui.add_space(4.0);

                // Shutter control
                ui.horizontal(|ui| {
                    ui.label("Shutter:");
                    let is_open = self.state.shutter_open.unwrap_or(false);

                    // Single button that toggles state
                    let button_text = if is_open { "🟡 OPEN" } else { "⬛ CLOSED" };
                    let button = egui::Button::new(button_text)
                        .min_size(egui::vec2(100.0, 24.0));

                    if ui.add(button).clicked() {
                        let new_state = !is_open;
                        tracing::info!("[GUI] Shutter button clicked! Setting to {}", new_state);
                        self.set_shutter(client.as_deref_mut(), runtime, &device_id, new_state);
                    }

                    // Safety indicator - warn if shutter open but emission off
                    let shutter_state = self.state.shutter_open.unwrap_or(false);
                    let emission_state = self.state.emission_enabled.unwrap_or(true);
                    if shutter_state && !emission_state {
                        ui.colored_label(egui::Color32::YELLOW, "⚠");
                    }
                });
            });
        });

        ui.add_space(8.0);
        ui.separator();

        // Wavelength control section
        ui.label(egui::RichText::new("Wavelength Control").strong());

        ui.horizontal(|ui| {
            ui.label("Wavelength:");

            // Text input
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.wavelength_input)
                    .desired_width(60.0)
                    .hint_text("nm"),
            );

            ui.label("nm");

            // Set button for text input
            if ui.button("Set").clicked() {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    if (690.0..=1040.0).contains(&wl) {
                        self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                    } else {
                        self.error = Some("Wavelength must be between 690-1040 nm".to_string());
                    }
                } else {
                    self.error = Some("Invalid wavelength value".to_string());
                }
            }

            // Update text input on enter
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(wl) = self.wavelength_input.parse::<f64>() {
                    if (690.0..=1040.0).contains(&wl) {
                        self.set_wavelength(client.as_deref_mut(), runtime, &device_id, wl);
                    }
                }
            }
        });

        // Wavelength slider
        ui.horizontal(|ui| {
            ui.label("690");

            let slider_response = ui.add(
                egui::Slider::new(&mut self.wavelength_slider, 690.0..=1040.0)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );

            ui.label("1040");

            // Track dragging state
            if slider_response.drag_started() {
                self.wavelength_dragging = true;
            }

            // Send command when drag ends
            if slider_response.drag_stopped() {
                self.wavelength_dragging = false;
                self.wavelength_input = format!("{:.1}", self.wavelength_slider);
                self.set_wavelength(
                    client.as_deref_mut(),
                    runtime,
                    &device_id,
                    self.wavelength_slider,
                );
            }

            // Update text input while dragging
            if self.wavelength_dragging {
                self.wavelength_input = format!("{:.1}", self.wavelength_slider);
            }
        });

        // Current wavelength display
        if let Some(wl) = self.state.wavelength_nm {
            ui.horizontal(|ui| {
                ui.label("Current:");
                ui.label(
                    egui::RichText::new(format!("{:.1} nm", wl))
                        .monospace()
                        .strong(),
                );
            });
        }

        ui.add_space(8.0);

        // Advanced section (collapsible)
        ui.collapsing("▶ Advanced Parameters", |ui| {
            if ui.button("🔄 Refresh State").clicked() {
                self.fetch_state(client.as_deref_mut(), runtime, &device_id);
            }

            egui::Grid::new("maitai_params")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Device ID:");
                    ui.label(&device_id);
                    ui.end_row();

                    ui.label("Driver:");
                    ui.label(&device.driver_type);
                    ui.end_row();

                    ui.label("Emission:");
                    ui.label(format!(
                        "{}",
                        self.state
                            .emission_enabled
                            .map(|e| if e { "Enabled" } else { "Disabled" })
                            .unwrap_or("Unknown")
                    ));
                    ui.end_row();

                    ui.label("Shutter:");
                    ui.label(format!(
                        "{}",
                        self.state
                            .shutter_open
                            .map(|o| if o { "Open" } else { "Closed" })
                            .unwrap_or("Unknown")
                    ));
                    ui.end_row();

                    ui.label("Wavelength:");
                    ui.label(format!(
                        "{} nm",
                        self.state
                            .wavelength_nm
                            .map(|w| format!("{:.1}", w))
                            .unwrap_or_else(|| "-".to_string())
                    ));
                    ui.end_row();

                    ui.label("Power:");
                    ui.label(format!(
                        "{} W",
                        self.state
                            .power_mw
                            .map(|p| format!("{:.1}", p))
                            .unwrap_or_else(|| "-".to_string())
                    ));
                    ui.end_row();
                });
        });

        // Request repaint if actions in flight
        if self.actions_in_flight > 0 {
            ui.ctx().request_repaint();
        }
    }

    fn device_type(&self) -> &'static str {
        "MaiTai Ti:Sapphire Laser"
    }
}
