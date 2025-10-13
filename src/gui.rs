//! The eframe/egui implementation for the GUI.
use crate::{
    app::{DaqApp, DaqAppInner},
    core::DataPoint,
    log_capture::LogBuffer,
};
use eframe::egui;
use egui_dock::{DockArea, DockState, Style, TabViewer};
use egui_plot::{Line, Plot, PlotPoints};
use log::{error, LevelFilter};
use std::collections::VecDeque;
use tokio::sync::broadcast;

mod log_panel;

const PLOT_DATA_CAPACITY: usize = 1000;

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
    plot_data: VecDeque<[f64; 2]>,
    last_timestamp: f64,
    // Log panel state
    log_filter_text: String,
    log_level_filter: LevelFilter,
    scroll_to_bottom: bool,
}

impl Gui {
    /// Creates a new GUI.
    pub fn new(_cc: &eframe::CreationContext<'_>, app: DaqApp) -> Self {
        let (data_receiver, log_buffer) = app.with_inner(|inner| {
            (inner.data_sender.subscribe(), inner.log_buffer.clone())
        });
        Self {
            app,
            data_receiver,
            log_buffer,
            plot_data: VecDeque::with_capacity(PLOT_DATA_CAPACITY),
            last_timestamp: 0.0,
            log_filter_text: String::new(),
            log_level_filter: LevelFilter::Info,
            scroll_to_bottom: true,
        }
    }

    /// Fetches new data points from the broadcast channel.
    fn update_data(&mut self) {
        while let Ok(data_point) = self.data_receiver.try_recv() {
            for (_location, tab) in self.dock_state.iter_all_tabs_mut() {
                if tab.channel == data_point.channel {
                    if tab.plot_data.len() >= PLOT_DATA_CAPACITY {
                        tab.plot_data.pop_front();
                    }
                    let timestamp = data_point.timestamp.timestamp_micros() as f64 / 1_000_000.0;
                    if tab.last_timestamp == 0.0 {
                        tab.last_timestamp = timestamp;
                    }
                    tab.plot_data
                        .push_back([timestamp - tab.last_timestamp, data_point.value]);
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

        self.app.with_inner(|inner| {
            let available_channels: Vec<String> = inner.get_available_channels();

            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Rust DAQ Control Panel");
                    ui.separator();

                    egui::ComboBox::from_label("Channel")
                        .selected_text(self.selected_channel.clone())
                        .show_ui(ui, |ui| {
                            for channel in &available_channels {
                                ui.selectable_value(&mut self.selected_channel, channel.clone(), channel.clone());
                            }
                        });

                    if ui.button("Add Plot").clicked() {
                        self.dock_state.push_to_focused_leaf(PlotTab::new(self.selected_channel.clone()));
                    }
                });
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                instrument_control_panel(ui, inner);
            });

            let mut tab_viewer = PlotTabViewer {
                available_channels,
            };

            DockArea::new(&mut self.dock_state)
                .style(Style::from_egui(ctx.style().as_ref()))
                .show(ctx, &mut tab_viewer);
        });

        // Request a repaint to ensure the GUI is continuously updated
        ctx.request_repaint();
    }
}

fn instrument_control_panel(ui: &mut egui::Ui, inner: &mut DaqAppInner) {
    ui.heading("Instruments");
    let available_instruments: Vec<String> = inner.instrument_registry.list().collect();

    for id in available_instruments {
        ui.horizontal(|ui| {
            ui.label(id.clone());
            let is_running = inner.instruments.contains_key(&id);
            if is_running {
                if ui.button("Stop").clicked() {
                    inner.stop_instrument(&id);
                }
            } else if ui.button("Start").clicked() {
                if let Err(e) = inner.spawn_instrument(&id) {
                    error!("Failed to start instrument '{}': {}", id, e);
                }
            }
        });
    }
}

struct PlotTabViewer {
    available_channels: Vec<String>,
}

impl TabViewer for PlotTabViewer {
    type Tab = PlotTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.channel.clone().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        egui::ComboBox::from_label("Channel")
            .selected_text(tab.channel.clone())
            .show_ui(ui, |ui| {
                for channel in &self.available_channels {
                    ui.selectable_value(&mut tab.channel, channel.clone(), channel.clone());
                }
            });

        live_plot(ui, &tab.plot_data, &tab.channel);
    }
}

fn live_plot(ui: &mut egui::Ui, data: &VecDeque<[f64; 2]>, channel: &str) {
    ui.heading(format!("Live Data ({})", channel));
    let line = Line::new(PlotPoints::from_iter(data.iter().copied()));
    Plot::new(channel).view_aspect(2.0).show(ui, |plot_ui| {
        plot_ui.line(line);
    });
}
