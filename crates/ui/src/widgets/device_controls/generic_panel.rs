//! Generic capability-based device control panel.
//!
//! Replaces per-device panels (MaiTai, PowerMeter, Rotator, Stage, AnalogOutput)
//! with a single `GenericDevicePanel` that auto-composes compact capability widgets
//! based on `DeviceInfo.capabilities`.
//!
//! Each capability renders in 1-2 rows using standard egui widgets.

use crate::layout;
use crate::runtime::Runtime;
use crate::time::{Duration, Instant};
use egui::Ui;

use crate::device_ext::DeviceInfoExt;
use crate::widgets::Gauge;
use crate::widgets::device_controls::{
    DevicePanelState, LatestRequestTracker, action_button, filled_action_button, panel_hint_text,
    panel_value_text, parse_f64_input, parse_positive_step_input, request_panel_repaint,
    show_panel_header, show_panel_messages, show_panel_section,
};
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
    // Readable refresh
    ReadValue {
        request_id: u64,
        result: Result<(f64, String), String>,
    },
    // Movable user command
    Moved(Result<(), String>),
    // Emission / Shutter / Wavelength user commands
    EmissionCommand(Result<bool, String>),
    ShutterCommand(Result<bool, String>),
    WavelengthCommand(Result<f64, String>),
    // Settable (analog output)
    SetValue(Result<f64, String>),
    // Full state fetch (position, online, etc.)
    DeviceState {
        request_id: u64,
        result: Result<DeviceStateSnapshot, String>,
    },
    // Coalesced background refresh for capability state
    AuxState {
        request_id: u64,
        emission: Option<Result<bool, String>>,
        shutter: Option<Result<bool, String>>,
        wavelength: Option<Result<f64, String>>,
    },
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
    panel_state: DevicePanelState<GenericAction>,
    read_request_tracker: LatestRequestTracker,
    device_state_request_tracker: LatestRequestTracker,
    aux_request_tracker: LatestRequestTracker,
    refresh_after_command: bool,

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
            panel_state: DevicePanelState::new(),
            read_request_tracker: LatestRequestTracker::default(),
            device_state_request_tracker: LatestRequestTracker::default(),
            aux_request_tracker: LatestRequestTracker::default(),
            refresh_after_command: false,
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
                    input: format!("{default_wl:.0}"),
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
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            match result {
                GenericAction::ReadValue { request_id, result } => {
                    self.panel_state.background_task_completed();
                    if !self.read_request_tracker.is_current(request_id) {
                        continue;
                    }
                    if let Some(ref mut reading) = self.reading {
                        match result {
                            Ok((value, units)) => {
                                reading.raw_value = Some(value);
                                reading.raw_units = units;
                                self.panel_state.error = None;
                            }
                            Err(e) => {
                                tracing::warn!(device_id = ?self.panel_state.device_id, "Read failed: {e}");
                                self.panel_state.set_error(format!("Read failed: {e}"));
                            }
                        }
                    }
                }
                GenericAction::Moved(result) => {
                    self.panel_state.action_completed();
                    if let Some(ref mut motion) = self.motion {
                        motion.moving = false;
                    }
                    match result {
                        Ok(()) => {
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Move completed");
                        }
                        Err(e) => {
                            tracing::warn!(device_id = ?self.panel_state.device_id, "Move failed: {e}");
                            self.panel_state.set_error(format!("Move failed: {e}"));
                        }
                    }
                }
                GenericAction::EmissionCommand(result) => {
                    self.panel_state.action_completed();
                    if let Some(ref mut emission) = self.emission {
                        match result {
                            Ok(value) => {
                                emission.value = Some(value);
                                self.refresh_after_command = true;
                                self.panel_state.set_status(if value {
                                    "Emission enabled"
                                } else {
                                    "Emission disabled"
                                });
                            }
                            Err(e) => {
                                tracing::warn!(device_id = ?self.panel_state.device_id, "Emission failed: {e}");
                                self.panel_state.set_error(format!("Emission failed: {e}"));
                            }
                        }
                    }
                }
                GenericAction::ShutterCommand(result) => {
                    self.panel_state.action_completed();
                    if let Some(ref mut shutter) = self.shutter {
                        match result {
                            Ok(value) => {
                                shutter.value = Some(value);
                                self.refresh_after_command = true;
                                self.panel_state.set_status(if value {
                                    "Shutter open"
                                } else {
                                    "Shutter closed"
                                });
                            }
                            Err(e) => {
                                tracing::warn!(device_id = ?self.panel_state.device_id, "Shutter failed: {e}");
                                self.panel_state.set_error(format!("Shutter failed: {e}"));
                            }
                        }
                    }
                }
                GenericAction::WavelengthCommand(result) => {
                    self.panel_state.action_completed();
                    if let Some(ref mut wl) = self.wavelength {
                        match result {
                            Ok(nm) => {
                                wl.current_nm = Some(nm);
                                if !wl.dragging {
                                    wl.slider_value = nm;
                                    wl.input = format!("{nm:.1}");
                                }
                                self.refresh_after_command = true;
                                self.panel_state
                                    .set_status(format!("Wavelength set to {nm:.1} nm"));
                            }
                            Err(e) => {
                                tracing::warn!(device_id = ?self.panel_state.device_id, "Wavelength failed: {e}");
                                self.panel_state
                                    .set_error(format!("Wavelength failed: {e}"));
                            }
                        }
                    }
                }
                GenericAction::SetValue(result) => {
                    self.panel_state.action_completed();
                    if let Some(ref mut settable) = self.settable {
                        match result {
                            Ok(voltage) => {
                                settable.voltage = voltage;
                                settable.voltage_input = format!("{voltage:.3}");
                                self.panel_state
                                    .set_status(format!("Set to {voltage:.3} V"));
                            }
                            Err(e) => {
                                tracing::warn!(device_id = ?self.panel_state.device_id, "Write failed: {e}");
                                self.panel_state.set_error(format!("Write failed: {e}"));
                            }
                        }
                    }
                }
                GenericAction::DeviceState { request_id, result } => {
                    self.panel_state.background_task_completed();
                    if !self.device_state_request_tracker.is_current(request_id) {
                        continue;
                    }
                    match result {
                        Ok(snapshot) => {
                            if let Some(ref mut motion) = self.motion
                                && let Some(position) = snapshot.position
                            {
                                motion.position = Some(position);
                                if !motion.moving {
                                    motion.position_input = format!("{position:.2}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(device_id = ?self.panel_state.device_id, "State refresh failed: {e}");
                        }
                    }
                }
                GenericAction::AuxState {
                    request_id,
                    emission,
                    shutter,
                    wavelength,
                } => {
                    self.panel_state.background_task_completed();
                    if !self.aux_request_tracker.is_current(request_id) {
                        continue;
                    }

                    if let Some(ref mut emission_state) = self.emission
                        && let Some(result) = emission
                    {
                        match result {
                            Ok(value) => emission_state.value = Some(value),
                            Err(e) => {
                                tracing::debug!(
                                  device_id = ?self.panel_state.device_id,
                                  "Emission refresh failed: {e}"
                                );
                            }
                        }
                    }

                    if let Some(ref mut shutter_state) = self.shutter
                        && let Some(result) = shutter
                    {
                        match result {
                            Ok(value) => shutter_state.value = Some(value),
                            Err(e) => {
                                tracing::debug!(
                                  device_id = ?self.panel_state.device_id,
                                  "Shutter refresh failed: {e}"
                                );
                            }
                        }
                    }

                    if let Some(ref mut wavelength_state) = self.wavelength
                        && let Some(result) = wavelength
                    {
                        match result {
                            Ok(value) => {
                                wavelength_state.current_nm = Some(value);
                                if !wavelength_state.dragging {
                                    wavelength_state.slider_value = value;
                                    wavelength_state.input = format!("{value:.1}");
                                }
                            }
                            Err(e) => {
                                tracing::debug!(
                                  device_id = ?self.panel_state.device_id,
                                  "Wavelength refresh failed: {e}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Async dispatch helpers
    // -----------------------------------------------------------------------

    fn read_power(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
        let Some(client) = client else { return };

        self.panel_state.background_task_started();
        let request_id = self.read_request_tracker.issue();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .read_value(&device_id)
                .await
                .map(|response| (response.value, response.units))
                .map_err(|e| e.to_string());
            let _ = tx
                .send(GenericAction::ReadValue { request_id, result })
                .await;
        });

        if let Some(ref mut reading) = self.reading {
            reading.last_refresh = Some(Instant::now());
        }
    }

    fn fetch_device_state(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else { return };

        self.panel_state.background_task_started();
        let request_id = self.device_state_request_tracker.issue();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .get_device_state(&device_id)
                .await
                .map(|proto| DeviceStateSnapshot {
                    position: proto.position,
                })
                .map_err(|e| e.to_string());
            let _ = tx
                .send(GenericAction::DeviceState { request_id, result })
                .await;
        });

        if let Some(ref mut motion) = self.motion {
            motion.last_position_refresh = Some(Instant::now());
        }
    }

    fn fetch_aux_state(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.emission.is_none() && self.shutter.is_none() && self.wavelength.is_none() {
            return;
        }

        let Some(client) = client else { return };

        let has_emission = self.emission.is_some();
        let has_shutter = self.shutter.is_some();
        let has_wavelength = self.wavelength.is_some();

        self.panel_state.background_task_started();
        let request_id = self.aux_request_tracker.issue();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let emission = if has_emission {
                Some(
                    client
                        .get_emission(&device_id)
                        .await
                        .map_err(|e| e.to_string()),
                )
            } else {
                None
            };
            let shutter = if has_shutter {
                Some(
                    client
                        .get_shutter(&device_id)
                        .await
                        .map_err(|e| e.to_string()),
                )
            } else {
                None
            };
            let wavelength = if has_wavelength {
                Some(
                    client
                        .get_wavelength(&device_id)
                        .await
                        .map_err(|e| e.to_string()),
                )
            } else {
                None
            };

            let _ = tx
                .send(GenericAction::AuxState {
                    request_id,
                    emission,
                    shutter,
                    wavelength,
                })
                .await;
        });
    }

    fn fetch_full_state(
        &mut self,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        self.fetch_device_state(client.as_deref_mut(), runtime, device_id);
        self.fetch_aux_state(client, runtime, device_id);
    }

    fn move_absolute(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        position: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        if let Some(ref mut motion) = self.motion {
            if !can_send_command(motion.last_command_time, Self::COMMAND_DEBOUNCE) {
                return;
            }
            motion.moving = true;
            motion.last_command_time = Some(Instant::now());
        }

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .move_absolute(&device_id, position)
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
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        if let Some(ref mut motion) = self.motion {
            if !can_send_command(motion.last_command_time, Self::COMMAND_DEBOUNCE) {
                return;
            }
            motion.moving = true;
            motion.last_command_time = Some(Instant::now());
        }

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .move_relative(&device_id, delta)
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
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_emission(&device_id, enabled)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::EmissionCommand(result)).await;
        });
    }

    fn set_shutter_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        open: bool,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_shutter(&device_id, open)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::ShutterCommand(result)).await;
        });
    }

    fn set_wavelength_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        nm: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_wavelength(&device_id, nm)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::WavelengthCommand(result)).await;
        });
    }

    fn write_voltage_rpc(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        voltage: f64,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        let voltage = if let Some(ref settable) = self.settable {
            voltage.clamp(settable.min_voltage, settable.max_voltage)
        } else {
            voltage
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();

        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id, "voltage", &voltage.to_string())
                .await
                .map(|_| voltage)
                .map_err(|e| e.to_string());
            let _ = tx.send(GenericAction::SetValue(result)).await;
        });
    }

    fn validate_motion_input(input: &str, field_name: &str) -> Result<f64, String> {
        let value = parse_f64_input(input, field_name)?;
        if !value.is_finite() {
            return Err(format!("Invalid {field_name}: must be finite"));
        }
        Ok(value)
    }

    fn queue_refresh_if_needed(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.refresh_after_command && !self.panel_state.is_refreshing() {
            self.refresh_after_command = false;
            self.fetch_full_state(client, runtime, device_id);
        }
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
        ui.set_max_width(ui.available_width());

        self.poll_results();

        let device_id = device.id.clone();
        self.panel_state.device_id = Some(device_id.clone());

        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.fetch_full_state(client.as_deref_mut(), runtime, &device_id);
            if self.reading.is_some() {
                self.read_power(client.as_deref_mut(), runtime, &device_id);
            }
        }

        self.queue_refresh_if_needed(client.as_deref_mut(), runtime, &device_id);

        let is_busy = self.panel_state.is_busy();
        let is_refreshing = self.panel_state.is_refreshing();

        if let Some(ref reading) = self.reading {
            let should_refresh = reading.auto_refresh
                && !is_busy
                && !is_refreshing
                && reading
                    .last_refresh
                    .map(|instant| instant.elapsed() >= Self::REFRESH_INTERVAL)
                    .unwrap_or(true);
            if should_refresh && client.is_some() {
                self.read_power(client.as_deref_mut(), runtime, &device_id);
            }
        }

        if self.motion.is_some() && !is_busy && !is_refreshing {
            let should_refresh_position = self
                .motion
                .as_ref()
                .and_then(|motion| motion.last_position_refresh)
                .map(|instant| instant.elapsed() >= Self::REFRESH_INTERVAL)
                .unwrap_or(true);
            if should_refresh_position && client.is_some() {
                self.fetch_device_state(client.as_deref_mut(), runtime, &device_id);
            }
        }

        show_panel_header(ui, &device.name, None, is_busy, is_refreshing);
        show_panel_messages(ui, &self.panel_state.error, &self.panel_state.status);
        ui.add_space(8.0);

        if let Some(ref reading) = self.reading {
            show_panel_section(ui, "Readout", |ui| {
                render_reading_row(ui, reading);
            });
            ui.add_space(8.0);
        }

        if self.emission.is_some() || self.shutter.is_some() {
            show_panel_section(ui, "Outputs", |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let Some(ref emission) = self.emission {
                        let is_on = emission.value.unwrap_or(false);
                        ui.label("Emission:");
                        let text = if is_on { "Enabled" } else { "Disabled" };
                        let fill = if is_on {
                            layout::colors::SUCCESS
                        } else {
                            layout::colors::MUTED
                        };
                        let button =
                            filled_action_button(text, fill).min_size(egui::vec2(72.0, 22.0));
                        if ui.add_enabled(!is_busy, button).clicked() {
                            self.set_emission_rpc(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                !is_on,
                            );
                        }
                    }

                    if let Some(ref shutter) = self.shutter {
                        if self.emission.is_some() {
                            ui.separator();
                        }
                        let is_open = shutter.value.unwrap_or(false);
                        let (text, fill) = if is_open {
                            ("Open", layout::colors::ERROR)
                        } else {
                            ("Closed", layout::colors::SUCCESS)
                        };
                        let button = filled_action_button(format!("Shutter {text}"), fill)
                            .min_size(egui::vec2(112.0, 22.0));
                        if ui.add_enabled(!is_busy, button).clicked() {
                            self.set_shutter_rpc(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                !is_open,
                            );
                        }
                    }
                });
            });
            ui.add_space(8.0);
        }

        if self.wavelength.is_some() {
            show_panel_section(ui, "Wavelength", |ui| {
                let mut wl = self.wavelength.take().expect("wavelength state exists");
                let min_nm = wl.min_nm;
                let max_nm = wl.max_nm;

                ui.horizontal_wrapped(|ui| {
                    ui.label("Wavelength:");
                    let slider_response = ui.add_enabled(
                        !is_busy,
                        egui::Slider::new(&mut wl.slider_value, min_nm..=max_nm)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always),
                    );

                    let input_response = ui.add_enabled(
                        !is_busy,
                        egui::TextEdit::singleline(&mut wl.input)
                            .desired_width(56.0)
                            .hint_text("nm"),
                    );
                    ui.label("nm");

                    if slider_response.drag_started() {
                        wl.dragging = true;
                    }
                    if slider_response.drag_stopped() {
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

                    let mut submit = |panel: &mut Self, value: &str| match parse_f64_input(
                        value,
                        "wavelength",
                    ) {
                        Ok(nm) if (min_nm..=max_nm).contains(&nm) => {
                            panel.set_wavelength_rpc(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                nm,
                            );
                        }
                        Ok(_) => panel
                            .panel_state
                            .set_error(format!("Wavelength must be {min_nm:.1}..{max_nm:.1} nm")),
                        Err(e) => panel.panel_state.set_error(e),
                    };

                    if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                        submit(self, &wl.input);
                    }

                    if input_response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        && !is_busy
                    {
                        submit(self, &wl.input);
                    }
                });

                self.wavelength = Some(wl);
            });
            ui.add_space(8.0);
        }

        if self.motion.is_some() {
            show_panel_section(ui, "Motion", |ui| {
                let mut motion = self.motion.take().expect("motion state exists");
                let motion_busy = motion.moving || is_busy;
                let position_units = motion.position_units.as_str();

                ui.horizontal_wrapped(|ui| {
                    if let Some(position) = motion.position {
                        let text = if position_units.is_empty() {
                            format!("{position:.2}")
                        } else {
                            format!("{position:.2} {position_units}")
                        };
                        ui.label(panel_value_text(text));
                    } else {
                        ui.label(panel_value_text("---"));
                    }

                    let step = match parse_positive_step_input(&motion.jog_step, "step size") {
                        Ok(step) => Some(step),
                        Err(error) => {
                            if !motion.jog_step.trim().is_empty() {
                                ui.colored_label(layout::colors::WARNING, panel_hint_text(error));
                            }
                            None
                        }
                    };

                    let units = if position_units.is_empty() {
                        "units"
                    } else {
                        position_units
                    };

                    for multiplier in [-10.0_f64, -1.0_f64, 1.0_f64, 10.0_f64] {
                        let label = if multiplier.is_sign_negative() {
                            format!("{:.0}", step.unwrap_or(1.0) * multiplier)
                        } else {
                            format!("+{:.0}", step.unwrap_or(1.0) * multiplier)
                        };
                        let enabled = !motion_busy && step.is_some();
                        if ui
                            .add_enabled(enabled, action_button(label))
                            .on_hover_text(format!(
                                "Jog {:+.2} {units}",
                                step.unwrap_or(1.0) * multiplier
                            ))
                            .clicked()
                            && let Some(step) = step
                        {
                            self.move_relative(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                step * multiplier,
                            );
                        }
                    }

                    ui.separator();

                    ui.label("Go to:");
                    let response = ui.add_enabled(
                        !motion_busy,
                        egui::TextEdit::singleline(&mut motion.position_input)
                            .desired_width(58.0)
                            .hint_text("position"),
                    );

                    let mut submit_absolute =
                        |panel: &mut Self, value: &str| match Self::validate_motion_input(
                            value, "position",
                        ) {
                            Ok(position) => panel.move_absolute(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                position,
                            ),
                            Err(e) => panel.panel_state.set_error(e),
                        };

                    if ui.add_enabled(!motion_busy, action_button("Go")).clicked() {
                        submit_absolute(self, &motion.position_input);
                    }
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        && !motion_busy
                    {
                        submit_absolute(self, &motion.position_input);
                    }

                    if ui
                        .add_enabled(!motion_busy, action_button("Home"))
                        .on_hover_text("Move to 0.0")
                        .clicked()
                    {
                        self.move_absolute(client.as_deref_mut(), runtime, &device_id, 0.0);
                    }

                    ui.separator();
                    ui.label("Step:");
                    ui.add_enabled(
                        !motion_busy,
                        egui::TextEdit::singleline(&mut motion.jog_step).desired_width(42.0),
                    );
                });

                self.motion = Some(motion);
            });
            ui.add_space(8.0);
        }

        if self.settable.is_some() {
            show_panel_section(ui, "Analog Output", |ui| {
                let mut settable = self.settable.take().expect("settable state exists");

                ui.horizontal_wrapped(|ui| {
                    ui.label(panel_value_text(format!("{:.3} V", settable.voltage)));

                    let mut voltage = settable.voltage;
                    let slider = egui::Slider::new(
                        &mut voltage,
                        settable.min_voltage..=settable.max_voltage,
                    )
                    .suffix("V")
                    .clamping(egui::SliderClamping::Always);

                    if ui.add_enabled(!is_busy, slider).changed() {
                        settable.voltage = voltage;
                        settable.voltage_input = format!("{voltage:.3}");
                        self.write_voltage_rpc(client.as_deref_mut(), runtime, &device_id, voltage);
                    }

                    ui.separator();

                    if ui.add_enabled(!is_busy, action_button("0 V")).clicked() {
                        self.write_voltage_rpc(client.as_deref_mut(), runtime, &device_id, 0.0);
                    }
                    if ui
                        .add_enabled(
                            !is_busy,
                            action_button(format!("{:.0} V", settable.min_voltage)),
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
                            action_button(format!("{:.0} V", settable.max_voltage)),
                        )
                        .clicked()
                    {
                        self.write_voltage_rpc(
                            client.as_deref_mut(),
                            runtime,
                            &device_id,
                            settable.max_voltage,
                        );
                    }
                });

                self.settable = Some(settable);
            });
        }

        request_panel_repaint(
            ui,
            self.reading
                .as_ref()
                .map(|r| r.auto_refresh)
                .unwrap_or(false)
                || is_busy
                || is_refreshing,
        );
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn render_reading_row(ui: &mut Ui, reading: &ReadingState) {
    ui.horizontal_wrapped(|ui| {
        let raw = reading.raw_value.unwrap_or(0.0);
        let units = &reading.raw_units;

        if is_power_unit(units) {
            let power_mw = normalize_power_to_mw(raw, units);
            #[allow(clippy::cast_possible_truncation)]
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
            ui.label(panel_value_text(format!("{value:.4} {unit}")));
        } else {
            let display_unit = if units.is_empty() { "" } else { units.as_str() };
            #[allow(clippy::cast_possible_truncation)]
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
            ui.label(panel_value_text(format!("{raw:.4} {display_unit}")));
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
    last.map(|instant| instant.elapsed() >= debounce)
        .unwrap_or(true)
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
        assert_eq!(normalize_power_to_mw(1_000_000.0, "W"), 1_000_000_000.0);
        assert_eq!(normalize_power_to_mw(-5.0, "mW"), -5.0);
    }

    #[test]
    fn test_normalize_power_case_insensitive() {
        assert_eq!(normalize_power_to_mw(1.0, "w"), 1000.0);
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
