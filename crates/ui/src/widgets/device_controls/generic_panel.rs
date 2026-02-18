//! Generic capability-based device control panel.
//!
//! Replaces per-device panels (MaiTai, PowerMeter, Rotator, Stage, AnalogOutput)
//! with a single `GenericDevicePanel` that auto-composes compact capability widgets
//! based on `DeviceInfo.capabilities`.
//!
//! Each capability renders in 1-2 rows using standard egui widgets.

use std::time::{Duration, Instant};

use egui::Ui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::device_ext::DeviceInfoExt;
use crate::widgets::Gauge;
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Check if units are power-related (W, mW, µW, nW).
fn is_power_unit(units: &str) -> bool {
    matches!(
        units.trim(),
        "W" | "w" | "mW" | "mw" | "uW" | "uw" | "µW" | "nW" | "nw" | ""
    )
}

// ---------------------------------------------------------------------------
// Action enum (unified for all capabilities)
// ---------------------------------------------------------------------------

enum GenericAction {
    // Readable
    ReadValue(Result<(f64, String), String>),
    // Movable
    Moved(Result<(), String>),
    // Emission / Shutter — bool tag: true = user command, false = refresh
    Emission(Result<bool, String>, bool),
    Shutter(Result<bool, String>, bool),
    // WavelengthTunable — bool tag: true = user command, false = refresh
    Wavelength(Result<f64, String>, bool),
    // Settable (analog output)
    SetValue(Result<f64, String>),
    // Full state fetch (position, online, etc.)
    DeviceState(Result<DeviceStateSnapshot, String>),
}

#[derive(Debug, Clone, Default)]
struct DeviceStateSnapshot {
    position: Option<f64>,
}

// ---------------------------------------------------------------------------
// Per-capability state structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ReadingState {
    raw_value: Option<f64>,
    raw_units: String,
    auto_refresh: bool,
    last_refresh: Option<Instant>,
}

impl Default for ReadingState {
    fn default() -> Self {
        Self {
            raw_value: None,
            raw_units: String::new(),
            auto_refresh: true,
            last_refresh: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MotionState {
    position: Option<f64>,
    moving: bool,
    position_input: String,
    jog_step: String,
    position_units: String,
    last_command_time: Option<Instant>,
    last_position_refresh: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
struct ToggleState {
    value: Option<bool>,
}

#[derive(Debug, Clone)]
struct WavelengthState {
    current_nm: Option<f64>,
    slider_value: f64,
    input: String,
    dragging: bool,
    min_nm: f64,
    max_nm: f64,
}

impl Default for WavelengthState {
    fn default() -> Self {
        Self {
            current_nm: None,
            slider_value: 800.0,
            input: "800".to_string(),
            dragging: false,
            min_nm: 690.0,
            max_nm: 1040.0,
        }
    }
}

#[derive(Debug, Clone)]
struct SettableState {
    voltage: f64,
    voltage_input: String,
    min_voltage: f64,
    max_voltage: f64,
}

impl Default for SettableState {
    fn default() -> Self {
        Self {
            voltage: 0.0,
            voltage_input: "0.000".to_string(),
            min_voltage: -10.0,
            max_voltage: 10.0,
        }
    }
}

// ---------------------------------------------------------------------------
// GenericDevicePanel
// ---------------------------------------------------------------------------

/// A single panel that composes compact, capability-based widgets for any device.
pub struct GenericDevicePanel {
    action_tx: mpsc::Sender<GenericAction>,
    action_rx: mpsc::Receiver<GenericAction>,
    actions_in_flight: usize,
    error: Option<String>,
    status: Option<String>,
    device_id: Option<String>,
    initial_fetch_done: bool,

    // Capability-specific state (Some = device has this capability)
    reading: Option<ReadingState>,
    motion: Option<MotionState>,
    emission: Option<ToggleState>,
    shutter: Option<ToggleState>,
    wavelength: Option<WavelengthState>,
    settable: Option<SettableState>,
}

impl GenericDevicePanel {
    const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
    const COMMAND_DEBOUNCE: Duration = Duration::from_millis(250);

    /// Create a panel from `DeviceInfo`, using `DeviceInfoExt` helpers and metadata.
    pub fn from_device_info(device: &DeviceInfo) -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);

        // Get wavelength range from metadata with fallback; sanitize to prevent
        // panics from invalid metadata (min > max or NaN).
        let (min_wl, max_wl) = device
            .metadata
            .as_ref()
            .and_then(|m| m.min_wavelength_nm.zip(m.max_wavelength_nm))
            .and_then(|(lo, hi)| {
                if lo.is_finite() && hi.is_finite() && lo <= hi {
                    Some((lo, hi))
                } else {
                    None
                }
            })
            .unwrap_or((690.0, 1040.0));

        let default_wl = 800.0_f64.clamp(min_wl, max_wl);

        // Get position units from metadata (default: no units, since movable
        // devices can be rotary or linear and we shouldn't guess).
        let position_units = device
            .metadata
            .as_ref()
            .and_then(|m| m.position_units.clone())
            .unwrap_or_default();

        Self {
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            initial_fetch_done: false,
            reading: if device.is_readable() {
                Some(ReadingState::default())
            } else {
                None
            },
            motion: if device.is_movable() {
                Some(MotionState {
                    jog_step: "1.0".to_string(),
                    position_units,
                    ..Default::default()
                })
            } else {
                None
            },
            emission: if device.is_emission_controllable() {
                Some(ToggleState::default())
            } else {
                None
            },
            shutter: if device.is_shutter_controllable() {
                Some(ToggleState::default())
            } else {
                None
            },
            wavelength: if device.is_wavelength_tunable() {
                Some(WavelengthState {
                    current_nm: None,
                    slider_value: default_wl,
                    input: format!("{:.0}", default_wl),
                    dragging: false,
                    min_nm: min_wl,
                    max_nm: max_wl,
                })
            } else {
                None
            },
            settable: if device.has_capability("settable") {
                Some(SettableState::default())
            } else {
                None
            },
        }
    }

    // -----------------------------------------------------------------------
    // Poll async results
    // -----------------------------------------------------------------------

    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            // Only decrement for user-initiated commands (not background refreshes)
            let is_user_command = matches!(
                &result,
                GenericAction::Moved(_)
                    | GenericAction::SetValue(_)
                    | GenericAction::Emission(_, true)
                    | GenericAction::Shutter(_, true)
                    | GenericAction::Wavelength(_, true)
            );
            if is_user_command {
                self.actions_in_flight = self.actions_in_flight.saturating_sub(1);
            }

            match result {
                GenericAction::ReadValue(res) => {
                    if let Some(ref mut reading) = self.reading {
                        match res {
                            Ok((value, units)) => {
                                reading.raw_value = Some(value);
                                reading.raw_units = units;
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Read failed: {}", e)),
                        }
                    }
                }
                GenericAction::Moved(res) => {
                    if let Some(ref mut m) = self.motion {
                        m.moving = false;
                    }
                    match res {
                        Ok(()) => {
                            self.status = Some("Move completed".to_string());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("Move failed: {}", e)),
                    }
                }
                GenericAction::Emission(res, _) => {
                    if let Some(ref mut t) = self.emission {
                        match res {
                            Ok(v) => {
                                t.value = Some(v);
                                self.status = Some(if v {
                                    "Emission ON".to_string()
                                } else {
                                    "Emission OFF".to_string()
                                });
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Emission: {}", e)),
                        }
                    }
                }
                GenericAction::Shutter(res, _) => {
                    if let Some(ref mut t) = self.shutter {
                        match res {
                            Ok(v) => {
                                t.value = Some(v);
                                self.status = Some(if v {
                                    "Shutter OPEN".to_string()
                                } else {
                                    "Shutter CLOSED".to_string()
                                });
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Shutter: {}", e)),
                        }
                    }
                }
                GenericAction::Wavelength(res, _) => {
                    if let Some(ref mut wl) = self.wavelength {
                        match res {
                            Ok(nm) => {
                                wl.current_nm = Some(nm);
                                if !wl.dragging {
                                    wl.slider_value = nm;
                                    wl.input = format!("{:.1}", nm);
                                }
                                self.status = Some(format!("Wavelength: {:.1} nm", nm));
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Wavelength: {}", e)),
                        }
                    }
                }
                GenericAction::SetValue(res) => {
                    if let Some(ref mut s) = self.settable {
                        match res {
                            Ok(v) => {
                                s.voltage = v;
                                s.voltage_input = format!("{:.3}", v);
                                self.status = Some(format!("Set to {:.3} V", v));
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Write failed: {}", e)),
                        }
                    }
                }
                GenericAction::DeviceState(Ok(snap)) => {
                    if let Some(ref mut m) = self.motion {
                        if let Some(pos) = snap.position {
                            m.position = Some(pos);
                            if !m.moving {
                                m.position_input = format!("{:.2}", pos);
                            }
                        }
                    }
                }
                GenericAction::DeviceState(Err(_)) => {
                    // Silently ignore device state fetch errors
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Async dispatch helpers
    // -----------------------------------------------------------------------

    fn read_power(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else { return };
        // No actions_in_flight increment — this is a background refresh
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .read_value(&id)
                .await
                .map(|r| (r.value, r.units))
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::ReadValue(result)).await;
        });
        if let Some(ref mut reading) = self.reading {
            reading.last_refresh = Some(Instant::now());
        }
    }

    fn fetch_state(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else { return };
        // No actions_in_flight increment — this is a background refresh
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .get_device_state(&id)
                .await
                .map(|proto| DeviceStateSnapshot {
                    position: proto.position,
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::DeviceState(result)).await;
        });
        if let Some(ref mut m) = self.motion {
            m.last_position_refresh = Some(Instant::now());
        }
    }

    fn fetch_full_state(
        &mut self,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        // Fetch device state (position, online)
        self.fetch_state(client.as_deref_mut(), runtime, device_id);

        // Fetch emission/shutter/wavelength if applicable
        if self.emission.is_some() || self.shutter.is_some() || self.wavelength.is_some() {
            let Some(client) = client else { return };
            let has_emission = self.emission.is_some();
            let has_shutter = self.shutter.is_some();
            let has_wavelength = self.wavelength.is_some();
            // No actions_in_flight increment — these are background refresh queries
            let mut c = client.clone();
            let tx = self.action_tx.clone();
            let id = device_id.to_string();

            runtime.spawn(async move {
                // Sequential queries for serial devices (is_command = false)
                if has_emission {
                    let result = c.get_emission(&id).await.map_err(|e| e.to_string());
                    let _ = tx.send(GenericAction::Emission(result, false)).await;
                }
                if has_shutter {
                    let result = c.get_shutter(&id).await.map_err(|e| e.to_string());
                    let _ = tx.send(GenericAction::Shutter(result, false)).await;
                }
                if has_wavelength {
                    let result = c.get_wavelength(&id).await.map_err(|e| e.to_string());
                    let _ = tx.send(GenericAction::Wavelength(result, false)).await;
                }
            });
        }
    }

    fn move_absolute(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        position: f64,
    ) {
        let Some(client) = client else { return };
        if let Some(ref mut m) = self.motion {
            if !can_send_command(m.last_command_time, Self::COMMAND_DEBOUNCE) {
                return;
            }
            m.moving = true;
            m.last_command_time = Some(Instant::now());
        }
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .move_absolute(&id, position)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::Moved(result)).await;
        });
    }

    fn move_relative(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        delta: f64,
    ) {
        let Some(client) = client else { return };
        if let Some(ref mut m) = self.motion {
            if !can_send_command(m.last_command_time, Self::COMMAND_DEBOUNCE) {
                return;
            }
            m.moving = true;
            m.last_command_time = Some(Instant::now());
        }
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .move_relative(&id, delta)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::Moved(result)).await;
        });
    }

    fn set_emission_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        enabled: bool,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .set_emission(&id, enabled)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::Emission(result, true)).await;
        });
    }

    fn set_shutter_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        open: bool,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c.set_shutter(&id, open).await.map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::Shutter(result, true)).await;
        });
    }

    fn set_wavelength_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        nm: f64,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c.set_wavelength(&id, nm).await.map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::Wavelength(result, true)).await;
        });
    }

    fn write_voltage_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        voltage: f64,
    ) {
        let Some(client) = client else { return };
        let voltage = if let Some(ref s) = self.settable {
            voltage.clamp(s.min_voltage, s.max_voltage)
        } else {
            voltage
        };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .set_parameter(&id, "voltage", &voltage.to_string())
                .await
                .map(|_| voltage)
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::SetValue(result)).await;
        });
    }

    // -----------------------------------------------------------------------
    // UI rendering
    // -----------------------------------------------------------------------

    /// Render the generic device panel.
    pub fn ui(
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
            self.fetch_full_state(client.as_deref_mut(), runtime, &device_id);
            if self.reading.is_some() {
                self.read_power(client.as_deref_mut(), runtime, &device_id);
            }
        }

        // Auto-refresh for readable devices
        if let Some(ref reading) = self.reading {
            let should = reading.auto_refresh
                && self.actions_in_flight == 0
                && reading
                    .last_refresh
                    .map(|t| t.elapsed() >= Self::REFRESH_INTERVAL)
                    .unwrap_or(true);
            if should && client.is_some() {
                self.read_power(client.as_deref_mut(), runtime, &device_id);
            }
        }

        // Auto-refresh for motion devices (position polling, independent timer)
        if self.motion.is_some() && self.actions_in_flight == 0 {
            let should_refresh_position = self
                .motion
                .as_ref()
                .and_then(|m| m.last_position_refresh)
                .map(|t| t.elapsed() >= Self::REFRESH_INTERVAL)
                .unwrap_or(true);
            if should_refresh_position && client.is_some() {
                self.fetch_state(client.as_deref_mut(), runtime, &device_id);
            }
        }

        // Error/status
        if let Some(ref err) = self.error {
            ui.colored_label(egui::Color32::RED, err);
        }
        if let Some(ref status) = self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        let is_busy = self.actions_in_flight > 0;

        // --- Reading row ---
        if let Some(ref reading) = self.reading {
            render_reading_row(ui, reading);
        }

        // --- Emission + Shutter row (compact, same line) ---
        if self.emission.is_some() || self.shutter.is_some() {
            ui.horizontal(|ui| {
                if let Some(ref emission) = self.emission {
                    let is_on = emission.value.unwrap_or(false);
                    ui.label("Em:");
                    let text = if is_on { "ON" } else { "OFF" };
                    let color = if is_on {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::GRAY
                    };
                    let btn = egui::Button::new(egui::RichText::new(text).color(color))
                        .min_size(egui::vec2(40.0, 18.0));
                    if ui.add(btn).clicked() {
                        self.set_emission_rpc(client.as_deref_mut(), runtime, &device_id, !is_on);
                    }
                }
                if let Some(ref shutter) = self.shutter {
                    if self.emission.is_some() {
                        ui.separator();
                    }
                    let is_open = shutter.value.unwrap_or(false);
                    let (text, text_color, fill) = if is_open {
                        (
                            "SHUTTER OPEN",
                            egui::Color32::WHITE,
                            crate::layout::colors::ERROR,
                        )
                    } else {
                        (
                            "Shutter Closed",
                            egui::Color32::WHITE,
                            crate::layout::colors::SUCCESS,
                        )
                    };
                    let btn =
                        egui::Button::new(egui::RichText::new(text).color(text_color).strong())
                            .fill(fill)
                            .min_size(egui::vec2(100.0, 18.0));
                    if ui.add(btn).clicked() {
                        self.set_shutter_rpc(client.as_deref_mut(), runtime, &device_id, !is_open);
                    }
                }
            });
        }

        // --- Wavelength row ---
        if self.wavelength.is_some() {
            let mut wl = self.wavelength.take().unwrap();
            let wl_min = wl.min_nm;
            let wl_max = wl.max_nm;
            ui.horizontal(|ui| {
                ui.label("\u{03bb}:");
                let slider_resp = ui.add(
                    egui::Slider::new(&mut wl.slider_value, wl_min..=wl_max)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                );

                let response = ui.add(
                    egui::TextEdit::singleline(&mut wl.input)
                        .desired_width(45.0)
                        .hint_text("nm"),
                );
                ui.label("nm");

                if slider_resp.drag_started() {
                    wl.dragging = true;
                }
                if slider_resp.drag_stopped() {
                    wl.dragging = false;
                    wl.input = format!("{:.1}", wl.slider_value);
                    self.set_wavelength_rpc(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        wl.slider_value,
                    );
                }
                if wl.dragging {
                    wl.input = format!("{:.1}", wl.slider_value);
                }

                if ui.button("Set").clicked() {
                    if let Ok(nm) = wl.input.parse::<f64>() {
                        if (wl_min..=wl_max).contains(&nm) {
                            self.set_wavelength_rpc(client.as_deref_mut(), runtime, &device_id, nm);
                        } else {
                            self.error = Some(format!("Wavelength must be {wl_min}-{wl_max} nm"));
                        }
                    }
                }

                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Ok(nm) = wl.input.parse::<f64>() {
                        if (wl_min..=wl_max).contains(&nm) {
                            self.set_wavelength_rpc(client.as_deref_mut(), runtime, &device_id, nm);
                        }
                    }
                }
            });
            self.wavelength = Some(wl);
        }

        // --- Motion row (single compact row) ---
        if self.motion.is_some() {
            let mut motion = self.motion.take().unwrap();
            let motion_busy = motion.moving || is_busy;
            let pos_units = motion.position_units.as_str();

            ui.horizontal(|ui| {
                // Position display
                if let Some(pos) = motion.position {
                    let text = if pos_units.is_empty() {
                        format!("{pos:.2}")
                    } else {
                        format!("{pos:.2} {pos_units}")
                    };
                    ui.label(egui::RichText::new(text).monospace());
                } else {
                    ui.label(egui::RichText::new("---").monospace());
                }

                let step: f64 = motion.jog_step.parse().unwrap_or(1.0);

                // Jog buttons
                if ui
                    .add_enabled(
                        !motion_busy,
                        egui::Button::new(format!("{:.0}", -step * 10.0)),
                    )
                    .clicked()
                {
                    self.move_relative(client.as_deref_mut(), runtime, &device_id, -step * 10.0);
                }
                if ui
                    .add_enabled(!motion_busy, egui::Button::new(format!("{:.0}", -step)))
                    .clicked()
                {
                    self.move_relative(client.as_deref_mut(), runtime, &device_id, -step);
                }
                if ui
                    .add_enabled(!motion_busy, egui::Button::new(format!("+{:.0}", step)))
                    .clicked()
                {
                    self.move_relative(client.as_deref_mut(), runtime, &device_id, step);
                }
                if ui
                    .add_enabled(
                        !motion_busy,
                        egui::Button::new(format!("+{:.0}", step * 10.0)),
                    )
                    .clicked()
                {
                    self.move_relative(client.as_deref_mut(), runtime, &device_id, step * 10.0);
                }

                ui.separator();

                // Go-to input
                let response = ui.add(
                    egui::TextEdit::singleline(&mut motion.position_input)
                        .desired_width(50.0)
                        .hint_text("go to"),
                );
                if ui
                    .add_enabled(!motion_busy, egui::Button::new("Go"))
                    .clicked()
                {
                    if let Ok(pos) = motion.position_input.parse::<f64>() {
                        self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
                    } else {
                        self.error = Some("Invalid position".to_string());
                    }
                }
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !motion_busy
                {
                    if let Ok(pos) = motion.position_input.parse::<f64>() {
                        self.move_absolute(client.as_deref_mut(), runtime, &device_id, pos);
                    }
                }

                if ui
                    .add_enabled(!motion_busy, egui::Button::new("H"))
                    .on_hover_text("Home (0.0)")
                    .clicked()
                {
                    self.move_absolute(client.as_deref_mut(), runtime, &device_id, 0.0);
                }

                ui.separator();

                // Step size
                ui.label("stp:");
                ui.add(egui::TextEdit::singleline(&mut motion.jog_step).desired_width(30.0));
            });

            self.motion = Some(motion);
        }

        // --- Settable row (analog output, single compact row) ---
        if self.settable.is_some() {
            let mut settable = self.settable.take().unwrap();

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{:.3}V", settable.voltage)).monospace());

                let mut voltage = settable.voltage;
                let slider =
                    egui::Slider::new(&mut voltage, settable.min_voltage..=settable.max_voltage)
                        .suffix("V")
                        .clamping(egui::SliderClamping::Always);

                if ui.add_enabled(!is_busy, slider).changed() {
                    settable.voltage = voltage;
                    settable.voltage_input = format!("{:.3}", voltage);
                    self.write_voltage_rpc(client.as_deref_mut(), runtime, &device_id, voltage);
                }

                ui.separator();

                if ui.add_enabled(!is_busy, egui::Button::new("0V")).clicked() {
                    self.write_voltage_rpc(client.as_deref_mut(), runtime, &device_id, 0.0);
                }
                if ui
                    .add_enabled(
                        !is_busy,
                        egui::Button::new(format!("{:.0}V", settable.min_voltage)),
                    )
                    .clicked()
                {
                    self.write_voltage_rpc(
                        client.as_deref_mut(),
                        runtime,
                        &device_id,
                        settable.min_voltage,
                    );
                }
                if ui
                    .add_enabled(
                        !is_busy,
                        egui::Button::new(format!("{:.0}V", settable.max_voltage)),
                    )
                    .clicked()
                {
                    self.write_voltage_rpc(client, runtime, &device_id, settable.max_voltage);
                }
            });

            self.settable = Some(settable);
        }

        // Spinner when busy
        if is_busy {
            ui.spinner();
        }

        // Request repaint for auto-refresh
        if (self
            .reading
            .as_ref()
            .map(|r| r.auto_refresh)
            .unwrap_or(false))
            || is_busy
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn render_reading_row(ui: &mut Ui, reading: &ReadingState) {
    ui.horizontal(|ui| {
        let raw = reading.raw_value.unwrap_or(0.0);
        let units = &reading.raw_units;

        if is_power_unit(units) {
            let power_mw = normalize_power_to_mw(raw, units);
            let (value, unit, max_val) = if power_mw >= 1000.0 {
                (power_mw as f32 / 1000.0, "W", 5.0)
            } else if power_mw >= 1.0 {
                (power_mw as f32, "mW", 1000.0)
            } else {
                (power_mw as f32 * 1000.0, "\u{00b5}W", 1000.0)
            };

            ui.add(
                Gauge::new(value)
                    .range(0.0, max_val)
                    .label("Power")
                    .unit(unit)
                    .size(28.0),
            );
            ui.label(egui::RichText::new(format!("{value:.4} {unit}")).monospace());
        } else {
            let display_unit = if units.is_empty() { "" } else { units.as_str() };
            let raw_f32 = raw as f32;
            let max_val = if raw_f32.abs() < 1.0 {
                1.0
            } else {
                raw_f32.abs() * 2.0
            };

            ui.add(
                Gauge::new(raw_f32)
                    .range(-max_val, max_val)
                    .label("Value")
                    .unit(display_unit)
                    .size(28.0),
            );
            ui.label(egui::RichText::new(format!("{raw:.4} {display_unit}")).monospace());
        }
    });
}

fn normalize_power_to_mw(value: f64, units: &str) -> f64 {
    match units.trim() {
        "W" | "w" => value * 1000.0,
        "mW" | "mw" => value,
        "uW" | "uw" | "µW" => value / 1000.0,
        "nW" | "nw" => value / 1_000_000.0,
        "" => value * 1000.0,
        _ => value,
    }
}

fn can_send_command(last: Option<Instant>, debounce: Duration) -> bool {
    last.map(|t| t.elapsed() >= debounce).unwrap_or(true)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// Helper: build a minimal `DeviceInfo` with the given capability strings.
    fn device_with_caps(caps: &[&str]) -> DeviceInfo {
        DeviceInfo {
            id: "test".into(),
            name: "Test Device".into(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_from_device_info_readable_movable() {
        let device = device_with_caps(&["readable", "movable"]);
        let panel = GenericDevicePanel::from_device_info(&device);
        assert!(panel.reading.is_some());
        assert!(panel.motion.is_some());
        assert!(panel.emission.is_none());
        assert!(panel.shutter.is_none());
        assert!(panel.wavelength.is_none());
        assert!(panel.settable.is_none());
    }

    #[test]
    fn test_from_device_info_laser() {
        let device = device_with_caps(&[
            "readable",
            "emission_controllable",
            "shutter_controllable",
            "wavelength_tunable",
        ]);
        let panel = GenericDevicePanel::from_device_info(&device);
        assert!(panel.reading.is_some());
        assert!(panel.emission.is_some());
        assert!(panel.shutter.is_some());
        assert!(panel.wavelength.is_some());
        assert!(panel.motion.is_none());
        assert!(panel.settable.is_none());
    }

    #[test]
    fn test_from_device_info_settable() {
        let device = device_with_caps(&["settable"]);
        let panel = GenericDevicePanel::from_device_info(&device);
        assert!(panel.settable.is_some());
        assert!(panel.reading.is_none());
    }

    #[test]
    fn test_normalize_power() {
        assert_eq!(normalize_power_to_mw(1.0, "W"), 1000.0);
        assert_eq!(normalize_power_to_mw(5.0, "mW"), 5.0);
        assert_eq!(normalize_power_to_mw(1000.0, "uW"), 1.0);
        assert_eq!(normalize_power_to_mw(1_000_000.0, "nW"), 1.0);
        assert_eq!(normalize_power_to_mw(0.001, ""), 1.0);
        assert_eq!(normalize_power_to_mw(42.0, "dBm"), 42.0);
    }

    #[test]
    fn test_can_send_command_none() {
        assert!(can_send_command(None, Duration::from_millis(250)));
    }

    #[test]
    fn test_can_send_command_expired() {
        let past = Instant::now().checked_sub(Duration::from_millis(500));
        assert!(can_send_command(past, Duration::from_millis(250)));
    }

    #[test]
    fn test_can_send_command_too_soon() {
        let now = Some(Instant::now());
        assert!(!can_send_command(now, Duration::from_millis(250)));
    }

    #[test]
    fn test_from_device_info_empty() {
        let device = device_with_caps(&[]);
        let panel = GenericDevicePanel::from_device_info(&device);
        assert!(panel.reading.is_none());
        assert!(panel.motion.is_none());
        assert!(panel.emission.is_none());
        assert!(panel.shutter.is_none());
        assert!(panel.wavelength.is_none());
        assert!(panel.settable.is_none());
    }

    #[test]
    fn test_from_device_info_all() {
        let device = device_with_caps(&[
            "readable",
            "movable",
            "emission_controllable",
            "shutter_controllable",
            "wavelength_tunable",
            "settable",
        ]);
        let panel = GenericDevicePanel::from_device_info(&device);
        assert!(panel.reading.is_some());
        assert!(panel.motion.is_some());
        assert!(panel.emission.is_some());
        assert!(panel.shutter.is_some());
        assert!(panel.wavelength.is_some());
        assert!(panel.settable.is_some());
    }

    #[test]
    fn test_normalize_power_edge_cases() {
        assert_eq!(normalize_power_to_mw(0.0, "W"), 0.0);
        assert_eq!(normalize_power_to_mw(1000000.0, "W"), 1000000000.0);
        assert_eq!(normalize_power_to_mw(-5.0, "mW"), -5.0);
    }

    #[test]
    fn test_normalize_power_case_insensitive() {
        assert_eq!(normalize_power_to_mw(1.0, "w"), 1000.0);
        // "MW" and "UW" are not recognized as power units; passthrough unchanged
        assert_eq!(normalize_power_to_mw(5.0, "MW"), 5.0);
        assert_eq!(normalize_power_to_mw(1000.0, "UW"), 1000.0);
    }

    #[test]
    fn test_can_send_command_exactly_at_boundary() {
        let exactly_250ms_ago = Instant::now().checked_sub(Duration::from_millis(250));
        assert!(can_send_command(
            exactly_250ms_ago,
            Duration::from_millis(250)
        ));
    }

    #[test]
    fn test_reading_state_default() {
        let state = ReadingState::default();
        assert_eq!(state.raw_value, None);
        assert_eq!(state.raw_units, "");
        assert!(state.auto_refresh);
        assert_eq!(state.last_refresh, None);
    }

    #[test]
    fn test_motion_state_default() {
        let state = MotionState::default();
        assert_eq!(state.position, None);
        assert!(!state.moving);
        assert_eq!(state.position_input, "");
        assert_eq!(state.jog_step, "");
        assert_eq!(state.position_units, "");
        assert_eq!(state.last_command_time, None);
        assert_eq!(state.last_position_refresh, None);
    }

    #[test]
    fn test_wavelength_state_default() {
        let state = WavelengthState::default();
        assert_eq!(state.current_nm, None);
        assert_eq!(state.slider_value, 800.0);
        assert_eq!(state.input, "800");
        assert!(!state.dragging);
        assert_eq!(state.min_nm, 690.0);
        assert_eq!(state.max_nm, 1040.0);
    }

    #[test]
    fn test_toggle_state_default() {
        let state = ToggleState::default();
        assert_eq!(state.value, None);
    }

    #[test]
    fn test_is_power_unit() {
        assert!(is_power_unit("W"));
        assert!(is_power_unit("mW"));
        assert!(is_power_unit("µW"));
        assert!(is_power_unit("nW"));
        assert!(is_power_unit(""));
        assert!(!is_power_unit("V"));
        assert!(!is_power_unit("dBm"));
    }
}
