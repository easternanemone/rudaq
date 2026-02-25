//! Main application state and UI logic.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use tokio::sync::mpsc;

use crate::connection::{
    clear_legacy_daemon_address, migrate_legacy_daemon_address, resolve_address, AddressSource,
    DaemonAddress,
};
use crate::connection_state_ext::ConnectionStateExt;
use crate::daemon_launcher::{AutoConnectState, DaemonLauncher, DaemonMode};
use crate::device_ext::DeviceInfoExt;
use crate::icons;
use crate::layout;
use crate::panels::{
    instrument_manager::{config_loader::DeviceConfigCache, config_renderer::ConfigDrivenPanel},
    ComediPanel, ConnectionDiagnostics, ConnectionStatus as LogConnectionStatus, DevicesPanel,
    DocumentViewerPanel, ExperimentDesignerPanel, GettingStartedPanel, ImageViewerPanel,
    InstrumentManagerPanel, LoggingPanel, ModulesPanel, PlanRunnerPanel, RunComparisonPanel,
    RunHistoryPanel, ScanBuilderPanel, ScansPanel, ScriptsPanel, SignalPlotterPanel, StoragePanel,
};
use crate::shortcuts::{CheatSheetPanel, ShortcutAction, ShortcutContext, ShortcutManager};
use crate::theme::{self, ThemePreference};
use crate::widgets::{
    DeviceControlWidget, GenericDevicePanel, MaiTaiControlPanel, PowerMeterControlPanel,
    RotatorControlPanel, StageControlPanel, StatusBar,
};
use client::reconnect::{friendly_error_message, ConnectionManager, ConnectionState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

/// Layout version constant. Increment this when the default dock layout changes
/// to force users with stale saved layouts to get the new default.
/// v1: Initial version (had Devices panel as default in some builds)
/// v2: Instruments panel as default (bd-kj7i fix)
/// v3: Add ImageViewer tab alongside Instruments in default layout
const LAYOUT_VERSION: u32 = 3;

/// Storage key for layout version
const LAYOUT_VERSION_KEY: &str = "layout_version";

/// Directory for session state files (bd-izdj.30)
fn session_dir() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rust-daq")
}

/// Per-process session file path to avoid cross-process races (bd-izdj.30)
fn session_file_path() -> std::path::PathBuf {
    session_dir().join(format!("gui_session_{}.json", std::process::id()))
}

/// Check if a PID is still alive
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks process existence without sending a signal
        // SAFETY: kill(pid, 0) is a standard POSIX existence check with no side effects
        #[allow(unsafe_code)]
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) == 0 };
        alive
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        // Conservative: assume alive on non-Unix platforms
        true
    }
}

/// Write session state to disk atomically (marks GUI as running)
fn write_session_file(daemon_url: &str) {
    let path = session_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let session = serde_json::json!({
        "running": true,
        "daemon_url": daemon_url,
        "pid": std::process::id(),
        "started_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });
    // Atomic write: temp file + rename to prevent partial/corrupt reads
    let tmp_path = path.with_extension("json.tmp");
    if std::fs::write(&tmp_path, session.to_string()).is_err() {
        tracing::warn!("Failed to write session temp file: {}", tmp_path.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        tracing::warn!("Failed to rename session file: {}", e);
    }
}

/// Check if a previous session crashed by scanning for session files
/// with running=true whose PID is no longer alive.
fn check_crashed_session() -> Option<String> {
    let dir = session_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Cannot read session dir: {}", e);
            return None;
        }
    };

    let my_pid = std::process::id();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("gui_session_")
            || !std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to read session file {}: {}", path.display(), e);
                continue;
            }
        };
        let session: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse session file {}: {}", path.display(), e);
                continue;
            }
        };

        let was_running = session
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !was_running {
            continue;
        }

        let pid = session.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        // Skip our own session file
        if pid == my_pid {
            continue;
        }

        // If the PID is no longer alive, this was a crashed session
        if !is_pid_alive(pid) {
            // Clean up the stale session file
            let _ = std::fs::remove_file(&path);
            return session
                .get("daemon_url")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string());
        }
    }

    None
}

/// Remove only our own session file (marks clean shutdown)
fn clear_session_file() {
    let path = session_file_path();
    if let Err(e) = std::fs::remove_file(&path) {
        // ENOENT is fine — file may not exist if write_session_file was never called
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Failed to remove session file: {}", e);
        }
    }
}

/// Result of a health check sent through the channel (bd-j3xz.3.3: includes RTT).
enum HealthCheckResult {
    /// Health check succeeded with round-trip time in milliseconds.
    Success { rtt_ms: f64 },
    /// Health check failed with error message.
    Failed(String),
}

/// Device reconciliation result (validates persisted panels against daemon)
enum DeviceReconcileMsg {
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
enum CommandWidgetKind {
    Action,
    Status,
}

impl CommandWidgetKind {
    fn label(self) -> &'static str {
        match self {
            Self::Action => "Action",
            Self::Status => "Status",
        }
    }
}

enum CommandWidgetResult {
    Completed {
        widget_id: usize,
        result: Result<String, String>,
    },
}

struct CommandWidget {
    id: usize,
    kind: CommandWidgetKind,
    command: String,
    label: String,
    args_json: String,
    auto_refresh: bool,
    refresh_interval_ms: u64,
    last_refresh: Option<Instant>,
    pending: bool,
    last_result: Option<String>,
    last_error: Option<String>,
}

impl CommandWidget {
    fn new(
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

struct CommandWidgetPalette {
    widgets: Vec<CommandWidget>,
    next_widget_id: usize,
    selected_command_idx: usize,
    add_kind: CommandWidgetKind,
    add_label: String,
    add_args_json: String,
    action_tx: mpsc::Sender<CommandWidgetResult>,
    action_rx: mpsc::Receiver<CommandWidgetResult>,
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
    fn add_widget(
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

    fn command_catalog(device: &DeviceInfo) -> Vec<String> {
        let mut commands = device
            .metadata
            .as_ref()
            .map(|m| m.available_commands.clone())
            .unwrap_or_default();
        commands.sort();
        commands.dedup();
        commands
    }

    fn manifest_summary_params(device: &DeviceInfo) -> Vec<String> {
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

    fn infer_status_command(param: &str, commands: &[String]) -> Option<String> {
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

    fn format_results(payload: &str) -> String {
        let trimmed = payload.trim();
        if trimmed.is_empty() {
            return "ok".to_string();
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string()),
            Err(_) => trimmed.to_string(),
        }
    }

    fn summarize(value: &str, max_chars: usize) -> String {
        let total = value.chars().count();
        if total <= max_chars {
            return value.to_string();
        }
        let mut out: String = value.chars().take(max_chars).collect();
        out.push('…');
        out
    }

    fn poll_results(&mut self) {
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

    fn execute_widget_command(
        &mut self,
        widget_id: usize,
        device_id: &str,
        command: &str,
        args_json: &str,
        client: Option<&mut DaqClient>,
        runtime: &tokio::runtime::Runtime,
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

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        device: &DeviceInfo,
        mut client: Option<&mut DaqClient>,
        runtime: &tokio::runtime::Runtime,
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

/// Main application state
pub struct DaqApp {
    /// gRPC client (wrapped in Option for lazy initialization)
    client: Option<DaqClient>,

    /// Connection manager (handles state machine and auto-reconnect)
    connection: ConnectionManager,

    /// Validated daemon address (normalized, with source tracking)
    daemon_address: DaemonAddress,

    /// Text input field for address (may be invalid during editing)
    address_input: String,

    /// Address validation error (shown in UI)
    address_error: Option<String>,

    /// Daemon version (retrieved via GetDaemonInfo)
    daemon_version: Option<String>,

    /// GUI version (from CARGO_PKG_VERSION)
    gui_version: String,

    /// Dock state for panel management
    dock_state: Option<DockState<Panel>>,

    /// Queue for deferred UI actions (e.g. opening tabs from Nav panel)
    ui_actions: Vec<UiAction>,

    /// Panel states
    getting_started_panel: GettingStartedPanel,
    devices_panel: DevicesPanel,
    scripts_panel: ScriptsPanel,
    scans_panel: ScansPanel,
    storage_panel: StoragePanel,
    run_history_panel: RunHistoryPanel,
    run_comparison_panel: RunComparisonPanel,
    modules_panel: ModulesPanel,
    plan_runner_panel: PlanRunnerPanel,
    scan_builder_panel: ScanBuilderPanel,
    experiment_designer_panel: ExperimentDesignerPanel,
    document_viewer_panel: DocumentViewerPanel,
    instrument_manager_panel: InstrumentManagerPanel,
    signal_plotter_panel: SignalPlotterPanel,
    image_viewer_panel: ImageViewerPanel,
    logging_panel: LoggingPanel,

    /// Tokio runtime for async operations
    runtime: tokio::runtime::Runtime,

    /// Channel for health check results
    health_tx: mpsc::Sender<HealthCheckResult>,
    health_rx: mpsc::Receiver<HealthCheckResult>,

    /// Device reconciliation epoch (incremented on each reconcile request)
    device_reconcile_epoch: u64,

    /// Channel for device reconciliation results
    device_reconcile_tx: mpsc::Sender<DeviceReconcileMsg>,
    device_reconcile_rx: mpsc::Receiver<DeviceReconcileMsg>,

    /// Previous connection state (for detecting transitions)
    was_connected: bool,

    /// Daemon mode configuration (local auto-start, remote, or lab hardware)
    daemon_mode: DaemonMode,

    /// Daemon process launcher (for auto-start local modes)
    daemon_launcher: Option<DaemonLauncher>,

    /// Auto-connect lifecycle state
    auto_connect_state: AutoConnectState,

    /// Receiver for tracing log events (forwarded to logging panel)
    log_receiver: tokio::sync::mpsc::Receiver<crate::gui_log_layer::GuiLogEvent>,

    /// Theme preference (light/dark/system)
    theme_preference: ThemePreference,

    /// Status bar widget for connection indicator and version display
    status_bar: StatusBar,

    /// Device control panel ID to device info mapping (for dockable device panels)
    device_panel_info: HashMap<usize, DevicePanelInfo>,

    /// Next available device panel ID
    next_device_panel_id: usize,

    /// Docked device control panels using GenericDevicePanel (keyed by panel ID)
    docked_panels: HashMap<usize, GenericDevicePanel>,
    /// Docked MaiTai panels (advanced layout mode)
    docked_maitai_panels: HashMap<usize, MaiTaiControlPanel>,
    /// Docked power meter panels (advanced layout mode)
    docked_power_meter_panels: HashMap<usize, PowerMeterControlPanel>,
    /// Docked rotator panels (advanced layout mode)
    docked_rotator_panels: HashMap<usize, RotatorControlPanel>,
    /// Docked stage panels (advanced layout mode)
    docked_stage_panels: HashMap<usize, StageControlPanel>,
    /// Docked Comedi panels (advanced layout mode)
    docked_comedi_panels: HashMap<usize, ComediPanel>,
    /// Docked config-driven panels (from TOML `[ui.control_panel]`)
    docked_config_driven_panels: HashMap<usize, ConfigDrivenPanel>,
    /// Device config cache for TOML-driven panel dispatch
    config_cache: DeviceConfigCache,
    /// gRPC UI config cache for docked panels (keyed by panel ID)
    grpc_ui_config_cache: HashMap<usize, Option<hardware::config::schema::ControlPanelConfig>>,
    /// User-added command widgets for advanced control panels (keyed by panel ID)
    docked_command_widgets: HashMap<usize, CommandWidgetPalette>,
    /// Preferred control-panel layout mode for docked pop-outs
    control_panel_layout_mode: ControlPanelLayoutMode,

    /// Settings window state
    settings_window: crate::settings::SettingsWindow,

    /// Application settings
    app_settings: crate::settings::AppSettings,

    /// Connection presets loaded from gui.toml
    gui_presets: Vec<crate::gui_config::DaemonPreset>,

    /// PVCAM live view streaming state (requires rerun_viewer + instrument_photometrics)
    /// Works in mock mode without pvcam_hardware, or with real SDK when pvcam_hardware enabled
    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    pvcam_streaming: bool,
    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    pvcam_task: Option<tokio::task::JoinHandle<()>>,

    /// Keyboard shortcuts manager
    shortcut_manager: ShortcutManager,

    /// Cheat sheet panel (shown with Shift+?)
    cheat_sheet_panel: CheatSheetPanel,

    /// Cheat sheet visibility state
    show_cheat_sheet: bool,

    /// Whether this session recovered from a crash (bd-izdj.30)
    recovered_from_crash: bool,
}

/// Action to perform on the UI state
enum UiAction {
    FocusTab(Panel),
    /// Open a device control panel as a docked tab
    OpenDeviceControl {
        /// Full device info with capability flags
        device_info: Box<DeviceInfo>,
    },
    /// Close a device control panel by ID
    CloseDevicePanel {
        id: usize,
    },
}

/// Layout mode for docked pop-out control panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ControlPanelLayoutMode {
    /// Compact capability-driven controls.
    #[default]
    Simple,
    /// Rich device-specific controls (matches Instruments panel behavior).
    Advanced,
}

impl ControlPanelLayoutMode {
    fn label(self) -> &'static str {
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
enum DockedAdvancedPanelKind {
    Generic,
    MaiTai,
    Comedi,
    PowerMeter,
    Rotator,
    Stage,
}

fn docked_advanced_panel_kind_for_device(device: &DeviceInfo) -> DockedAdvancedPanelKind {
    let driver_lower = device.driver_type.to_lowercase();

    if driver_lower.contains("maitai")
        || driver_lower.contains("mai_tai")
        || (device.is_wavelength_tunable() && device.is_emission_controllable())
    {
        DockedAdvancedPanelKind::MaiTai
    } else if driver_lower.contains("comedi")
        || driver_lower.contains("ni_daq")
        || driver_lower.contains("nidaq")
        || driver_lower.contains("pci-mio")
        || driver_lower.contains("pcimio")
    {
        DockedAdvancedPanelKind::Comedi
    } else if driver_lower.contains("1830")
        || driver_lower.contains("power_meter")
        || (device.is_readable() && !device.is_movable() && !device.is_frame_producer())
    {
        DockedAdvancedPanelKind::PowerMeter
    } else if driver_lower.contains("ell14") || driver_lower.contains("rotator") {
        DockedAdvancedPanelKind::Rotator
    } else if device.is_movable() {
        DockedAdvancedPanelKind::Stage
    } else {
        DockedAdvancedPanelKind::Generic
    }
}

/// Determine panel kind from device capabilities
fn panel_kind_for_device(device: &DeviceInfo) -> DevicePanelKind {
    let driver_lower = device.driver_type.to_lowercase();

    if device.is_emission_controllable() || device.is_shutter_controllable() {
        DevicePanelKind::MaiTai
    } else if driver_lower.contains("comedi_analog_output")
        || driver_lower.contains("analog_output")
    {
        DevicePanelKind::AnalogOutput
    } else if device.is_readable() && !device.is_movable() {
        DevicePanelKind::PowerMeter
    } else if device.is_movable() {
        if driver_lower.contains("ell14") || driver_lower.contains("rotator") {
            DevicePanelKind::Rotator
        } else {
            DevicePanelKind::Stage
        }
    } else {
        DevicePanelKind::Stage // fallback
    }
}

/// Info about a docked device control panel (runtime state)
#[derive(Debug, Clone)]
pub(crate) struct DevicePanelInfo {
    /// Full device info with capability flags (avoids inferring capabilities from driver_type)
    device_info: DeviceInfo,
    /// Availability after reconciliation with daemon
    availability: DeviceAvailability,
    /// Panel kind (for detecting capability changes)
    kind: DevicePanelKind,
}

/// Serializable version of device panel info for layout persistence.
/// Contains only the fields needed to restore the panel on app restart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedPanelInfo {
    device_id: String,
    device_name: String,
    driver_type: String,
    #[serde(default)]
    capabilities: Vec<String>,
    // Legacy fields for backward compatibility during deserialization
    #[serde(default)]
    is_emission_controllable: bool,
    #[serde(default)]
    is_shutter_controllable: bool,
    #[serde(default)]
    is_wavelength_tunable: bool,
    #[serde(default)]
    is_readable: bool,
    #[serde(default)]
    is_movable: bool,
}

impl From<&DeviceInfo> for PersistedPanelInfo {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            device_id: info.id.clone(),
            device_name: info.name.clone(),
            driver_type: info.driver_type.clone(),
            capabilities: info.capabilities.clone(),
            // Legacy fields no longer populated
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
        // Migrate from legacy booleans if capabilities is empty (old format)
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
            category: 0,
            is_movable: false,
            is_readable: false,
            is_triggerable: false,
            is_frame_producer: false,
            is_exposure_controllable: false,
            is_shutter_controllable: false,
            is_wavelength_tunable: false,
            is_emission_controllable: false,
            is_parameterized: false,
            capabilities,
            metadata: None,
        }
    }
}

/// Available panels in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Panel {
    Nav,
    GettingStarted,
    Instruments,
    Devices,
    Scripts,
    Scans,
    ScanBuilder,
    ExperimentDesigner,
    Storage,
    RunHistory,
    RunComparison,
    Modules,
    PlanRunner,
    DocumentViewer,
    SignalPlotter,
    ImageViewer,
    Logs,
    /// Dockable device control panel (uses id to lookup device_id in app state)
    DeviceControl {
        id: usize,
    },
}

impl DaqApp {
    /// Create a new application instance with the specified daemon mode
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        daemon_mode: DaemonMode,
        log_receiver: tokio::sync::mpsc::Receiver<crate::gui_log_layer::GuiLogEvent>,
    ) -> Self {
        // Load phosphor icons into egui fonts
        let mut fonts = egui::FontDefinitions::default();
        icons::add_to_fonts(&mut fonts);
        cc.egui_ctx.set_fonts(fonts);

        // Load or default theme preference
        let theme_preference: ThemePreference = cc
            .storage
            .and_then(|s| eframe::get_value(s, "theme_preference"))
            .unwrap_or_default();
        theme::apply_theme(&cc.egui_ctx, theme_preference);

        // Load or initialize keyboard shortcuts
        let shortcut_manager: ShortcutManager = cc
            .storage
            .and_then(|s| eframe::get_value(s, "shortcut_manager"))
            .unwrap_or_default();

        // Configure egui style with consistent spacing
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = layout::ITEM_SPACING;
        cc.egui_ctx.set_style(style);

        // Create tokio runtime for gRPC calls
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");
        // Start daemon launcher if in an auto-start local mode
        let daemon_launcher = if daemon_mode.should_auto_start() {
            let port = daemon_mode.port().unwrap_or(50051);
            let mut launcher = DaemonLauncher::new(port);
            if let Err(e) = launcher.start_with_mode(&daemon_mode) {
                tracing::error!("Failed to start daemon: {}", e);
            }
            Some(launcher)
        } else {
            None
        };

        // Determine auto-connect state based on mode
        let auto_connect_state = if daemon_mode.should_auto_start() {
            AutoConnectState::WaitingForDaemon {
                since: Instant::now(),
            }
        } else {
            // For remote mode, we can try to connect immediately
            AutoConnectState::ReadyToConnect
        };

        // Load application settings from storage (before address resolution)
        let mut app_settings: crate::settings::AppSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "app_settings"))
            .unwrap_or_default();
        let control_panel_layout_mode: ControlPanelLayoutMode = cc
            .storage
            .and_then(|s| eframe::get_value(s, "control_panel_layout_mode"))
            .unwrap_or(ControlPanelLayoutMode::Simple);

        // Migrate legacy "daemon_address" key → AppSettings (one-time)
        if let Some(storage) = cc.storage {
            if let Some(legacy_addr) = migrate_legacy_daemon_address(storage) {
                if !legacy_addr.trim().is_empty()
                    && app_settings.connection.daemon_address
                        == crate::settings::ConnectionSettings::default().daemon_address
                {
                    tracing::info!(
                        "Migrating legacy daemon_address '{}' to AppSettings",
                        legacy_addr
                    );
                    app_settings.connection.daemon_address = legacy_addr;
                }
            }
        }

        // Load GUI config (presets + defaults) from gui.toml
        let gui_config = match crate::gui_config::load_gui_config() {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("GUI config error: {}", e);
                None
            }
        };
        let gui_presets = gui_config
            .as_ref()
            .map(|c| c.presets.clone())
            .unwrap_or_default();
        if let Some(ref config) = gui_config {
            crate::gui_config::apply_defaults(&mut app_settings, &config.defaults);
        }

        // Find the default preset URL (if any) for address resolution
        let default_preset_url = gui_presets
            .iter()
            .find(|p| p.default)
            .map(|p| p.grpc_url.as_str());

        // Use daemon mode URL as the address, or fall back to stored/env/default
        let persisted_addr = &app_settings.connection.daemon_address;
        let persisted = if persisted_addr.trim().is_empty()
            || *persisted_addr == crate::settings::ConnectionSettings::default().daemon_address
        {
            None
        } else {
            Some(persisted_addr.as_str())
        };
        let daemon_address = if matches!(daemon_mode, DaemonMode::Remote { .. }) {
            // For remote mode, use the provided URL directly.
            // Detect preset URLs so AddressSource is correct for priority resolution.
            let url = daemon_mode.daemon_url();
            let source = if gui_presets.iter().any(|p| p.grpc_url == url) {
                AddressSource::Preset
            } else {
                AddressSource::UserInput
            };
            DaemonAddress::parse(&url, source)
                .unwrap_or_else(|_| resolve_address(None, persisted, default_preset_url))
        } else {
            // For local modes, use the generated URL
            DaemonAddress::parse(&daemon_mode.daemon_url(), AddressSource::Default)
                .unwrap_or_else(|_| resolve_address(None, persisted, default_preset_url))
        };
        let address_input = daemon_address.original().to_string();

        // Create health check channel
        let (health_tx, health_rx) = mpsc::channel(4);

        // Create device reconciliation channel
        let (device_reconcile_tx, device_reconcile_rx) = mpsc::channel(4);

        // Load persisted device panel info
        // GenericDevicePanel instances are created lazily on first render
        let (device_panel_info, next_device_panel_id) = if let Some(storage) = cc.storage {
            let persisted: HashMap<usize, PersistedPanelInfo> =
                eframe::get_value(storage, "device_panel_info").unwrap_or_default();
            let next_id: usize = eframe::get_value(storage, "next_device_panel_id").unwrap_or(0);

            let mut device_info_map = HashMap::new();
            for (id, persisted_info) in persisted {
                let device_info: DeviceInfo = persisted_info.clone().into();
                let kind = panel_kind_for_device(&device_info);
                device_info_map.insert(
                    id,
                    DevicePanelInfo {
                        device_info,
                        availability: DeviceAvailability::Pending,
                        kind,
                    },
                );
            }

            (device_info_map, next_id)
        } else {
            (HashMap::new(), 0)
        };

        // Initialize dock state and filter out orphaned DeviceControl panels
        // Check layout version to detect stale saved layouts
        let mut dock_state = if let Some(storage) = cc.storage {
            let stored_version: Option<u32> = eframe::get_value(storage, LAYOUT_VERSION_KEY);
            match stored_version {
                Some(v) if v == LAYOUT_VERSION => {
                    // Version matches, use stored layout
                    eframe::get_value(storage, eframe::APP_KEY)
                        .unwrap_or_else(Self::default_dock_state)
                }
                Some(v) => {
                    // Version mismatch - reset to default
                    tracing::info!(
                        "Layout version changed ({} -> {}), resetting to default layout",
                        v,
                        LAYOUT_VERSION
                    );
                    Self::default_dock_state()
                }
                None => {
                    // No version stored (first run or pre-versioning) - reset to default
                    tracing::info!(
                        "No layout version found, resetting to default layout (v{})",
                        LAYOUT_VERSION
                    );
                    Self::default_dock_state()
                }
            }
        } else {
            Self::default_dock_state()
        };

        // Remove DeviceControl panels that have no matching device_panel_info
        // (can happen if storage is corrupted or panels were manually edited)
        let orphaned_ids: Vec<usize> = dock_state
            .iter_all_tabs()
            .filter_map(|(_, tab)| {
                if let Panel::DeviceControl { id } = tab {
                    if !device_panel_info.contains_key(id) {
                        Some(*id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        for id in orphaned_ids {
            dock_state.retain_tabs(
                |tab| !matches!(tab, Panel::DeviceControl { id: panel_id } if *panel_id == id),
            );
        }

        // Crash recovery detection (bd-izdj.30)
        let recovered_from_crash = if let Some(crashed_url) = check_crashed_session() {
            tracing::warn!(
                daemon_url = %crashed_url,
                "Previous GUI session crashed — recovering session state"
            );
            true
        } else {
            false
        };

        // Mark session as running for crash detection on next launch
        write_session_file(daemon_address.as_str());

        Self {
            client: None,
            connection: ConnectionManager::new(),
            daemon_address,
            address_input,
            address_error: None,
            daemon_version: None,
            gui_version: env!("CARGO_PKG_VERSION").to_string(),
            dock_state: Some(dock_state),
            ui_actions: Vec::new(),
            getting_started_panel: GettingStartedPanel::default(),
            devices_panel: DevicesPanel::default(),
            scripts_panel: ScriptsPanel::default(),
            scans_panel: ScansPanel::default(),
            storage_panel: StoragePanel::default(),
            run_history_panel: RunHistoryPanel::default(),
            run_comparison_panel: RunComparisonPanel::default(),
            modules_panel: ModulesPanel::default(),
            plan_runner_panel: PlanRunnerPanel::default(),
            scan_builder_panel: ScanBuilderPanel::default(),
            experiment_designer_panel: ExperimentDesignerPanel::default(),
            document_viewer_panel: DocumentViewerPanel::default(),
            instrument_manager_panel: InstrumentManagerPanel::default(),
            signal_plotter_panel: SignalPlotterPanel::new(),
            image_viewer_panel: ImageViewerPanel::new(),
            logging_panel: LoggingPanel::new(),
            runtime,
            health_tx,
            health_rx,
            device_reconcile_epoch: 0,
            device_reconcile_tx,
            device_reconcile_rx,
            was_connected: false,
            daemon_mode,
            daemon_launcher,
            auto_connect_state,
            log_receiver,
            theme_preference,
            status_bar: StatusBar::new(),
            device_panel_info,
            next_device_panel_id,
            docked_panels: HashMap::new(),
            docked_maitai_panels: HashMap::new(),
            docked_power_meter_panels: HashMap::new(),
            docked_rotator_panels: HashMap::new(),
            docked_stage_panels: HashMap::new(),
            docked_comedi_panels: HashMap::new(),
            docked_config_driven_panels: HashMap::new(),
            grpc_ui_config_cache: HashMap::new(),
            config_cache: {
                let mut cache = DeviceConfigCache::new();
                if let Err(e) = cache.load_all() {
                    tracing::warn!("Failed to load device configs at startup: {}", e);
                }
                cache
            },
            docked_command_widgets: HashMap::new(),
            control_panel_layout_mode,
            settings_window: crate::settings::SettingsWindow::default(),
            app_settings,
            gui_presets,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_streaming: false,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_task: None,
            shortcut_manager,
            cheat_sheet_panel: CheatSheetPanel::new(),
            show_cheat_sheet: false,
            recovered_from_crash,
        }
    }

    /// Create the default dock layout
    fn default_dock_state() -> DockState<Panel> {
        // Start with Instruments + ImageViewer as tabbed panels in the main content area
        let mut dock_state = DockState::new(vec![Panel::Instruments, Panel::ImageViewer]);
        let surface = dock_state.main_surface_mut();

        // Split left for Nav
        let [_nav, content] = surface.split_left(NodeIndex::root(), 0.15, vec![Panel::Nav]);

        // Split bottom of content for Logs
        let [_content, _logs] = surface.split_below(content, 0.75, vec![Panel::Logs]);

        dock_state
    }

    /// Attempt to connect to the daemon
    fn connect(&mut self) {
        if self.connection.is_busy() {
            return;
        }

        // Validate and normalize the address input
        match DaemonAddress::parse(&self.address_input, AddressSource::UserInput) {
            Ok(addr) => {
                self.daemon_address = addr;
                self.address_error = None;
            }
            Err(e) => {
                self.address_error = Some(e.to_string());
                self.logging_panel
                    .error("Connection", &format!("Invalid address: {}", e));
                return;
            }
        }

        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
        self.logging_panel.info(
            "Connection",
            &format!(
                "Connecting to {} ({})",
                self.daemon_address,
                self.daemon_address.source().label()
            ),
        );

        self.connection
            .connect(self.daemon_address.clone(), &self.runtime);
    }

    /// Disconnect from the daemon
    fn disconnect(&mut self) {
        self.client = None;
        self.daemon_version = None;
        self.connection.disconnect();
        self.logging_panel.connection_status = LogConnectionStatus::Disconnected;
        self.logging_panel
            .info("Connection", "Disconnected from daemon");
    }

    /// Switch to a different daemon mode
    fn switch_daemon_mode(&mut self, mode: DaemonMode) {
        tracing::info!("Switching daemon mode to: {}", mode.label());

        // Stop existing daemon before switching modes.
        if let Some(ref mut launcher) = self.daemon_launcher {
            launcher.stop();
        }
        self.daemon_launcher = None;

        // Disconnect current connection
        self.disconnect();

        // Update daemon mode
        self.daemon_mode = mode.clone();

        // Update address
        if let Ok(addr) = DaemonAddress::parse(&mode.daemon_url(), AddressSource::Default) {
            self.daemon_address = addr;
            self.address_input = self.daemon_address.original().to_string();
        }

        // Start new daemon if needed
        if mode.should_auto_start() {
            let port = mode.port().unwrap_or(50051);
            let mut launcher = DaemonLauncher::new(port);
            if let Err(e) = launcher.start_with_mode(&mode) {
                self.logging_panel
                    .error("Daemon", &format!("Failed to start: {}", e));
            }
            self.daemon_launcher = Some(launcher);
            self.auto_connect_state = AutoConnectState::WaitingForDaemon {
                since: Instant::now(),
            };
        } else {
            // For remote mode, try to connect immediately
            self.auto_connect_state = AutoConnectState::ReadyToConnect;
        }

        self.logging_panel
            .info("Daemon", &format!("Switched to {} mode", mode.label()));
    }

    /// Render the top menu bar
    fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui
                        .button(format!("{} Settings", crate::icons::action::SETTINGS))
                        .clicked()
                    {
                        self.settings_window.open();
                        ui.close();
                    }
                });

                // Daemon menu for mode selection and control
                ui.menu_button("Daemon", |ui| {
                    // Current mode indicator
                    ui.label(format!("Mode: {}", self.daemon_mode.label()));
                    ui.separator();

                    // Mode selection buttons
                    if ui.button("Local (Mock)").clicked() {
                        self.switch_daemon_mode(DaemonMode::LocalAuto { port: 50051 });
                        ui.close();
                    }

                    if ui.button("Lab Native").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabHardware { port: 50051 });
                        ui.close();
                    }

                    if ui.button("Lab Universal").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabUniversal { port: 50051 });
                        ui.close();
                    }

                    if ui.button("Lab Hybrid+DB").clicked() {
                        self.switch_daemon_mode(DaemonMode::LabHybridDb { port: 50051 });
                        ui.close();
                    }

                    // Remote connection - use the address input
                    if ui.button("Use Remote Address").clicked() {
                        // Parse current address input as remote URL
                        if let Ok(addr) =
                            DaemonAddress::parse(&self.address_input, AddressSource::UserInput)
                        {
                            self.switch_daemon_mode(DaemonMode::Remote {
                                url: addr.to_string(),
                            });
                        }
                        ui.close();
                    }

                    ui.small("Hybrid+DB requires daemon build with db-surreal feature flags.");

                    // Connection presets from gui.toml
                    if !self.gui_presets.is_empty() {
                        ui.separator();
                        ui.label("Presets");
                        let mut selected_preset_url: Option<String> = None;
                        for i in 0..self.gui_presets.len() {
                            let preset = &self.gui_presets[i];
                            let label = if preset.default {
                                format!("{} \u{2605}", preset.name)
                            } else {
                                preset.name.clone()
                            };
                            let response = ui.button(&label);
                            if !preset.description.is_empty() {
                                response.clone().on_hover_text(&preset.description);
                            }
                            if response.clicked() {
                                selected_preset_url = Some(preset.grpc_url.clone());
                                ui.close();
                            }
                        }
                        if let Some(url) = selected_preset_url {
                            self.address_input.clone_from(&url);
                            self.switch_daemon_mode(DaemonMode::Remote { url });
                        }
                    }

                    ui.separator();

                    // Daemon status
                    if let Some(ref mut launcher) = self.daemon_launcher {
                        if launcher.is_running() {
                            ui.colored_label(egui::Color32::GREEN, "● Local daemon running");
                            if let Some(uptime) = launcher.uptime() {
                                ui.small(format!("Uptime: {}s", uptime.as_secs()));
                            }
                            if ui.button("Stop Daemon").clicked() {
                                launcher.stop();
                                self.disconnect();
                                ui.close();
                            }
                        } else {
                            ui.colored_label(egui::Color32::RED, "● Local daemon stopped");
                            if let Some(err) = launcher.last_error() {
                                ui.small(err);
                            }
                            if ui.button("Restart Daemon").clicked() {
                                if let Err(e) = launcher.start_with_mode(&self.daemon_mode) {
                                    self.logging_panel.error("Daemon", &e);
                                } else {
                                    self.auto_connect_state = AutoConnectState::WaitingForDaemon {
                                        since: Instant::now(),
                                    };
                                }
                                ui.close();
                            }
                        }
                    } else {
                        ui.label("Remote mode - no local daemon");
                    }
                });

                if theme::theme_toggle_button(ui, &mut self.theme_preference) {
                    theme::apply_theme(ctx, self.theme_preference);
                }

                ui.menu_button("View", |ui| {
                    if ui.button("Reset Layout").clicked() {
                        self.dock_state = Some(Self::default_dock_state());
                        ui.close();
                    }
                    ui.separator();
                    ui.label("Control Panels");
                    if ui
                        .selectable_label(
                            self.control_panel_layout_mode == ControlPanelLayoutMode::Simple,
                            "Simple",
                        )
                        .clicked()
                    {
                        self.set_control_panel_layout_mode(ControlPanelLayoutMode::Simple);
                        ui.close();
                    }
                    if ui
                        .selectable_label(
                            self.control_panel_layout_mode == ControlPanelLayoutMode::Advanced,
                            "Advanced",
                        )
                        .clicked()
                    {
                        self.set_control_panel_layout_mode(ControlPanelLayoutMode::Advanced);
                        ui.close();
                    }
                    ui.separator();

                    if ui.button("Getting Started").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::GettingStarted));
                        ui.close();
                    }
                    if ui.button("Devices").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Devices));
                        ui.close();
                    }
                    if ui.button("Image Viewer").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::ImageViewer));
                        ui.close();
                    }
                    if ui.button("Scripts").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scripts));
                        ui.close();
                    }
                    if ui.button("Scans").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Scans));
                        ui.close();
                    }
                    if ui.button("Scan Builder").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::ScanBuilder));
                        ui.close();
                    }
                    if ui.button("Experiment Designer").clicked() {
                        self.ui_actions
                            .push(UiAction::FocusTab(Panel::ExperimentDesigner));
                        ui.close();
                    }
                    if ui.button("Storage").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Storage));
                        ui.close();
                    }
                    if ui.button("Modules").clicked() {
                        self.ui_actions.push(UiAction::FocusTab(Panel::Modules));
                        ui.close();
                    }
                });
            });
        });
    }

    /// Render version mismatch warning (if applicable)
    /// Show a transient banner when recovering from a crash (bd-izdj.30)
    fn render_crash_recovery_banner(&mut self, ctx: &egui::Context) {
        if !self.recovered_from_crash {
            return;
        }

        egui::TopBottomPanel::top("crash_recovery_banner")
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color =
                        Some(egui::Color32::from_rgb(100, 200, 255));
                    ui.label(icons::status::INFO);
                    ui.label("Session restored after unexpected shutdown. Panel layout and connection settings have been recovered.");
                    if ui.small_button("Dismiss").clicked() {
                        self.recovered_from_crash = false;
                    }
                });
                ui.add_space(2.0);
            });
    }

    fn render_version_warning(&self, ctx: &egui::Context) {
        // Only show warning if connected and versions don't match
        if self.connection.state().is_connected() {
            if let Some(ref daemon_ver) = self.daemon_version {
                if daemon_ver != &self.gui_version {
                    egui::TopBottomPanel::top("version_warning")
                        .show_separator_line(false)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(255, 200, 0));
                                ui.label(icons::status::WARNING);
                                ui.label(format!(
                                    "Version mismatch: Daemon {} ≠ GUI {}. Some features may not work correctly.",
                                    daemon_ver, self.gui_version
                                ));
                            });
                            ui.add_space(2.0);
                        });
                }
            }
        }
    }

    /// Render the connection status bar
    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Show auto-connect status if active
                match &self.auto_connect_state {
                    AutoConnectState::WaitingForDaemon { since } => {
                        ui.spinner();
                        ui.label(format!(
                            "Starting daemon... ({:.0}s)",
                            since.elapsed().as_secs_f64()
                        ));
                        ui.separator();
                        ui.label(format!("Mode: {}", self.daemon_mode.label()));
                        return; // Don't show rest of status bar during startup
                    }
                    AutoConnectState::ReadyToConnect => {
                        ui.spinner();
                        ui.label("Connecting...");
                        ui.separator();
                        ui.label(format!("Mode: {}", self.daemon_mode.label()));
                        return; // Don't show rest of status bar during startup
                    }
                    AutoConnectState::Complete | AutoConnectState::Skipped => {
                        // Continue with normal status bar
                    }
                }

                // Extract state info upfront to avoid borrow conflicts
                let state_color = self.connection.state().color();
                let state_label = self.connection.state().label();
                let is_connected = self.connection.state().is_connected();
                let is_connecting = self.connection.state().is_connecting();
                let is_disconnected =
                    matches!(self.connection.state(), ConnectionState::Disconnected);
                let error_info = match self.connection.state() {
                    ConnectionState::Error { message, retriable } => {
                        Some((message.clone(), *retriable))
                    }
                    ConnectionState::CircuitBreaker { last_error, .. } => {
                        Some((last_error.clone(), true))
                    }
                    _ => None,
                };
                let seconds_until_retry = self.connection.seconds_until_retry();

                // Connection status indicator
                ui.colored_label(state_color, "●");
                ui.label(state_label);

                // Show reconnect countdown if reconnecting
                if let Some(secs) = seconds_until_retry {
                    ui.label(format!("({:.0}s)", secs));
                }

                ui.separator();

                // Address input with source indicator
                ui.label("Daemon:");

                // Show source as tooltip on the label
                let source_label = format!("[{}]", self.daemon_address.source().label());
                ui.label(
                    egui::RichText::new(&source_label)
                        .small()
                        .color(egui::Color32::GRAY),
                )
                .on_hover_text(format!("Source: {}", self.daemon_address.source()));

                // Text input - show with error highlight if invalid
                let text_color = if self.address_error.is_some() {
                    Some(egui::Color32::RED)
                } else {
                    None
                };
                let mut text_edit = egui::TextEdit::singleline(&mut self.address_input)
                    .hint_text("http://127.0.0.1:50051");
                if let Some(color) = text_color {
                    text_edit = text_edit.text_color(color);
                }
                let response = ui.add_sized([200.0, 18.0], text_edit);

                // Check for Enter key press before potentially consuming response
                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                // Show resolved URL as tooltip when connected
                if is_connected {
                    response.on_hover_text(format!("Resolved: {}", self.daemon_address.as_str()));
                }

                // Connect/Disconnect/Cancel buttons based on state
                if is_disconnected {
                    if ui.button("Connect").clicked() || enter_pressed {
                        self.connect();
                    }
                } else if let Some((_, retriable)) = &error_info {
                    if *retriable {
                        if ui.button("Retry").clicked() || enter_pressed {
                            self.connection
                                .retry(self.daemon_address.clone(), &self.runtime);
                            self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                        }
                    } else if ui.button("Connect").clicked() || enter_pressed {
                        self.connect();
                    }
                } else if is_connected {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                } else if is_connecting {
                    if ui.button("Cancel").clicked() {
                        self.connection.cancel();
                        self.logging_panel.connection_status = LogConnectionStatus::Disconnected;
                        self.logging_panel
                            .info("Connection", "Connection attempt cancelled");
                    }
                    ui.spinner();
                }

                // Show validation error
                if let Some(ref err) = self.address_error {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, err);
                }
                // Show connection error with friendly message
                else if let Some((err_msg, _)) = &error_info {
                    ui.separator();
                    let friendly = friendly_error_message(err_msg);
                    ui.colored_label(egui::Color32::RED, &friendly)
                        .on_hover_text(format!("Raw error: {}", err_msg)); // Show raw error on hover
                }
            });
        });
    }

    #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
    fn start_pvcam_stream(&mut self) {
        use common::capabilities::{FrameObserver, FrameProducer};
        use common::data::FrameView;
        use driver_pvcam::PvcamDriver;
        use rerun::archetypes::Tensor;
        use rerun::RecordingStreamBuilder;
        use std::sync::atomic::{AtomicU64, Ordering};

        /// Frame data with dimensions for channel transport
        struct PreviewFrame {
            data: Vec<u8>,
            width: u32,
            height: u32,
        }

        /// Observer that sends frame copies to Rerun for GUI preview (bd-0dax.6.2)
        ///
        /// Implements the FrameObserver pattern for tap-based frame delivery.
        /// Uses a bounded channel with try_send to avoid blocking the frame loop.
        struct RerunPreviewObserver {
            tx: tokio::sync::mpsc::Sender<PreviewFrame>,
            /// Counter for decimation (send every Nth frame)
            counter: AtomicU64,
            /// Decimation interval (1 = every frame, 10 = every 10th)
            decimation: u64,
        }

        impl FrameObserver for RerunPreviewObserver {
            fn on_frame(&self, frame: &FrameView<'_>) {
                // Only process 16-bit frames
                if frame.bit_depth != 16 {
                    return;
                }

                // Decimation: skip frames based on interval
                let count = self.counter.fetch_add(1, Ordering::Relaxed);
                if count % self.decimation != 0 {
                    return;
                }

                // Non-blocking send with copy (taps must copy, not hold references)
                if let Ok(permit) = self.tx.try_reserve() {
                    permit.send(PreviewFrame {
                        data: frame.pixels().to_vec(),
                        width: frame.width,
                        height: frame.height,
                    });
                }
                // If channel is full, we just drop this frame (backpressure)
            }

            fn name(&self) -> &str {
                "rerun_preview"
            }
        }

        let handle = self.runtime.handle().clone();
        self.pvcam_task = Some(handle.spawn(async move {
            // Connect PVCAM driver and open rerun stream
            let driver = match PvcamDriver::new_async("PrimeBSI".to_string()).await {
                Ok(d) => d,
                Err(err) => {
                    eprintln!("PVCAM init failed: {err}");
                    return;
                }
            };

            // Create channel for frame data (bounded to prevent memory buildup)
            let (tx, mut rx) = tokio::sync::mpsc::channel::<PreviewFrame>(4);

            // Create observer
            let observer = RerunPreviewObserver {
                tx,
                counter: AtomicU64::new(0),
                decimation: 1, // Send every frame (adjust for lower preview FPS)
            };

            // Register the observer using the tap system (replaces deprecated subscribe_frames)
            let observer_handle = match driver.register_observer(Box::new(observer)).await {
                Ok(h) => h,
                Err(err) => {
                    eprintln!("Failed to register frame observer: {err}");
                    return;
                }
            };

            if let Err(err) = driver.start_stream().await {
                eprintln!("PVCAM start_stream failed: {err}");
                let _ = driver.unregister_observer(observer_handle).await;
                return;
            }

            // Spawn viewer or connect to existing one
            let rec = match RecordingStreamBuilder::new("PVCAM Live").spawn() {
                Ok(r) => r,
                Err(err) => {
                    eprintln!("Failed to spawn rerun viewer: {err}");
                    let _ = driver.stop_stream().await;
                    let _ = driver.unregister_observer(observer_handle).await;
                    return;
                }
            };

            // Process frames from the observer channel
            while let Some(frame) = rx.recv().await {
                // Convert raw bytes to u16 slice and create tensor
                let u16_data: &[u16] = bytemuck::cast_slice(&frame.data);
                let shape = vec![frame.height as u64, frame.width as u64];
                let tensor_data = rerun::TensorData::new(
                    shape,
                    rerun::TensorBuffer::U16(u16_data.to_vec().into()),
                );
                let tensor = Tensor::new(tensor_data);
                let _ = rec.log("/pvcam/image", &tensor);
            }

            // Cleanup
            let _ = driver.stop_stream().await;
            let _ = driver.unregister_observer(observer_handle).await;
        }));

        self.pvcam_streaming = true;
    }

    fn poll_logs(&mut self) {
        // Drain all pending log events from the channel
        while let Ok(event) = self.log_receiver.try_recv() {
            self.logging_panel
                .log(event.level, &event.target, &event.message);
        }
    }
}

/// Additional DaqApp methods in a separate impl block (split for helper functions)
impl DaqApp {
    fn set_control_panel_layout_mode(&mut self, mode: ControlPanelLayoutMode) {
        if self.control_panel_layout_mode == mode {
            return;
        }
        self.control_panel_layout_mode = mode;
        self.invalidate_all_panel_widgets();
        self.logging_panel.info(
            "UI",
            &format!("Control panel layout set to {}", mode.label()),
        );
    }

    fn invalidate_all_panel_widgets(&mut self) {
        self.docked_panels.clear();
        self.docked_maitai_panels.clear();
        self.docked_power_meter_panels.clear();
        self.docked_rotator_panels.clear();
        self.docked_stage_panels.clear();
        self.docked_comedi_panels.clear();
        self.docked_config_driven_panels.clear();
        self.grpc_ui_config_cache.clear();
        self.docked_command_widgets.clear();
    }

    /// Remove all state associated with a device control panel.
    ///
    /// Returns the removed DevicePanelInfo if the panel existed, None otherwise.
    /// Used for cleanup when panels are closed or during app shutdown.
    pub(crate) fn remove_panel_data(&mut self, id: usize) -> Option<DevicePanelInfo> {
        self.docked_panels.remove(&id);
        self.docked_maitai_panels.remove(&id);
        self.docked_power_meter_panels.remove(&id);
        self.docked_rotator_panels.remove(&id);
        self.docked_stage_panels.remove(&id);
        self.docked_comedi_panels.remove(&id);
        self.docked_config_driven_panels.remove(&id);
        self.grpc_ui_config_cache.remove(&id);
        self.docked_command_widgets.remove(&id);
        self.device_panel_info.remove(&id)
    }

    fn poll_connect_results(&mut self, ctx: &egui::Context) {
        // Poll connection manager for results
        if let Some((client, daemon_version)) =
            self.connection.poll(&self.runtime, &self.daemon_address)
        {
            self.client = Some(client);
            self.daemon_version.clone_from(&daemon_version);
            self.logging_panel.connection_status = LogConnectionStatus::Connected;
            self.logging_panel.info(
                "Connection",
                &format!(
                    "Connected to {} ({})",
                    self.daemon_address.as_str(),
                    self.daemon_address.source().label()
                ),
            );

            // Log version info
            match daemon_version {
                Some(ref daemon_ver) => {
                    tracing::info!(
                        "Daemon version: {}, GUI version: {}",
                        daemon_ver,
                        self.gui_version
                    );
                    if daemon_ver != &self.gui_version {
                        tracing::warn!(
                            "Version mismatch detected! Daemon: {}, GUI: {}. Some features may not work correctly.",
                            daemon_ver,
                            self.gui_version
                        );
                    }
                }
                None => {
                    tracing::warn!("Connected but failed to get daemon version");
                }
            }
        }

        // Update logging panel status based on connection state
        match self.connection.state() {
            ConnectionState::Error { .. } => {
                if self.logging_panel.connection_status != LogConnectionStatus::Error {
                    self.logging_panel.connection_status = LogConnectionStatus::Error;
                    if let Some(err) = self.connection.state().error_message() {
                        self.logging_panel
                            .error("Connection", &format!("Connection failed: {}", err));
                    }
                }
            }
            ConnectionState::Reconnecting { attempt, .. } => {
                self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                if let Some(err) = self.connection.state().error_message() {
                    self.logging_panel.warn(
                        "Connection",
                        &format!("Reconnecting (attempt {}): {}", attempt, err),
                    );
                }
            }
            ConnectionState::CircuitBreaker { last_error, .. } => {
                if self.logging_panel.connection_status != LogConnectionStatus::CircuitBreaker {
                    self.logging_panel.connection_status = LogConnectionStatus::CircuitBreaker;
                    self.logging_panel.warn(
                        "Connection",
                        &format!("Circuit breaker open: {}", last_error),
                    );
                }
            }
            ConnectionState::HalfOpen { .. } => {
                self.logging_panel.connection_status = LogConnectionStatus::Connecting;
            }
            _ => {}
        }

        // Request repaint if connection attempt is in progress
        if self.connection.is_busy() || self.connection.seconds_until_retry().is_some() {
            ctx.request_repaint();
        }
    }

    /// Check if a health check should be spawned and spawn it.
    fn maybe_spawn_health_check(&mut self) {
        if !self.connection.should_health_check() {
            return;
        }
        let Some(ref client) = self.client else {
            return;
        };

        // Mark health check as started
        self.connection.mark_health_check_started();

        // Clone what we need for the async task
        let mut client = client.clone();
        let tx = self.health_tx.clone();

        self.runtime.spawn(async move {
            // Measure RTT for the health check (bd-j3xz.3.3)
            let start = std::time::Instant::now();
            match client.health_check().await {
                Ok(()) => {
                    let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let _ = tx.send(HealthCheckResult::Success { rtt_ms }).await;
                }
                Err(e) => {
                    let _ = tx.send(HealthCheckResult::Failed(e.to_string())).await;
                }
            }
        });
    }

    /// Poll for health check results.
    fn poll_health_checks(&mut self) {
        while let Ok(result) = self.health_rx.try_recv() {
            match result {
                HealthCheckResult::Success { rtt_ms } => {
                    self.connection.record_health_success(rtt_ms);
                }
                HealthCheckResult::Failed(error) => {
                    let should_reconnect = self.connection.record_health_failure(&error);

                    if should_reconnect {
                        // Clear client - connection is stale
                        self.client = None;
                        self.daemon_version = None;
                        self.logging_panel.connection_status = LogConnectionStatus::Connecting;
                        self.logging_panel.warn(
                            "Connection",
                            &format!("Connection lost ({}), reconnecting...", error),
                        );

                        // Trigger reconnect
                        self.connection
                            .trigger_health_reconnect(self.daemon_address.clone(), &self.runtime);
                    }
                }
            }
        }
    }

    /// Update the logging panel's connection diagnostics from the ConnectionManager (bd-j3xz.3.3).
    fn update_connection_diagnostics(&mut self) {
        let health_status = self.connection.health_status();

        // Calculate relative times
        let secs_since_last_success = health_status
            .last_success
            .map(|t| t.elapsed().as_secs_f64());
        let secs_since_last_error = health_status
            .last_error_at
            .map(|t| t.elapsed().as_secs_f64());

        self.logging_panel.connection_diagnostics = ConnectionDiagnostics {
            last_rtt_ms: health_status.last_rtt_ms,
            total_errors: health_status.total_errors,
            secs_since_last_error,
            last_error_message: health_status.last_error_message.clone(),
            secs_since_last_success,
            consecutive_failures: health_status.consecutive_failures,
        };
    }

    /// Process auto-connect state machine
    fn process_auto_connect(&mut self, ctx: &egui::Context) {
        use std::time::Duration;

        match &self.auto_connect_state {
            AutoConnectState::WaitingForDaemon { since } => {
                let elapsed = since.elapsed();

                // Check if daemon process has started
                if let Some(ref mut launcher) = self.daemon_launcher {
                    if launcher.is_running() && elapsed > Duration::from_millis(500) {
                        // Give daemon time to start listening
                        tracing::info!("Daemon is running, initiating auto-connect");
                        self.auto_connect_state = AutoConnectState::ReadyToConnect;
                    } else if elapsed > Duration::from_secs(10) {
                        // Timeout - daemon didn't start
                        tracing::error!("Timeout waiting for daemon to start");
                        self.auto_connect_state = AutoConnectState::Skipped;
                        self.logging_panel
                            .error("Daemon", "Timeout waiting for daemon to start");
                    }
                } else {
                    // No launcher but in WaitingForDaemon - shouldn't happen, skip
                    self.auto_connect_state = AutoConnectState::Skipped;
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            AutoConnectState::ReadyToConnect => {
                if !self.connection.is_busy() {
                    tracing::info!("Auto-connecting to daemon at {}", self.daemon_address);
                    self.connect();
                    self.auto_connect_state = AutoConnectState::Complete;
                }
            }
            AutoConnectState::Complete | AutoConnectState::Skipped => {
                // No action needed
            }
        }
    }

    /// Called when connection is established - trigger panel refreshes
    fn on_connection_established(&mut self) {
        tracing::info!("Connection established - triggering panel refreshes");

        // Reset panels to force them to refresh their data
        // This clears cached data and triggers new loads on next render
        self.devices_panel = DevicesPanel::default();
        self.scripts_panel = ScriptsPanel::default();
        self.modules_panel = ModulesPanel::default();
        self.storage_panel = StoragePanel::default();
        self.run_history_panel = RunHistoryPanel::default();
        self.run_comparison_panel = RunComparisonPanel::default();

        // Reset InstrumentManagerPanel to trigger auto-refresh on reconnect
        // (keeps panel state like selected device, but clears device list and refresh flag)
        self.instrument_manager_panel.reset_refresh_state();

        self.logging_panel
            .info("Connection", "Connected - panels will refresh data");

        // Start device reconciliation to validate persisted panels
        self.start_device_reconcile();
    }

    /// Start device reconciliation - validates persisted panels against daemon
    fn start_device_reconcile(&mut self) {
        let Some(ref client) = self.client else {
            return;
        };

        // Increment epoch to invalidate stale results
        self.device_reconcile_epoch = self.device_reconcile_epoch.wrapping_add(1);
        let epoch = self.device_reconcile_epoch;
        let daemon_url = self.daemon_address.to_string();

        // Clone what we need for async task
        let mut client = client.clone();
        let tx = self.device_reconcile_tx.clone();

        self.runtime.spawn(async move {
            match client.list_devices().await {
                Ok(devices) => {
                    let _ = tx
                        .send(DeviceReconcileMsg::Ok {
                            epoch,
                            daemon_url,
                            devices,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(DeviceReconcileMsg::Err {
                            epoch,
                            daemon_url,
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    /// Poll for device reconciliation results and apply if epoch matches
    fn poll_device_reconcile(&mut self) {
        while let Ok(msg) = self.device_reconcile_rx.try_recv() {
            match msg {
                DeviceReconcileMsg::Ok {
                    epoch,
                    daemon_url,
                    devices,
                } => {
                    // Ignore stale results
                    if epoch != self.device_reconcile_epoch
                        || daemon_url != self.daemon_address.to_string()
                    {
                        tracing::debug!(
                            epoch,
                            current_epoch = self.device_reconcile_epoch,
                            "Ignoring stale device reconciliation result"
                        );
                        continue;
                    }

                    self.apply_device_reconcile(devices);
                }
                DeviceReconcileMsg::Err {
                    epoch,
                    daemon_url,
                    error,
                } => {
                    // Ignore stale errors
                    if epoch != self.device_reconcile_epoch
                        || daemon_url != self.daemon_address.to_string()
                    {
                        continue;
                    }

                    tracing::warn!("Device reconciliation failed: {}", error);
                    // Mark all panels as Pending (will retry on next connection)
                    for info in self.device_panel_info.values_mut() {
                        info.availability = DeviceAvailability::Pending;
                    }
                }
            }
        }
    }

    /// Apply device reconciliation results - update availability and migrate panels if needed
    fn apply_device_reconcile(&mut self, devices: Vec<DeviceInfo>) {
        let device_map: HashMap<String, DeviceInfo> =
            devices.into_iter().map(|d| (d.id.clone(), d)).collect();

        // Collect panel migrations to avoid borrowing conflicts
        let mut migrations: Vec<(usize, DevicePanelKind)> = Vec::new();

        for (panel_id, panel_info) in &mut self.device_panel_info {
            let device_id = &panel_info.device_info.id;

            if let Some(daemon_device) = device_map.get(device_id) {
                // Device found on daemon
                panel_info.availability = DeviceAvailability::Available;

                // Check if capabilities changed (requires panel migration)
                let new_kind = panel_kind_for_device(daemon_device);
                if new_kind != panel_info.kind {
                    tracing::info!(
                        panel_id,
                        device_id,
                        old_kind = ?panel_info.kind,
                        new_kind = ?new_kind,
                        "Device capabilities changed - migrating panel"
                    );

                    // Update kind and device info
                    panel_info.kind = new_kind;
                    panel_info.device_info = daemon_device.clone();

                    // Defer migration to avoid borrow conflicts
                    migrations.push((*panel_id, new_kind));
                } else {
                    // Just update device info (metadata may have changed)
                    panel_info.device_info = daemon_device.clone();
                }
            } else {
                // Device not found on daemon
                panel_info.availability = DeviceAvailability::Missing;
                tracing::warn!(
                    panel_id,
                    device_id,
                    "Device panel references missing device"
                );
            }
        }

        // Apply panel migrations
        for (panel_id, _new_kind) in migrations {
            self.invalidate_panel_widget(panel_id);
        }
    }

    /// Invalidate a panel widget so it will be lazily recreated with updated capabilities.
    fn invalidate_panel_widget(&mut self, panel_id: usize) {
        self.docked_panels.remove(&panel_id);
        self.docked_maitai_panels.remove(&panel_id);
        self.docked_power_meter_panels.remove(&panel_id);
        self.docked_rotator_panels.remove(&panel_id);
        self.docked_stage_panels.remove(&panel_id);
        self.docked_comedi_panels.remove(&panel_id);
        self.docked_config_driven_panels.remove(&panel_id);
        self.docked_command_widgets.remove(&panel_id);
    }

    /// Detect connection state transitions and handle them
    fn detect_connection_transitions(&mut self) {
        let is_connected = self.connection.state().is_connected();

        if is_connected && !self.was_connected {
            // Just connected - trigger panel refreshes
            self.on_connection_established();
        }

        self.was_connected = is_connected;
    }

    /// Check and handle global keyboard shortcuts
    fn check_global_shortcuts(&mut self, ctx: &egui::Context) {
        // Check toggle cheat sheet (Shift+?)
        if self.shortcut_manager.check_action(
            ctx,
            ShortcutContext::Global,
            ShortcutAction::ToggleCheatSheet,
        ) {
            self.show_cheat_sheet = !self.show_cheat_sheet;
        }

        // Note: Other global shortcuts (OpenSettings, SaveCurrent) will be handled
        // by specific panels or settings UI when implemented
    }
}

struct DaqTabViewer<'a> {
    app: &'a mut DaqApp,
}

impl TabViewer for DaqTabViewer<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Panel::Nav => "Navigation".into(),
            Panel::GettingStarted => {
                format!("{} Getting Started", icons::nav::GETTING_STARTED).into()
            }
            Panel::Instruments => format!("{} Instruments", icons::nav::INSTRUMENT_MANAGER).into(),
            Panel::Devices => format!("{} Devices", icons::nav::DEVICES).into(),
            Panel::Scripts => format!("{} Scripts", icons::nav::SCRIPTS).into(),
            Panel::Scans => format!("{} Scans", icons::nav::SCANS).into(),
            Panel::ScanBuilder => "Scan Builder".into(),
            Panel::ExperimentDesigner => "Experiment Designer".into(),
            Panel::Storage => format!("{} Storage", icons::nav::STORAGE).into(),
            Panel::RunHistory => "📚 Run History".into(),
            Panel::RunComparison => "📊 Compare Runs".into(),
            Panel::Modules => format!("{} Modules", icons::nav::MODULES).into(),
            Panel::PlanRunner => format!("{} Plan Runner", icons::nav::PLAN_RUNNER).into(),
            Panel::DocumentViewer => format!("{} Documents", icons::nav::DOCUMENT_VIEWER).into(),
            Panel::SignalPlotter => format!("{} Signal Plotter", icons::nav::SIGNAL_PLOTTER).into(),
            Panel::ImageViewer => format!("{} Image Viewer", icons::nav::IMAGE_VIEWER).into(),
            Panel::Logs => format!("{} Logs", icons::nav::LOGGING).into(),
            Panel::DeviceControl { id } => {
                // Look up device name from the panel ID mapping
                if let Some(info) = self.app.device_panel_info.get(id) {
                    format!("🎛 {}", info.device_info.name).into()
                } else {
                    "🎛 Device".into()
                }
            }
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        !matches!(tab, Panel::Nav)
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> OnCloseResponse {
        // Clean up device panel state when a DeviceControl tab is closed
        if let Panel::DeviceControl { id } = tab {
            self.app.remove_panel_data(*id);
        }
        OnCloseResponse::Close
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        // Constrain each dock tab to its own available width to avoid
        // content driving slight horizontal overflow.
        ui.set_max_width(ui.available_width());

        match tab {
            Panel::Nav => self.render_nav(ui),
            Panel::GettingStarted => self.app.getting_started_panel.ui(ui),
            Panel::Instruments => self.app.instrument_manager_panel.ui(
                ui,
                self.app.client.as_mut(),
                &self.app.runtime,
            ),
            Panel::Devices => {
                self.app
                    .devices_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Scripts => {
                self.app
                    .scripts_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Scans => {
                self.app
                    .scans_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::ScanBuilder => {
                self.app
                    .scan_builder_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::ExperimentDesigner => self.app.experiment_designer_panel.ui(
                ui,
                self.app.client.as_mut(),
                Some(&self.app.runtime),
            ),
            Panel::Storage => {
                self.app
                    .storage_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::RunHistory => {
                self.app
                    .run_history_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::RunComparison => {
                self.app
                    .run_comparison_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Modules => {
                self.app
                    .modules_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::PlanRunner => {
                self.app
                    .plan_runner_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::DocumentViewer => {
                self.app
                    .document_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::SignalPlotter => {
                self.app.signal_plotter_panel.drain_updates();
                self.app.signal_plotter_panel.ui(ui);
            }
            Panel::ImageViewer => {
                self.app
                    .image_viewer_panel
                    .ui(ui, self.app.client.as_mut(), &self.app.runtime);
            }
            Panel::Logs => self.app.logging_panel.ui(ui),
            Panel::DeviceControl { id } => {
                self.render_device_control(ui, *id);
            }
        }
    }
}

impl DaqTabViewer<'_> {
    fn nav_button(&mut self, ui: &mut egui::Ui, icon: &str, label: &str, panel: Panel) {
        let text = format!("{} {}", icon, label);
        if ui.button(text).clicked() {
            self.app.ui_actions.push(UiAction::FocusTab(panel));
        }
    }

    fn section_label(ui: &mut egui::Ui, text: &str) {
        ui.add_space(layout::SECTION_SPACING / 2.0);
        ui.label(
            egui::RichText::new(text)
                .small()
                .color(layout::colors::MUTED),
        );
    }

    fn render_nav(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Navigation");
            ui.separator();

            self.nav_button(
                ui,
                icons::nav::GETTING_STARTED,
                "Getting Started",
                Panel::GettingStarted,
            );

            Self::section_label(ui, "Hardware");
            self.nav_button(
                ui,
                icons::nav::INSTRUMENT_MANAGER,
                "Instruments",
                Panel::Instruments,
            );
            self.nav_button(ui, icons::nav::DEVICES, "Devices", Panel::Devices);

            Self::section_label(ui, "Visualization");
            self.nav_button(
                ui,
                icons::nav::SIGNAL_PLOTTER,
                "Signal Plotter",
                Panel::SignalPlotter,
            );
            self.nav_button(
                ui,
                icons::nav::IMAGE_VIEWER,
                "Image Viewer",
                Panel::ImageViewer,
            );

            Self::section_label(ui, "Experiment");
            self.nav_button(ui, icons::nav::SCRIPTS, "Scripts", Panel::Scripts);
            self.nav_button(ui, icons::nav::SCANS, "Scans", Panel::Scans);
            self.nav_button(ui, icons::nav::SCANS, "Scan Builder", Panel::ScanBuilder);
            self.nav_button(
                ui,
                icons::nav::SCANS,
                "Experiment Designer",
                Panel::ExperimentDesigner,
            );
            self.nav_button(
                ui,
                icons::nav::PLAN_RUNNER,
                "Plan Runner",
                Panel::PlanRunner,
            );

            Self::section_label(ui, "Data");
            self.nav_button(ui, icons::nav::STORAGE, "Storage", Panel::Storage);
            self.nav_button(ui, "📚", "Run History", Panel::RunHistory);
            self.nav_button(
                ui,
                icons::nav::DOCUMENT_VIEWER,
                "Documents",
                Panel::DocumentViewer,
            );

            Self::section_label(ui, "System");
            self.nav_button(ui, icons::nav::MODULES, "Modules", Panel::Modules);
            self.nav_button(ui, icons::nav::LOGGING, "Logs", Panel::Logs);

            ui.separator();
            ui.add_space(layout::SECTION_SPACING / 2.0);

            if ui
                .button(format!("{} Open Rerun", icons::CHART_LINE))
                .clicked()
            {
                let _ = std::process::Command::new("rerun").spawn();
            }

            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            {
                ui.add_space(layout::SECTION_SPACING / 2.0);
                let (icon, label) = if self.app.pvcam_streaming {
                    (icons::action::STOP, "Stop PVCAM Live")
                } else {
                    (icons::action::RECORD, "PVCAM Live to Rerun")
                };
                if ui.button(format!("{} {}", icon, label)).clicked() {
                    if self.app.pvcam_streaming {
                        if let Some(task) = self.app.pvcam_task.take() {
                            task.abort();
                        }
                        self.app.pvcam_streaming = false;
                    } else {
                        self.app.start_pvcam_stream();
                    }
                }
            }
        });
    }

    /// Render a docked device control panel
    fn render_device_control(&mut self, ui: &mut egui::Ui, panel_id: usize) {
        // Get device info for this panel (stored with full capability flags)
        let Some(info) = self.app.device_panel_info.get(&panel_id).cloned() else {
            ui.label("Device panel not found");
            return;
        };

        let device_info = &info.device_info;

        // Gate rendering based on availability
        match info.availability {
            DeviceAvailability::Pending => {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.spinner();
                    ui.label("Validating device with daemon...");
                    ui.add_space(10.0);
                    ui.label(format!("Device: {}", device_info.name));
                    ui.label(format!("ID: {}", device_info.id));
                });
                return;
            }
            DeviceAvailability::Missing => {
                ui.vertical(|ui| {
                    ui.add_space(10.0);
                    ui.colored_label(egui::Color32::RED, "⚠ Device Not Found");
                    ui.add_space(10.0);

                    ui.label(format!("Device: {}", device_info.name));
                    ui.label(format!("ID: {}", device_info.id));
                    ui.label(format!("Daemon: {}", self.app.daemon_address));

                    ui.add_space(10.0);
                    ui.label("This device is not available on the connected daemon.");
                    ui.label("The daemon may have been restarted with a different configuration.");

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Refresh").clicked() {
                            self.app.start_device_reconcile();
                        }
                        if ui.button("Close Panel").clicked() {
                            self.app
                                .ui_actions
                                .push(UiAction::CloseDevicePanel { id: panel_id });
                        }
                    });
                });
                return;
            }
            DeviceAvailability::Available => {
                // Continue to normal rendering below
            }
        }

        // --- Priority 0: gRPC-driven panel from device metadata ---
        let grpc_config = self
            .app
            .grpc_ui_config_cache
            .entry(panel_id)
            .or_insert_with(|| {
                crate::panels::instrument_manager::dispatch::try_grpc_ui_config(device_info)
            });
        if let Some(panel_config) = grpc_config {
            let panel_config = panel_config.clone();
            let panel = self
                .app
                .docked_config_driven_panels
                .entry(panel_id)
                .or_insert_with(|| ConfigDrivenPanel::new(panel_config));
            ui.push_id(("docked", panel_id), |ui| {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            });
            return;
        }

        // --- Priority 1: Config-driven panel from local TOML ---
        if let Some(panel_config) = self
            .app
            .config_cache
            .get_ui_config_for_driver(&device_info.driver_type)
        {
            let panel_config: hardware::config::schema::ControlPanelConfig = panel_config.clone();
            let panel = self
                .app
                .docked_config_driven_panels
                .entry(panel_id)
                .or_insert_with(|| ConfigDrivenPanel::new(panel_config));
            ui.push_id(("docked", panel_id), |ui| {
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            });
            return;
        }

        let layout_mode = self.app.control_panel_layout_mode;
        ui.push_id(("docked", panel_id), |ui| match layout_mode {
            ControlPanelLayoutMode::Simple => {
                let panel = self
                    .app
                    .docked_panels
                    .entry(panel_id)
                    .or_insert_with(|| GenericDevicePanel::from_device_info(device_info));
                panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
            }
            ControlPanelLayoutMode::Advanced => {
                match docked_advanced_panel_kind_for_device(device_info) {
                    DockedAdvancedPanelKind::MaiTai => {
                        let panel = self.app.docked_maitai_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Comedi => {
                        let panel = self.app.docked_comedi_panels.entry(panel_id).or_default();
                        panel.ui(ui, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::PowerMeter => {
                        let panel = self
                            .app
                            .docked_power_meter_panels
                            .entry(panel_id)
                            .or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Rotator => {
                        let panel = self.app.docked_rotator_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Stage => {
                        let panel = self.app.docked_stage_panels.entry(panel_id).or_default();
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                    DockedAdvancedPanelKind::Generic => {
                        let panel =
                            self.app.docked_panels.entry(panel_id).or_insert_with(|| {
                                GenericDevicePanel::from_device_info(device_info)
                            });
                        panel.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                    }
                }

                let mut command_widgets = self
                    .app
                    .docked_command_widgets
                    .remove(&panel_id)
                    .unwrap_or_default();
                command_widgets.ui(ui, device_info, self.app.client.as_mut(), &self.app.runtime);
                self.app
                    .docked_command_widgets
                    .insert(panel_id, command_widgets);
            }
        });
    }
}

impl eframe::App for DaqApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_logs();
        self.poll_connect_results(ctx);
        self.poll_device_reconcile(); // bd-vjzq
        self.maybe_spawn_health_check();
        self.poll_health_checks();
        self.update_connection_diagnostics(); // bd-j3xz.3.3

        // Process auto-connect state machine
        self.process_auto_connect(ctx);

        // Detect connection state transitions (for panel refresh on connect)
        self.detect_connection_transitions();

        // Check global keyboard shortcuts
        self.check_global_shortcuts(ctx);

        // Handle additional keyboard shortcuts (Ctrl+, opens settings)
        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(egui::Key::Comma) {
                self.settings_window.open();
            }
        });

        self.render_menu_bar(ctx);
        self.render_version_warning(ctx);
        self.render_crash_recovery_banner(ctx);
        self.render_status_bar(ctx);

        // Render settings window
        if self.settings_window.show(ctx, &mut self.app_settings) {
            // Settings were applied - update dependent systems
            if self.theme_preference != self.app_settings.appearance.theme {
                self.theme_preference = self.app_settings.appearance.theme;
                theme::apply_theme(ctx, self.theme_preference);
            }
            // Font and UI scale changes will be applied on next frame
            ctx.set_zoom_factor(self.app_settings.appearance.ui_scale);
        }

        let error_count = self.connection.health_status().total_errors;
        let error_count = if error_count > 0 {
            Some(error_count)
        } else {
            None
        };
        self.status_bar
            .show(ctx, self.connection.state(), error_count);

        // Render Dock Area
        let mut dock_state = self
            .dock_state
            .take()
            .unwrap_or_else(Self::default_dock_state);
        let mut viewer = DaqTabViewer { app: self };
        DockArea::new(&mut dock_state)
            .style(Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);

        // Check for pop-out requests from InstrumentManagerPanel
        if let Some(request) = self.instrument_manager_panel.take_pop_out_request() {
            self.ui_actions.push(UiAction::OpenDeviceControl {
                device_info: Box::new(request.device_info),
            });
        }

        // Check for image viewer navigation requests from InstrumentManagerPanel
        if let Some(device_id) = self.instrument_manager_panel.take_image_viewer_request() {
            tracing::info!(
                device_id = %device_id,
                "Navigating to Image Viewer for live stream"
            );
            self.ui_actions.push(UiAction::FocusTab(Panel::ImageViewer));
            if let Some(client) = self.client.as_mut() {
                self.image_viewer_panel
                    .set_device(&device_id, client, &self.runtime);
            }
        }

        // Collect panels to close to avoid borrow conflicts
        let mut panels_to_close = Vec::new();

        // Process deferred UI actions
        for action in self.ui_actions.drain(..) {
            match action {
                UiAction::FocusTab(panel) => {
                    if let Some((surface, node, tab)) = dock_state.find_tab(&panel) {
                        dock_state.set_active_tab((surface, node, tab));
                        dock_state.set_focused_node_and_surface((surface, node));
                    } else {
                        // Add to focused leaf or fallback to root
                        dock_state.main_surface_mut().push_to_focused_leaf(panel);
                    }
                }
                UiAction::CloseDevicePanel { id } => {
                    // Remove panel from dock
                    dock_state.retain_tabs(|tab| {
                        !matches!(tab, Panel::DeviceControl { id: panel_id } if *panel_id == id)
                    });
                    // Defer cleanup to avoid borrow conflicts
                    panels_to_close.push(id);
                }
                UiAction::OpenDeviceControl { device_info } => {
                    let device_info = *device_info;
                    // Generate a new panel ID with saturation on overflow
                    // (practically impossible to hit usize::MAX panels, but prevents ID collisions)
                    let panel_id = self.next_device_panel_id;
                    self.next_device_panel_id = self.next_device_panel_id.saturating_add(1);

                    // Debug logging for panel routing diagnosis (bd-kj7i)
                    tracing::info!(
                        panel_id = panel_id,
                        device_id = %device_info.id,
                        device_name = %device_info.name,
                        driver_type = %device_info.driver_type,
                        is_emission_controllable = device_info.is_emission_controllable(),
                        is_shutter_controllable = device_info.is_shutter_controllable(),
                        is_wavelength_tunable = device_info.is_wavelength_tunable(),
                        is_readable = device_info.is_readable(),
                        is_movable = device_info.is_movable(),
                        "OpenDeviceControl: creating pop-out panel with capabilities"
                    );

                    // Determine panel kind from device capabilities
                    let kind = panel_kind_for_device(&device_info);

                    // Store device info (full proto with capability flags)
                    self.device_panel_info.insert(
                        panel_id,
                        DevicePanelInfo {
                            device_info: device_info.clone(),
                            availability: DeviceAvailability::Available, // Fresh from daemon
                            kind,
                        },
                    );

                    // GenericDevicePanel created lazily on first render

                    // Add the panel to the dock
                    let panel = Panel::DeviceControl { id: panel_id };
                    dock_state.main_surface_mut().push_to_focused_leaf(panel);
                }
            }
        }

        // Clean up closed panels
        for id in panels_to_close {
            self.remove_panel_data(id);
        }

        self.dock_state = Some(dock_state);

        // Render cheat sheet panel if visible
        if self.show_cheat_sheet {
            self.cheat_sheet_panel
                .show(ctx, &mut self.show_cheat_sheet, &self.shortcut_manager);
        }
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        // Persist state every 5 seconds for crash recovery (bd-izdj.30)
        std::time::Duration::from_secs(5)
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Persist daemon address via AppSettings (single source of truth)
        if self.connection.state().is_connected() {
            self.app_settings.connection.daemon_address =
                self.daemon_address.original().to_string();
        }
        // Clear legacy key if still present (one-time cleanup)
        clear_legacy_daemon_address(storage);

        // Update session file with current daemon URL (bd-izdj.30)
        write_session_file(self.daemon_address.as_str());

        if let Some(dock_state) = &self.dock_state {
            eframe::set_value(storage, eframe::APP_KEY, dock_state);
        }

        // Persist layout version for stale layout detection on next load
        eframe::set_value(storage, LAYOUT_VERSION_KEY, &LAYOUT_VERSION);

        eframe::set_value(storage, "theme_preference", &self.theme_preference);

        // Persist application settings
        eframe::set_value(storage, "app_settings", &self.app_settings);

        // Persist keyboard shortcuts
        eframe::set_value(storage, "shortcut_manager", &self.shortcut_manager);
        eframe::set_value(
            storage,
            "control_panel_layout_mode",
            &self.control_panel_layout_mode,
        );

        // Persist device panel info for layout restoration
        let persisted_panels: HashMap<usize, PersistedPanelInfo> = self
            .device_panel_info
            .iter()
            .map(|(id, info)| (*id, PersistedPanelInfo::from(&info.device_info)))
            .collect();
        eframe::set_value(storage, "device_panel_info", &persisted_panels);
        eframe::set_value(storage, "next_device_panel_id", &self.next_device_panel_id);
    }
}

impl Drop for DaqApp {
    fn drop(&mut self) {
        // Mark clean shutdown — remove session file (bd-izdj.30)
        clear_session_file();

        tracing::debug!("DaqApp shutting down, cleaning up device panel state");

        // Collect panel IDs to avoid borrow conflicts during cleanup
        let panel_ids: Vec<usize> = self.device_panel_info.keys().copied().collect();

        // Clean up all device panel state
        for id in panel_ids {
            self.remove_panel_data(id);
        }

        // Shutdown daemon launcher if running
        if let Some(launcher) = self.daemon_launcher.take() {
            drop(launcher); // DaemonLauncher should have its own Drop that terminates the process
        }

        tracing::debug!("DaqApp shutdown complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_serialization() {
        let panel = Panel::Nav;
        let serialized = serde_json::to_string(&panel).unwrap();
        assert_eq!(serialized, "\"Nav\"");

        let deserialized: Panel = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, Panel::Nav);
    }

    #[test]
    fn test_default_dock_layout() {
        let dock_state = DaqApp::default_dock_state();

        let mut found_nav = false;
        let mut found_logs = false;
        let mut found_instruments = false;
        let mut found_image_viewer = false;

        for ((_surface, _node), tab) in dock_state.iter_all_tabs() {
            match tab {
                Panel::Nav => found_nav = true,
                Panel::Logs => found_logs = true,
                Panel::Instruments => found_instruments = true,
                Panel::ImageViewer => found_image_viewer = true,
                _ => {}
            }
        }

        assert!(found_nav, "Navigation panel missing from default layout");
        assert!(found_logs, "Logs panel missing from default layout");
        assert!(
            found_instruments,
            "Instruments panel missing from default layout"
        );
        assert!(
            found_image_viewer,
            "Image Viewer panel missing from default layout"
        );
    }

    #[test]
    fn test_command_widget_infer_status_command() {
        let commands = vec![
            "get_wavelength".to_string(),
            "read_power".to_string(),
            "close_shutter".to_string(),
        ];

        let inferred_wavelength =
            CommandWidgetPalette::infer_status_command("wavelength_nm", &commands)
                .expect("should infer wavelength getter command");
        let inferred_power = CommandWidgetPalette::infer_status_command("power", &commands)
            .expect("should infer power read command");

        assert_eq!(inferred_wavelength, "get_wavelength");
        assert_eq!(inferred_power, "read_power");
    }

    #[test]
    fn test_command_widget_manifest_summary_params() {
        let device = DeviceInfo {
            id: "laser".to_string(),
            name: "Laser".to_string(),
            driver_type: "maitai".to_string(),
            metadata: Some(protocol::daq::DeviceMetadata {
                ui_schema_json: Some(
                    r#"{"status_display":{"summary_params":["wavelength_nm","power_mw"]}}"#
                        .to_string(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        };

        let summary_params = CommandWidgetPalette::manifest_summary_params(&device);
        assert_eq!(
            summary_params,
            vec!["power_mw".to_string(), "wavelength_nm".to_string()]
        );
    }

    #[test]
    fn test_panel_kind_for_maitai_device() {
        let device = DeviceInfo {
            id: "laser".to_string(),
            name: "MaiTai Laser".to_string(),
            driver_type: "maitai".to_string(),
            category: 0,
            capabilities: vec![
                "emission_controllable".to_string(),
                "shutter_controllable".to_string(),
            ],
            metadata: None,
            ..Default::default()
        };

        assert_eq!(panel_kind_for_device(&device), DevicePanelKind::MaiTai);
    }

    #[test]
    fn test_panel_kind_for_power_meter() {
        let device = DeviceInfo {
            id: "pm1".to_string(),
            name: "Power Meter".to_string(),
            driver_type: "newport_1830c".to_string(),
            category: 0,
            capabilities: vec!["readable".to_string()],
            metadata: None,
            ..Default::default()
        };

        assert_eq!(panel_kind_for_device(&device), DevicePanelKind::PowerMeter);
    }

    #[test]
    fn test_panel_kind_for_rotator() {
        let device = DeviceInfo {
            id: "rot1".to_string(),
            name: "Rotator".to_string(),
            driver_type: "ell14".to_string(),
            category: 0,
            capabilities: vec!["movable".to_string()],
            metadata: None,
            ..Default::default()
        };

        assert_eq!(panel_kind_for_device(&device), DevicePanelKind::Rotator);
    }

    #[test]
    fn test_panel_kind_for_stage() {
        let device = DeviceInfo {
            id: "stage1".to_string(),
            name: "Linear Stage".to_string(),
            driver_type: "esp300".to_string(),
            category: 0,
            capabilities: vec!["movable".to_string()],
            metadata: None,
            ..Default::default()
        };

        assert_eq!(panel_kind_for_device(&device), DevicePanelKind::Stage);
    }

    #[test]
    fn test_panel_kind_for_analog_output() {
        let device = DeviceInfo {
            id: "ao1".to_string(),
            name: "Analog Output".to_string(),
            driver_type: "comedi_analog_output".to_string(),
            category: 0,
            capabilities: vec!["settable".to_string()],
            metadata: None,
            ..Default::default()
        };

        assert_eq!(
            panel_kind_for_device(&device),
            DevicePanelKind::AnalogOutput
        );
    }

    #[test]
    fn test_persisted_panel_info_to_device_info() {
        let persisted = PersistedPanelInfo {
            device_id: "test_device".to_string(),
            device_name: "Test Device".to_string(),
            driver_type: "mock".to_string(),
            capabilities: vec!["movable".to_string(), "readable".to_string()],
            is_emission_controllable: false,
            is_shutter_controllable: false,
            is_wavelength_tunable: false,
            is_readable: false,
            is_movable: false,
        };

        let device_info: DeviceInfo = persisted.into();

        assert_eq!(device_info.id, "test_device");
        assert_eq!(device_info.name, "Test Device");
        assert_eq!(device_info.driver_type, "mock");
        assert_eq!(device_info.capabilities.len(), 2);
        assert!(device_info.capabilities.contains(&"movable".to_string()));
        assert!(device_info.capabilities.contains(&"readable".to_string()));
    }

    #[test]
    fn test_persisted_panel_info_legacy_migration() {
        // Test migration from legacy boolean fields
        let persisted = PersistedPanelInfo {
            device_id: "legacy_device".to_string(),
            device_name: "Legacy Device".to_string(),
            driver_type: "old_driver".to_string(),
            capabilities: vec![], // Empty, should migrate from booleans
            is_emission_controllable: true,
            is_shutter_controllable: true,
            is_wavelength_tunable: false,
            is_readable: true,
            is_movable: false,
        };

        let device_info: DeviceInfo = persisted.into();

        assert_eq!(device_info.capabilities.len(), 3);
        assert!(device_info.capabilities.contains(&"readable".to_string()));
        assert!(device_info
            .capabilities
            .contains(&"shutter_controllable".to_string()));
        assert!(device_info
            .capabilities
            .contains(&"emission_controllable".to_string()));
    }

    #[test]
    fn test_device_info_to_persisted_panel_info() {
        let device_info = DeviceInfo {
            id: "device1".to_string(),
            name: "Device 1".to_string(),
            driver_type: "test_driver".to_string(),
            category: 0,
            capabilities: vec!["movable".to_string(), "readable".to_string()],
            metadata: None,
            ..Default::default()
        };

        let persisted = PersistedPanelInfo::from(&device_info);

        assert_eq!(persisted.device_id, "device1");
        assert_eq!(persisted.device_name, "Device 1");
        assert_eq!(persisted.driver_type, "test_driver");
        assert_eq!(persisted.capabilities.len(), 2);
        // Legacy fields should be false
        assert!(!persisted.is_movable);
        assert!(!persisted.is_readable);
    }

    #[test]
    fn test_device_availability_default() {
        let availability = DeviceAvailability::default();
        assert_eq!(availability, DeviceAvailability::Pending);
    }

    #[test]
    fn test_panel_equality() {
        assert_eq!(Panel::Nav, Panel::Nav);
        assert_ne!(Panel::Nav, Panel::Instruments);

        let device_panel1 = Panel::DeviceControl { id: 1 };
        let device_panel2 = Panel::DeviceControl { id: 2 };
        assert_ne!(device_panel1, device_panel2);

        let device_panel3 = Panel::DeviceControl { id: 1 };
        assert_eq!(device_panel1, device_panel3);
    }

    #[test]
    fn test_device_panel_kind_equality() {
        assert_eq!(DevicePanelKind::MaiTai, DevicePanelKind::MaiTai);
        assert_ne!(DevicePanelKind::MaiTai, DevicePanelKind::PowerMeter);
    }
}
