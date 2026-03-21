//! Main application state and UI logic.

use crate::time::{Duration, Instant};
use std::collections::HashMap;

use eframe::egui;
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use tokio::sync::mpsc;

#[cfg(not(target_arch = "wasm32"))]
use crate::connection::{
    clear_legacy_daemon_address, migrate_legacy_daemon_address, resolve_address, AddressSource,
    DaemonAddress,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::connection_state_ext::ConnectionStateExt;
#[cfg(not(target_arch = "wasm32"))]
use crate::daemon_launcher::{AutoConnectState, DaemonLauncher, DaemonMode};
use crate::device_ext::DeviceInfoExt;
use crate::icons;
use crate::layout;
use crate::panels::instrument_manager::{
    config_loader::DeviceConfigCache, config_renderer::ConfigDrivenPanel,
};
use crate::panels::{
    ComediPanel, DocumentViewerPanel, ExperimentDesignerPanel, GettingStartedPanel,
    ImageViewerPanel, InstrumentManagerPanel, LoggingPanel, ModulesPanel, PlanRunnerPanel,
    RunHistoryPanel, ScanBuilderPanel, ScriptsPanel, SignalPlotterPanel, StoragePanel,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::panels::{ConnectionDiagnostics, ConnectionStatus as LogConnectionStatus};
use crate::shortcuts::{CheatSheetPanel, ShortcutAction, ShortcutContext, ShortcutManager};
use crate::theme::{self, ThemePreference};
use crate::widgets::{
    DeviceControlWidget, GenericDevicePanel, MaiTaiControlPanel, PowerMeterControlPanel,
    RotatorControlPanel, StageControlPanel, StatusBar,
};
#[cfg(not(target_arch = "wasm32"))]
use client::reconnect::{friendly_error_message, ConnectionManager, ConnectionState};
use client::DaqClient;
use protocol::daq::DeviceInfo;

mod automation;
mod connection;
mod devices;
mod dock_layout;
mod lifecycle;
mod rendering;
mod session;
mod tabs;
mod types;

use tabs::DaqTabViewer;
use types::*;
pub use types::{ControlPanelLayoutMode, DeviceAvailability, DevicePanelKind, Panel};

#[cfg(not(target_arch = "wasm32"))]
use session::*;

/// Layout version constant. Increment this when the default dock layout changes
/// to force users with stale saved layouts to get the new default.
/// v1: Initial version (had Devices panel as default in some builds)
/// v2: Instruments panel as default (bd-kj7i fix)
/// v3: Add ImageViewer tab alongside Instruments in default layout
const LAYOUT_VERSION: u32 = 3;

/// Storage key for layout version
const LAYOUT_VERSION_KEY: &str = "layout_version";

/// Main application state
pub struct DaqApp {
    /// gRPC client (wrapped in Option for lazy initialization)
    client: Option<DaqClient>,

    /// Connection manager (handles state machine and auto-reconnect)
    #[cfg(not(target_arch = "wasm32"))]
    connection: ConnectionManager,

    /// Validated daemon address (normalized, with source tracking)
    #[cfg(not(target_arch = "wasm32"))]
    daemon_address: DaemonAddress,

    /// Text input field for address (may be invalid during editing)
    #[cfg(not(target_arch = "wasm32"))]
    address_input: String,

    /// Address validation error (shown in UI)
    #[cfg(not(target_arch = "wasm32"))]
    address_error: Option<String>,

    /// Daemon version (retrieved via GetDaemonInfo)
    daemon_version: Option<String>,

    /// GUI version (from CARGO_PKG_VERSION)
    #[cfg(not(target_arch = "wasm32"))]
    gui_version: String,

    /// Dock state for panel management
    dock_state: Option<DockState<Panel>>,

    /// Queue for deferred UI actions (e.g. opening tabs from Nav panel)
    ui_actions: Vec<UiAction>,

    /// Panel states
    getting_started_panel: GettingStartedPanel,
    scripts_panel: ScriptsPanel,
    storage_panel: StoragePanel,
    run_history_panel: RunHistoryPanel,
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
    runtime: crate::runtime::Runtime,

    /// Channel for health check results
    #[cfg(not(target_arch = "wasm32"))]
    health_tx: mpsc::Sender<HealthCheckResult>,
    #[cfg(not(target_arch = "wasm32"))]
    health_rx: mpsc::Receiver<HealthCheckResult>,

    /// Device reconciliation epoch (incremented on each reconcile request)
    device_reconcile_epoch: u64,

    /// Channel for device reconciliation results
    device_reconcile_tx: mpsc::Sender<DeviceReconcileMsg>,
    device_reconcile_rx: mpsc::Receiver<DeviceReconcileMsg>,

    /// Previous connection state (for detecting transitions)
    was_connected: bool,

    /// Daemon mode configuration (local auto-start, remote, or lab hardware)
    #[cfg(not(target_arch = "wasm32"))]
    daemon_mode: DaemonMode,

    /// Daemon process launcher (for auto-start local modes)
    #[cfg(not(target_arch = "wasm32"))]
    daemon_launcher: Option<DaemonLauncher>,

    /// Auto-connect lifecycle state
    #[cfg(not(target_arch = "wasm32"))]
    auto_connect_state: AutoConnectState,

    /// Receiver for tracing log events (forwarded to logging panel)
    #[cfg(not(target_arch = "wasm32"))]
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
    /// Docked config-driven panels (from gRPC `ui_schema_json` or local TOML `[ui.control_panel]`)
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

    /// Native preferences (cross-platform file persistence via `preferences` crate).
    /// Loaded on startup, saved on shutdown. Complements eframe storage.
    #[cfg(not(target_arch = "wasm32"))]
    native_prefs: crate::preferences::AppPreferences,

    /// Connection presets loaded from gui.toml
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
    recovered_from_crash: bool,

    /// WASM connection state (URL input + pending connect)
    #[cfg(target_arch = "wasm32")]
    wasm_connection: WasmConnectionState,

    /// Whether touch-friendly style has been applied (avoids per-frame style_mut calls)
    #[cfg(target_arch = "wasm32")]
    touch_style_applied: bool,

    /// Automation command queue — JS pushes, `update()` drains
    #[cfg(target_arch = "wasm32")]
    automation_commands: crate::automation::CommandQueue,

    /// Automation state snapshot — `update()` writes, JS reads via `getStatus()`
    #[cfg(target_arch = "wasm32")]
    automation_state: crate::automation::StateHolder,
}

impl DaqApp {
    /// Create a new application instance with the specified daemon mode
    #[cfg(not(target_arch = "wasm32"))]
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

        // Load native preferences early for fallback during address resolution
        let native_prefs = crate::preferences::AppPreferences::load_or_default();

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

        // Apply native_prefs as fallback if eframe storage has default daemon address
        let default_addr = crate::settings::ConnectionSettings::default().daemon_address;
        if app_settings.connection.daemon_address == default_addr
            && native_prefs.daemon_url != crate::preferences::AppPreferences::default().daemon_url
        {
            tracing::info!(
                "Restoring daemon address '{}' from native preferences",
                native_prefs.daemon_url
            );
            app_settings
                .connection
                .daemon_address
                .clone_from(&native_prefs.daemon_url);
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

        let mut app = Self {
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
            scripts_panel: ScriptsPanel::default(),
            storage_panel: StoragePanel::default(),
            run_history_panel: RunHistoryPanel::default(),
            modules_panel: ModulesPanel::default(),
            plan_runner_panel: PlanRunnerPanel::default(),
            scan_builder_panel: ScanBuilderPanel::default(),
            #[allow(clippy::default_constructed_unit_structs)] // unit struct stub on wasm32
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
            native_prefs,
            gui_presets,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_streaming: false,
            #[cfg(all(feature = "rerun_viewer", feature = "pvcam"))]
            pvcam_task: None,
            shortcut_manager,
            cheat_sheet_panel: CheatSheetPanel::new(),
            show_cheat_sheet: false,
            recovered_from_crash,
        };

        // Restore last echelle profile path and auto-trigger load on next connection
        if let Some(path) = cc
            .storage
            .and_then(|s| eframe::get_value::<String>(s, "echelle_profile_path"))
        {
            if !path.is_empty() {
                app.image_viewer_panel.echelle_cal_ui.save_as_path_text = path.clone();
                app.image_viewer_panel.request_remote_profile_load(path);
            }
        }

        app
    }

    /// Skips daemon launching, session files, crash detection, and ConnectionManager.
    #[cfg(target_arch = "wasm32")]
    pub fn new_wasm(cc: &eframe::CreationContext<'_>) -> Self {
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

        // Configure egui style
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = layout::ITEM_SPACING;
        cc.egui_ctx.set_style(style);

        // WASM runtime (delegates to spawn_local)
        let runtime = crate::runtime::Runtime;

        // Load app settings
        let app_settings: crate::settings::AppSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "app_settings"))
            .unwrap_or_default();
        let control_panel_layout_mode: ControlPanelLayoutMode = cc
            .storage
            .and_then(|s| eframe::get_value(s, "control_panel_layout_mode"))
            .unwrap_or(ControlPanelLayoutMode::Simple);

        // Device reconciliation channel
        let (device_reconcile_tx, device_reconcile_rx) = mpsc::channel(4);

        // Load persisted device panel info
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

        // Initialize dock state
        let mut dock_state = if let Some(storage) = cc.storage {
            let stored_version: Option<u32> = eframe::get_value(storage, LAYOUT_VERSION_KEY);
            match stored_version {
                Some(v) if v == LAYOUT_VERSION => eframe::get_value(storage, eframe::APP_KEY)
                    .unwrap_or_else(Self::default_dock_state),
                _ => Self::default_dock_state(),
            }
        } else {
            Self::default_dock_state()
        };

        // Remove orphaned DeviceControl panels
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

        // Get shared handles for the automation bridge (JS ↔ DaqApp)
        let (automation_commands, automation_state) = crate::automation::get_bridge_handles();

        let mut app = Self {
            client: None,
            daemon_version: None,
            dock_state: Some(dock_state),
            ui_actions: Vec::new(),
            getting_started_panel: GettingStartedPanel::default(),
            scripts_panel: ScriptsPanel::default(),
            storage_panel: StoragePanel::default(),
            run_history_panel: RunHistoryPanel::default(),
            modules_panel: ModulesPanel::default(),
            plan_runner_panel: PlanRunnerPanel::default(),
            scan_builder_panel: ScanBuilderPanel::default(),
            #[allow(clippy::default_constructed_unit_structs)] // unit struct stub on wasm32
            experiment_designer_panel: ExperimentDesignerPanel::default(),
            document_viewer_panel: DocumentViewerPanel::default(),
            instrument_manager_panel: InstrumentManagerPanel::default(),
            signal_plotter_panel: SignalPlotterPanel::new(),
            image_viewer_panel: ImageViewerPanel::new(),
            logging_panel: LoggingPanel::new(),
            runtime,
            device_reconcile_epoch: 0,
            device_reconcile_tx,
            device_reconcile_rx,
            was_connected: false,
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
            shortcut_manager,
            cheat_sheet_panel: CheatSheetPanel::new(),
            show_cheat_sheet: false,
            wasm_connection: {
                let url = cc
                    .storage
                    .and_then(|s| eframe::get_value::<String>(s, WASM_SERVER_URL_KEY))
                    .unwrap_or_else(|| WASM_DEFAULT_SERVER_URL.to_string());
                WasmConnectionState {
                    url_input: url,
                    ..Default::default()
                }
            },
            touch_style_applied: false,
            automation_commands,
            automation_state,
        };

        // Restore last echelle profile path and auto-trigger load on next connection
        if let Some(path) = cc
            .storage
            .and_then(|s| eframe::get_value::<String>(s, "echelle_profile_path"))
        {
            if !path.is_empty() {
                app.image_viewer_panel.echelle_cal_ui.save_as_path_text = path.clone();
                app.image_viewer_panel.request_remote_profile_load(path);
            }
        }

        app
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_data_integrity_status_message_matches_warn_events() {
        let event = crate::gui_log_layer::GuiLogEvent {
            level: crate::panels::LogLevel::Warn,
            target: "data_integrity".to_string(),
            message: "DataIntegrityFault: camera-a dropped 2 frame(s)".to_string(),
        };

        let (level, message) =
            data_integrity_status_message(&event).expect("expected status bar warning");
        assert!(matches!(level, crate::widgets::StatusLevel::Warning));
        assert_eq!(message, event.message);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_data_integrity_status_message_ignores_non_fault_logs() {
        let event = crate::gui_log_layer::GuiLogEvent {
            level: crate::panels::LogLevel::Info,
            target: "server".to_string(),
            message: "Acquisition started".to_string(),
        };

        assert!(data_integrity_status_message(&event).is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_data_integrity_status_message_matches_resource_pressure_events() {
        let event = crate::gui_log_layer::GuiLogEvent {
            level: crate::panels::LogLevel::Error,
            target: "resource_pressure".to_string(),
            message: "ResourcePressureEvent: free disk on /data is 8.5 GiB".to_string(),
        };

        let (level, message) =
            data_integrity_status_message(&event).expect("expected status bar resource alert");
        assert!(matches!(level, crate::widgets::StatusLevel::Error));
        assert_eq!(message, event.message);
    }
}
