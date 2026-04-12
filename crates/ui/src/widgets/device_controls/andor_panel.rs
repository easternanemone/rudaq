//! Andor iStar gated camera control panel.
//!
//! Provides:
//! - Temperature display (sensor readback)
//! - Exposure time control
//! - Trigger mode selector (Internal / External)
//! - DDG (Digital Delay Generator) delay and width
//! - MCP gain slider
//! - Arm / Disarm controls
//! - Acquisition status indicator

use crate::layout;
use crate::runtime::Runtime;
use crate::time::{Duration, Instant};
use egui::Ui;
use std::cell::Cell;

use crate::widgets::device_controls::{
    DeviceControlWidget, DevicePanelState, LatestRequestTracker, action_button, device_info_rows,
    filled_action_button, panel_hint_text, panel_value_text, parameter_enum_values,
    parameter_numeric_range, parse_nonnegative_i64_input, parse_positive_f64_input,
    request_panel_repaint, resolve_parameter_name, scoped_widget_id, show_device_info_section,
    show_panel_columns_with_state, show_panel_header, show_panel_messages, show_panel_section,
};
use client::DaqClient;
use protocol::daq::{DeviceInfo, ParameterDescriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingKey {
    Temperature,
    Exposure,
    TriggerMode,
    DdgDelay,
    DdgWidth,
    McpGain,
    Armed,
    Cooling,
}

#[derive(Debug, Clone, Copy)]
struct BindingSpec {
    key: BindingKey,
    aliases: &'static [&'static str],
}

const FALLBACK_TRIGGER_MODES: [&str; 5] = [
    "Internal",
    "External",
    "Software",
    "External Start",
    "External Exposure",
];
const PARAM_SCHEMA_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const EXPOSURE_PARAM_ALIASES: [&str; 3] = ["exposure_s", "exposure", "ExposureTime"];
const TEMPERATURE_PARAM_ALIASES: [&str; 3] = ["temperature_c", "temperature", "SensorTemperature"];
const TRIGGER_MODE_PARAM_ALIASES: [&str; 2] = ["trigger_mode", "TriggerMode"];
const DDG_DELAY_PARAM_ALIASES: [&str; 3] = ["ddg_output_delay_ps", "ddg_delay", "DDGOutputDelay"];
const DDG_WIDTH_PARAM_ALIASES: [&str; 3] = ["ddg_output_width_ps", "ddg_width", "DDGOutputWidth"];
const MCP_GAIN_PARAM_ALIASES: [&str; 2] = ["mcp_gain", "MCPGain"];
const ARMED_PARAM_ALIASES: [&str; 2] = ["armed", "camera_acquiring"];
const COOLING_PARAM_ALIASES: [&str; 3] = ["cooling_enabled", "cooling", "SensorCooling"];
const ANDOR_BINDINGS: [BindingSpec; 8] = [
    BindingSpec {
        key: BindingKey::Temperature,
        aliases: &TEMPERATURE_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::Exposure,
        aliases: &EXPOSURE_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::TriggerMode,
        aliases: &TRIGGER_MODE_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::DdgDelay,
        aliases: &DDG_DELAY_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::DdgWidth,
        aliases: &DDG_WIDTH_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::McpGain,
        aliases: &MCP_GAIN_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::Armed,
        aliases: &ARMED_PARAM_ALIASES,
    },
    BindingSpec {
        key: BindingKey::Cooling,
        aliases: &COOLING_PARAM_ALIASES,
    },
];

/// Andor camera state cached from the daemon.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `online` reserved for future online/offline indicator
struct AndorCameraState {
    temperature: Option<f64>,
    exposure_s: Option<f64>,
    exposure_range_s: Option<(f64, f64)>,
    trigger_mode: Option<String>,
    trigger_mode_options: Vec<String>,
    ddg_delay_ps: Option<i64>,
    ddg_width_ps: Option<i64>,
    mcp_gain: Option<i32>,
    mcp_gain_range: Option<(i32, i32)>,
    armed: bool,
    cooling: bool,
    online: bool,
    schema_refreshed: bool,
    temperature_param_name: Option<String>,
    exposure_param_name: Option<String>,
    trigger_mode_param_name: Option<String>,
    ddg_delay_param_name: Option<String>,
    ddg_width_param_name: Option<String>,
    mcp_gain_param_name: Option<String>,
    armed_param_name: Option<String>,
    cooling_param_name: Option<String>,
}

impl Default for AndorCameraState {
    fn default() -> Self {
        Self {
            temperature: None,
            exposure_s: None,
            exposure_range_s: None,
            trigger_mode: None,
            trigger_mode_options: fallback_trigger_modes(),
            ddg_delay_ps: None,
            ddg_width_ps: None,
            mcp_gain: None,
            mcp_gain_range: None,
            armed: false,
            cooling: false,
            online: true,
            schema_refreshed: false,
            temperature_param_name: Some(TEMPERATURE_PARAM_ALIASES[0].to_string()),
            exposure_param_name: Some(EXPOSURE_PARAM_ALIASES[0].to_string()),
            trigger_mode_param_name: Some(TRIGGER_MODE_PARAM_ALIASES[0].to_string()),
            ddg_delay_param_name: Some(DDG_DELAY_PARAM_ALIASES[0].to_string()),
            ddg_width_param_name: Some(DDG_WIDTH_PARAM_ALIASES[0].to_string()),
            mcp_gain_param_name: Some(MCP_GAIN_PARAM_ALIASES[0].to_string()),
            armed_param_name: Some(ARMED_PARAM_ALIASES[0].to_string()),
            cooling_param_name: Some(COOLING_PARAM_ALIASES[0].to_string()),
        }
    }
}

impl AndorCameraState {
    fn binding_name(&self, key: BindingKey) -> Option<&str> {
        match key {
            BindingKey::Temperature => self.temperature_param_name.as_deref(),
            BindingKey::Exposure => self.exposure_param_name.as_deref(),
            BindingKey::TriggerMode => self.trigger_mode_param_name.as_deref(),
            BindingKey::DdgDelay => self.ddg_delay_param_name.as_deref(),
            BindingKey::DdgWidth => self.ddg_width_param_name.as_deref(),
            BindingKey::McpGain => self.mcp_gain_param_name.as_deref(),
            BindingKey::Armed => self.armed_param_name.as_deref(),
            BindingKey::Cooling => self.cooling_param_name.as_deref(),
        }
    }

    fn set_binding_name(&mut self, key: BindingKey, name: String) {
        match key {
            BindingKey::Temperature => self.temperature_param_name = Some(name),
            BindingKey::Exposure => self.exposure_param_name = Some(name),
            BindingKey::TriggerMode => self.trigger_mode_param_name = Some(name),
            BindingKey::DdgDelay => self.ddg_delay_param_name = Some(name),
            BindingKey::DdgWidth => self.ddg_width_param_name = Some(name),
            BindingKey::McpGain => self.mcp_gain_param_name = Some(name),
            BindingKey::Armed => self.armed_param_name = Some(name),
            BindingKey::Cooling => self.cooling_param_name = Some(name),
        }
    }

    fn apply_descriptor(&mut self, key: BindingKey, descriptor: &ParameterDescriptor) {
        match key {
            BindingKey::Exposure => {
                self.exposure_range_s = parameter_numeric_range(descriptor);
            }
            BindingKey::TriggerMode => {
                let options = parameter_enum_values(descriptor);
                if !options.is_empty() {
                    self.trigger_mode_options = options;
                }
            }
            BindingKey::McpGain => {
                self.mcp_gain_range = descriptor_i32_range(descriptor);
            }
            _ => {}
        }
    }

    fn apply_fetched_value(&mut self, key: BindingKey, name: String, value: String) {
        self.set_binding_name(key, name);
        match key {
            BindingKey::Temperature => {
                self.temperature = value.parse::<f64>().ok();
            }
            BindingKey::Exposure => {
                self.exposure_s = value.parse::<f64>().ok();
            }
            BindingKey::TriggerMode => {
                self.trigger_mode = Some(value);
            }
            BindingKey::DdgDelay => {
                self.ddg_delay_ps = value.parse::<i64>().ok();
            }
            BindingKey::DdgWidth => {
                self.ddg_width_ps = value.parse::<i64>().ok();
            }
            BindingKey::McpGain => {
                self.mcp_gain = value.parse::<i32>().ok();
            }
            BindingKey::Armed => {
                self.armed = parse_bool(&value).unwrap_or(false);
            }
            BindingKey::Cooling => {
                self.cooling = parse_bool(&value).unwrap_or(false);
            }
        }
    }
}

/// Async action results for the Andor camera panel.
enum ActionResult {
    FetchState {
        request_id: u64,
        result: Box<Result<AndorCameraState, String>>,
    },
    SetParameter(Result<String, String>),
    Arm(Result<(), String>),
    Disarm(Result<(), String>),
}

#[derive(Clone, Copy)]
enum AndorUiAction {
    SetExposure,
    SetTriggerMode(usize),
    SetDdgDelay,
    SetDdgWidth,
    SetMcpGain,
    Arm,
    Disarm,
    Refresh,
}

/// Andor iStar gated camera control panel.
pub struct AndorCameraPanel {
    panel_state: DevicePanelState<ActionResult>,
    fetch_request_tracker: LatestRequestTracker,
    refresh_after_command: bool,
    state: AndorCameraState,
    exposure_input: String,
    ddg_delay_input: String,
    ddg_width_input: String,
    mcp_gain_input: f32,
    trigger_mode_idx: usize,
    exposure_editing: bool,
    ddg_delay_editing: bool,
    ddg_width_editing: bool,
    last_schema_refresh: Option<Instant>,
}

impl Default for AndorCameraPanel {
    fn default() -> Self {
        Self {
            panel_state: DevicePanelState::new(),
            fetch_request_tracker: LatestRequestTracker::default(),
            refresh_after_command: false,
            state: AndorCameraState::default(),
            exposure_input: "0.01".to_string(),
            ddg_delay_input: "1300000".to_string(),
            ddg_width_input: "10000000".to_string(),
            mcp_gain_input: 0.0,
            trigger_mode_idx: 0,
            exposure_editing: false,
            ddg_delay_editing: false,
            ddg_width_editing: false,
            last_schema_refresh: None,
        }
    }
}

impl AndorCameraPanel {
    const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

    fn trigger_modes(&self) -> Vec<String> {
        if self.state.trigger_mode_options.is_empty() {
            fallback_trigger_modes()
        } else {
            self.state.trigger_mode_options.clone()
        }
    }

    fn poll_results(&mut self) {
        while let Ok(result) = self.panel_state.action_rx.try_recv() {
            match result {
                ActionResult::FetchState { request_id, result } => {
                    self.panel_state.background_task_completed();
                    if !self.fetch_request_tracker.is_current(request_id) {
                        continue;
                    }

                    match *result {
                        Ok(state) => {
                            if let Some(exp) = state.exposure_s
                                && !self.exposure_editing
                            {
                                self.exposure_input = format!("{exp:.6}");
                            }
                            if let Some(delay) = state.ddg_delay_ps
                                && !self.ddg_delay_editing
                            {
                                self.ddg_delay_input = delay.to_string();
                            }
                            if let Some(width) = state.ddg_width_ps
                                && !self.ddg_width_editing
                            {
                                self.ddg_width_input = width.to_string();
                            }
                            if let Some(gain) = state.mcp_gain {
                                #[allow(clippy::cast_precision_loss)]
                                {
                                    self.mcp_gain_input = gain as f32;
                                }
                            }

                            let trigger_modes = if state.trigger_mode_options.is_empty() {
                                fallback_trigger_modes()
                            } else {
                                state.trigger_mode_options.clone()
                            };
                            if let Some(ref mode) = state.trigger_mode {
                                self.trigger_mode_idx = trigger_modes
                                    .iter()
                                    .position(|candidate| candidate.eq_ignore_ascii_case(mode))
                                    .unwrap_or(0);
                            } else if self.trigger_mode_idx >= trigger_modes.len() {
                                self.trigger_mode_idx = 0;
                            }

                            self.state = state;
                            self.panel_state.error = None;
                        }
                        Err(e) => {
                            self.panel_state
                                .set_error(format!("Failed to fetch state: {e}"));
                        }
                    }
                }
                ActionResult::SetParameter(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(message) => {
                            self.refresh_after_command = true;
                            self.panel_state.set_status(message);
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Set failed: {e}"));
                        }
                    }
                }
                ActionResult::Arm(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.armed = true;
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Camera armed");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Arm failed: {e}"));
                        }
                    }
                }
                ActionResult::Disarm(result) => {
                    self.panel_state.action_completed();
                    match result {
                        Ok(()) => {
                            self.state.armed = false;
                            self.refresh_after_command = true;
                            self.panel_state.set_status("Camera disarmed");
                        }
                        Err(e) => {
                            self.panel_state.set_error(format!("Disarm failed: {e}"));
                        }
                    }
                }
            }
        }
    }

    fn fetch_state(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        refresh_schema: bool,
    ) {
        let Some(client) = client else {
            return;
        };

        self.panel_state.mark_refreshed();
        self.panel_state.background_task_started();
        let request_id = self.fetch_request_tracker.issue();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();
        let mut seed_state = self.state.clone();

        runtime.spawn(async move {
            async fn get_parameter_value(
                client: &mut DaqClient,
                device_id: &str,
                name: &str,
            ) -> Option<String> {
                client
                    .get_parameter(device_id, name)
                    .await
                    .ok()
                    .map(|value| value.value)
            }

            async fn fetch_binding(
                client: &mut DaqClient,
                device_id: &str,
                preferred_name: Option<&str>,
                aliases: &[&str],
            ) -> Option<(String, String)> {
                for candidate in candidate_names(preferred_name, aliases) {
                    if let Some(value) = get_parameter_value(client, device_id, &candidate).await {
                        return Some((candidate, value));
                    }
                }
                None
            }

            if refresh_schema || !seed_state.schema_refreshed {
                let descriptors = client.list_parameters(&device_id).await.unwrap_or_default();
                if !descriptors.is_empty() {
                    for binding in ANDOR_BINDINGS {
                        if let Some(name) = resolve_parameter_name(
                            &descriptors,
                            seed_state.binding_name(binding.key),
                            binding.aliases,
                        ) {
                            if let Some(descriptor) =
                                descriptors.iter().find(|desc| desc.name == name)
                            {
                                seed_state.apply_descriptor(binding.key, descriptor);
                            }
                            seed_state.set_binding_name(binding.key, name);
                        }
                    }
                    seed_state.schema_refreshed = true;
                }
            }

            if seed_state.trigger_mode_options.is_empty() {
                seed_state.trigger_mode_options = fallback_trigger_modes();
            }

            for binding in ANDOR_BINDINGS {
                if let Some((name, value)) = fetch_binding(
                    &mut client,
                    &device_id,
                    seed_state.binding_name(binding.key),
                    binding.aliases,
                )
                .await
                {
                    seed_state.apply_fetched_value(binding.key, name, value);
                }
            }

            let _ = tx
                .send(ActionResult::FetchState {
                    request_id,
                    result: Box::new(Ok(seed_state)),
                })
                .await;
        });
    }

    fn queue_refresh_if_needed(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if self.refresh_after_command && !self.panel_state.is_refreshing() {
            self.refresh_after_command = false;
            self.fetch_state(client, runtime, device_id, false);
        }
    }

    fn try_set_exposure(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let value = match parse_positive_f64_input(&self.exposure_input, "exposure") {
            Ok(value) => value,
            Err(error) => {
                self.panel_state.set_error(error);
                return;
            }
        };

        if let Some((min, max)) = self.state.exposure_range_s
            && (value < min || value > max)
        {
            self.panel_state
                .set_error(format!("Exposure out of range ({min:.6} .. {max:.6} s)"));
            return;
        }

        if let Some(parameter) = self.state.exposure_param_name.clone() {
            self.set_parameter(client, runtime, device_id, &parameter, &value.to_string());
        } else {
            self.panel_state
                .set_error("Exposure parameter is unavailable for this device");
        }
    }

    fn try_set_ddg_delay(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let value = match parse_nonnegative_i64_input(&self.ddg_delay_input, "DDG delay") {
            Ok(value) => value,
            Err(error) => {
                self.panel_state.set_error(error);
                return;
            }
        };

        if let Some(parameter) = self.state.ddg_delay_param_name.clone() {
            self.set_parameter(client, runtime, device_id, &parameter, &value.to_string());
        } else {
            self.panel_state
                .set_error("DDG delay parameter is unavailable for this device");
        }
    }

    fn try_set_ddg_width(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        let value = match parse_nonnegative_i64_input(&self.ddg_width_input, "DDG width") {
            Ok(value) => value,
            Err(error) => {
                self.panel_state.set_error(error);
                return;
            }
        };

        if let Some(parameter) = self.state.ddg_width_param_name.clone() {
            self.set_parameter(client, runtime, device_id, &parameter, &value.to_string());
        } else {
            self.panel_state
                .set_error("DDG width parameter is unavailable for this device");
        }
    }

    fn try_set_mcp_gain(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
        if !self.mcp_gain_input.is_finite() {
            self.panel_state
                .set_error("Invalid MCP gain value: must be a finite number");
            return;
        }

        #[allow(clippy::cast_possible_truncation)]
        let value = self.mcp_gain_input.round() as i32;
        let (min_gain, max_gain) = self.state.mcp_gain_range.unwrap_or((0, 4095));
        if value < min_gain || value > max_gain {
            self.panel_state
                .set_error(format!("MCP gain out of range ({min_gain} .. {max_gain})"));
            return;
        }

        if let Some(parameter) = self.state.mcp_gain_param_name.clone() {
            self.set_parameter(client, runtime, device_id, &parameter, &value.to_string());
        } else {
            self.panel_state
                .set_error("MCP gain parameter is unavailable for this device");
        }
    }

    fn set_parameter(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
        parameter: &str,
        value: &str,
    ) {
        let Some(client) = client else {
            self.panel_state.set_error("Not connected");
            return;
        };

        self.panel_state.action_started();
        let mut client = client.clone();
        let tx = self.panel_state.action_tx.clone();
        let device_id = device_id.to_string();
        let parameter = parameter.to_string();
        let value = value.to_string();

        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id, &parameter, &value)
                .await
                .map(|_| format!("{parameter} set to {value}"))
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::SetParameter(result)).await;
        });
    }

    fn arm(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "arm", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Arm(result)).await;
        });
    }

    fn disarm(&mut self, client: Option<&mut DaqClient>, runtime: &Runtime, device_id: &str) {
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
                .execute_device_command(&device_id, "disarm", "")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(ActionResult::Disarm(result)).await;
        });
    }
}

impl DeviceControlWidget for AndorCameraPanel {
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        self.poll_results();

        let device_id = device.id.clone();
        self.panel_state.device_id = Some(device_id.clone());

        if !self.panel_state.initial_fetch_done && client.is_some() {
            self.panel_state.initial_fetch_done = true;
            self.last_schema_refresh = Some(Instant::now());
            self.fetch_state(client.as_deref_mut(), runtime, &device_id, true);
        }

        self.queue_refresh_if_needed(client.as_deref_mut(), runtime, &device_id);

        if self.panel_state.should_refresh(Self::REFRESH_INTERVAL) {
            let refresh_schema = !self.state.schema_refreshed
                || self
                    .last_schema_refresh
                    .map(|instant| instant.elapsed() >= PARAM_SCHEMA_REFRESH_INTERVAL)
                    .unwrap_or(true);
            if refresh_schema {
                self.last_schema_refresh = Some(Instant::now());
            }
            self.fetch_state(client.as_deref_mut(), runtime, &device_id, refresh_schema);
        }

        let is_busy = self.panel_state.is_busy();
        let is_refreshing = self.panel_state.is_refreshing();
        let trigger_modes = self.trigger_modes();
        if self.trigger_mode_idx >= trigger_modes.len() {
            self.trigger_mode_idx = 0;
        }

        let badge = Some(if self.state.armed {
            ("Armed", layout::colors::SUCCESS)
        } else {
            ("Idle", layout::colors::MUTED)
        });
        let pending_action = Cell::new(None);

        show_panel_header(ui, "Andor iStar", badge, is_busy, is_refreshing);
        show_panel_messages(
            ui,
            self.panel_state.error.as_deref(),
            self.panel_state.status.as_deref(),
        );
        ui.add_space(8.0);

        show_panel_columns_with_state(
            ui,
            self,
            |ui, panel| {
                show_panel_section(ui, "Acquisition", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Exposure (s):");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.exposure_input)
                                .desired_width(100.0)
                                .hint_text("seconds"),
                        );
                        panel.exposure_editing = response.has_focus();

                        if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                            pending_action.set(Some(AndorUiAction::SetExposure));
                        }
                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            pending_action.set(Some(AndorUiAction::SetExposure));
                        }
                    });

                    if let Some((min, max)) = panel.state.exposure_range_s {
                        ui.label(panel_hint_text(format!("Range: {min:.6} .. {max:.6} s")));
                    }

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Trigger mode:");
                        let previous_idx = panel.trigger_mode_idx;
                        let selected_text = trigger_modes
                            .get(panel.trigger_mode_idx)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string());
                        egui::ComboBox::from_id_salt(scoped_widget_id(&device_id, "trigger_mode"))
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for (index, mode) in trigger_modes.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut panel.trigger_mode_idx,
                                        index,
                                        mode.as_str(),
                                    );
                                }
                            });

                        if panel.trigger_mode_idx != previous_idx && !is_busy {
                            pending_action
                                .set(Some(AndorUiAction::SetTriggerMode(panel.trigger_mode_idx)));
                        }
                    });
                });

                ui.add_space(layout::SECTION_SPACING / 2.0);
                show_panel_section(ui, "Gate Timing", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Delay (ps):");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.ddg_delay_input)
                                .desired_width(120.0)
                                .hint_text("picoseconds"),
                        );
                        panel.ddg_delay_editing = response.has_focus();
                        if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                            pending_action.set(Some(AndorUiAction::SetDdgDelay));
                        }
                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            pending_action.set(Some(AndorUiAction::SetDdgDelay));
                        }
                    });

                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Width (ps):");
                        let response = ui.add_enabled(
                            !is_busy,
                            egui::TextEdit::singleline(&mut panel.ddg_width_input)
                                .desired_width(120.0)
                                .hint_text("picoseconds"),
                        );
                        panel.ddg_width_editing = response.has_focus();
                        if ui.add_enabled(!is_busy, egui::Button::new("Set")).clicked() {
                            pending_action.set(Some(AndorUiAction::SetDdgWidth));
                        }
                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !is_busy
                        {
                            pending_action.set(Some(AndorUiAction::SetDdgWidth));
                        }
                    });

                    if let (Some(delay), Some(width)) =
                        (panel.state.ddg_delay_ps, panel.state.ddg_width_ps)
                    {
                        #[allow(clippy::cast_precision_loss)]
                        let delay_us = delay as f64 / 1_000_000.0;
                        #[allow(clippy::cast_precision_loss)]
                        let width_us = width as f64 / 1_000_000.0;
                        ui.label(panel_hint_text(format!(
                            "{delay_us:.1} us delay, {width_us:.1} us gate"
                        )));
                    }
                });
            },
            |ui, panel| {
                show_panel_section(ui, "Detector", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Temperature:");
                        if let Some(temp) = panel.state.temperature {
                            let color = if temp <= -10.0 {
                                layout::colors::INFO
                            } else {
                                layout::colors::WARNING
                            };
                            ui.colored_label(color, panel_value_text(format!("{temp:.1} C")));
                        } else {
                            ui.label("---");
                        }

                        if panel.state.cooling {
                            ui.colored_label(layout::colors::INFO, "Cooling");
                        }
                    });

                    ui.add_space(8.0);
                    let (mcp_min, mcp_max) = panel.state.mcp_gain_range.unwrap_or((0, 4095));
                    ui.horizontal_wrapped(|ui| {
                        ui.label("MCP gain:");
                        #[allow(clippy::cast_precision_loss)]
                        let slider_response = ui.add_enabled(
                            !is_busy,
                            egui::Slider::new(
                                &mut panel.mcp_gain_input,
                                mcp_min as f32..=mcp_max as f32,
                            )
                            .show_value(true),
                        );

                        if !is_busy
                            && slider_response.changed()
                            && (slider_response.drag_stopped()
                                || !ui.input(|input| input.pointer.any_down()))
                        {
                            pending_action.set(Some(AndorUiAction::SetMcpGain));
                        }
                    });
                    ui.label(panel_hint_text(format!("Range: {mcp_min} .. {mcp_max}")));
                });

                ui.add_space(layout::SECTION_SPACING / 2.0);
                show_panel_section(ui, "Actions", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if panel.state.armed {
                            let disarm_button = action_button("Disarm").fill(layout::colors::MUTED);
                            if ui.add_enabled(!is_busy, disarm_button).clicked() {
                                pending_action.set(Some(AndorUiAction::Disarm));
                            }
                        } else {
                            let arm_button = filled_action_button("Arm", layout::colors::SUCCESS);
                            if ui.add_enabled(!is_busy, arm_button).clicked() {
                                pending_action.set(Some(AndorUiAction::Arm));
                            }
                        }

                        if ui
                            .add_enabled(!is_refreshing, action_button("Refresh"))
                            .clicked()
                        {
                            pending_action.set(Some(AndorUiAction::Refresh));
                        }
                    });
                });

                ui.add_space(layout::SECTION_SPACING / 2.0);
                let rows = device_info_rows(
                    device,
                    [
                        (
                            "Temperature parameter".to_string(),
                            panel
                                .state
                                .temperature_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "Exposure parameter".to_string(),
                            panel
                                .state
                                .exposure_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "Trigger parameter".to_string(),
                            panel
                                .state
                                .trigger_mode_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "DDG delay parameter".to_string(),
                            panel
                                .state
                                .ddg_delay_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "DDG width parameter".to_string(),
                            panel
                                .state
                                .ddg_width_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "MCP gain parameter".to_string(),
                            panel
                                .state
                                .mcp_gain_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "Armed parameter".to_string(),
                            panel
                                .state
                                .armed_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                        (
                            "Cooling parameter".to_string(),
                            panel
                                .state
                                .cooling_param_name
                                .clone()
                                .unwrap_or_else(|| "<unresolved>".to_string()),
                        ),
                    ],
                );
                show_device_info_section(ui, scoped_widget_id(&device_id, "andor_info"), &rows);
            },
        );

        if let Some(action) = pending_action.get() {
            match action {
                AndorUiAction::SetExposure => {
                    self.try_set_exposure(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::SetTriggerMode(index) => {
                    if let Some(mode) = trigger_modes.get(index).cloned() {
                        if let Some(parameter) = self.state.trigger_mode_param_name.clone() {
                            self.set_parameter(
                                client.as_deref_mut(),
                                runtime,
                                &device_id,
                                &parameter,
                                &mode,
                            );
                        } else {
                            self.panel_state
                                .set_error("Trigger-mode parameter is unavailable for this device");
                        }
                    }
                }
                AndorUiAction::SetDdgDelay => {
                    self.try_set_ddg_delay(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::SetDdgWidth => {
                    self.try_set_ddg_width(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::SetMcpGain => {
                    self.try_set_mcp_gain(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::Arm => {
                    self.arm(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::Disarm => {
                    self.disarm(client.as_deref_mut(), runtime, &device_id);
                }
                AndorUiAction::Refresh => {
                    self.last_schema_refresh = Some(Instant::now());
                    self.fetch_state(client, runtime, &device_id, true);
                }
            }
        }

        request_panel_repaint(ui, is_busy || is_refreshing);
    }

    fn device_type(&self) -> &'static str {
        "AndorCamera"
    }
}

fn fallback_trigger_modes() -> Vec<String> {
    FALLBACK_TRIGGER_MODES
        .iter()
        .map(|mode| (*mode).to_string())
        .collect()
}

fn candidate_names(preferred_name: Option<&str>, aliases: &[&str]) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(name) = preferred_name
        && !name.trim().is_empty()
    {
        candidates.push(name.to_string());
    }

    for alias in aliases {
        if !candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(alias))
        {
            candidates.push((*alias).to_string());
        }
    }

    candidates
}

fn parse_bool(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "true" | "1" | "yes" | "on" | "enabled" => Some(true),
        "false" | "0" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn descriptor_i32_range(descriptor: &ParameterDescriptor) -> Option<(i32, i32)> {
    let (min, max) = parameter_numeric_range(descriptor)?;
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    if min < f64::from(i32::MIN) || max > f64::from(i32::MAX) {
        return None;
    }

    #[allow(clippy::cast_possible_truncation)]
    let min_i32 = min.round() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let max_i32 = max.round() as i32;
    (min_i32 <= max_i32).then_some((min_i32, max_i32))
}
