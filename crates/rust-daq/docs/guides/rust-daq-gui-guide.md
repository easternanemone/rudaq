# GUI Development Guide: egui-based Scientific Interface (V5, Headless-First)

## Overview

This guide covers developing the graphical user interface for the Rust DAQ application using `egui`, a modern immediate-mode GUI framework.  
In the current V5 **headless-first** architecture:

- The **core DAQ** runs as a standalone daemon (`rust_daq` bin).
- The daemon exposes control and data over **gRPC** (`ControlService`, `HardwareService`, `ScanService`, etc.).
- The primary GUI is a native `egui` / `eframe` desktop binary: **`rust_daq_gui_egui`**.
- The GUI talks to the daemon purely as a **gRPC client** – no direct actor wiring or in-process coupling.

## 1. Core GUI Architecture (Channel-Based)

The GUI uses a **channel-based message-passing architecture** validated by Codex:

- **UI Thread**: egui immediate-mode rendering, **never blocks**
- **Backend Thread**: tokio runtime with gRPC client, manages streams
- **Communication**: `watch` channels for latest-only state, bounded `mpsc` for events

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                         UI Thread                               │
│  ┌─────────────────┐    ┌──────────────────────────────────┐   │
│  │  DaqGuiApp      │    │  egui rendering loop             │   │
│  │  - devices[]    │◄───│  - process_backend_events()      │   │
│  │  - status       │    │  - render panels                 │   │
│  │  - selected_id  │    │  - send commands (non-blocking)  │   │
│  └────────┬────────┘    └──────────────────────────────────┘   │
│           │                                                     │
│           │ UiChannels                                          │
│           │  - state_rx (watch)     ← latest device state       │
│           │  - event_rx (mpsc)      ← one-shot events           │
│           │  - metrics_rx (watch)   ← backend metrics           │
│           │  - cmd_tx (mpsc)        → commands to backend       │
└───────────┼─────────────────────────────────────────────────────┘
            │
            │ Channel Boundary (thread-safe)
            │
┌───────────┼─────────────────────────────────────────────────────┐
│           │                                                     │
│           │ BackendHandle                                       │
│           │  - state_tx (watch)     → device state updates      │
│           │  - event_tx (mpsc)      → events to UI              │
│           │  - metrics_tx (watch)   → metrics updates           │
│           │  - cmd_rx (mpsc)        ← commands from UI          │
│           ▼                                                     │
│  ┌─────────────────┐    ┌──────────────────────────────────┐   │
│  │  Backend        │    │  tokio runtime                   │   │
│  │  - gRPC client  │◄───│  - command processing            │   │
│  │  - stream tasks │    │  - state streaming               │   │
│  └─────────────────┘    └──────────────────────────────────┘   │
│                         Backend Thread                          │
└─────────────────────────────────────────────────────────────────┘
            │
            │ gRPC (tonic)
            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    rust_daq daemon                              │
└─────────────────────────────────────────────────────────────────┘
```

### Main Application Structure

```rust
use eframe::{egui, App, Frame};
use rust_daq::gui::{
    create_channels, spawn_backend, BackendCommand, BackendEvent, 
    ConnectionStatus, UiChannels, ParameterDescriptor,
};

struct DaqGuiApp {
    daemon_addr: String,
    connection_status: ConnectionStatus,
    status_line: String,
    devices: Vec<DeviceRow>,
    channels: UiChannels,
    selected_device_id: Option<String>,
    is_streaming: bool,
}

impl DaqGuiApp {
    fn new() -> Self {
        // Create channels and spawn backend thread
        let (channels, backend_handle) = create_channels();
        let _backend_thread = spawn_backend(backend_handle);

        Self {
            daemon_addr: "127.0.0.1:50051".to_string(),
            connection_status: ConnectionStatus::Disconnected,
            status_line: String::from("Not connected"),
            devices: Vec::new(),
            channels,
            selected_device_id: None,
            is_streaming: false,
        }
    }
}
```

### Non-Blocking Event Processing

The UI thread processes events without blocking:

```rust
fn process_backend_events(&mut self) {
    // drain_events() is non-blocking, returns all available events
    for event in self.channels.drain_events() {
        match event {
            BackendEvent::DevicesRefreshed { devices } => {
                self.devices = devices.into_iter().map(DeviceRow::from).collect();
            }
            // DeviceStateUpdated now goes through watch channel (see sync_device_state)
            BackendEvent::DeviceStateUpdated { .. } => {
                // Legacy: state updates now via watch channel for better performance
            }
            BackendEvent::ConnectionChanged { status } => {
                self.connection_status = status;
            }
            BackendEvent::Error { message } => {
                self.status_line = format!("Error: {}", message);
            }
            // ... handle other events
        }
    }
}

/// Sync device state from watch channel (non-blocking, always latest)
fn sync_device_state(&mut self) {
    let snapshot = self.channels.get_state();
    for row in &mut self.devices {
        if let Some(device_state) = snapshot.devices.get(&row.id) {
            row.state_fields = device_state.fields.clone();
            row.last_updated = device_state.updated_at;
        }
    }
}
```

### Sending Commands

Commands are sent non-blocking via `try_send`:

```rust
// Connect to daemon
self.channels.send_command(BackendCommand::Connect {
    address: self.daemon_addr.clone(),
});

// Refresh device list
self.channels.send_command(BackendCommand::RefreshDevices);

// Read a value
self.channels.send_command(BackendCommand::ReadValue {
    device_id: device_id.clone(),
});

// Fetch parameters for dynamic controls
self.channels.send_command(BackendCommand::FetchParameters {
    device_id: selected_id.clone(),
});
```

## 2. Channel Types and Capacities

The module uses different channel types for different data patterns:

| Channel | Type | Capacity | Purpose |
|---------|------|----------|---------|
| `state_tx/rx` | `watch` | 1 (latest) | Device state snapshots |
| `metrics_tx/rx` | `watch` | 1 (latest) | Backend performance metrics |
| `event_tx/rx` | `mpsc` | 64 | One-shot events (devices, errors) |
| `cmd_tx/rx` | `mpsc` | 32 | Commands from UI to backend |
| `plot_tx/rx` | `mpsc` | 1024 | Plot data points |

```rust
// From src/gui/channels.rs
pub const STATE_CHANNEL_CAPACITY: usize = 256;
pub const PLOT_CHANNEL_CAPACITY: usize = 1024;
pub const EVENT_CHANNEL_CAPACITY: usize = 64;
pub const COMMAND_CHANNEL_CAPACITY: usize = 32;
```

## 3. Panel-Based Layout

The GUI is organized into panels:

```rust
impl App for DaqGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Check for UI starvation
        self.check_starvation();
        
        // Non-blocking event processing
        self.process_backend_events();
        
        // Sync device state from watch channel (non-blocking, always latest)
        self.sync_device_state();

        // Top panel: connection controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("rust-daq Control Panel");
            // Address input, Connect/Disconnect buttons
        });

        // Bottom panel: metrics
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            let metrics = self.channels.get_metrics();
            ui.label(format!("Frames dropped: {}", metrics.frames_dropped));
        });

        // Right panel: device details (when selected)
        if let Some(selected_id) = &self.selected_device_id {
            egui::SidePanel::right("device_detail_panel")
                .default_width(300.0)
                .show(ctx, |ui| {
                    // Device info, interactive parameter widgets, live state
                });
        }

        // Central panel: device list
        egui::CentralPanel::default().show(ctx, |ui| {
            // Device grid with selection
        });

        // Request repaint at ~30fps
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
```

## 4. Dynamic Control Panels (Parameter Widgets)

Parameters are auto-discovered via `ListParameters` gRPC call and rendered based on type:

```rust
// src/gui/widgets.rs provides parameter widgets

pub enum ParameterType {
    Float,   // Slider or DragValue with range
    Int,     // DragValue with integer steps
    Bool,    // Checkbox
    String,  // Text input
    Enum,    // ComboBox
}

pub struct ParameterDescriptor {
    pub device_id: String,
    pub name: String,
    pub description: String,
    pub dtype: ParameterType,
    pub units: String,
    pub readable: bool,
    pub writable: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub enum_values: Vec<String>,
    pub current_value: Option<String>,
}

// Render appropriate widget based on type
pub fn parameter_widget(
    ui: &mut Ui,
    param: &ParameterDescriptor,
    state: &mut ParameterEditState,
) -> WidgetResult {
    match param.dtype {
        ParameterType::Float => render_float(ui, param, state),
        ParameterType::Int => render_int(ui, param, state),
        ParameterType::Bool => render_bool(ui, param, state),
        ParameterType::String => render_string(ui, param, state),
        ParameterType::Enum => render_enum(ui, param, state),
    }
}
```

### Interactive Widget Usage

To handle widget changes and send commands to the backend, use the `WidgetResult` enum:

```rust
use rust_daq::gui::{parameter_widget, WidgetResult, BackendCommand};

// In the UI panel rendering (e.g., right side panel)
// Collect commands to send after UI rendering (avoids borrow issues)
let mut pending_commands = Vec::new();

for param in &parameters {
    let state = param_edit_states
        .entry(format!("{}:{}", param.device_id, param.name))
        .or_default();

    match parameter_widget(ui, param, state) {
        WidgetResult::Committed(value) => {
            // User committed change (released slider, pressed enter, etc.)
            pending_commands.push(BackendCommand::SetParameter {
                device_id: param.device_id.clone(),
                name: param.name.clone(),
                value,
            });
            state.reset(); // Allow re-init from server response
        }
        WidgetResult::Changed(_) => {
            // Value changed but not committed (e.g., dragging slider)
            // Could show pending indicator
        }
        WidgetResult::NoChange => {}
    }
}

// Send all collected commands after UI rendering
for cmd in pending_commands {
    self.channels.send_command(cmd);
}
```

## 5. Real-Time Device State Streaming

The backend subscribes to device state updates via gRPC streaming and pushes them to the UI via a **watch channel** for optimal performance (always latest state, no backlog):

```rust
// Backend starts streaming after connection
async fn start_state_stream(&mut self, device_ids: Vec<String>) {
    let request = SubscribeDeviceStateRequest { device_ids };
    let state_tx = self.handle.state_tx.clone();
    
    match self.client.subscribe_device_state(request).await {
        Ok(response) => {
            let mut stream = response.into_inner();
            
            // Process stream in background task
            while let Some(update) = stream.next().await {
                // Use send_modify for atomic updates to watch channel
                state_tx.send_modify(|snapshot| {
                    let device_state = snapshot.devices
                        .entry(update.device_id.clone())
                        .or_insert_with(|| DeviceState::default());
                    
                    device_state.fields.extend(update.fields);
                    device_state.version = update.version;
                    device_state.updated_at = Some(Instant::now());
                    snapshot.is_connected = true;
                });
            }
        }
        Err(e) => {
            // Exponential backoff reconnection (100ms → 30s max)
        }
    }
}
```

The UI synchronizes state from the watch channel each frame (non-blocking):

```rust
fn sync_device_state(&mut self) {
    let snapshot = self.channels.get_state();
    for row in &mut self.devices {
        if let Some(device_state) = snapshot.devices.get(&row.id) {
            row.state_fields = device_state.fields.clone();
            row.last_updated = device_state.updated_at;
        }
    }
}
```

## 6. Building and Running

```bash
# Build with GUI features
cargo build --features "networking,gui_egui"

# Run the GUI
cargo run --features "networking,gui_egui" --bin rust_daq_gui_egui

# Run daemon in another terminal
cargo run --features networking -- daemon --port 50051
```

## 7. Module Structure

```
src/gui/
├── mod.rs       # Module exports
├── types.rs     # DTOs: DeviceInfo, BackendCommand, BackendEvent, etc.
├── channels.rs  # UiChannels, BackendHandle, create_channels()
├── backend.rs   # Backend thread with tokio runtime, gRPC client
└── widgets.rs   # Parameter widget rendering

src/gui_main.rs  # Main GUI application (eframe::App)
```

## 8. Key Design Decisions

1. **Channel-based, not async in UI**: The UI thread is synchronous egui; async gRPC lives in the backend thread
2. **watch for latest-only**: Device state uses `watch` channels - old values are overwritten, no backlog
3. **Bounded mpsc for events**: Events use bounded channels with `try_send` to prevent blocking
4. **Non-blocking everywhere**: All UI operations use `try_recv`, `drain_events`, `try_send`
5. **Graceful degradation**: If channels are full, data is dropped rather than blocking
6. **Interval-based metrics**: Backend uses `tokio::time::interval` with `MissedTickBehavior::Skip` instead of `sleep` to prevent metrics starvation in `select!` loops
7. **Channel failure logging**: All channel `try_send` failures are logged via `tracing::warn` for debugging dropped data
8. **Pending commands pattern**: UI collects commands during rendering, sends after to avoid borrow checker issues with egui's immediate-mode

## 9. Styling and Themes

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

## 10. Performance Optimization

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

This GUI development guide provides the foundation for creating a professional, responsive, and feature-rich interface for scientific data acquisition applications using `egui` and the V5 headless-first architecture with gRPC streaming.