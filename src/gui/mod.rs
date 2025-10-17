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

pub mod storage_manager;
pub mod instrument_controls;

use self::storage_manager::StorageManager;
use self::instrument_controls::*;
use crate::{
    app::DaqApp,
    core::DataPoint,
    log_capture::LogBuffer,
};
use eframe::egui;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use egui_plot::{Line, Plot, PlotPoints};
use log::{error, LevelFilter};
use std::collections::{HashMap, VecDeque};
use tokio::sync::broadcast;

mod log_panel;

const PLOT_DATA_CAPACITY: usize = 1000;

/// Represents the different types of tabs that can be docked
enum DockTab {
    Plot(PlotTab),
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

/// The main GUI struct.
pub struct Gui {
    app: DaqApp,
    data_receiver: broadcast::Receiver<DataPoint>,
    log_buffer: LogBuffer,
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
    /// Value: latest DataPoint for that channel
    /// This provides single source of truth for instrument state in GUI
    data_cache: HashMap<String, DataPoint>,
}

impl Gui {
    /// Creates a new GUI.
    pub fn new(_cc: &eframe::CreationContext<'_>, app: DaqApp) -> Self {
        let (data_receiver, log_buffer) =
            app.with_inner(|inner| (inner.data_sender.subscribe(), inner.log_buffer.clone()));

        let mut dock_state = DockState::new(vec![
            DockTab::Plot(PlotTab::new("sine_wave".to_string()))
        ]);
        dock_state.push_to_focused_leaf(DockTab::Plot(PlotTab::new("cosine_wave".to_string())));

        Self {
            app,
            data_receiver,
            log_buffer,
            dock_state,
            selected_channel: "sine_wave".to_string(),
            storage_manager: StorageManager::new(),
            show_storage: false,
            log_filter_text: String::new(),
            log_level_filter: LevelFilter::Info,
            scroll_to_bottom: true,
            data_cache: HashMap::new(),
        }
    }

    /// Fetches new data points from the broadcast channel and updates the data cache.
    /// Updates both plot tabs and the centralized data cache for instrument control panels.
    fn update_data(&mut self) {
        while let Ok(data_point) = self.data_receiver.try_recv() {
            // Update the central data cache with instrument_id:channel key
            let cache_key = format!("{}:{}", data_point.instrument_id, data_point.channel);
            self.data_cache.insert(cache_key, data_point.clone());

            // Update any relevant plot tabs
            for (_location, tab) in self.dock_state.iter_all_tabs_mut() {
                if let DockTab::Plot(plot_tab) = tab {
                    if plot_tab.channel == data_point.channel {
                        if plot_tab.plot_data.len() >= PLOT_DATA_CAPACITY {
                            plot_tab.plot_data.pop_front();
                        }
                        let timestamp =
                            data_point.timestamp.timestamp_micros() as f64 / 1_000_000.0;
                        if plot_tab.last_timestamp == 0.0 {
                            plot_tab.last_timestamp = timestamp;
                        }
                        plot_tab.plot_data.push_back([
                            timestamp - plot_tab.last_timestamp,
                            data_point.value,
                        ]);
                    }
                }
            }
        }
    }
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_data();

        egui::TopBottomPanel::bottom("bottom_panel")
            .resizable(true)
            .min_height(150.0)
            .show(ctx, |ui| {
                log_panel::render(ui, self);
            });

        // Collect instrument data first
        let (instruments, available_channels) = self.app.with_inner(|inner| {
            let instruments: Vec<(String, toml::Value, bool)> = inner
                .settings
                .instruments
                .iter()
                .map(|(k, v)| (k.clone(), v.clone(), inner.instruments.contains_key(k)))
                .collect();
            (instruments, inner.get_available_channels())
        });

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
                        .push_to_focused_leaf(DockTab::Plot(PlotTab::new(self.selected_channel.clone())));
                }

                ui.separator();

                // Instrument control buttons
                egui::menu::menu_button(ui, "Instrument Controls", |ui| {
                    if ui.button("🔬 MaiTai Laser").clicked() {
                        self.dock_state.push_to_focused_leaf(
                            DockTab::MaiTaiControl(MaiTaiControlPanel::new("maitai".to_string()))
                        );
                        ui.close_menu();
                    }
                    if ui.button("📊 Newport 1830-C").clicked() {
                        self.dock_state.push_to_focused_leaf(
                            DockTab::Newport1830CControl(Newport1830CControlPanel::new("newport_1830c".to_string()))
                        );
                        ui.close_menu();
                    }
                    if ui.button("🔄 Elliptec Rotators").clicked() {
                        self.dock_state.push_to_focused_leaf(
                            DockTab::ElliptecControl(ElliptecControlPanel::new("elliptec".to_string(), vec![0, 1]))
                        );
                        ui.close_menu();
                    }
                    if ui.button("⚙️ ESP300 Motion").clicked() {
                        self.dock_state.push_to_focused_leaf(
                            DockTab::ESP300Control(ESP300ControlPanel::new("esp300".to_string(), 3))
                        );
                        ui.close_menu();
                    }
                    if ui.button("📷 PVCAM Camera").clicked() {
                        self.dock_state.push_to_focused_leaf(
                            DockTab::PVCAMControl(PVCAMControlPanel::new("pvcam".to_string()))
                        );
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
                    self.storage_manager.ui(ui, &self.app);
                });
        }

        egui::SidePanel::left("control_panel")
            .resizable(true)
            .min_width(200.0)
            .show(ctx, |ui| {
                render_instrument_panel(ui, &instruments, &self.app, &mut self.dock_state, &self.data_cache);
            });

        let mut tab_viewer = DockTabViewer {
            available_channels,
            app: &self.app,
            data_cache: &self.data_cache,
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            // Check for dropped instruments
            let (_inner_response, dropped_payload) = ui.dnd_drop_zone::<(String, String, toml::Value), _>(
                egui::Frame::none(),
                |ui| {
                    DockArea::new(&mut self.dock_state)
                        .style(Style::from_egui(ctx.style().as_ref()))
                        .show_inside(ui, &mut tab_viewer);
                },
            );

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
fn display_cached_value(
    ui: &mut egui::Ui,
    data_cache: &HashMap<String, DataPoint>,
    channel: &str,
    label: &str,
) {
    if let Some(data_point) = data_cache.get(channel) {
        ui.label(format!(
            "{}: {:.3} {}",
            label, data_point.value, data_point.unit
        ));
    } else {
        ui.label(format!("{}: No data", label));
    }
}

fn render_instrument_panel(
    ui: &mut egui::Ui,
    instruments: &[(String, toml::Value, bool)],
    app: &DaqApp,
    dock_state: &mut DockState<DockTab>,
    data_cache: &HashMap<String, DataPoint>,
) {
    ui.heading("Instruments");

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, config, is_running) in instruments {
            let inst_type = config.get("type").and_then(|v| v.as_str()).unwrap_or("");

            // Make the entire group draggable by wrapping it in dnd_drag_source
            let drag_id = egui::Id::new(format!("drag_{}", id));
            let drag_payload = (inst_type.to_string(), id.clone(), config.clone());

            let response = ui.dnd_drag_source(drag_id, drag_payload, |ui| {
                egui::Frame::group(ui.style())
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(id);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if *is_running {
                                    ui.colored_label(egui::Color32::GREEN, "● Running");
                                    if ui.button("Stop").clicked() {
                                        app.with_inner(|inner| inner.stop_instrument(id));
                                    }
                                } else {
                                    ui.colored_label(egui::Color32::GRAY, "● Stopped");
                                    if ui.button("Start").clicked() {
                                        app.with_inner(|inner| {
                                            if let Err(e) = inner.spawn_instrument(id) {
                                                error!("Failed to start instrument '{}': {}", id, e);
                                            }
                                        });
                                    }
                                }
                            });
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
                            if let Some(rate) = config.get("sample_rate_hz").and_then(|v| v.as_float()) {
                                ui.label(format!("Sample Rate: {} Hz", rate));
                            }
                            if let Some(channels) = config.get("channels").and_then(|v| v.as_array()) {
                                let channel_names: Vec<String> = channels
                                    .iter()
                                    .filter_map(|c| c.as_str().map(|s| s.to_string()))
                                    .collect();
                                ui.label(format!("Channels: {}", channel_names.join(", ")));
                            }
                        }
                        "scpi_keithley" => {
                            ui.separator();
                            if let Some(addr) = config.get("address").and_then(|v| v.as_str()) {
                                ui.label(format!("Address: {}", addr));
                            }
                            if let Some(port) = config.get("port").and_then(|v| v.as_integer()) {
                                ui.label(format!("Port: {}", port));
                            }
                        }
                        "maitai" => {
                            ui.separator();
                            if let Some(wl) = config.get("wavelength").and_then(|v| v.as_float()) {
                                ui.label(format!("Wavelength: {:.1} nm", wl));
                            }
                            if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
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
                            if let Some(wl) = config.get("wavelength").and_then(|v| v.as_float()) {
                                ui.label(format!("Wavelength: {:.1} nm", wl));
                            }
                            if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
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
                            if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
                                ui.label(format!("Port: {}", port));
                            }
                            if let Some(addrs) = config.get("device_addresses").and_then(|v| v.as_array()) {
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
                            if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
                                ui.label(format!("Port: {}", port));
                            }
                            let num_axes = config.get("num_axes").and_then(|v| v.as_integer()).unwrap_or(3) as usize;
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
                        "pvcam" => {
                            ui.separator();
                            if let Some(cam) = config.get("camera_name").and_then(|v| v.as_str()) {
                                ui.label(format!("Camera: {}", cam));
                            }
                            if let Some(exp) = config.get("exposure_ms").and_then(|v| v.as_float()) {
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
                            if let Some(resource) = config.get("resource_string").and_then(|v| v.as_str()) {
                                ui.label(format!("Resource: {}", resource));
                            }
                        }
                        _ => {}
                    }

                    ui.add_space(5.0);
                })
                .response
            }).response;

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
            dock_state.push_to_focused_leaf(
                DockTab::MaiTaiControl(MaiTaiControlPanel::new(id.to_string()))
            );
        }
        "newport_1830c" => {
            dock_state.push_to_focused_leaf(
                DockTab::Newport1830CControl(Newport1830CControlPanel::new(id.to_string()))
            );
        }
        "elliptec" => {
            let device_addrs = if let Some(addrs) = config.get("device_addresses").and_then(|v| v.as_array()) {
                addrs.iter().filter_map(|a| a.as_integer().map(|i| i as u8)).collect()
            } else {
                vec![0, 1]
            };
            dock_state.push_to_focused_leaf(
                DockTab::ElliptecControl(ElliptecControlPanel::new(id.to_string(), device_addrs))
            );
        }
        "esp300" => {
            let num_axes = config.get("num_axes").and_then(|v| v.as_integer()).unwrap_or(3) as usize;
            dock_state.push_to_focused_leaf(
                DockTab::ESP300Control(ESP300ControlPanel::new(id.to_string(), num_axes))
            );
        }
        "pvcam" => {
            dock_state.push_to_focused_leaf(
                DockTab::PVCAMControl(PVCAMControlPanel::new(id.to_string()))
            );
        }
        _ => {}
    }
}

struct DockTabViewer<'a> {
    available_channels: Vec<String>,
    app: &'a DaqApp,
    data_cache: &'a HashMap<String, DataPoint>,
}

impl<'a> TabViewer for DockTabViewer<'a> {
    type Tab = DockTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            DockTab::Plot(plot_tab) => plot_tab.channel.clone().into(),
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
                            ui.selectable_value(&mut plot_tab.channel, channel.clone(), channel.clone());
                        }
                    });

                live_plot(ui, &plot_tab.plot_data, &plot_tab.channel);
            }
            DockTab::MaiTaiControl(panel) => {
                panel.ui(ui, self.app, self.data_cache);
            }
            DockTab::Newport1830CControl(panel) => {
                panel.ui(ui, self.app, self.data_cache);
            }
            DockTab::ElliptecControl(panel) => {
                panel.ui(ui, self.app, self.data_cache);
            }
            DockTab::ESP300Control(panel) => {
                panel.ui(ui, self.app, self.data_cache);
            }
            DockTab::PVCAMControl(panel) => {
                panel.ui(ui, self.app, self.data_cache);
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
