// Internal consumer of the deprecated DeviceConfig schema for UI rendering.
#![allow(deprecated)]
//! Config-driven UI rendering for device control panels.
//!
//! This module provides [`ConfigDrivenPanel`], a stateful control panel that renders
//! device controls based on [`ControlPanelConfig`] from TOML device configuration files.
//!
//! ## Architecture
//!
//! `ConfigDrivenPanel` implements [`DeviceControlWidget`] so it plugs into both the
//! inline Instruments pane and docked pop-out tabs. It uses the same async channel
//! pattern as `GenericDevicePanel`: a unified `ConfigPanelAction` enum flows through
//! an `mpsc` channel, and `poll_results()` dispatches each result to the correct
//! per-section state.
//!
//! ## Borrow pattern
//!
//! Section render methods use the **take/replace pattern**: section state is
//! `std::mem::replace`'d out of the vec, rendered (referencing the local copy),
//! and then put back. This avoids borrowing `self.sections[idx]` simultaneously
//! with `self.dispatch_*()` methods.

use crate::time::{Duration, Instant};

use crate::runtime::Runtime;
use eframe::egui;
use egui::Ui;
use tokio::sync::mpsc;

use crate::widgets::{DeviceControlWidget, Gauge};
use client::DaqClient;
use egui_plot::{Line, Plot, PlotPoints};
use hardware::config::schema::{
    ButtonStyle, ControlPanelConfig, ControlSection, PanelLayout, ParameterWidget, PresetValue,
};
use protocol::daq::DeviceInfo;

/// Position polling interval for motion sections.
const POSITION_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

/// Command debounce interval (prevents rapid-fire commands to serial devices).
const COMMAND_DEBOUNCE: Duration = Duration::from_millis(250);

// =============================================================================
// Action enum — results from async gRPC operations
// =============================================================================

enum ConfigPanelAction {
    PositionUpdate(Result<f64, String>),
    Moved(Result<(), String>),
    ReadValue(Result<(f64, String), String>),
    ShutterState(Result<bool, String>, bool),
    WavelengthValue(Result<f64, String>, bool),
    ParameterRead {
        idx: usize,
        result: Result<String, String>,
    },
    ParameterWrite {
        idx: usize,
        result: Result<String, String>,
    },
    StatusValues {
        idx: usize,
        result: Result<Vec<(String, String)>, String>,
    },
    CommandExecuted {
        _idx: usize,
        result: Result<String, String>,
    },
    Stopped(Result<(), String>),
}

// =============================================================================
// Per-section mutable state
// =============================================================================

enum SectionState {
    Motion(MotionSectionState),
    PresetButtons,
    CustomAction(CustomActionState),
    Camera,
    Shutter(ShutterSectionState),
    Wavelength(WavelengthSectionState),
    Parameter(ParameterSectionState),
    StatusDisplay(StatusDisplaySectionState),
    Sensor(SensorSectionState),
    Separator,
    Custom,
}

#[derive(Default)]
struct MotionSectionState {
    position: Option<f64>,
    moving: bool,
    position_input: String,
    last_command_time: Option<Instant>,
    last_position_refresh: Option<Instant>,
}

#[derive(Default)]
struct CustomActionState {
    confirming: bool,
}

#[derive(Default)]
struct ShutterSectionState {
    is_open: Option<bool>,
}

struct WavelengthSectionState {
    current_nm: Option<f64>,
    slider_value: f64,
    input: String,
    dragging: bool,
    min_nm: f64,
    max_nm: f64,
}

impl Default for WavelengthSectionState {
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

#[derive(Default)]
struct ParameterSectionState {
    value: Option<String>,
    input: String,
    /// Last time a command was dispatched (for spinner/DragValue debounce)
    last_command_time: Option<Instant>,
}

#[derive(Default)]
struct StatusDisplaySectionState {
    values: Vec<(String, String)>,
}

#[derive(Default)]
struct SensorSectionState {
    value: Option<f64>,
    units: String,
    last_refresh: Option<Instant>,
    trend_data: Vec<(f64, f64)>,
    trend_start: Option<Instant>,
}

// =============================================================================
// ConfigDrivenPanel
// =============================================================================

/// A device control panel that renders UI based on TOML configuration.
///
/// Reads `[ui.control_panel]` sections from device config files and produces
/// interactive controls backed by gRPC calls to the daemon.
pub struct ConfigDrivenPanel {
    config: ControlPanelConfig,
    action_tx: mpsc::Sender<ConfigPanelAction>,
    action_rx: mpsc::Receiver<ConfigPanelAction>,
    actions_in_flight: usize,
    error: Option<String>,
    status: Option<String>,
    device_id: Option<String>,
    initial_fetch_done: bool,
    last_refresh: Option<Instant>,
    sections: Vec<SectionState>,
}

impl ConfigDrivenPanel {
    /// Create a new config-driven panel from a [`ControlPanelConfig`].
    pub fn new(config: ControlPanelConfig) -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);

        let sections = config
            .sections
            .iter()
            .map(|section| match section {
                ControlSection::Motion(_) => SectionState::Motion(MotionSectionState::default()),
                ControlSection::PresetButtons(_) => SectionState::PresetButtons,
                ControlSection::CustomAction(_) => {
                    SectionState::CustomAction(CustomActionState::default())
                }
                ControlSection::Camera(_) => SectionState::Camera,
                ControlSection::Shutter(_) => SectionState::Shutter(ShutterSectionState::default()),
                ControlSection::Wavelength(_) => {
                    SectionState::Wavelength(WavelengthSectionState::default())
                }
                ControlSection::Parameter(_) => {
                    SectionState::Parameter(ParameterSectionState::default())
                }
                ControlSection::StatusDisplay(_) => {
                    SectionState::StatusDisplay(StatusDisplaySectionState::default())
                }
                ControlSection::Sensor(_) => SectionState::Sensor(SensorSectionState::default()),
                ControlSection::Separator(_) => SectionState::Separator,
                ControlSection::Custom(_) => SectionState::Custom,
            })
            .collect();

        Self {
            config,
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            initial_fetch_done: false,
            last_refresh: None,
            sections,
        }
    }

    // =========================================================================
    // Async result handling
    // =========================================================================

    fn poll_results(&mut self) {
        while let Ok(result) = self.action_rx.try_recv() {
            let is_user_command = matches!(
                &result,
                ConfigPanelAction::Moved(_)
                    | ConfigPanelAction::Stopped(_)
                    | ConfigPanelAction::ShutterState(_, true)
                    | ConfigPanelAction::WavelengthValue(_, true)
                    | ConfigPanelAction::ParameterWrite { .. }
                    | ConfigPanelAction::CommandExecuted { .. }
            );
            if is_user_command {
                self.actions_in_flight = self.actions_in_flight.saturating_sub(1);
            }

            match result {
                ConfigPanelAction::PositionUpdate(res) => {
                    if let Ok(pos) = res {
                        for section in &mut self.sections {
                            if let SectionState::Motion(ref mut s) = section {
                                s.position = Some(pos);
                            }
                        }
                    }
                }
                ConfigPanelAction::Moved(res) => {
                    for section in &mut self.sections {
                        if let SectionState::Motion(ref mut s) = section {
                            s.moving = false;
                        }
                    }
                    match res {
                        Ok(()) => {
                            self.status = Some("Move completed".to_string());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("Move failed: {e}")),
                    }
                }
                ConfigPanelAction::ReadValue(res) => {
                    for section in &mut self.sections {
                        if let SectionState::Sensor(ref mut s) = section {
                            if let Ok((value, ref units)) = res {
                                s.value = Some(value);
                                s.units.clone_from(units);
                                if let Some(start) = s.trend_start {
                                    let t = start.elapsed().as_secs_f64();
                                    s.trend_data.push((t, value));
                                    if s.trend_data.len() > 300 {
                                        s.trend_data.remove(0);
                                    }
                                }
                            }
                        }
                    }
                    if let Err(e) = res {
                        self.error = Some(format!("Read failed: {e}"));
                    }
                }
                ConfigPanelAction::ShutterState(res, _) => {
                    for section in &mut self.sections {
                        if let SectionState::Shutter(ref mut s) = section {
                            if let Ok(is_open) = res {
                                s.is_open = Some(is_open);
                            }
                        }
                    }
                    match res {
                        Ok(is_open) => {
                            self.status = Some(if is_open {
                                "Shutter OPEN".to_string()
                            } else {
                                "Shutter CLOSED".to_string()
                            });
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("Shutter: {e}")),
                    }
                }
                ConfigPanelAction::WavelengthValue(res, _) => {
                    for section in &mut self.sections {
                        if let SectionState::Wavelength(ref mut s) = section {
                            if let Ok(nm) = res {
                                s.current_nm = Some(nm);
                                if !s.dragging {
                                    s.slider_value = nm;
                                    s.input = format!("{nm:.1}");
                                }
                            }
                        }
                    }
                    match res {
                        Ok(nm) => {
                            self.status = Some(format!("Wavelength: {nm:.1} nm"));
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("Wavelength: {e}")),
                    }
                }
                ConfigPanelAction::ParameterRead { idx, result } => {
                    if let Some(SectionState::Parameter(ref mut s)) = self.sections.get_mut(idx) {
                        match result {
                            Ok(val) => {
                                if s.input.is_empty() {
                                    s.input.clone_from(&val);
                                }
                                s.value = Some(val);
                            }
                            Err(e) => self.error = Some(format!("Read param: {e}")),
                        }
                    }
                }
                ConfigPanelAction::ParameterWrite { idx, result } => {
                    if let Some(SectionState::Parameter(ref mut s)) = self.sections.get_mut(idx) {
                        match result {
                            Ok(val) => {
                                s.input.clone_from(&val);
                                s.value = Some(val);
                                self.status = Some("Parameter updated".to_string());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Set param: {e}")),
                        }
                    }
                }
                ConfigPanelAction::StatusValues { idx, result } => {
                    if let Some(SectionState::StatusDisplay(ref mut s)) = self.sections.get_mut(idx)
                    {
                        match result {
                            Ok(values) => s.values = values,
                            Err(e) => self.error = Some(format!("Status: {e}")),
                        }
                    }
                }
                ConfigPanelAction::CommandExecuted { result, .. } => match result {
                    Ok(msg) => {
                        self.status = Some(if msg.is_empty() {
                            "Command executed".to_string()
                        } else {
                            msg
                        });
                        self.error = None;
                    }
                    Err(e) => self.error = Some(format!("Command failed: {e}")),
                },
                ConfigPanelAction::Stopped(res) => {
                    // Clear motion state so the UI doesn't remain "busy"
                    for section in &mut self.sections {
                        if let SectionState::Motion(ref mut motion_state) = section {
                            motion_state.moving = false;
                        }
                    }
                    match res {
                        Ok(()) => {
                            self.status = Some("Stopped".to_string());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("Stop failed: {e}")),
                    }
                }
            }
        }
    }

    // =========================================================================
    // Async dispatch helpers (do NOT touch section state)
    // =========================================================================

    fn fetch_initial_state(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else { return };
        self.last_refresh = Some(Instant::now());

        let has_motion = self
            .config
            .sections
            .iter()
            .any(|s| matches!(s, ControlSection::Motion(_)));
        let has_sensor = self
            .config
            .sections
            .iter()
            .any(|s| matches!(s, ControlSection::Sensor(_)));
        let has_shutter = self
            .config
            .sections
            .iter()
            .any(|s| matches!(s, ControlSection::Shutter(_)));
        let has_wavelength = self
            .config
            .sections
            .iter()
            .any(|s| matches!(s, ControlSection::Wavelength(_)));

        let param_sections: Vec<(usize, String)> = self
            .config
            .sections
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if let ControlSection::Parameter(cfg) = s {
                    Some((i, cfg.parameter.clone()))
                } else {
                    None
                }
            })
            .collect();
        let status_sections: Vec<(usize, Vec<String>)> = self
            .config
            .sections
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if let ControlSection::StatusDisplay(cfg) = s {
                    Some((i, cfg.parameters.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Initialize sensor trend tracking
        for section in &mut self.sections {
            if let SectionState::Sensor(ref mut s) = section {
                s.trend_start = Some(Instant::now());
            }
        }

        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();

        // All queries in a single task, sequential (serial device safety)
        runtime.spawn(async move {
            if has_motion {
                let result = c
                    .get_device_state(&id)
                    .await
                    .map(|s| s.position.unwrap_or(0.0))
                    .map_err(|e| e.to_string());
                let _ = tx.send(ConfigPanelAction::PositionUpdate(result)).await;
            }
            if has_sensor {
                let result = c
                    .read_value(&id)
                    .await
                    .map(|r| (r.value, r.units))
                    .map_err(|e| e.to_string());
                let _ = tx.send(ConfigPanelAction::ReadValue(result)).await;
            }
            if has_shutter {
                let result = c.get_shutter(&id).await.map_err(|e| e.to_string());
                let _ = tx
                    .send(ConfigPanelAction::ShutterState(result, false))
                    .await;
            }
            if has_wavelength {
                let result = c.get_wavelength(&id).await.map_err(|e| e.to_string());
                let _ = tx
                    .send(ConfigPanelAction::WavelengthValue(result, false))
                    .await;
            }
            for (idx, param_name) in &param_sections {
                let result = c
                    .get_parameter(&id, param_name)
                    .await
                    .map(|p| p.value)
                    .map_err(|e| e.to_string());
                let _ = tx
                    .send(ConfigPanelAction::ParameterRead { idx: *idx, result })
                    .await;
            }
            for (idx, params) in &status_sections {
                let mut values = Vec::new();
                for param_name in params {
                    match c.get_parameter(&id, param_name).await {
                        Ok(p) => values.push((param_name.clone(), p.value)),
                        Err(_) => values.push((param_name.clone(), "?".to_string())),
                    }
                }
                let _ = tx
                    .send(ConfigPanelAction::StatusValues {
                        idx: *idx,
                        result: Ok(values),
                    })
                    .await;
            }
        });
    }

    fn dispatch_position_refresh(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else { return };
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .get_device_state(&id)
                .await
                .map(|s| s.position.unwrap_or(0.0))
                .map_err(|e| e.to_string());
            let _ = tx.send(ConfigPanelAction::PositionUpdate(result)).await;
        });
    }

    fn dispatch_sensor_refresh(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else { return };
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .read_value(&id)
                .await
                .map(|r| (r.value, r.units))
                .map_err(|e| e.to_string());
            let _ = tx.send(ConfigPanelAction::ReadValue(result)).await;
        });
    }

    fn dispatch_move_absolute(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        position: f64,
    ) {
        let Some(client) = client else { return };
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
            let _ = tx.send(ConfigPanelAction::Moved(result)).await;
        });
    }

    fn dispatch_move_relative(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        delta: f64,
    ) {
        let Some(client) = client else { return };
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
            let _ = tx.send(ConfigPanelAction::Moved(result)).await;
        });
    }

    fn dispatch_set_shutter(
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
            let _ = tx.send(ConfigPanelAction::ShutterState(result, true)).await;
        });
    }

    fn dispatch_set_wavelength(
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
            let _ = tx
                .send(ConfigPanelAction::WavelengthValue(result, true))
                .await;
        });
    }

    fn dispatch_command(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        idx: usize,
        command: &str,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        let cmd = command.to_string();
        let args = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
        runtime.spawn(async move {
            let result = c
                .execute_device_command(&id, &cmd, &args)
                .await
                .map(|_| "OK".to_string())
                .map_err(|e| e.to_string());
            let _ = tx
                .send(ConfigPanelAction::CommandExecuted { _idx: idx, result })
                .await;
        });
    }

    fn dispatch_set_parameter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        idx: usize,
        param_name: &str,
        value: &str,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        let name = param_name.to_string();
        let val = value.to_string();
        runtime.spawn(async move {
            let result = c
                .set_parameter(&id, &name, &val)
                .await
                .map(|r| r.actual_value)
                .map_err(|e| e.to_string());
            let _ = tx
                .send(ConfigPanelAction::ParameterWrite { idx, result })
                .await;
        });
    }

    fn dispatch_stop(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let Some(client) = client else { return };
        self.actions_in_flight += 1;
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        runtime.spawn(async move {
            let result = c
                .execute_device_command(&id, "stop", "{}")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ConfigPanelAction::Stopped(result)).await;
        });
    }

    fn dispatch_status_refresh(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        idx: usize,
        parameters: &[String],
    ) {
        let Some(client) = client else { return };
        let mut c = client.clone();
        let tx = self.action_tx.clone();
        let id = device_id.to_string();
        let params = parameters.to_vec();
        runtime.spawn(async move {
            let mut values = Vec::new();
            for param_name in &params {
                match c.get_parameter(&id, param_name).await {
                    Ok(p) => values.push((param_name.clone(), p.value)),
                    Err(_) => values.push((param_name.clone(), "?".to_string())),
                }
            }
            let _ = tx
                .send(ConfigPanelAction::StatusValues {
                    idx,
                    result: Ok(values),
                })
                .await;
        });
    }

    // =========================================================================
    // Auto-refresh
    // =========================================================================

    fn has_auto_refresh(&self) -> bool {
        for (section, config) in self.sections.iter().zip(self.config.sections.iter()) {
            match (section, config) {
                (SectionState::Sensor(_), ControlSection::Sensor(cfg)) if cfg.refresh_ms > 0 => {
                    return true;
                }
                (SectionState::Motion(_), ControlSection::Motion(_)) => return true,
                _ => {}
            }
        }
        false
    }

    /// Schedule repaint based on the next due refresh time rather than a flat 100ms.
    fn request_smart_repaint(&self, ui: &mut Ui) {
        let mut min_delay = Duration::from_millis(100); // Fallback for actions_in_flight spinner

        for (section, config) in self.sections.iter().zip(self.config.sections.iter()) {
            match (section, config) {
                (SectionState::Motion(s), ControlSection::Motion(_)) => {
                    let remaining = s
                        .last_position_refresh
                        .map(|t| POSITION_REFRESH_INTERVAL.saturating_sub(t.elapsed()))
                        .unwrap_or(Duration::ZERO);
                    min_delay = min_delay.min(remaining.max(Duration::from_millis(10)));
                }
                (SectionState::Sensor(s), ControlSection::Sensor(cfg)) if cfg.refresh_ms > 0 => {
                    let interval = Duration::from_millis(u64::from(cfg.refresh_ms));
                    let remaining = s
                        .last_refresh
                        .map(|t| interval.saturating_sub(t.elapsed()))
                        .unwrap_or(Duration::ZERO);
                    min_delay = min_delay.min(remaining.max(Duration::from_millis(10)));
                }
                _ => {}
            }
        }

        ui.ctx().request_repaint_after(min_delay);
    }

    fn auto_refresh(
        &mut self,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.actions_in_flight > 0 || client.is_none() {
            return;
        }

        // Position refresh
        let should_refresh_position = self.sections.iter().any(|s| {
            if let SectionState::Motion(state) = s {
                state
                    .last_position_refresh
                    .map(|t| t.elapsed() >= POSITION_REFRESH_INTERVAL)
                    .unwrap_or(true)
            } else {
                false
            }
        });
        if should_refresh_position {
            self.dispatch_position_refresh(client.as_deref_mut(), runtime, device_id);
            for section in &mut self.sections {
                if let SectionState::Motion(ref mut s) = section {
                    s.last_position_refresh = Some(Instant::now());
                }
            }
            return; // One refresh per frame (serial device safety)
        }

        // Sensor refresh
        let should_refresh_sensor =
            self.sections
                .iter()
                .zip(self.config.sections.iter())
                .any(|(state, config)| {
                    if let (SectionState::Sensor(s), ControlSection::Sensor(cfg)) = (state, config)
                    {
                        if cfg.refresh_ms > 0 {
                            let interval = Duration::from_millis(u64::from(cfg.refresh_ms));
                            s.last_refresh
                                .map(|t| t.elapsed() >= interval)
                                .unwrap_or(true)
                        } else {
                            // refresh_ms == 0 means manual-only; disable auto-refresh
                            false
                        }
                    } else {
                        false
                    }
                });
        if should_refresh_sensor {
            self.dispatch_sensor_refresh(client.as_deref_mut(), runtime, device_id);
            for section in &mut self.sections {
                if let SectionState::Sensor(ref mut s) = section {
                    s.last_refresh = Some(Instant::now());
                }
            }
        }
    }

    // =========================================================================
    // Section rendering (uses take/replace for section state)
    // =========================================================================

    fn render_section(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        // Clone config section to break borrow on self.config
        let section_config = self.config.sections[idx].clone();

        ui.push_id(("config_section", idx), |ui| match &section_config {
            ControlSection::Separator(cfg) => {
                if cfg.visible {
                    ui.separator();
                } else {
                    ui.add_space(f32::from(cfg.height));
                }
            }
            ControlSection::Motion(cfg) => {
                self.render_motion(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::PresetButtons(cfg) => {
                self.render_presets(ui, cfg, device_id, client, runtime);
            }
            ControlSection::Sensor(cfg) => {
                self.render_sensor(ui, idx, cfg);
            }
            ControlSection::StatusDisplay(cfg) => {
                self.render_status_display(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::Shutter(cfg) => {
                self.render_shutter(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::Wavelength(cfg) => {
                self.render_wavelength(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::CustomAction(cfg) => {
                self.render_custom_action(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::Parameter(cfg) => {
                self.render_parameter(ui, idx, cfg, device_id, client, runtime);
            }
            ControlSection::Camera(cfg) => {
                ui.group(|ui| {
                    ui.label(egui::RichText::new(&cfg.label).strong());
                    ui.weak("Camera controls — use Image Viewer for full camera support");
                });
            }
            ControlSection::Custom(cfg) => {
                ui.group(|ui| {
                    ui.label(egui::RichText::new(&cfg.label).strong());
                    ui.weak(format!("Custom widget: {}", cfg.widget));
                });
            }
        });
    }

    fn render_motion(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::MotionSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        // Take state out to avoid borrow conflict with self.dispatch_*()
        let mut section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::Motion(ref mut state) = section else {
            self.sections[idx] = section;
            return;
        };

        let is_busy = self.actions_in_flight > 0 || state.moving;
        let precision = cfg.precision as usize;
        let unit = cfg.unit.as_deref().unwrap_or("");

        ui.group(|ui| {
            ui.label(egui::RichText::new(&cfg.label).strong());

            // Position display
            if let Some(pos) = state.position {
                let text = if unit.is_empty() {
                    format!("{pos:.precision$}")
                } else {
                    format!("{pos:.precision$} {unit}")
                };
                ui.label(egui::RichText::new(text).monospace().size(16.0));
            } else {
                ui.label(egui::RichText::new("---").monospace());
            }

            // Jog buttons
            if cfg.show_jog && !cfg.jog_steps.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for &step in &cfg.jog_steps {
                        if ui
                            .add_enabled(!is_busy, egui::Button::new(format!("-{step}")))
                            .clicked()
                            && can_send_command(state.last_command_time, COMMAND_DEBOUNCE)
                        {
                            state.moving = true;
                            state.last_command_time = Some(Instant::now());
                            self.dispatch_move_relative(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                -step,
                            );
                        }
                    }
                    ui.separator();
                    for &step in &cfg.jog_steps {
                        if ui
                            .add_enabled(!is_busy, egui::Button::new(format!("+{step}")))
                            .clicked()
                            && can_send_command(state.last_command_time, COMMAND_DEBOUNCE)
                        {
                            state.moving = true;
                            state.last_command_time = Some(Instant::now());
                            self.dispatch_move_relative(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                step,
                            );
                        }
                    }
                });
            }

            // Absolute move
            ui.horizontal(|ui| {
                ui.label("Go to:");
                let response = ui
                    .add(egui::TextEdit::singleline(&mut state.position_input).desired_width(80.0));
                if ui.add_enabled(!is_busy, egui::Button::new("Go")).clicked() {
                    if let Ok(pos) = state.position_input.parse::<f64>() {
                        state.moving = true;
                        state.last_command_time = Some(Instant::now());
                        self.dispatch_move_absolute(client.as_deref_mut(), runtime, device_id, pos);
                    } else {
                        self.error = Some("Invalid position value".to_string());
                    }
                }
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !is_busy
                {
                    if let Ok(pos) = state.position_input.parse::<f64>() {
                        state.moving = true;
                        state.last_command_time = Some(Instant::now());
                        self.dispatch_move_absolute(client.as_deref_mut(), runtime, device_id, pos);
                    }
                }
            });

            // Home / Stop
            if cfg.show_home || cfg.show_stop {
                ui.horizontal(|ui| {
                    if cfg.show_home
                        && ui
                            .add_enabled(!is_busy, egui::Button::new("⌂ Home"))
                            .clicked()
                    {
                        state.moving = true;
                        state.last_command_time = Some(Instant::now());
                        self.dispatch_move_absolute(client.as_deref_mut(), runtime, device_id, 0.0);
                    }
                    if cfg.show_stop
                        && ui
                            .add_enabled(!is_busy, egui::Button::new("⏹ Stop"))
                            .clicked()
                    {
                        self.dispatch_stop(client.as_deref_mut(), runtime, device_id);
                    }
                });
            }
        });

        self.sections[idx] = section;
    }

    fn render_presets(
        &mut self,
        ui: &mut Ui,
        cfg: &hardware::config::schema::PresetButtonsSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let is_busy = self.actions_in_flight > 0;

        ui.group(|ui| {
            ui.label(egui::RichText::new(&cfg.label).strong());

            let mut layout_fn = |ui: &mut Ui, panel: &mut Self| {
                for preset in &cfg.presets {
                    let (label, value) = match preset {
                        PresetValue::Number(v) => (format!("{v:.1}"), *v),
                        PresetValue::Labeled { label, value } => (label.clone(), *value),
                    };
                    if ui
                        .add_enabled(!is_busy, egui::Button::new(&label))
                        .clicked()
                    {
                        panel.dispatch_move_absolute(
                            client.as_deref_mut(),
                            runtime,
                            device_id,
                            value,
                        );
                    }
                }
            };

            if cfg.vertical {
                layout_fn(ui, self);
            } else {
                ui.horizontal_wrapped(|ui| {
                    for preset in &cfg.presets {
                        let (label, value) = match preset {
                            PresetValue::Number(v) => (format!("{v:.1}"), *v),
                            PresetValue::Labeled { label, value } => (label.clone(), *value),
                        };
                        if ui
                            .add_enabled(!is_busy, egui::Button::new(&label))
                            .clicked()
                        {
                            self.dispatch_move_absolute(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                value,
                            );
                        }
                    }
                });
            }
        });
    }

    fn render_sensor(
        &self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::SensorSectionConfig,
    ) {
        let SectionState::Sensor(ref state) = self.sections[idx] else {
            return;
        };
        let precision = cfg.precision as usize;
        let unit = cfg.unit.as_deref().unwrap_or(&state.units);

        ui.group(|ui| {
            ui.label(egui::RichText::new(&cfg.label).strong());

            if let Some(value) = state.value {
                // Gauge widget with auto-scaling
                let raw_f32 = value as f32;
                let max_val = if raw_f32.abs() < 1.0 {
                    1.0
                } else {
                    raw_f32.abs() * 2.0
                };
                ui.add(
                    Gauge::new(raw_f32)
                        .range(0.0, max_val)
                        .label(&cfg.label)
                        .unit(unit)
                        .size(28.0),
                );

                let text = if unit.is_empty() {
                    format!("{value:.precision$}")
                } else {
                    format!("{value:.precision$} {unit}")
                };
                ui.label(egui::RichText::new(text).monospace().size(18.0));
            } else {
                ui.label(egui::RichText::new("---").monospace());
                ui.spinner();
            }

            // Trend chart
            if cfg.show_trend && state.trend_data.len() >= 2 {
                let plot_points: Vec<[f64; 2]> =
                    state.trend_data.iter().map(|(t, v)| [*t, *v]).collect();
                let line = Line::new("trend", PlotPoints::new(plot_points));
                Plot::new(("sensor_trend", idx))
                    .height(80.0)
                    .show_axes([false, true])
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .show(ui, |plot_ui| {
                        plot_ui.line(line);
                    });
            }
        });
    }

    fn render_status_display(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::StatusDisplaySectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        // Take state out to avoid borrow conflict with self.dispatch_*()
        let section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::StatusDisplay(ref state) = section else {
            self.sections[idx] = section;
            return;
        };

        ui.group(|ui| {
            ui.horizontal(|ui| {
                if !cfg.label.is_empty() {
                    ui.label(egui::RichText::new(&cfg.label).strong());
                }
                if ui
                    .add_enabled(self.actions_in_flight == 0, egui::Button::new("⟳").small())
                    .on_hover_text("Refresh")
                    .clicked()
                {
                    self.dispatch_status_refresh(
                        client.as_deref_mut(),
                        runtime,
                        device_id,
                        idx,
                        &cfg.parameters,
                    );
                }
            });

            if state.values.is_empty() {
                ui.weak("Loading...");
            } else if cfg.compact {
                let display: Vec<String> = state
                    .values
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect();
                ui.label(display.join(" | "));
            } else {
                egui::Grid::new(("status_grid", idx))
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for (name, value) in &state.values {
                            ui.label(name);
                            ui.label(egui::RichText::new(value).monospace());
                            ui.end_row();
                        }
                    });
            }
        });

        self.sections[idx] = section;
    }

    fn render_shutter(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::ShutterSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::Shutter(ref state) = section else {
            self.sections[idx] = section;
            return;
        };
        let is_open = state.is_open.unwrap_or(false);
        let is_busy = self.actions_in_flight > 0;

        ui.group(|ui| {
            ui.label(egui::RichText::new(&cfg.label).strong());

            if cfg.toggle_style {
                let (text, text_color, fill) = if is_open {
                    (
                        "SHUTTER OPEN",
                        egui::Color32::WHITE,
                        egui::Color32::from_rgb(220, 50, 50),
                    )
                } else {
                    (
                        "Shutter Closed",
                        egui::Color32::WHITE,
                        egui::Color32::from_rgb(50, 150, 50),
                    )
                };
                let btn = egui::Button::new(egui::RichText::new(text).color(text_color).strong())
                    .fill(fill)
                    .min_size(egui::vec2(120.0, 24.0));
                if ui.add_enabled(!is_busy, btn).clicked() {
                    self.dispatch_set_shutter(client.as_deref_mut(), runtime, device_id, !is_open);
                }
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!is_busy && !is_open, egui::Button::new("Open"))
                        .clicked()
                    {
                        self.dispatch_set_shutter(client.as_deref_mut(), runtime, device_id, true);
                    }
                    if ui
                        .add_enabled(!is_busy && is_open, egui::Button::new("Close"))
                        .clicked()
                    {
                        self.dispatch_set_shutter(client.as_deref_mut(), runtime, device_id, false);
                    }
                    if is_open {
                        ui.colored_label(egui::Color32::from_rgb(220, 50, 50), "● OPEN");
                    } else {
                        ui.colored_label(egui::Color32::GREEN, "● Closed");
                    }
                });
            }
        });

        self.sections[idx] = section;
    }

    fn render_wavelength(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::WavelengthSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let mut section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::Wavelength(ref mut state) = section else {
            self.sections[idx] = section;
            return;
        };
        let is_busy = self.actions_in_flight > 0;

        ui.group(|ui| {
            ui.label(egui::RichText::new(&cfg.label).strong());

            if let Some(nm) = state.current_nm {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{nm:.1} nm"))
                            .monospace()
                            .size(16.0),
                    );
                    if cfg.show_color {
                        let color = wavelength_to_rgb(nm);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(24.0, 16.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, color);
                    }
                });
            }

            if cfg.show_slider {
                ui.horizontal(|ui| {
                    ui.label("λ:");
                    let slider_resp = ui.add(
                        egui::Slider::new(&mut state.slider_value, state.min_nm..=state.max_nm)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always),
                    );
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut state.input)
                            .desired_width(50.0)
                            .hint_text("nm"),
                    );
                    ui.label("nm");

                    if slider_resp.drag_started() {
                        state.dragging = true;
                    }
                    if slider_resp.drag_stopped() {
                        state.dragging = false;
                        state.input = format!("{:.1}", state.slider_value);
                        self.dispatch_set_wavelength(
                            client.as_deref_mut(),
                            runtime,
                            device_id,
                            state.slider_value,
                        );
                    }
                    if state.dragging {
                        state.input = format!("{:.1}", state.slider_value);
                    }

                    if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                        if let Ok(nm) = state.input.parse::<f64>() {
                            self.dispatch_set_wavelength(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                nm,
                            );
                        }
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if let Ok(nm) = state.input.parse::<f64>() {
                            self.dispatch_set_wavelength(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                nm,
                            );
                        }
                    }
                });
            }

            if !cfg.presets.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for &nm in &cfg.presets {
                        if ui
                            .add_enabled(!is_busy, egui::Button::new(format!("{nm:.0}")))
                            .clicked()
                        {
                            self.dispatch_set_wavelength(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                nm,
                            );
                        }
                    }
                });
            }
        });

        self.sections[idx] = section;
    }

    fn render_custom_action(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::CustomActionSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let mut section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::CustomAction(ref mut state) = section else {
            self.sections[idx] = section;
            return;
        };
        let is_busy = self.actions_in_flight > 0;

        if cfg.confirm.is_some() && !state.confirming {
            if ui
                .add_enabled(!is_busy, styled_button(&cfg.label, cfg.style))
                .clicked()
            {
                state.confirming = true;
            }
        } else if state.confirming {
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    cfg.confirm.as_deref().unwrap_or("Are you sure?"),
                );
                if ui.button("Yes").clicked() {
                    state.confirming = false;
                    self.dispatch_command(
                        client.as_deref_mut(),
                        runtime,
                        device_id,
                        idx,
                        &cfg.command,
                        &cfg.params,
                    );
                }
                if ui.button("Cancel").clicked() {
                    state.confirming = false;
                }
            });
        } else if ui
            .add_enabled(!is_busy, styled_button(&cfg.label, cfg.style))
            .clicked()
        {
            self.dispatch_command(
                client.as_deref_mut(),
                runtime,
                device_id,
                idx,
                &cfg.command,
                &cfg.params,
            );
        }

        self.sections[idx] = section;
    }

    fn render_parameter(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::ParameterSectionConfig,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        let mut section = std::mem::replace(&mut self.sections[idx], SectionState::Separator);
        let SectionState::Parameter(ref mut state) = section else {
            self.sections[idx] = section;
            return;
        };
        let is_busy = self.actions_in_flight > 0;
        let label = if cfg.label.is_empty() {
            &cfg.parameter
        } else {
            &cfg.label
        };

        ui.group(|ui| {
            ui.label(egui::RichText::new(label).strong());

            if cfg.read_only {
                let display = state.value.as_deref().unwrap_or("---");
                ui.label(egui::RichText::new(display).monospace());
            } else {
                match cfg.widget {
                    ParameterWidget::Toggle => {
                        // Boolean toggle — interpret "true"/"false"/"1"/"0"
                        let mut checked = matches!(
                            state.input.to_lowercase().as_str(),
                            "true" | "1" | "on" | "yes"
                        );
                        if ui.checkbox(&mut checked, "").changed() {
                            state.input = checked.to_string();
                            let param = cfg.parameter.clone();
                            let val = state.input.clone();
                            self.dispatch_set_parameter(
                                client.as_deref_mut(),
                                runtime,
                                device_id,
                                idx,
                                &param,
                                &val,
                            );
                        }
                    }
                    ParameterWidget::Slider => {
                        // Numeric slider — derive range from current value.
                        // Only dispatch on drag_stopped to avoid spamming serial devices.
                        if let Ok(mut val) = state.input.parse::<f64>() {
                            let max = if val.abs() < 1.0 {
                                1.0
                            } else {
                                val.abs() * 2.0
                            };
                            let min = if val >= 0.0 { 0.0 } else { -max };
                            ui.horizontal(|ui| {
                                let resp = ui.add(egui::Slider::new(&mut val, min..=max));
                                if resp.changed() {
                                    state.input = format!("{val}");
                                }
                                if resp.drag_stopped() {
                                    state.input = format!("{val}");
                                    let param = cfg.parameter.clone();
                                    self.dispatch_set_parameter(
                                        client.as_deref_mut(),
                                        runtime,
                                        device_id,
                                        idx,
                                        &param,
                                        &state.input,
                                    );
                                }
                            });
                        } else {
                            // Can't parse as number — fall back to text input
                            self.render_parameter_text_input(
                                ui, idx, cfg, state, is_busy, device_id, client, runtime,
                            );
                        }
                    }
                    ParameterWidget::Spinner => {
                        // Numeric spinner via DragValue — debounced to avoid spamming
                        if let Ok(mut val) = state.input.parse::<f64>() {
                            ui.horizontal(|ui| {
                                let changed =
                                    ui.add(egui::DragValue::new(&mut val).speed(0.1)).changed();
                                if changed {
                                    state.input = format!("{val}");
                                    if can_send_command(state.last_command_time, COMMAND_DEBOUNCE) {
                                        state.last_command_time = Some(Instant::now());
                                        let param = cfg.parameter.clone();
                                        self.dispatch_set_parameter(
                                            client.as_deref_mut(),
                                            runtime,
                                            device_id,
                                            idx,
                                            &param,
                                            &state.input,
                                        );
                                    }
                                }
                            });
                        } else {
                            self.render_parameter_text_input(
                                ui, idx, cfg, state, is_busy, device_id, client, runtime,
                            );
                        }
                    }
                    ParameterWidget::Auto
                    | ParameterWidget::TextInput
                    | ParameterWidget::Dropdown => {
                        // TextInput is the default; Dropdown falls back here since the schema
                        // has no `choices` field yet; Auto uses TextInput as safest default
                        self.render_parameter_text_input(
                            ui, idx, cfg, state, is_busy, device_id, client, runtime,
                        );
                    }
                }
            }
        });

        self.sections[idx] = section;
    }

    /// Shared text-input rendering for Parameter sections (used by TextInput, Auto, Dropdown,
    /// and as fallback when Slider/Spinner can't parse the value).
    #[allow(clippy::too_many_arguments)]
    fn render_parameter_text_input(
        &mut self,
        ui: &mut Ui,
        idx: usize,
        cfg: &hardware::config::schema::ParameterSectionConfig,
        state: &mut ParameterSectionState,
        is_busy: bool,
        device_id: &str,
        client: &mut Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.input).desired_width(100.0));

            let has_changes = state
                .value
                .as_ref()
                .map(|v| v != &state.input)
                .unwrap_or(!state.input.is_empty());

            if ui
                .add_enabled(!is_busy && has_changes, egui::Button::new("Set"))
                .clicked()
            {
                let param = cfg.parameter.clone();
                let val = state.input.clone();
                self.dispatch_set_parameter(
                    client.as_deref_mut(),
                    runtime,
                    device_id,
                    idx,
                    &param,
                    &val,
                );
            }
        });
    }
}

// =============================================================================
// DeviceControlWidget trait implementation
// =============================================================================

impl DeviceControlWidget for ConfigDrivenPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        ui.set_max_width(ui.available_width());
        self.poll_results();

        let device_id = device.id.clone();
        self.device_id = Some(device_id.clone());

        if !self.initial_fetch_done && client.is_some() {
            self.initial_fetch_done = true;
            self.fetch_initial_state(client.as_deref_mut(), runtime, &device_id);
        }

        self.auto_refresh(&mut client, runtime, &device_id);

        // Closure that renders the panel body (shared between collapsible and non-collapsible)
        let mut render_body = |panel: &mut Self, ui: &mut Ui| {
            if let Some(ref err) = panel.error {
                ui.colored_label(egui::Color32::RED, err);
            }
            if let Some(ref status) = panel.status {
                ui.colored_label(egui::Color32::GREEN, status);
            }

            let section_count = panel.config.sections.len();
            match panel.config.layout {
                PanelLayout::Vertical | PanelLayout::Grid => {
                    for idx in 0..section_count {
                        panel.render_section(ui, idx, &device_id, &mut client, runtime);
                    }
                }
                PanelLayout::Horizontal => {
                    ui.horizontal(|ui| {
                        for idx in 0..section_count {
                            panel.render_section(ui, idx, &device_id, &mut client, runtime);
                        }
                    });
                }
            }

            if panel.actions_in_flight > 0 {
                ui.spinner();
            }
        };

        if self.config.collapsible {
            egui::CollapsingHeader::new(&device.name)
                .default_open(true)
                .show(ui, |ui| {
                    render_body(self, ui);
                });
        } else {
            if self.config.show_header {
                ui.heading(&device.name);
                ui.separator();
            }
            render_body(self, ui);
        }

        if self.actions_in_flight > 0 || self.has_auto_refresh() {
            self.request_smart_repaint(ui);
        }
    }

    fn device_type(&self) -> &'static str {
        "config_driven"
    }
}

// =============================================================================
// Free functions
// =============================================================================

fn styled_button<'a>(label: &str, style: ButtonStyle) -> egui::Button<'a> {
    match style {
        ButtonStyle::Danger => egui::Button::new(
            egui::RichText::new(label.to_string())
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(220, 50, 50)),
        ButtonStyle::Success => egui::Button::new(
            egui::RichText::new(label.to_string())
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(50, 150, 50)),
        ButtonStyle::Primary => egui::Button::new(
            egui::RichText::new(label.to_string())
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(egui::Color32::from_rgb(50, 100, 200)),
        _ => egui::Button::new(label.to_string()),
    }
}

fn can_send_command(last: Option<Instant>, debounce: Duration) -> bool {
    last.map(|t| t.elapsed() >= debounce).unwrap_or(true)
}

/// Convert a wavelength in nanometers to an approximate RGB color.
///
/// Uses a standard piecewise linear approximation for the visible spectrum (380–780 nm).
/// Wavelengths outside this range (e.g. IR Ti:Sapphire 690–1040 nm) are shown as
/// deep red with decreasing intensity. Near-UV appears violet.
fn wavelength_to_rgb(nm: f64) -> egui::Color32 {
    let (r, g, b) = if nm < 380.0 {
        (0.4, 0.0, 0.4) // UV → dim violet
    } else if nm < 440.0 {
        let t = (nm - 380.0) / (440.0 - 380.0);
        (0.4 * (1.0 - t), 0.0, 0.4 + 0.6 * t) // violet → blue
    } else if nm < 490.0 {
        let t = (nm - 440.0) / (490.0 - 440.0);
        (0.0, t, 1.0) // blue → cyan
    } else if nm < 510.0 {
        let t = (nm - 490.0) / (510.0 - 490.0);
        (0.0, 1.0, 1.0 - t) // cyan → green
    } else if nm < 580.0 {
        let t = (nm - 510.0) / (580.0 - 510.0);
        (t, 1.0, 0.0) // green → yellow
    } else if nm < 645.0 {
        let t = (nm - 580.0) / (645.0 - 580.0);
        (1.0, 1.0 - t, 0.0) // yellow → red
    } else if nm < 780.0 {
        (1.0, 0.0, 0.0) // red
    } else {
        // Near-IR: fade red toward dark (Ti:Sapphire range)
        let fade = (1.0 - (nm - 780.0) / 300.0).clamp(0.2, 1.0);
        (fade, 0.0, 0.0)
    };

    egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardware::config::schema::{
        CameraSectionConfig, ControlPanelConfig, CustomActionSectionConfig, CustomSectionConfig,
        MotionSectionConfig, ParameterSectionConfig, SensorSectionConfig, SeparatorConfig,
        ShutterSectionConfig, StatusDisplaySectionConfig, WavelengthSectionConfig,
    };

    #[test]
    fn test_config_driven_panel_creation() {
        let config = ControlPanelConfig::default();
        let panel = ConfigDrivenPanel::new(config);
        assert_eq!(panel.sections.len(), 0);
        assert!(!panel.initial_fetch_done);
        assert_eq!(panel.actions_in_flight, 0);
    }

    #[test]
    fn test_all_section_types_create_correct_state() {
        let config = ControlPanelConfig {
            sections: vec![
                ControlSection::Motion(MotionSectionConfig::default()),
                ControlSection::PresetButtons(Default::default()),
                ControlSection::CustomAction(CustomActionSectionConfig::default()),
                ControlSection::Camera(CameraSectionConfig::default()),
                ControlSection::Shutter(ShutterSectionConfig::default()),
                ControlSection::Wavelength(WavelengthSectionConfig::default()),
                ControlSection::Parameter(ParameterSectionConfig {
                    parameter: "test".to_string(),
                    ..Default::default()
                }),
                ControlSection::StatusDisplay(StatusDisplaySectionConfig::default()),
                ControlSection::Sensor(SensorSectionConfig::default()),
                ControlSection::Separator(SeparatorConfig::default()),
                ControlSection::Custom(CustomSectionConfig::default()),
            ],
            ..Default::default()
        };
        let panel = ConfigDrivenPanel::new(config);
        assert_eq!(panel.sections.len(), 11);
        assert!(matches!(panel.sections[0], SectionState::Motion(_)));
        assert!(matches!(panel.sections[1], SectionState::PresetButtons));
        assert!(matches!(panel.sections[2], SectionState::CustomAction(_)));
        assert!(matches!(panel.sections[3], SectionState::Camera));
        assert!(matches!(panel.sections[4], SectionState::Shutter(_)));
        assert!(matches!(panel.sections[5], SectionState::Wavelength(_)));
        assert!(matches!(panel.sections[6], SectionState::Parameter(_)));
        assert!(matches!(panel.sections[7], SectionState::StatusDisplay(_)));
        assert!(matches!(panel.sections[8], SectionState::Sensor(_)));
        assert!(matches!(panel.sections[9], SectionState::Separator));
        assert!(matches!(panel.sections[10], SectionState::Custom));
    }

    #[test]
    fn test_has_auto_refresh() {
        // Motion sections always auto-refresh (position polling)
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Motion(MotionSectionConfig::default())],
            ..Default::default()
        };
        assert!(ConfigDrivenPanel::new(config).has_auto_refresh());

        // Sensor with refresh_ms > 0 auto-refreshes
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Sensor(SensorSectionConfig {
                refresh_ms: 1000,
                ..Default::default()
            })],
            ..Default::default()
        };
        assert!(ConfigDrivenPanel::new(config).has_auto_refresh());

        // Sensor with refresh_ms = 0 does NOT auto-refresh
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Sensor(SensorSectionConfig {
                refresh_ms: 0,
                ..Default::default()
            })],
            ..Default::default()
        };
        assert!(!ConfigDrivenPanel::new(config).has_auto_refresh());

        // Empty panel does NOT auto-refresh
        let config = ControlPanelConfig::default();
        assert!(!ConfigDrivenPanel::new(config).has_auto_refresh());
    }

    #[test]
    fn test_can_send_command_debounce() {
        assert!(can_send_command(None, COMMAND_DEBOUNCE));
        assert!(!can_send_command(Some(Instant::now()), COMMAND_DEBOUNCE));
        let past = Instant::now().checked_sub(Duration::from_millis(500));
        assert!(can_send_command(past, COMMAND_DEBOUNCE));
    }

    #[test]
    fn test_device_type_returns_config_driven() {
        let config = ControlPanelConfig::default();
        let panel = ConfigDrivenPanel::new(config);
        assert_eq!(panel.device_type(), "config_driven");
    }

    #[test]
    fn test_wavelength_to_rgb_visible_spectrum() {
        // UV → violet-ish
        let uv = wavelength_to_rgb(350.0);
        assert!(uv.b() > 0);

        // Blue (450 nm)
        let blue = wavelength_to_rgb(450.0);
        assert!(blue.b() > blue.r());

        // Green (530 nm)
        let green = wavelength_to_rgb(530.0);
        assert!(green.g() > green.r());
        assert!(green.g() > green.b());

        // Red (650 nm)
        let red = wavelength_to_rgb(650.0);
        assert_eq!(red, egui::Color32::from_rgb(255, 0, 0));

        // Near-IR (800 nm Ti:Sapphire) → faded red
        let ir = wavelength_to_rgb(800.0);
        assert!(ir.r() > 0);
        assert_eq!(ir.g(), 0);
        assert_eq!(ir.b(), 0);

        // Deep IR (1000 nm) → even more faded red, but not black
        let deep_ir = wavelength_to_rgb(1000.0);
        assert!(deep_ir.r() > 0);
        assert!(deep_ir.r() < ir.r()); // fainter than 800nm
    }

    #[test]
    fn test_styled_button_variants() {
        // Just verify they don't panic
        let _ = styled_button("Danger", ButtonStyle::Danger);
        let _ = styled_button("Success", ButtonStyle::Success);
        let _ = styled_button("Primary", ButtonStyle::Primary);
        let _ = styled_button("Default", ButtonStyle::Default);
    }

    #[test]
    fn test_sensor_state_trend_tracking() {
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Sensor(SensorSectionConfig {
                show_trend: true,
                refresh_ms: 1000,
                ..Default::default()
            })],
            ..Default::default()
        };
        let panel = ConfigDrivenPanel::new(config);

        // Trend data starts empty, populated by poll_results
        if let SectionState::Sensor(ref s) = panel.sections[0] {
            assert!(s.trend_data.is_empty());
            assert!(s.trend_start.is_none()); // set during initial fetch
        } else {
            panic!("Expected SectionState::Sensor");
        }
    }

    #[test]
    fn test_wavelength_state_defaults() {
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Wavelength(
                WavelengthSectionConfig::default(),
            )],
            ..Default::default()
        };
        let panel = ConfigDrivenPanel::new(config);

        if let SectionState::Wavelength(ref s) = panel.sections[0] {
            assert_eq!(s.min_nm, 690.0);
            assert_eq!(s.max_nm, 1040.0);
            assert_eq!(s.slider_value, 800.0);
            assert!(!s.dragging);
        } else {
            panic!("Expected SectionState::Wavelength");
        }
    }

    #[test]
    fn test_motion_state_defaults() {
        let config = ControlPanelConfig {
            sections: vec![ControlSection::Motion(MotionSectionConfig::default())],
            ..Default::default()
        };
        let panel = ConfigDrivenPanel::new(config);

        if let SectionState::Motion(ref s) = panel.sections[0] {
            assert!(s.position.is_none());
            assert!(!s.moving);
            assert!(s.position_input.is_empty());
            assert!(s.last_command_time.is_none());
        } else {
            panic!("Expected SectionState::Motion");
        }
    }
}
