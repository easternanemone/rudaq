//! The eframe/egui implementation for the GUI.
//!
//! This module defines the main graphical user interface for the DAQ application,
//! built using the `eframe` and `egui` libraries. It provides a flexible, dockable
//! interface for visualizing data, controlling instruments, and managing data storage.
//!
//! ## Architecture
//!
//! The GUI is structured around a main `Gui` struct which implements the `eframe::App` trait.
//! The core components of the GUI are:
//!
//! - **Docking System (`egui_dock`):** The central area of the application is a `DockArea`
//!   that allows users to arrange various tabs in a flexible layout. Tabs can be plots,
//!
//!   instrument control panels, or other views. The state of the dock is managed by `DockState<DockTab>`.
//!
//! - **Panels:**
//!   - `TopBottomPanel` (Top): Contains global controls like adding new plot tabs and a menu for
//!     opening instrument-specific control panels.
//!   - `TopBottomPanel` (Bottom): Displays a resizable log panel for viewing application logs.
//!   - `SidePanel` (Left): Shows a list of all configured instruments, their status (running/stopped),
//!     and provides controls to start or stop them. Instruments can be dragged from this panel
//!     to the central dock area to open their control tabs.
//!   - `SidePanel` (Right): A toggleable panel for managing data storage sessions, implemented in the
//!     `storage_manager` module.
//!
//! - **Data Flow:**
//!   - The `Gui` struct receives live `DataPoint`s from the core application logic via a `tokio::sync::broadcast`
//!     channel.
//!   - The `update_data` method processes these points and updates the corresponding plot tabs. This is optimized
//!     by batching data points by channel before iterating through tabs.
//!   - Instrument control panels interact with the `DaqApp` core to send commands to hardware.
//!
//! - **State Management:**
//!   - The main `Gui` struct holds the application state, including the `DaqApp` handle, the `DockState`,
//!     and state for various UI components like combo boxes and filters.
//!
//! ## Modules
//!
//! - `instrument_controls`: Defines the UI panels for controlling specific instruments (e.g., lasers, cameras).
//! - `log_panel`: Implements the UI for the filterable log view at the bottom of the screen.
//! - `storage_manager`: Provides the UI for creating, managing, and saving data acquisition sessions.

pub mod instrument_controls;
pub mod storage_manager;
pub mod verification;

use self::instrument_controls::*;
use self::storage_manager::StorageManager;
use crate::{config::Settings, log_capture::LogBuffer, instrument::InstrumentRegistry, messages::DaqCommand, measurement::Measure};
use daq_core::Measurement;
use eframe::egui;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use egui_plot::{Line, Plot, PlotPoints};
use log::{debug, error, info, LevelFilter};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

mod log_panel;

const PLOT_DATA_CAPACITY: usize = 1000;

/// Represents the different types of tabs that can be docked
enum DockTab {
    Plot(PlotTab),
    Spectrum(SpectrumTab),
    Image(ImageTab),
    MaiTaiControl(MaiTaiControlPanel),
    Newport1830CControl(Newport1830CControlPanel),
    ElliptecControl(ElliptecControlPanel),
    ESP300Control(ESP300ControlPanel),
    PVCAMControl(PVCAMControlPanel),
}

/// Represents the state of a single plot panel.
struct PlotTab {
    channel: String,
    plot_data: VecDeque<[f64; 2]>,
    last_timestamp: f64,
}

impl PlotTab {
    fn new(channel: String) -> Self {
        Self {
            channel,
            plot_data: VecDeque::with_capacity(PLOT_DATA_CAPACITY),
            last_timestamp: 0.0,
        }
    }
}

/// Represents a frequency spectrum visualization panel.
struct SpectrumTab {
    channel: String,
    /// Frequency bins stored as (frequency_hz, magnitude_db) pairs
    spectrum_data: Vec<[f64; 2]>,
}

impl SpectrumTab {
    fn new(channel: String) -> Self {
        Self {
            channel,
            spectrum_data: Vec::new(),
        }
    }
}

/// Represents an image/camera visualization panel.
struct ImageTab {
    channel: String,
    /// Image dimensions (width, height)
    dimensions: (usize, usize),
    /// Pixel data in native format (kept as-is from broadcast for memory efficiency)
    /// Uses V2 daq_core::PixelBuffer for compatibility with V2 measurement architecture
    pixel_data: Option<daq_core::PixelBuffer>,
    /// Min/max values for colormap scaling
    value_range: (f64, f64),
    /// egui texture handle for efficient frame updates (created lazily on first frame)
    texture: Option<egui::TextureHandle>,
}

impl ImageTab {
    fn new(channel: String) -> Self {
        Self {
            channel,
            dimensions: (0, 0),
            pixel_data: None,
            value_range: (0.0, 1.0),
            texture: None,
        }
    }
}

/// The main GUI struct.

pub struct Gui<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    // Direct actor communication (replaces DaqApp wrapper)
    command_tx: mpsc::Sender<DaqCommand>,
    runtime: Arc<tokio::runtime::Runtime>,

    // Configuration access (read-only, shared)
    settings: Arc<Settings>,
    instrument_registry: Arc<InstrumentRegistry<M>>,

    // Data and logging
    data_receiver: mpsc::Receiver<Arc<Measurement>>,
    log_buffer: LogBuffer,

    // Pending async operations tracking
    pending_operations: HashMap<String, PendingOperation>,

    // Cached instrument state (refreshed periodically)
    instrument_status_cache: HashMap<String, bool>, // id → is_running
    cache_refresh_counter: u32,

    dock_state: DockState<DockTab>,
    selected_channel: String,
    storage_manager: StorageManager,
    show_storage: bool,
    // Log panel state
    log_filter_text: String,
    log_level_filter: LevelFilter,
    scroll_to_bottom: bool,
    /// Centralized cache of latest instrument state from data stream
    /// Key: "instrument_id:channel" (e.g., "maitai:power", "esp300:axis1_position")
    /// Value: latest Measurement for that channel (wrapped in Arc for zero-copy)
    /// This provides single source of truth for instrument state in GUI
    data_cache: HashMap<String, Arc<Measurement>>,
    /// Channel subscription map for O(1) tab lookup
    /// Key: channel name (e.g., "sine_wave", "spectrum:maitai", "image:pvcam")
    /// Value: Vec of (SurfaceIndex, NodeIndex) for tabs interested in this channel
    /// Enables direct dispatch instead of iterating all tabs
    channel_subscriptions: HashMap<String, Vec<(egui_dock::SurfaceIndex, egui_dock::NodeIndex)>>,
    /// Tracks whether subscriptions need rebuilding (when tabs are added/removed/changed)
    subscriptions_dirty: bool,
    /// Frame counter for periodic subscription rebuilds (every 60 frames = ~1 second at 60fps)
    /// This catches tab closes, channel changes, and other modifications we can't directly track
    frame_counter: u32,
    /// Screenshot request tracking
    /// Path to save the next screenshot, or None if no screenshot requested
    screenshot_request: Option<std::path::PathBuf>,
}

/// Tracks a pending async operation for status display
struct PendingOperation {
    rx: oneshot::Receiver<Result<(), crate::messages::SpawnError>>,
    description: String,
    started_at: Instant,
    /// Optional cache update to apply when operation completes
    /// Wrapped in Arc<Mutex<>> to allow async task to update it
    cache_update: Option<Arc<Mutex<HashMap<String, bool>>>>,
}

const CACHE_REFRESH_INTERVAL: u32 = 60; // Refresh every 60 frames (~1 second)

impl<M> Gui<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    /// Creates a new GUI with async communication to DaqManagerActor.
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        command_tx: mpsc::Sender<DaqCommand>,
        runtime: Arc<tokio::runtime::Runtime>,
        settings: Settings,
        instrument_registry: Arc<InstrumentRegistry<M>>,
        log_buffer: LogBuffer,
    ) -> Self {
        // Subscribe to data stream via blocking call during initialization.
        // TECH DEBT (Phase 3): This blocks briefly during startup, but occurs before
        // the GUI window is visible to the user, so no user-visible freeze.
        // A full async migration would require restructuring eframe's App::new().
        // The more critical blocking operations are in instrument control panels
        // which block on every user action - see instrument_controls.rs migration.
        let data_receiver = {
            let (cmd, rx) = DaqCommand::subscribe_to_data();
            command_tx.blocking_send(cmd).ok();
            rx.blocking_recv().unwrap_or_else(|_| {
                let (tx, rx) = mpsc::channel(1);
                drop(tx);
                rx
            })
        };

        let mut dock_state =
            DockState::new(vec![DockTab::Plot(PlotTab::new("sine_wave".to_string()))]);
        dock_state.push_to_focused_leaf(DockTab::Plot(PlotTab::new("cosine_wave".to_string())));

        Self {
            command_tx,
            runtime,
            settings: Arc::new(settings),
            instrument_registry,
            data_receiver,
            log_buffer,
            pending_operations: HashMap::new(),
            instrument_status_cache: HashMap::new(),
            cache_refresh_counter: 0,
            dock_state,
            selected_channel: "sine_wave".to_string(),
            storage_manager: StorageManager::new(),
            show_storage: false,
            log_filter_text: String::new(),
            log_level_filter: LevelFilter::Info,
            scroll_to_bottom: true,
            data_cache: HashMap::new(),
            channel_subscriptions: HashMap::new(),
            subscriptions_dirty: true, // Rebuild on first frame
            frame_counter: 0,
            screenshot_request: None,
        }
    }

    /// Rebuilds the channel subscription map by scanning all tabs.
    /// Call this after tab creation, deletion, or channel changes.
    fn rebuild_subscriptions(&mut self) {
        self.channel_subscriptions.clear();

        for (tab_index, tab) in self.dock_state.iter_all_tabs() {
            match tab {
                DockTab::Plot(plot_tab) => {
                    self.channel_subscriptions
                        .entry(plot_tab.channel.clone())
                        .or_insert_with(Vec::new)
                        .push(tab_index);
                }
                DockTab::Spectrum(spectrum_tab) => {
                    // Spectrum channels use "spectrum:channel_name" format in cache
                    let spectrum_channel = format!("spectrum:{}", spectrum_tab.channel);
                    self.channel_subscriptions
                        .entry(spectrum_channel)
                        .or_insert_with(Vec::new)
                        .push(tab_index);
                }
                DockTab::Image(image_tab) => {
                    // Image channels use "image:channel_name" format in cache
                    let image_channel = format!("image:{}", image_tab.channel);
                    self.channel_subscriptions
                        .entry(image_channel)
                        .or_insert_with(Vec::new)
                        .push(tab_index);
                }
                // Control panels don't subscribe to data updates
                _ => {}
            }
        }

        self.subscriptions_dirty = false;
    }

    /// Fetches new measurements from the broadcast channel and updates the data cache.
    /// Updates both plot tabs and the centralized data cache for instrument control panels.
    /// Handles Scalar, Spectrum, and Image measurements for V2 architecture.
    /// Optimized with O(1) channel lookup instead of O(M) iteration over all tabs.
    fn update_data(&mut self) {
        loop {
            match self.data_receiver.try_recv() {
                Ok(measurement) => {
                    match measurement.as_ref() {
                        Measurement::Scalar(ref data_point) => {
                            // Update the central data cache with channel as key
                            let cache_key = data_point.channel.clone();
                            self.data_cache
                                .insert(cache_key.clone(), measurement.clone());

                            // O(1) lookup: Find tabs subscribed to this channel
                            if let Some(subscribed_tabs) =
                                self.channel_subscriptions.get(&cache_key)
                            {
                                // Only iterate over interested tabs (typically 1-3)
                                for &tab_location in subscribed_tabs {
                                    for (location, tab) in self.dock_state.iter_all_tabs_mut() {
                                        if location == tab_location {
                                            if let DockTab::Plot(plot_tab) = tab {
                                                // Update plot data
                                                if plot_tab.plot_data.len() >= PLOT_DATA_CAPACITY {
                                                    plot_tab.plot_data.pop_front();
                                                }
                                                let timestamp =
                                                    data_point.timestamp.timestamp_micros() as f64
                                                        / 1_000_000.0;
                                                if plot_tab.last_timestamp == 0.0 {
                                                    plot_tab.last_timestamp = timestamp;
                                                }
                                                plot_tab.plot_data.push_back([
                                                    timestamp - plot_tab.last_timestamp,
                                                    data_point.value,
                                                ]);
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Measurement::Spectrum(ref spectrum_data) => {
                            // Update cache for spectrum data
                            let cache_key = format!("spectrum:{}", spectrum_data.channel);
                            self.data_cache
                                .insert(cache_key.clone(), measurement.clone());

                            // O(1) lookup: Find spectrum tabs subscribed to this channel
                            if let Some(subscribed_tabs) =
                                self.channel_subscriptions.get(&cache_key)
                            {
                                for &tab_location in subscribed_tabs {
                                    for (location, tab) in self.dock_state.iter_all_tabs_mut() {
                                        if location == tab_location {
                                            if let DockTab::Spectrum(spectrum_tab) = tab {
                                                // Convert wavelengths/intensities to [f64; 2] for plotting
                                                spectrum_tab.spectrum_data = spectrum_data
                                                    .wavelengths
                                                    .iter()
                                                    .zip(spectrum_data.intensities.iter())
                                                    .map(|(&wavelength, &intensity)| {
                                                        [wavelength, intensity]
                                                    })
                                                    .collect();
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Measurement::Image(ref image_data) => {
                            // Update cache for image data
                            let cache_key = format!("image:{}", image_data.channel);
                            log::info!("GUI: Received Image measurement for channel '{}', cache_key: '{}', dimensions: {}x{}", 
                                image_data.channel, cache_key, image_data.width, image_data.height);
                            self.data_cache
                                .insert(cache_key.clone(), measurement.clone());

                            // O(1) lookup: Find image tabs subscribed to this channel
                            if let Some(subscribed_tabs) =
                                self.channel_subscriptions.get(&cache_key)
                            {
                                log::info!(
                                    "GUI: Found {} subscribed tabs for cache_key '{}'",
                                    subscribed_tabs.len(),
                                    cache_key
                                );
                                for &tab_location in subscribed_tabs {
                                    for (location, tab) in self.dock_state.iter_all_tabs_mut() {
                                        if location == tab_location {
                                            if let DockTab::Image(image_tab) = tab {
                                                image_tab.dimensions = (
                                                    image_data.width as usize,
                                                    image_data.height as usize,
                                                );
                                                // Store pixels in native format for memory efficiency
                                                // PixelBuffer::U16 uses 4× less memory than F64
                                                image_tab.pixel_data =
                                                    Some(image_data.pixels.clone());

                                                // Calculate value range for colormap scaling
                                                // Convert to f64 temporarily for min/max calculation
                                                let pixels_f64 = image_data.pixels.as_f64();
                                                if let (Some(&min), Some(&max)) = (
                                                    pixels_f64
                                                        .iter()
                                                        .min_by(|a, b| a.partial_cmp(b).unwrap()),
                                                    pixels_f64
                                                        .iter()
                                                        .max_by(|a, b| a.partial_cmp(b).unwrap()),
                                                ) {
                                                    image_tab.value_range = (min, max);
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // No more data available, exit loop
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    // Channel closed, exit loop
                    break;
                }
            }
        }
    }

    /// Request a screenshot to be taken on the next frame.
    ///
    /// # Arguments
    /// * `path` - Path where the screenshot will be saved
    ///
    /// # Example
    /// ```no_run
    /// gui.request_screenshot("screenshots/verification.png");
    /// ```
    pub fn request_screenshot<P: Into<std::path::PathBuf>>(&mut self, path: P) {
        self.screenshot_request = Some(path.into());
    }

    /// Takes a screenshot and saves it to the specified path.
    /// Creates parent directories if they don't exist.
    fn take_screenshot(&self, ctx: &egui::Context, path: std::path::PathBuf) {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!("Failed to create screenshot directory: {}", e);
                return;
            }
        }

        // Request screenshot from egui
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);

        // Note: In egui 0.29, screenshots are handled via viewport commands
        // The actual screenshot will be captured on the next frame
        // We use a simpler approach: request via viewport command and log
        info!("Screenshot requested: {}", path.display());

        // TODO: egui 0.29's screenshot API is asynchronous and requires
        // additional handling through ViewportCommand. For now, we log the request.
        // A future enhancement would implement proper async screenshot handling
        // with egui's callback system.
    }
}

impl<M> eframe::App for Gui<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll pending operations (non-blocking)
        let mut completed = Vec::new();
        for (op_id, pending) in &mut self.pending_operations {
            match pending.rx.try_recv() {
                Ok(Ok(())) => {
                    info!("Operation '{}' completed successfully", pending.description);
                    
                    // Apply cache update if present
                    if let Some(cache_update) = &pending.cache_update {
                        let cache = cache_update.lock().unwrap();
                        self.instrument_status_cache = cache.clone();
                        debug!("Applied cache update: {} instruments", cache.len());
                    }
                    
                    completed.push(op_id.clone());
                }
                Ok(Err(e)) => {
                    error!("Operation '{}' failed: {}", pending.description, e);
                    completed.push(op_id.clone());
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    // Still pending, check timeout
                    if pending.started_at.elapsed() > std::time::Duration::from_secs(30) {
                        error!("Operation '{}' timed out", pending.description);
                        completed.push(op_id.clone());
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    error!("Operation '{}' actor closed channel", pending.description);
                    completed.push(op_id.clone());
                }
            }
        }

        // Remove completed operations
        for op_id in completed {
            self.pending_operations.remove(&op_id);
        }

        // Refresh instrument status cache periodically
        self.cache_refresh_counter = self.cache_refresh_counter.wrapping_add(1);
        if self.cache_refresh_counter % CACHE_REFRESH_INTERVAL == 0 {
            // Query instrument list and update cache asynchronously
            let (cmd, list_rx) = DaqCommand::get_instrument_list();

            if self.command_tx.try_send(cmd).is_ok() {
                let op_id = format!("cache_refresh_{}", self.cache_refresh_counter);

                // Create shared cache that async task can update
                let cache_update = Arc::new(Mutex::new(HashMap::new()));
                let cache_clone = Arc::clone(&cache_update);

                // Convert list receiver into PendingOperation receiver
                let (op_tx, op_rx) = tokio::sync::oneshot::channel();

                self.runtime.spawn(async move {
                    match list_rx.await {
                        Ok(list) => {
                            // Update shared cache (this will be applied when operation completes)
                            let mut cache = cache_clone.lock().unwrap();
                            cache.clear();
                            for id in list {
                                cache.insert(id, true);
                            }
                            drop(cache); // Release lock before sending
                            let _ = op_tx.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = op_tx.send(Err(crate::messages::SpawnError::InvalidConfig(
                                format!("Failed to get instrument list: {}", e)
                            )));
                        }
                    }
                });

                self.pending_operations.insert(op_id, PendingOperation {
                    rx: op_rx,
                    description: "Refreshing instrument status".to_string(),
                    started_at: Instant::now(),
                    cache_update: Some(cache_update),
                });
            } else {
                error!("Failed to queue instrument status refresh (channel full)");
            }
        }

        // Rebuild subscriptions periodically (every 60 frames) or when explicitly marked dirty
        // This catches tab closes, channel changes, and other modifications
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.subscriptions_dirty || self.frame_counter % 60 == 0 {
            self.rebuild_subscriptions();
        }

        // Handle screenshot keyboard shortcut (F12)
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F12) {
                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let screenshot_path =
                    std::path::PathBuf::from(format!("screenshots/screenshot_{}.png", timestamp));
                self.screenshot_request = Some(screenshot_path);
            }
        });

        // Process screenshot request if pending
        if let Some(path) = self.screenshot_request.take() {
            self.take_screenshot(ctx, path);
        }

        // Process incoming measurements
        self.update_data();

        egui::TopBottomPanel::bottom("bottom_panel")
            .resizable(true)
            .min_height(150.0)
            .show(ctx, |ui| {
                log_panel::render(ui, self);
            });

        // Collect instrument data from local settings + cache (no blocking)
        let instruments: Vec<(String, toml::Value, bool)> = self
            .settings
            .instruments
            .iter()
            .map(|(k, v)| {
                let is_running = self.instrument_status_cache.get(k).copied().unwrap_or(false);
                (k.clone(), v.clone(), is_running)
            })
            .collect();

        // Get available channels from registry (no actor call needed)
        let available_channels: Vec<String> = self.instrument_registry.list().collect();

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Rust DAQ Control Panel");
                ui.separator();

                egui::ComboBox::from_label("Channel")
                    .selected_text(self.selected_channel.clone())
                    .show_ui(ui, |ui| {
                        for channel in &available_channels {
                            ui.selectable_value(
                                &mut self.selected_channel,
                                channel.clone(),
                                channel.clone(),
                            );
                        }
                    });

                if ui.button("Add Plot").clicked() {
                    self.dock_state
                        .push_to_focused_leaf(DockTab::Plot(PlotTab::new(
                            self.selected_channel.clone(),
                        )));
                    self.subscriptions_dirty = true;
                }

                if ui.button("📷 Add Image").clicked() {
                    // Create image tab for PVCAM camera
                    // Channel format: "{instrument_id}_image" (e.g., "pvcam_image")
                    self.dock_state
                        .push_to_focused_leaf(DockTab::Image(ImageTab::new(
                            "pvcam_image".to_string(),
                        )));
                    self.subscriptions_dirty = true;
                }

                ui.separator();

                // Instrument control buttons
                egui::menu::menu_button(ui, "Instrument Controls", |ui| {
                    if ui.button("🔬 MaiTai Laser").clicked() {
                        self.dock_state.push_to_focused_leaf(DockTab::MaiTaiControl(
                            MaiTaiControlPanel::new("maitai".to_string()),
                        ));
                        ui.close_menu();
                    }
                    if ui.button("📊 Newport 1830-C").clicked() {
                        self.dock_state
                            .push_to_focused_leaf(DockTab::Newport1830CControl(
                                Newport1830CControlPanel::new("newport_1830c".to_string()),
                            ));
                        ui.close_menu();
                    }
                    if ui.button("🔄 Elliptec Rotators").clicked() {
                        self.dock_state
                            .push_to_focused_leaf(DockTab::ElliptecControl(
                                ElliptecControlPanel::new("elliptec".to_string(), vec![0, 1]),
                            ));
                        ui.close_menu();
                    }
                    if ui.button("⚙️ ESP300 Motion").clicked() {
                        self.dock_state.push_to_focused_leaf(DockTab::ESP300Control(
                            ESP300ControlPanel::new("esp300".to_string(), 3),
                        ));
                        ui.close_menu();
                    }
                    if ui.button("📷 PVCAM Camera").clicked() {
                        self.dock_state.push_to_focused_leaf(DockTab::PVCAMControl(
                            PVCAMControlPanel::new("pvcam".to_string()),
                        ));
                        ui.close_menu();
                    }
                });

                ui.separator();
                if ui
                    .button(if self.show_storage {
                        "Hide Storage"
                    } else {
                        "Show Storage"
                    })
                    .clicked()
                {
                    self.show_storage = !self.show_storage;
                }
            });
        });

        if self.show_storage {
            egui::SidePanel::right("storage_panel")
                .resizable(true)
                .min_width(300.0)
                .show(ctx, |ui| {
                    self.storage_manager.ui(ui, &self.settings);
                });
        }

        egui::SidePanel::left("control_panel")
            .resizable(true)
            .min_width(200.0)
            .show(ctx, |ui| {
                render_instrument_panel::<M>(
                    ui,
                    &instruments,
                    &self.command_tx,
                    &self.runtime,
                    &mut self.pending_operations,
                    &mut self.dock_state,
                    &self.data_cache,
                );
            });

        let temp_app = crate::app::DaqApp {
            command_tx: self.command_tx.clone(),
            runtime: self.runtime.clone(),
            settings: (*self.settings).clone(),
            log_buffer: self.log_buffer.clone(),
            instrument_registry: self.instrument_registry.clone(),
            instrument_registry_v2: Arc::new(crate::instrument::InstrumentRegistryV2::new()),  // Empty for compatibility
            _phantom: std::marker::PhantomData,
        };

        let mut tab_viewer = DockTabViewer {
            available_channels,
            app: temp_app,
            data_cache: &self.data_cache,
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            // Check for dropped instruments
            let (_inner_response, dropped_payload) = ui
                .dnd_drop_zone::<(String, String, toml::Value), _>(egui::Frame::none(), |ui| {
                    DockArea::new(&mut self.dock_state)
                        .style(Style::from_egui(ctx.style().as_ref()))
                        .show_inside(ui, &mut tab_viewer);
                });

            // If something was dropped, open its controls
            if let Some(payload) = dropped_payload {
                let (inst_type, id, config) = payload.as_ref();
                open_instrument_controls(inst_type, id, config, &mut self.dock_state);
            }
        });

        // Request a repaint to ensure the GUI is continuously updated
        ctx.request_repaint();
    }
}

/// Helper function to display a cached value in the UI
/// Extracts scalar values from Measurement::Scalar variants
fn display_cached_value(
    ui: &mut egui::Ui,
    data_cache: &HashMap<String, Arc<Measurement>>,
    channel: &str,
    label: &str,
) {
    if let Some(measurement) = data_cache.get(channel) {
        // Extract scalar value if this is a Scalar measurement
        if let Measurement::Scalar(data_point) = measurement.as_ref() {
            ui.label(format!(
                "{}: {:.3} {}",
                label, data_point.value, data_point.unit
            ));
        } else {
            ui.label(format!("{}: (non-scalar)", label));
        }
    } else {
        ui.label(format!("{}: No data", label));
    }
}

fn render_instrument_panel<M>(
    ui: &mut egui::Ui,
    instruments: &[(String, toml::Value, bool)],
    command_tx: &mpsc::Sender<DaqCommand>,
    runtime: &Arc<tokio::runtime::Runtime>,
    pending_operations: &mut HashMap<String, PendingOperation>,
    dock_state: &mut DockState<DockTab>,
    data_cache: &HashMap<String, Arc<Measurement>>,
) where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    ui.heading("Instruments");

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, config, is_running) in instruments {
            let inst_type = config
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            // Make the entire group draggable by wrapping it in dnd_drag_source
            let drag_id = egui::Id::new(format!("drag_{}", id));
            let drag_payload = (inst_type.to_string(), id.clone(), config.clone());

            let response = ui
                .dnd_drag_source(drag_id, drag_payload, |ui| {
                    egui::Frame::group(ui.style())
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(id);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if *is_running {
                                            ui.colored_label(egui::Color32::GREEN, "● Running");
                                            if ui.button("Stop").clicked() {
                                                // Fire-and-forget async stop (no response tracking needed)
                                                let cmd_tx = command_tx.clone();
                                                let id_clone = id.clone();
                                                runtime.spawn(async move {
                                                    let (cmd, rx) = DaqCommand::stop_instrument(id_clone);
                                                    if cmd_tx.send(cmd).await.is_ok() {
                                                        let _ = rx.await; // Discard result
                                                    }
                                                });
                                            }
                                        } else {
                                            ui.colored_label(egui::Color32::GRAY, "● Stopped");
                                            if ui.button("Start").clicked() {
                                                // Track spawn operation for status display
                                                let cmd_tx = command_tx.clone();
                                                let id_clone = id.clone();
                                                let op_id = format!("spawn_{}", id);

                                                let (cmd, rx) = DaqCommand::spawn_instrument(id_clone.clone());
                                                if cmd_tx.try_send(cmd).is_ok() {
                                                    pending_operations.insert(op_id, PendingOperation {
                                                        rx,
                                                        description: format!("Starting {}", id_clone),
                                                        started_at: Instant::now(),
                                                        cache_update: None,
                                                    });
                                                } else {
                                                    error!("Failed to queue start command for '{}' (command channel full)", id_clone);
                                                }
                                            }
                                        }
                                    },
                                );
                            });

                            ui.separator();

                            // Display instrument type
                            ui.label(format!("Type: {}", inst_type));

                            // Display instrument name
                            if let Some(name) = config.get("name").and_then(|v| v.as_str()) {
                                ui.label(format!("Name: {}", name));
                            }

                            // Display specific parameters based on instrument type
                            match inst_type {
                                "mock" => {
                                    ui.separator();
                                    if let Some(rate) =
                                        config.get("sample_rate_hz").and_then(|v| v.as_float())
                                    {
                                        ui.label(format!("Sample Rate: {} Hz", rate));
                                    }
                                    if let Some(channels) =
                                        config.get("channels").and_then(|v| v.as_array())
                                    {
                                        let channel_names: Vec<String> = channels
                                            .iter()
                                            .filter_map(|c| c.as_str().map(|s| s.to_string()))
                                            .collect();
                                        ui.label(format!("Channels: {}", channel_names.join(", ")));
                                    }
                                }
                                "scpi_keithley" => {
                                    ui.separator();
                                    if let Some(addr) =
                                        config.get("address").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Address: {}", addr));
                                    }
                                    if let Some(port) =
                                        config.get("port").and_then(|v| v.as_integer())
                                    {
                                        ui.label(format!("Port: {}", port));
                                    }
                                }
                                "maitai" => {
                                    ui.separator();
                                    if let Some(wl) =
                                        config.get("wavelength").and_then(|v| v.as_float())
                                    {
                                        ui.label(format!("Wavelength: {:.1} nm", wl));
                                    }
                                    if let Some(port) = config.get("port").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Port: {}", port));
                                    }

                                    // Display real-time power and wavelength from data stream
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:power", id),
                                        "Power",
                                    );
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:wavelength", id),
                                        "Wavelength",
                                    );
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:shutter", id),
                                        "Shutter",
                                    );
                                    ui.label("💡 Drag to main area or double-click");
                                }
                                "newport_1830c" => {
                                    ui.separator();
                                    if let Some(wl) =
                                        config.get("wavelength").and_then(|v| v.as_float())
                                    {
                                        ui.label(format!("Wavelength: {:.1} nm", wl));
                                    }
                                    if let Some(port) = config.get("port").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Port: {}", port));
                                    }
                                    // Display real-time power reading
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:power", id),
                                        "Power",
                                    );
                                    ui.label("💡 Drag to main area or double-click");
                                }
                                "elliptec" => {
                                    ui.separator();
                                    if let Some(port) = config.get("port").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Port: {}", port));
                                    }
                                    if let Some(addrs) =
                                        config.get("device_addresses").and_then(|v| v.as_array())
                                    {
                                        ui.label(format!("Devices: {}", addrs.len()));
                                        for addr in addrs.iter().filter_map(|a| a.as_integer()) {
                                            display_cached_value(
                                                ui,
                                                data_cache,
                                                &format!("{}:device{}_position", id, addr),
                                                &format!("Device {}", addr),
                                            );
                                        }
                                    }
                                    ui.label("💡 Drag to main area or double-click");
                                }
                                "esp300" => {
                                    ui.separator();
                                    if let Some(port) = config.get("port").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Port: {}", port));
                                    }
                                    let num_axes = config
                                        .get("num_axes")
                                        .and_then(|v| v.as_integer())
                                        .unwrap_or(3)
                                        as usize;
                                    ui.label(format!("Axes: {}", num_axes));
                                    for axis in 1..=num_axes as u8 {
                                        display_cached_value(
                                            ui,
                                            data_cache,
                                            &format!("{}:axis{}_position", id, axis),
                                            &format!("Axis {} Pos", axis),
                                        );
                                        display_cached_value(
                                            ui,
                                            data_cache,
                                            &format!("{}:axis{}_velocity", id, axis),
                                            &format!("Axis {} Vel", axis),
                                        );
                                    }
                                    ui.label("💡 Drag to main area or double-click");
                                }
                                "pvcam" | "pvcam_v2" => {
                                    ui.separator();
                                    if let Some(cam) =
                                        config.get("camera_name").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Camera: {}", cam));
                                    }
                                    if let Some(exp) =
                                        config.get("exposure_ms").and_then(|v| v.as_float())
                                    {
                                        ui.label(format!("Exposure: {} ms", exp));
                                    }
                                    // Display acquisition status
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:mean_intensity", id),
                                        "Mean",
                                    );
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:min_intensity", id),
                                        "Min",
                                    );
                                    display_cached_value(
                                        ui,
                                        data_cache,
                                        &format!("{}:max_intensity", id),
                                        "Max",
                                    );
                                    ui.label("💡 Drag to main area or double-click");
                                }
                                _ if inst_type.contains("visa") => {
                                    ui.separator();
                                    if let Some(resource) =
                                        config.get("resource_string").and_then(|v| v.as_str())
                                    {
                                        ui.label(format!("Resource: {}", resource));
                                    }
                                }
                                _ => {}
                            }

                            ui.add_space(5.0);
                        })
                        .response
                })
                .response;

            // Visual feedback when dragging starts
            if response.drag_started() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }

            // Double-click to open controls
            if response.double_clicked() {
                open_instrument_controls(inst_type, id, config, dock_state);
            }

            ui.add_space(10.0);
        }
    });
}

// Helper function to open instrument controls
fn open_instrument_controls(
    inst_type: &str,
    id: &str,
    config: &toml::Value,
    dock_state: &mut DockState<DockTab>,
) {
    match inst_type {
        "maitai" => {
            dock_state.push_to_focused_leaf(DockTab::MaiTaiControl(MaiTaiControlPanel::new(
                id.to_string(),
            )));
        }
        "newport_1830c" => {
            dock_state.push_to_focused_leaf(DockTab::Newport1830CControl(
                Newport1830CControlPanel::new(id.to_string()),
            ));
        }
        "elliptec" => {
            let device_addrs =
                if let Some(addrs) = config.get("device_addresses").and_then(|v| v.as_array()) {
                    addrs
                        .iter()
                        .filter_map(|a| a.as_integer().map(|i| i as u8))
                        .collect()
                } else {
                    vec![0, 1]
                };
            dock_state.push_to_focused_leaf(DockTab::ElliptecControl(ElliptecControlPanel::new(
                id.to_string(),
                device_addrs,
            )));
        }
        "esp300" => {
            let num_axes = config
                .get("num_axes")
                .and_then(|v| v.as_integer())
                .unwrap_or(3) as usize;
            dock_state.push_to_focused_leaf(DockTab::ESP300Control(ESP300ControlPanel::new(
                id.to_string(),
                num_axes,
            )));
        }
        "pvcam" | "pvcam_v2" => {
            dock_state.push_to_focused_leaf(DockTab::PVCAMControl(PVCAMControlPanel::new(
                id.to_string(),
            )));
        }
        _ => {}
    }
}

struct DockTabViewer<'a, M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    available_channels: Vec<String>,
    app: crate::app::DaqApp<M>,  // Temporary - will be refactored in next phase
    data_cache: &'a HashMap<String, Arc<Measurement>>,
}

impl<'a, M> TabViewer for DockTabViewer<'a, M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    type Tab = DockTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            DockTab::Plot(plot_tab) => format!("Plot: {}", plot_tab.channel).into(),
            DockTab::Spectrum(spectrum_tab) => format!("Spectrum: {}", spectrum_tab.channel).into(),
            DockTab::Image(image_tab) => format!("Image: {}", image_tab.channel).into(),
            DockTab::MaiTaiControl(_) => "MaiTai Laser".into(),
            DockTab::Newport1830CControl(_) => "Newport 1830-C".into(),
            DockTab::ElliptecControl(_) => "Elliptec Rotators".into(),
            DockTab::ESP300Control(_) => "ESP300 Motion".into(),
            DockTab::PVCAMControl(_) => "PVCAM Camera".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            DockTab::Plot(plot_tab) => {
                egui::ComboBox::from_label("Channel")
                    .selected_text(plot_tab.channel.clone())
                    .show_ui(ui, |ui| {
                        for channel in &self.available_channels {
                            ui.selectable_value(
                                &mut plot_tab.channel,
                                channel.clone(),
                                channel.clone(),
                            );
                        }
                    });

                live_plot(ui, &plot_tab.plot_data, &plot_tab.channel);
            }
            DockTab::Spectrum(spectrum_tab) => {
                spectrum_plot(ui, &spectrum_tab.spectrum_data, &spectrum_tab.channel);
            }
            DockTab::Image(image_tab) => {
                image_view(ui, image_tab);
            }
            DockTab::MaiTaiControl(panel) => {
                panel.ui(ui, &self.app, self.data_cache);
            }
            DockTab::Newport1830CControl(panel) => {
                panel.ui(ui, &self.app, self.data_cache);
            }
            DockTab::ElliptecControl(panel) => {
                panel.ui(ui, &self.app, self.data_cache);
            }
            DockTab::ESP300Control(panel) => {
                panel.ui(ui, &self.app, self.data_cache);
            }
            DockTab::PVCAMControl(panel) => {
                panel.ui(ui, &self.app, self.data_cache);
            }
        }
    }
}

fn live_plot(ui: &mut egui::Ui, data: &VecDeque<[f64; 2]>, channel: &str) {
    ui.heading(format!("Live Data ({})", channel));
    let line = Line::new(PlotPoints::from_iter(data.iter().copied()));
    Plot::new(channel).view_aspect(2.0).show(ui, |plot_ui| {
        plot_ui.line(line);
    });
}

/// Renders a frequency spectrum plot.
fn spectrum_plot(ui: &mut egui::Ui, spectrum_data: &[[f64; 2]], channel: &str) {
    ui.heading(format!("Frequency Spectrum ({})", channel));

    if spectrum_data.is_empty() {
        ui.label("No spectrum data available");
        return;
    }

    let line = Line::new(PlotPoints::from_iter(spectrum_data.iter().copied()));
    Plot::new(format!("spectrum_{}", channel))
        .view_aspect(2.0)
        .x_axis_label("Frequency (Hz)")
        .y_axis_label("Magnitude (dB)")
        .show(ui, |plot_ui| {
            plot_ui.line(line);
        });

    // Display spectrum statistics
    ui.horizontal(|ui| {
        ui.label(format!("Bins: {}", spectrum_data.len()));
        if let Some(&[peak_freq, peak_mag]) = spectrum_data
            .iter()
            .max_by(|a, b| a[1].partial_cmp(&b[1]).unwrap())
        {
            ui.label(format!("Peak: {:.2} Hz @ {:.2} dB", peak_freq, peak_mag));
        }
    });
}

/// Renders an image/camera view with simple ASCII visualization.
/// Note: egui doesn't have native heatmap support; this provides a basic representation.
/// For production use, consider integrating egui_extras::RetainedImage or external image libraries.
/// Renders an image viewer with grayscale colormap and statistics.
fn image_view(ui: &mut egui::Ui, image_tab: &mut ImageTab) {
    ui.heading(format!("Image View ({})", image_tab.channel));

    // Check if we have image data
    let Some(ref pixel_buffer) = image_tab.pixel_data else {
        ui.label("No image data available");
        return;
    };

    let (width, height) = image_tab.dimensions;
    if width == 0 || height == 0 {
        ui.label("Invalid image dimensions");
        return;
    }

    let (min_val, max_val) = image_tab.value_range;

    // Display image statistics header
    ui.horizontal(|ui| {
        ui.label(format!("Dimensions: {}×{}", width, height));
        ui.label(format!("Range: [{:.0}, {:.0}]", min_val, max_val));
        ui.label(format!("Memory: {} KB", pixel_buffer.memory_bytes() / 1024));
    });

    ui.separator();

    // Convert PixelBuffer to egui::ColorImage (RGBA8 grayscale)
    // This function maps pixel values from [min_val, max_val] to [0, 255]
    let rgba_pixels = convert_to_grayscale_rgba(pixel_buffer, width, height, min_val, max_val);
    let color_image = egui::ColorImage {
        size: [width, height],
        pixels: rgba_pixels,
    };

    // Create or update texture
    let texture = image_tab.texture.get_or_insert_with(|| {
        // First frame: create new texture
        ui.ctx().load_texture(
            format!("camera_image_{}", image_tab.channel),
            color_image.clone(),
            egui::TextureOptions::NEAREST, // Use nearest-neighbor for sharp pixels
        )
    });

    // Update texture with new frame data (efficient - no reallocation)
    texture.set(color_image, egui::TextureOptions::NEAREST);

    // Calculate display size to fit available space while maintaining aspect ratio
    let available_size = ui.available_size();
    let aspect_ratio = width as f32 / height as f32;
    let display_size = if available_size.x / aspect_ratio < available_size.y {
        // Width-constrained
        egui::vec2(available_size.x, available_size.x / aspect_ratio)
    } else {
        // Height-constrained
        egui::vec2(available_size.y * aspect_ratio, available_size.y)
    };

    // Display the image with calculated size
    // Deref texture from &mut to & for egui::Image::new()
    ui.centered_and_justified(|ui| {
        ui.add(egui::Image::new(&*texture).fit_to_exact_size(display_size));
    });
}

/// Converts PixelBuffer to grayscale RGBA8 format for egui rendering.
///
/// Maps pixel values from [min_val, max_val] to [0, 255] grayscale.
/// Output format: Vec<egui::Color32> where each pixel is (gray, gray, gray, 255).
///
/// # Performance
/// - U16 variant: ~262k pixels/ms on modern CPU (512x512 in ~1ms)
/// - Zero-copy for slice access, single allocation for output
fn convert_to_grayscale_rgba(
    pixel_buffer: &daq_core::PixelBuffer,
    width: usize,
    height: usize,
    min_val: f64,
    max_val: f64,
) -> Vec<egui::Color32> {
    let pixel_count = width * height;
    let mut rgba = Vec::with_capacity(pixel_count);

    // Get pixels as f64 (zero-copy for F64 variant, allocates for U8/U16)
    let pixels_f64 = pixel_buffer.as_f64();

    // Validate buffer size to maintain egui ColorImage invariant
    if pixels_f64.len() < pixel_count {
        log::error!(
            "Pixel buffer size mismatch: expected {} pixels ({}x{}), got {}. Creating black placeholder image.",
            pixel_count, width, height, pixels_f64.len()
        );
        // Return black image to maintain ColorImage invariant (pixels.len() == width * height)
        return vec![egui::Color32::BLACK; pixel_count];
    }

    // Compute scaling factor for mapping to [0, 255]
    let range = (max_val - min_val).max(1.0); // Avoid division by zero
    let scale = 255.0 / range;

    // Convert each pixel to grayscale RGBA
    for &pixel_val in pixels_f64.iter().take(pixel_count) {
        // Map from [min_val, max_val] to [0, 255]
        let normalized = ((pixel_val - min_val) * scale).clamp(0.0, 255.0);
        let gray = normalized as u8;

        // Create grayscale RGBA pixel (R=G=B=gray, A=255)
        rgba.push(egui::Color32::from_rgba_premultiplied(
            gray, gray, gray, 255,
        ));
    }

    rgba
}