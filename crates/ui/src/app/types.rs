//! Type definitions for the application module.
//!
//! Contains `CommandWidget*`, `WasmConnectionState`, `ControlPanelLayoutMode`,
//! `DevicePanelKind`, `DeviceAvailability`, `Panel` enum, `PersistedPanelInfo`,
//! and related types.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
/// Result of a health check sent through the channel (bd-j3xz.3.3: includes RTT).
pub(super) enum HealthCheckResult {
    /// Health check succeeded with round-trip time in milliseconds.
    Success { rtt_ms: f64 },
    /// Health check failed with error message.
    Failed(String),
}

/// Database status from the daemon (bd-9n9k.3).
#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct DbStatus {
    pub available: bool,
    pub engine: Option<String>,
    pub state_message: Option<String>,
}

/// Device reconciliation result (validates persisted panels against daemon)
pub(super) enum DeviceReconcileMsg {
    Ok {
        epoch: u64,
        daemon_url: String,
        devices: Vec<DeviceInfo>,
    },
    Err {
        epoch: u64,
        daemon_url: String,
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandWidgetKind {
    Action,
    Status,
}

impl CommandWidgetKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Action => "Action",
            Self::Status => "Status",
        }
    }
}

pub(super) enum CommandWidgetResult {
    Completed {
        widget_id: usize,
        result: Result<String, String>,
    },
}

pub(super) struct CommandWidget {
    pub(super) id: usize,
    pub(super) kind: CommandWidgetKind,
    pub(super) command: String,
    pub(super) label: String,
    pub(super) args_json: String,
    pub(super) auto_refresh: bool,
    pub(super) refresh_interval_ms: u64,
    pub(super) last_refresh: Option<Instant>,
    pub(super) pending: bool,
    pub(super) last_result: Option<String>,
    pub(super) last_error: Option<String>,
}

impl CommandWidget {
    pub(super) fn new(
        id: usize,
        kind: CommandWidgetKind,
        command: String,
        label: String,
        args_json: String,
    ) -> Self {
        Self {
            id,
            kind,
            command,
            label,
            args_json,
            auto_refresh: matches!(kind, CommandWidgetKind::Status),
            refresh_interval_ms: 1500,
            last_refresh: None,
            pending: false,
            last_result: None,
            last_error: None,
        }
    }
}

pub(super) struct CommandWidgetPalette {
    pub(super) widgets: Vec<CommandWidget>,
    pub(super) next_widget_id: usize,
    pub(super) selected_command_idx: usize,
    pub(super) add_kind: CommandWidgetKind,
    pub(super) add_label: String,
    pub(super) add_args_json: String,
    pub(super) action_tx: mpsc::Sender<CommandWidgetResult>,
    pub(super) action_rx: mpsc::Receiver<CommandWidgetResult>,
}

impl Default for CommandWidgetPalette {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(32);
        Self {
            widgets: Vec::new(),
            next_widget_id: 0,
            selected_command_idx: 0,
            add_kind: CommandWidgetKind::Action,
            add_label: String::new(),
            add_args_json: "{}".to_string(),
            action_tx,
            action_rx,
        }
    }
}

impl CommandWidgetPalette {
    pub(super) fn add_widget(
        &mut self,
        kind: CommandWidgetKind,
        command: String,
        label: String,
        args_json: String,
    ) {
        let widget = CommandWidget::new(self.next_widget_id, kind, command, label, args_json);
        self.next_widget_id = self.next_widget_id.saturating_add(1);
        self.widgets.push(widget);
    }

    pub(super) fn command_catalog(device: &DeviceInfo) -> Vec<String> {
        let mut commands = device
            .metadata
            .as_ref()
            .map(|m| m.available_commands.clone())
            .unwrap_or_default();
        commands.sort();
        commands.dedup();
        commands
    }

    pub(super) fn manifest_summary_params(device: &DeviceInfo) -> Vec<String> {
        let Some(ui_json) = device
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.ui_schema_json.as_ref())
        else {
            return Vec::new();
        };

        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(ui_json) else {
            return Vec::new();
        };

        let mut summary_params: Vec<String> = parsed
            .get("status_display")
            .and_then(|status| status.get("summary_params"))
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        summary_params.sort();
        summary_params.dedup();
        summary_params
    }

    pub(super) fn infer_status_command(param: &str, commands: &[String]) -> Option<String> {
        let normalized = param.trim().to_lowercase();
        if normalized.is_empty() {
            return None;
        }

        let mut candidates = vec![
            normalized.clone(),
            format!("get_{normalized}"),
            format!("read_{normalized}"),
            format!("query_{normalized}"),
        ];

        for suffix in ["_nm", "_ms", "_degrees", "_deg", "_value"] {
            if let Some(stripped) = normalized.strip_suffix(suffix) {
                candidates.push(stripped.to_string());
                candidates.push(format!("get_{stripped}"));
                candidates.push(format!("read_{stripped}"));
                candidates.push(format!("query_{stripped}"));
            }
        }

        for candidate in &candidates {
            if let Some(found) = commands.iter().find(|cmd| cmd.to_lowercase() == *candidate) {
                return Some(found.clone());
            }
        }

        commands
            .iter()
            .find(|cmd| {
                let lower = cmd.to_lowercase();
                (lower.starts_with("get_")
                    || lower.starts_with("read_")
                    || lower.starts_with("query_"))
                    && lower.contains(&normalized)
            })
            .cloned()
    }

    pub(super) fn format_results(payload: &str) -> String {
        let trimmed = payload.trim();
        if trimmed.is_empty() {
            return "ok".to_string();
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string()),
            Err(_) => trimmed.to_string(),
        }
    }

    pub(super) fn summarize(value: &str, max_chars: usize) -> String {
        let total = value.chars().count();
        if total <= max_chars {
            return value.to_string();
        }
        let mut out: String = value.chars().take(max_chars).collect();
        out.push('…');
        out
    }

    pub(super) fn poll_results(&mut self) {
        while let Ok(CommandWidgetResult::Completed { widget_id, result }) =
            self.action_rx.try_recv()
        {
            let Some(widget) = self.widgets.iter_mut().find(|w| w.id == widget_id) else {
                continue;
            };
            widget.pending = false;
            match result {
                Ok(value) => {
                    widget.last_result = Some(value);
                    widget.last_error = None;
                }
                Err(err) => {
                    widget.last_error = Some(err);
                }
            }
        }
    }

    pub(super) fn execute_widget_command(
        &mut self,
        widget_id: usize,
        device_id: &str,
        command: &str,
        args_json: &str,
        client: Option<&mut DaqClient>,
        runtime: &crate::runtime::Runtime,
    ) {
        let Some(widget) = self.widgets.iter_mut().find(|w| w.id == widget_id) else {
            return;
        };

        if let Err(e) = serde_json::from_str::<serde_json::Value>(args_json) {
            widget.pending = false;
            widget.last_error = Some(format!("args must be valid JSON: {}", e));
            return;
        }

        let Some(client) = client else {
            widget.pending = false;
            widget.last_error = Some("Not connected to daemon".to_string());
            return;
        };

        widget.pending = true;
        widget.last_refresh = Some(Instant::now());
        widget.last_error = None;

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        let device_id = device_id.to_string();
        let command = command.to_string();
        let args_json = args_json.to_string();

        runtime.spawn(async move {
            let result = match client
                .execute_device_command(&device_id, &command, &args_json)
                .await
            {
                Ok(resp) => {
                    if resp.success {
                        Ok(Self::format_results(&resp.results))
                    } else if !resp.error_message.is_empty() {
                        Err(resp.error_message)
                    } else {
                        Err("command failed".to_string())
                    }
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx
                .send(CommandWidgetResult::Completed { widget_id, result })
                .await;
        });
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &crate::runtime::Runtime,
    ) {
        self.poll_results();

        let commands = Self::command_catalog(device);
        let supports_commandable = device.has_capability("commandable");
        let show_widget_panel = supports_commandable || !self.widgets.is_empty();
        if !show_widget_panel {
            return;
        }

        ui.add_space(8.0);
        ui.separator();

        ui.collapsing("Command Widgets", |ui| {
            ui.set_max_width(ui.available_width());

            if !supports_commandable {
                ui.weak("Command widgets require the device to advertise `commandable`.");
                ui.weak("Existing widgets are shown read-only in this mode.");
            } else if commands.is_empty() {
                ui.weak("No command catalog published for this device.");
            } else {
                if self.selected_command_idx >= commands.len() {
                    self.selected_command_idx = 0;
                }

                ui.horizontal_wrapped(|ui| {
                    ui.label("Command:");
                    egui::ComboBox::from_id_salt(("cmd_widget_add_command", device.id.as_str()))
                        .selected_text(commands[self.selected_command_idx].as_str())
                        .show_ui(ui, |ui| {
                            for (idx, cmd) in commands.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_command_idx, idx, cmd);
                            }
                        });

                    ui.label("Type:");
                    egui::ComboBox::from_id_salt(("cmd_widget_add_kind", device.id.as_str()))
                        .selected_text(self.add_kind.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.add_kind,
                                CommandWidgetKind::Action,
                                CommandWidgetKind::Action.label(),
                            );
                            ui.selectable_value(
                                &mut self.add_kind,
                                CommandWidgetKind::Status,
                                CommandWidgetKind::Status.label(),
                            );
                        });
                });

                ui.horizontal_wrapped(|ui| {
                    ui.label("Label:");
                    ui.add(egui::TextEdit::singleline(&mut self.add_label).desired_width(150.0));
                    ui.label("Args:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_args_json)
                            .desired_width(210.0)
                            .hint_text("{\"key\": 1}"),
                    );
                    if ui.button("Add Widget").clicked() {
                        let command = commands[self.selected_command_idx].clone();
                        let label = if self.add_label.trim().is_empty() {
                            match self.add_kind {
                                CommandWidgetKind::Action => format!("Run {}", command),
                                CommandWidgetKind::Status => format!("Status {}", command),
                            }
                        } else {
                            self.add_label.trim().to_string()
                        };
                        let args_json = if self.add_args_json.trim().is_empty() {
                            "{}".to_string()
                        } else {
                            self.add_args_json.clone()
                        };
                        self.add_widget(self.add_kind, command, label, args_json);
                    }
                });

                let summary_params = Self::manifest_summary_params(device);
                if !summary_params.is_empty() {
                    ui.separator();
                    ui.label("Quick Add From Manifest");
                    ui.horizontal_wrapped(|ui| {
                        for param in &summary_params {
                            if let Some(command) = Self::infer_status_command(param, &commands) {
                                let label = format!("Status {param}");
                                if ui.button(label.as_str()).clicked() {
                                    self.add_widget(
                                        CommandWidgetKind::Status,
                                        command.clone(),
                                        label,
                                        "{}".to_string(),
                                    );
                                }
                            }
                        }
                    });
                }
            }

            if self.widgets.is_empty() {
                ui.weak("No custom command widgets added.");
                return;
            }

            let mut run_requests: Vec<(usize, String, String)> = Vec::new();
            let mut remove_ids = Vec::new();

            for widget in &mut self.widgets {
                ui.group(|ui| {
                    ui.set_max_width(ui.available_width());

                    ui.horizontal_wrapped(|ui| {
                        ui.label(match widget.kind {
                            CommandWidgetKind::Action => "Action",
                            CommandWidgetKind::Status => "Status",
                        });
                        ui.separator();
                        ui.label(widget.command.as_str());
                        ui.separator();
                        ui.add(
                            egui::TextEdit::singleline(&mut widget.label)
                                .desired_width(180.0)
                                .hint_text("Widget label"),
                        );

                        let run_text = match widget.kind {
                            CommandWidgetKind::Action => "Run",
                            CommandWidgetKind::Status => "Refresh",
                        };
                        if ui
                            .add_enabled(
                                supports_commandable && !widget.pending,
                                egui::Button::new(run_text),
                            )
                            .clicked()
                        {
                            run_requests.push((
                                widget.id,
                                widget.command.clone(),
                                widget.args_json.clone(),
                            ));
                        }

                        if widget.kind == CommandWidgetKind::Status {
                            ui.checkbox(&mut widget.auto_refresh, "Auto");
                            ui.label("Every");
                            ui.add(
                                egui::DragValue::new(&mut widget.refresh_interval_ms)
                                    .range(200..=60_000)
                                    .speed(10),
                            );
                            ui.label("ms");
                        }

                        if ui.button("Remove").clicked() {
                            remove_ids.push(widget.id);
                        }
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label("Args:");
                        ui.add(
                            egui::TextEdit::singleline(&mut widget.args_json)
                                .desired_width(ui.available_width().min(420.0)),
                        );
                    });

                    if widget.pending {
                        ui.spinner();
                    } else if !supports_commandable {
                        ui.weak("Execution disabled: device is not commandable.");
                    }

                    if let Some(err) = &widget.last_error {
                        ui.colored_label(egui::Color32::RED, err);
                    } else if let Some(value) = &widget.last_result {
                        let short = Self::summarize(value, 180);
                        ui.label(egui::RichText::new(short.clone()).monospace())
                            .on_hover_text(value);
                    } else {
                        ui.weak("No result yet.");
                    }
                });

                let should_auto_refresh = widget.kind == CommandWidgetKind::Status
                    && widget.auto_refresh
                    && supports_commandable
                    && !widget.pending
                    && widget
                        .last_refresh
                        .map(|t| t.elapsed() >= Duration::from_millis(widget.refresh_interval_ms))
                        .unwrap_or(true);

                if should_auto_refresh {
                    run_requests.push((
                        widget.id,
                        widget.command.clone(),
                        widget.args_json.clone(),
                    ));
                }
            }

            if !remove_ids.is_empty() {
                self.widgets.retain(|w| !remove_ids.contains(&w.id));
            }

            for (widget_id, command, args_json) in run_requests {
                self.execute_widget_command(
                    widget_id,
                    &device.id,
                    &command,
                    &args_json,
                    client.as_deref_mut(),
                    runtime,
                );
            }

            if self.widgets.iter().any(|w| w.pending || w.auto_refresh) {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        });
    }
}

/// Storage key for persisting the WASM server URL across reloads.
#[cfg(target_arch = "wasm32")]
pub(super) const WASM_SERVER_URL_KEY: &str = "wasm_server_url";

/// Default daemon URL for WASM builds (fallback when no URL param or origin detected).
#[cfg(target_arch = "wasm32")]
pub(super) const WASM_DEFAULT_SERVER_URL: &str = "http://localhost:8080";

/// Detect the daemon URL from the browser environment (bd-5k2m).
///
/// Priority: `?daemon=` URL param > page origin > hardcoded default.
/// When served from the daemon via `--web-ui-path`, the page origin IS the
/// daemon address, so auto-detection eliminates the need for manual input.
#[cfg(target_arch = "wasm32")]
pub(super) fn detect_daemon_url() -> String {
    // 1. Check ?daemon= URL parameter
    if let Some(window) = web_sys::window() {
        if let Ok(search) = window.location().search()
            && !search.is_empty()
            && let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search)
            && let Some(daemon) = params.get("daemon")
            && !daemon.is_empty()
        {
            tracing::info!("Using daemon URL from ?daemon= parameter: {}", daemon);
            return daemon;
        }

        // 2. Use page origin (works when served via --web-ui-path on daemon port)
        if let Ok(origin) = window.location().origin() {
            // Only use origin if it's not a file:// URL and not the default dev server
            if !origin.is_empty()
                && !origin.starts_with("file:")
                && origin != "null"
                && !origin.contains("localhost:8080")
            {
                tracing::info!("Using daemon URL from page origin: {}", origin);
                return origin;
            }
        }
    }

    // 3. Fallback to hardcoded default
    WASM_DEFAULT_SERVER_URL.to_string()
}

/// WASM-only connection state for browser-based GUI.
/// On native, ConnectionManager handles reconnection with exponential backoff.
/// On WASM, we use a simpler connect-once model via DaqClient::connect_web().
#[cfg(target_arch = "wasm32")]
pub(super) struct WasmConnectionState {
    /// URL input field (e.g. "http://10.0.0.40:8080")
    pub(super) url_input: String,
    /// Connection status message
    pub(super) status: String,
    /// Whether a connect attempt is in progress
    pub(super) connecting: bool,
    /// Pending connect result receiver
    pub(super) connect_rx: Option<mpsc::Receiver<Result<DaqClient, String>>>,
}

#[cfg(target_arch = "wasm32")]
impl Default for WasmConnectionState {
    fn default() -> Self {
        Self {
            url_input: detect_daemon_url(),
            status: "Disconnected".to_string(),
            connecting: false,
            connect_rx: None,
        }
    }
}

/// Action to perform on the UI state
pub(super) enum UiAction {
    FocusTab(Panel),
    /// Open a device control panel as a docked tab
    OpenDeviceControl {
        /// Full device info with capability flags
        device_info: Box<DeviceInfo>,
        /// Optional explicit dock destination resolved by the dock crate.
        dock_target: Option<TabDestination>,
    },
    /// Close a device control panel by ID
    CloseDevicePanel {
        id: usize,
    },
}

/// Layout mode for docked control panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ControlPanelLayoutMode {
    /// Compact capability-driven controls.
    #[default]
    Simple,
    /// Rich device-specific controls (matches Instruments panel behavior).
    Advanced,
}

impl ControlPanelLayoutMode {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Simple => "Simple",
            Self::Advanced => "Advanced",
        }
    }
}

/// Device availability state after reconciliation with daemon
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceAvailability {
    #[default]
    Pending, // Not yet verified against daemon
    Available, // Confirmed present on daemon
    Missing,   // Not found on daemon
}

/// Panel kind classification (for detecting capability changes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePanelKind {
    MaiTai,
    PowerMeter,
    Rotator,
    Stage,
    AnalogOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DockedAdvancedPanelKind {
    Generic,
    MaiTai,
    Comedi,
    PowerMeter,
    Rotator,
    Stage,
}

pub(super) fn docked_advanced_panel_kind_for_device(
    device: &DeviceInfo,
) -> DockedAdvancedPanelKind {
    use crate::panels::instrument_manager::dispatch::{self, PanelType};
    match dispatch::determine_panel_type_with_config(device, None) {
        PanelType::MaiTai => DockedAdvancedPanelKind::MaiTai,
        PanelType::Comedi => DockedAdvancedPanelKind::Comedi,
        PanelType::PowerMeter => DockedAdvancedPanelKind::PowerMeter,
        PanelType::Rotator => DockedAdvancedPanelKind::Rotator,
        PanelType::Stage | PanelType::DoverStage => DockedAdvancedPanelKind::Stage,
        _ => DockedAdvancedPanelKind::Generic,
    }
}

/// Determine panel kind from device capabilities
pub(super) fn panel_kind_for_device(device: &DeviceInfo) -> DevicePanelKind {
    use crate::panels::instrument_manager::dispatch::{self, PanelType};
    match dispatch::determine_panel_type_with_config(device, None) {
        PanelType::MaiTai => DevicePanelKind::MaiTai,
        PanelType::PowerMeter => DevicePanelKind::PowerMeter,
        PanelType::Rotator => DevicePanelKind::Rotator,
        PanelType::Stage | PanelType::DoverStage => DevicePanelKind::Stage,
        PanelType::Comedi => {
            // Only true analog-output Comedi devices map to AnalogOutput for migration tracking.
            // Analog input, DIO, and counter Comedi devices fall back to the Stage default kind.
            if device.driver_type.to_lowercase().contains("analog_output") {
                DevicePanelKind::AnalogOutput
            } else {
                DevicePanelKind::Stage
            }
        }
        _ => DevicePanelKind::Stage, // Camera, AndorCamera, Spectrograph, Generic, ConfigDriven
    }
}

/// Info about a docked device control panel (runtime state)
#[derive(Debug, Clone)]
pub(crate) struct DevicePanelInfo {
    /// Full device info with capability flags (avoids inferring capabilities from driver_type)
    pub(crate) device_info: DeviceInfo,
    /// Availability after reconciliation with daemon
    pub(crate) availability: DeviceAvailability,
    /// Panel kind (for detecting capability changes)
    pub(crate) kind: DevicePanelKind,
}

/// Serializable version of device panel info for layout persistence.
/// Contains only the fields needed to restore the panel on app restart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PersistedPanelInfo {
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) driver_type: String,
    #[serde(default)]
    pub(super) capabilities: Vec<String>,
    // LEGACY: Deprecated boolean capability fields, kept for deserialization of
    // old persisted panel state. Remove after v1.0. See
    // docs/reference/deprecation-plan.md Section 1.1.
    #[serde(default)]
    pub(super) is_emission_controllable: bool,
    #[serde(default)]
    pub(super) is_shutter_controllable: bool,
    #[serde(default)]
    pub(super) is_wavelength_tunable: bool,
    #[serde(default)]
    pub(super) is_readable: bool,
    #[serde(default)]
    pub(super) is_movable: bool,
}

impl From<&DeviceInfo> for PersistedPanelInfo {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            device_id: info.id.clone(),
            device_name: info.name.clone(),
            driver_type: info.driver_type.clone(),
            capabilities: info.capabilities.clone(),
            // LEGACY: No longer populated; kept for struct completeness. Remove with booleans.
            is_emission_controllable: false,
            is_shutter_controllable: false,
            is_wavelength_tunable: false,
            is_readable: false,
            is_movable: false,
        }
    }
}

impl From<PersistedPanelInfo> for DeviceInfo {
    fn from(info: PersistedPanelInfo) -> Self {
        // LEGACY: Migrate from legacy booleans if capabilities is empty (old persisted format).
        // Remove after v1.0 when all stored state uses `capabilities` strings.
        let capabilities = if info.capabilities.is_empty() {
            let mut caps = Vec::new();
            if info.is_movable {
                caps.push("movable".to_string());
            }
            if info.is_readable {
                caps.push("readable".to_string());
            }
            if info.is_shutter_controllable {
                caps.push("shutter_controllable".to_string());
            }
            if info.is_wavelength_tunable {
                caps.push("wavelength_tunable".to_string());
            }
            if info.is_emission_controllable {
                caps.push("emission_controllable".to_string());
            }
            caps
        } else {
            info.capabilities
        };
        #[allow(deprecated)]
        Self {
            id: info.device_id,
            name: info.device_name,
            driver_type: info.driver_type,
            capabilities,
            ..Default::default()
        }
    }
}

/// Available panels in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Panel {
    Nav,
    GettingStarted,
    Instruments,
    Scripts,
    ScanBuilder,
    ExperimentDesigner,
    Storage,
    RunHistory,
    Modules,
    PlanRunner,
    DocumentViewer,
    SignalPlotter,
    ImageViewer,
    Logs,
    Repl,
    /// Dockable device control panel (uses id to lookup device_id in app state)
    DeviceControl {
        id: usize,
    },
}
