# GUI Development Guide: egui-based Scientific Interface (V5, Headless-First)

## Overview

This guide covers developing the graphical user interface for the Rust DAQ application using `egui`, a modern immediate‑mode GUI framework.  
In the current V5 **headless‑first** architecture:

- The **core DAQ** runs as a standalone daemon (`rust_daq` bin).
- The daemon exposes control and data over **gRPC** (`ControlService`, `HardwareService`, `ScanService`, etc.).
- The primary GUI is a native `egui` / `eframe` desktop binary: **`rust_daq_gui_egui`**.
- The GUI talks to the daemon purely as a **gRPC client** – no direct actor wiring or in‑process coupling.

## 1. Core GUI Architecture

### Main Window Structure

The main window holds the `eframe::App` implementation and manages the overall UI state and panel interactions.  
Instead of talking to an in‑process `InstrumentManager` actor, it talks to the **remote daemon** via gRPC clients from `rust_daq::grpc`.

```rust
use eframe::egui;
use rust_daq::grpc::{
    HardwareServiceClient,
    ListDevicesRequest,
    ReadValueRequest,
    DeviceInfo,
};
use tonic::transport::Channel;

pub struct MainWindow {
    hardware: HardwareServiceClient<Channel>,
    ui_state: UiState,
    panels: GuiPanels,
    // Optional channels to receive streamed data from daemon
    // measurement_rx: tokio::sync::mpsc::Receiver<DataPoint>,
}

#[derive(Default)]
pub struct UiState {
    pub selected_instrument_id: Option<String>,
    pub show_settings: bool,
    pub plot_paused: bool,
    pub auto_scale: bool,
    pub data_recording: bool,
}

pub struct GuiPanels {
    pub instrument_panel: InstrumentPanel,
    pub plot_panel: PlotPanel,
    pub control_panel: ControlPanel,
    pub status_panel: StatusPanel,
    pub log_panel: LogPanel,
}

impl eframe::App for MainWindow {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.setup_style(ctx);
        self.create_menu_bar(ctx, frame);
        self.create_main_panels(ctx);

        // Process incoming data from daemon streams here (if configured).
        // For example:
        // while let Ok(data_point) = self.measurement_rx.try_recv() {
        //     self.panels.plot_panel.add_scalar(data_point);
        // }

        // Request repaint for real-time updates
        ctx.request_repaint();
    }
}
```

### Panel-Based Layout
The GUI will be organized into modular panels, each responsible for a specific part of the interface.

```rust
impl MainWindow {
    fn create_main_panels(&mut self, ctx: &egui::Context) {
        // Left sidebar for instruments and controls
        egui::SidePanel::left("instruments")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                self.panels.instrument_panel.show(ui, &mut self.ui_state, &self.instrument_manager);
                ui.separator();
                self.panels.control_panel.show(ui, &mut self.ui_state, &self.instrument_manager);
            });

        // Bottom panel for logs and status
        egui::TopBottomPanel::bottom("status")
            .resizable(true)
            .default_height(150.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.panels.log_panel.show(ui);
                });
            });

        // Central panel for plots and data visualization
        egui::CentralPanel::default().show(ctx, |ui| {
            self.panels.plot_panel.show(ui, &mut self.ui_state);
        });
    }
}
```

## 2. Real-Time Data Visualization with `egui_plot` and Arrow

The `PlotPanel` will receive `RecordBatch`es and render them using `egui_plot`.

### Plot Panel Implementation
```rust
use egui_plot::{Line, Plot, PlotPoints, PlotResponse, PlotUi};
use std::collections::HashMap;
use arrow::record_batch::RecordBatch;
use arrow::array::{Float64Array, UInt64Array, StringArray};

pub struct PlotPanel {
    // Buffer to store recent data for plotting, organized by channel
    data_buffers: HashMap<String, Vec<(f64, f64)>>, // (timestamp, value)
    max_points_per_channel: usize,
    // ... other plot state
}

impl PlotPanel {
    pub fn new() -> Self {
        Self {
            data_buffers: HashMap::new(),
            max_points_per_channel: 10000,
            // ...
        }
    }

    pub fn add_data(&mut self, record_batch: RecordBatch) {
        // Extract data from RecordBatch and add to buffers
        let timestamps = record_batch.column_by_name("timestamp_ns")
            .and_then(|col| col.as_any().downcast_ref::<UInt64Array>())
            .expect("RecordBatch must have 'timestamp_ns' column");
        let channels = record_batch.column_by_name("channel")
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
            .expect("RecordBatch must have 'channel' column");
        let values = record_batch.column_by_name("value")
            .and_then(|col| col.as_any().downcast_ref::<Float64Array>())
            .expect("RecordBatch must have 'value' column");

        for i in 0..record_batch.num_rows() {
            let timestamp_s = timestamps.value(i) as f64 / 1_000_000_000.0; // Convert ns to s
            let channel_name = channels.value(i).to_string();
            let value = values.value(i);

            let buffer = self.data_buffers.entry(channel_name).or_default();
            buffer.push((timestamp_s, value));
            // Maintain buffer size
            while buffer.len() > self.max_points_per_channel {
                buffer.remove(0);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, ui_state: &mut UiState) {
        ui.horizontal(|ui| {
            ui.label("Data Visualization");
            ui.separator();
            
            if ui.button("⏸ Pause").clicked() {
                ui_state.plot_paused = !ui_state.plot_paused;
            }
            
            ui.checkbox(&mut ui_state.auto_scale, "Auto Scale");
            
            if ui.button("📋 Export").clicked() {
                // Trigger data export via InstrumentManager actor
            }
        });

        let plot = Plot::new("main_plot")
            .view_aspect(2.0)
            .auto_bounds_x()
            .auto_bounds_y()
            .allow_zoom(true)
            .allow_drag(true)
            .allow_scroll(true)
            .show_axes([true, true])
            .show_grid(true);

        plot.show(ui, |plot_ui| {
            if !ui_state.plot_paused {
                self.draw_data_lines(plot_ui);
            }
            // ... other plot elements like cursors
        });
    }

    fn draw_data_lines(&self, plot_ui: &mut PlotUi) {
        for (channel_name, points) in &self.data_buffers {
            let line = Line::new(PlotPoints::new(points.clone()))
                .color(self.get_channel_color(channel_name))
                .width(2.0)
                .name(channel_name);
            
            plot_ui.line(line);
        }
    }

    fn get_channel_color(&self, channel_name: &str) -> egui::Color32 {
        // Simple hash-based color assignment for consistency
        let hash = channel_name.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
        match hash % 8 {
            0 => egui::Color32::from_rgb(255, 0, 0),     // Red
            1 => egui::Color32::from_rgb(0, 255, 0),     // Green
            2 => egui::Color32::from_rgb(0, 0, 255),     // Blue
            3 => egui::Color32::from_rgb(255, 255, 0),   // Yellow
            4 => egui::Color32::from_rgb(255, 0, 255),   // Magenta
            5 => egui::Color32::from_rgb(0, 255, 255),   // Cyan
            6 => egui::Color32::from_rgb(255, 165, 0),   // Orange
            7 => egui::Color32::from_rgb(128, 0, 128),   // Purple
            _ => egui::Color32::GRAY,
        }
    }
}
```

## 3. Instrument Control Panel (Actor Interaction)

The `InstrumentPanel` will send commands to and receive status updates from individual instrument actors via the `InstrumentManager`.

```rust
use kameo::ActorRef;
use crate::core::messages::{InstrumentCommand, InstrumentStatusUpdate}; // Define these messages
use crate::core::InstrumentActor; // Reference to individual instrument actors

pub struct InstrumentPanel {
    instrument_manager: ActorRef<InstrumentManagerActor>,
    instrument_status: HashMap<String, InstrumentStatusUpdate>, // Map instrument ID to its status
    // ... other state
}

impl InstrumentPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, ui_state: &mut UiState, instrument_manager: &ActorRef<InstrumentManagerActor>) {
        ui.heading("Instruments");
        
        // Instrument list
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for (id, status_update) in &self.instrument_status {
                    let selected = ui_state.selected_instrument_id
                        .as_ref()
                        .map(|s| s == id)
                        .unwrap_or(false);
                    
                    if ui.selectable_label(selected, id).clicked() {
                        ui_state.selected_instrument_id = Some(id.clone());
                    }
                    
                    ui.same_line();
                    self.show_status_indicator(ui, &status_update.status);
                }
            });

        ui.separator();

        // Selected instrument controls
        if let Some(selected_id) = &ui_state.selected_instrument_id {
            if let Some(status_update) = self.instrument_status.get(selected_id) {
                self.show_selected_instrument_controls(ui, selected_id, status_update, instrument_manager);
            }
        }
    }

    fn show_status_indicator(&self, ui: &mut egui::Ui, status: &str) {
        let (color, text) = match status {
            "Disconnected" => (egui::Color32::RED, "●"),
            "Connecting" => (egui::Color32::YELLOW, "●"),
            "Connected" => (egui::Color32::GREEN, "●"),
            "Acquiring" => (egui::Color32::BLUE, "●"),
            _ => (egui::Color32::DARK_RED, "●"), // Error or unknown
        };
        
        ui.colored_label(color, text);
    }

    fn show_selected_instrument_controls(&mut self, ui: &mut egui::Ui, id: &str, status_update: &InstrumentStatusUpdate, instrument_manager: &ActorRef<InstrumentManagerActor>) {
        ui.heading(id);
        
        // Connection controls
        ui.horizontal(|ui| {
            match status_update.status.as_str() {
                "Disconnected" => {
                    if ui.button("Connect").clicked() {
                        // Send connect command to InstrumentManager
                        let _ = instrument_manager.send(InstrumentCommand::Connect(id.to_string()));
                    }
                }
                "Connected" => {
                    if ui.button("Disconnect").clicked() {
                        let _ = instrument_manager.send(InstrumentCommand::Disconnect(id.to_string()));
                    }
                    
                    if ui.button("Start Acquisition").clicked() {
                        let _ = instrument_manager.send(InstrumentCommand::StartAcquisition(id.to_string()));
                    }
                }
                "Acquiring" => {
                    if ui.button("Stop Acquisition").clicked() {
                        let _ = instrument_manager.send(InstrumentCommand::StopAcquisition(id.to_string()));
                    }
                }
                _ => {}
            }
        });

        ui.separator();

        // Parameter controls (example)
        // These would be dynamically generated based on instrument capabilities
        ui.label(format!("Current Value: {}", status_update.current_value));
        if ui.button("Set Parameter X").clicked() {
            let _ = instrument_manager.send(InstrumentCommand::SetParameter(id.to_string(), "ParamX".to_string(), "123.4".to_string()));
        }
    }
}
```

## 4. Styling and Themes

### Custom Styling
`egui` allows extensive customization of its appearance to match application branding or user preferences.

```rust
impl MainWindow {
    fn setup_style(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        
        // Scientific dark theme
        style.visuals = egui::Visuals {
            dark_mode: true,
            override_text_color: Some(egui::Color32::from_gray(240)),
            widgets: egui::style::Widgets {
                noninteractive: egui::style::WidgetVisuals {
                    bg_fill: egui::Color32::from_gray(27),
                    bg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    fg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(240)),
                    rounding: egui::Rounding::same(4.0),
                    expansion: 0.0,
                },
                inactive: egui::style::WidgetVisuals {
                    bg_fill: egui::Color32::from_gray(35),
                    bg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    fg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(180)),
                    rounding: egui::Rounding::same(4.0),
                    expansion: 0.0,
                },
                hovered: egui::style::WidgetVisuals {
                    bg_fill: egui::Color32::from_gray(45),
                    bg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
                    fg_stroke: egui::Stroke::new(1.5, egui::Color32::from_gray(240)),
                    rounding: egui::Rounding::same(4.0),
                    expansion: 1.0,
                },
                active: egui::style::WidgetVisuals {
                    bg_fill: egui::Color32::from_gray(55),
                    bg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                    fg_stroke: egui::Stroke::new(2.0, egui::Color32::WHITE),
                    rounding: egui::Rounding::same(4.0),
                    expansion: 1.0,
                },
                open: egui::style::WidgetVisuals {
                    bg_fill: egui::Color32::from_gray(27),
                    bg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    fg_stroke: egui::Stroke::new(1.0, egui::Color32::from_gray(210)),
                    rounding: egui::Rounding::same(4.0),
                    expansion: 0.0,
                },
            },
            selection: egui::style::Selection {
                bg_fill: egui::Color32::from_rgb(0, 92, 128),
                stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(192, 222, 255)),
            },
            ..egui::Visuals::dark()
        };
        
        ctx.set_style(style);
    }
}
```

## 5. Performance Optimization

`egui` is inherently efficient, but for real-time data visualization, further optimizations can be applied.

### Efficient Data Handling for Plotting
Instead of copying large amounts of data, `egui_plot` can be fed directly from Arrow `RecordBatch`es or optimized buffers. Downsampling and caching can also improve performance.

```rust
impl PlotPanel {
    pub fn show_optimized(&mut self, ui: &mut egui::Ui, ui_state: &UiState) {
        // Only update plot data if not paused and new data has arrived
        if !ui_state.plot_paused && self.has_new_data { // `has_new_data` would be set when `add_data` is called
            self.update_plot_cache();
            self.has_new_data = false;
        }
        
        let plot = Plot::new("main_plot")
            .auto_bounds_x()
            .auto_bounds_y();
            
        plot.show(ui, |plot_ui| {
            // Render from cached lines
            for line in &self.cached_lines {
                plot_ui.line(line.clone());
            }
        });
    }
    
    fn update_plot_cache(&mut self) {
        self.cached_lines.clear();
        
        // Example: Downsample data from `data_buffers` for better performance
        for (channel_name, points) in &self.data_buffers {
            let downsampled_points = self.downsample_points(points, 1000); // Custom downsampling logic
            let line = Line::new(PlotPoints::new(downsampled_points))
                .color(self.get_channel_color(channel_name))
                .width(2.0)
                .name(channel_name);
            self.cached_lines.push(line);
        }
    }

    fn downsample_points(&self, points: &Vec<(f64, f64)>, max_points: usize) -> Vec<(f64, f64)> {
        if points.len() <= max_points {
            return points.clone();
        }
        // Simple stride-based downsampling
        let stride = points.len() / max_points;
        points.iter().step_by(stride).cloned().collect()
    }
}
```

This GUI development guide provides the foundation for creating a professional, responsive, and feature-rich interface for scientific data acquisition applications using `egui` and integrating seamlessly with the V4 Kameo actor architecture and Arrow data.