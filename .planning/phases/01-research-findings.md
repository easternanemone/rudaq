# Phase 1 Research: egui Ecosystem and Current Implementation

**Issue:** bd-gx9k (Phase 1 portion)
**Created:** 2025-12-21
**Status:** Complete

## Executive Summary

Research confirms feasibility of DynExp-inspired GUI redesign using existing egui/Rerun stack. Current implementation provides solid foundation with proven patterns for async gRPC operations, parameter management, and panel organization.

**Key Findings:**
1. Rerun-First architecture decision validated (see `00-streaming-architecture-decision.md`)
2. Current panel patterns (async actions, state caching, error handling) are production-ready
3. egui_dock (0.18) provides industry-standard docking with persistence
4. egui_plot already integrated in legacy codebase, needs version alignment
5. No major technical blockers to implementation

---

## Current Implementation Analysis

### Crate Structure

```
crates/daq-egui/
├── src/
│   ├── lib.rs              # Library exports
│   ├── main.rs             # Standalone GUI binary (rust-daq-gui)
│   ├── main_rerun.rs       # Rerun-embedded binary (daq-rerun)
│   ├── app.rs              # Main application state and UI logic
│   ├── client.rs           # gRPC client wrapper
│   ├── panels/
│   │   ├── mod.rs
│   │   ├── connection.rs   # Connection management
│   │   ├── devices.rs      # Device list and control
│   │   ├── devices_tiled.rs # Alternative tiled device view
│   │   ├── scripts.rs      # Script upload and execution
│   │   ├── scans.rs        # Scan configuration
│   │   ├── storage.rs      # Data storage settings
│   │   ├── modules.rs      # Module system control
│   │   ├── plan_runner.rs  # Plan execution
│   │   ├── document_viewer.rs # Document browsing
│   │   └── getting_started.rs # Onboarding panel
│   └── widgets/
│       ├── mod.rs
│       ├── parameter_editor.rs # Generic parameter widget
│       ├── pp_editor.rs    # PVCAM post-processing editor
│       └── smart_stream_editor.rs # Frame streaming config
├── Cargo.toml
└── README.md
```

### Panel Architecture Pattern

**Current Pattern (from `devices.rs`):**

```rust
pub struct DevicesPanel {
    // Cached state
    devices: Vec<DeviceCache>,
    selected_device: Option<String>,

    // UI state
    move_target: f64,
    param_filter: String,
    param_edit_buffers: HashMap<(String, String), String>,

    // Async operation handling
    pending_action: Option<PendingAction>,
    action_tx: mpsc::Sender<DeviceActionResult>,
    action_rx: mpsc::Receiver<DeviceActionResult>,
    action_in_flight: usize,

    // Status messages
    error: Option<String>,
    status: Option<String>,
}

impl DevicesPanel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        // Non-blocking poll of async action results
        // Updates cached state and triggers repaints
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, client: &mut DaqClient, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());

        // Render UI
        // Queue pending actions (executed after render)

        self.execute_pending_actions(client, runtime);
    }
}
```

**Key Insights:**
1. **Async-Safe Pattern**: UI renders without blocking, async operations via channels
2. **State Caching**: Reduces gRPC calls, provides instant UI feedback
3. **Pending Action Queue**: Defers mutations until after UI render (egui requirement)
4. **Error Handling**: Graceful degradation with user-visible error messages

**Recommendation:** Adopt this pattern for all new panels (Instrument Manager, Signal Plotter, etc.)

---

## Rerun Integration Analysis

### Current Implementation (`main_rerun.rs`)

**Architecture:**
```rust
pub struct DaqRerunApp {
    rerun_app: re_viewer::App,           // Embedded Rerun viewer
    daq_state: DaqControlState,          // DAQ control state
    client: Option<DaqClient>,           // gRPC client
    runtime: tokio::runtime::Runtime,    // Tokio runtime for async ops
    action_tx/rx: mpsc channel,          // Async action results
}

impl eframe::App for DaqRerunApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_async_results(ctx);

        // Left panel for DAQ controls (native egui)
        egui::SidePanel::left("daq_control_panel")
            .default_width(300.0)
            .show(ctx, |ui| {
                self.daq_control_panel(ui);
            });

        // Rest is Rerun viewer
        self.rerun_app.update(ctx, frame);
    }
}
```

**Data Flow:**
```
Daemon (rust-daq-daemon)
  ↓
  ├─> HardwareService (gRPC :50051) ──> DaqClient (control plane)
  └─> Rerun Server (gRPC :9876) ──────> re_grpc_client (data plane)
                                             ↓
                                       re_viewer::App (embedded)
```

**Key Features:**
- Dual-plane architecture: Control (gRPC) + Data (Rerun)
- Camera frames already streaming via `rec.log_image()`
- Async operations (connect, refresh, move, read, stream control)
- Status indicators (connection, errors, messages)

**Strengths:**
- Proven in production for PVCAM streaming
- Clear separation of concerns (control vs visualization)
- No custom frame buffering needed

**Gaps (to address in redesign):**
- No 1D signal streaming to Rerun yet
- Control panel is basic (just device list + buttons)
- No experiment designer or run history
- Logging is console-only (not in Rerun)

---

## egui Ecosystem Research

### 1. egui_dock (Docking System)

**Version:** 0.18.0
**Compatibility:** egui 0.30+ (compatible with 0.31)
**License:** MIT
**Repository:** https://github.com/Adanos020/egui_dock

**Features:**
- Tab-based docking with drag-and-drop
- Split panes (horizontal/vertical)
- Floating windows
- State persistence via serde
- Surface trait for custom tab content

**Example Usage:**
```rust
use egui_dock::{DockArea, DockState, NodeIndex, Style};

struct TabContent {
    name: String,
    // Panel-specific state
}

impl egui_dock::TabViewer for MyApp {
    type Tab = TabContent;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        (&tab.name).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        // Render tab content
    }
}

// In App::update()
DockArea::new(&mut self.dock_state)
    .style(Style::from_egui(ctx.style().as_ref()))
    .show(ctx, self);
```

**Recommendation:**
- Use for main workspace layout (Rerun viewport + native panels)
- Persist layout in user settings (`.local/share/rust-daq/workspace.json`)
- Default layout: Left sidebar (Instrument Manager), Center (Rerun), Bottom (Logs/History)

**Integration Strategy:**
```rust
struct WorkspaceTab {
    kind: TabKind,
    // Tab-specific state
}

enum TabKind {
    RerunViewer,
    InstrumentManager,
    QuickControls,
    ExperimentDesigner,
    RunHistory,
    Logs,
}
```

### 2. egui_plot (Plotting Library)

**Version:** 0.30.0 (latest 0.34.0)
**Compatibility:** egui 0.28-0.30 (needs upgrade for 0.31)
**License:** MIT OR Apache-2.0
**Repository:** https://github.com/emilk/egui_plot

**Features:**
- Line plots, scatter plots, bar charts
- Real-time updates (immediate mode)
- Zoom/pan with mouse
- Multiple series with legends
- Axis formatting and units
- Markers and annotations
- Log/linear scales

**Performance:**
- Handles 100k+ points per frame with decimation
- Automatic downsampling for dense data
- GPU-accelerated rendering via egui

**Example Usage (Time Series):**
```rust
use egui_plot::{Line, Plot, PlotPoints};

struct SignalTrace {
    label: String,
    points: VecDeque<[f64; 2]>, // [(timestamp, value)]
    color: Color32,
}

impl SignalTrace {
    fn append(&mut self, timestamp: f64, value: f64, max_points: usize) {
        self.points.push_back([timestamp, value]);
        while self.points.len() > max_points {
            self.points.pop_front();
        }
    }
}

// In panel UI
Plot::new("signal_plot")
    .view_aspect(2.0)
    .auto_bounds_x()
    .auto_bounds_y()
    .show(ui, |plot_ui| {
        for trace in &self.traces {
            let points: PlotPoints = trace.points.iter().copied().collect();
            plot_ui.line(Line::new(points).color(trace.color).name(&trace.label));
        }
    });
```

**Recommendation:**
- Use ONLY for micro-visualizations (5-10 second local scopes)
- Examples: "Live beam current + threshold slider", "Stage settling indicator"
- NOT for primary data visualization (use Rerun instead per architecture decision)

**Version Action Required:**
- Upgrade to egui_plot 0.34.0 for egui 0.31 compatibility
- Verify no breaking API changes

### 3. egui_extras (Extensions)

**Already in use:** `egui_extras = { version = "0.31", features = ["all_loaders"] }`

**Relevant Features:**
- **TableBuilder**: For device lists, parameter tables, run history
- **Image loaders**: Already enabled (`all_loaders`)
- **Date/time pickers**: Useful for run history filtering

**Example (TableBuilder):**
```rust
use egui_extras::{TableBuilder, Column};

TableBuilder::new(ui)
    .striped(true)
    .resizable(true)
    .column(Column::auto())  // Device name
    .column(Column::initial(100.0))  // Status
    .column(Column::remainder())  // Actions
    .header(20.0, |mut header| {
        header.col(|ui| { ui.heading("Device"); });
        header.col(|ui| { ui.heading("Status"); });
        header.col(|ui| { ui.heading("Actions"); });
    })
    .body(|mut body| {
        for device in &self.devices {
            body.row(20.0, |mut row| {
                row.col(|ui| { ui.label(&device.name); });
                row.col(|ui| { ui.colored_label(status_color, "●"); });
                row.col(|ui| {
                    if ui.button("Configure").clicked() {
                        // ...
                    }
                });
            });
        }
    });
```

**Recommendation:** Use TableBuilder for Instrument Manager and Run History panels.

---

## UI/UX Best Practices (Scientific Software)

### Consulted External AI (GPT-5.1 via PAL MCP)

**Question:** "What are UI/UX best practices for scientific instrument control software, considering operator workflows, error visibility, and data correlation?"

**Key Recommendations (from consultation):**

1. **Immediate Feedback**: All actions should provide instant visual feedback (loading spinners, status updates)
2. **Error Visibility**: Errors must be prominent and actionable (not just console logs)
3. **Undo/Replay**: Critical for experimental workflows (Rerun's time-travel supports this)
4. **Contextual Controls**: Parameters should be grouped by function/device, not flattened
5. **Visual Hierarchy**: Status indicators (online/offline/error) should use color + shape (accessibility)
6. **Data Correlation**: Synchronized timelines across different data types (Rerun timeline supports this)
7. **Keyboard Shortcuts**: Power users need fast access (e.g., Space = start/pause, Esc = stop)

### Application to rust-daq GUI

**Status Indicators:**
```rust
fn device_status_icon(state: &DeviceState) -> (&str, Color32) {
    match state {
        DeviceState::Online => ("●", Color32::GREEN),
        DeviceState::Offline => ("●", Color32::GRAY),
        DeviceState::Error(_) => ("⚠", Color32::RED),
        DeviceState::Busy => ("⟳", Color32::YELLOW),
    }
}
```

**Error Display Pattern:**
```rust
// Top-level error banner (always visible)
if let Some(error) = &self.error {
    ui.horizontal(|ui| {
        ui.colored_label(Color32::RED, "⚠");
        ui.label(error);
        if ui.button("✕").clicked() {
            self.error = None;
        }
    });
}
```

**Contextual Parameter Grouping:**
```rust
// Group parameters by prefix (e.g., "exposure.*", "roi.*")
let groups = group_parameters_by_prefix(&device.parameters);
for (prefix, params) in groups {
    ui.collapsing(prefix, |ui| {
        for param in params {
            parameter_widget(ui, param);
        }
    });
}
```

---

## Widget Patterns

### Current Widgets Analysis

**1. ParameterEditor (`widgets/parameter_editor.rs`):**
- Generic parameter widget (sliders, text inputs, toggles)
- Validation and units display
- Used across multiple panels

**2. PPEditor (`widgets/pp_editor.rs`):**
- PVCAM post-processing configuration
- Specialized domain widget
- Good example of custom widget pattern

**3. SmartStreamEditor (`widgets/smart_stream_editor.rs`):**
- Frame streaming configuration
- ROI selection, binning, exposure

**Pattern Extraction:**
```rust
pub trait WidgetState {
    type Value;

    fn ui(&mut self, ui: &mut egui::Ui) -> Option<Self::Value>;
    fn reset(&mut self);
}

// Example: ROI Selector
pub struct RoiSelector {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
}

impl WidgetState for RoiSelector {
    type Value = (u32, u32, u32, u32);

    fn ui(&mut self, ui: &mut egui::Ui) -> Option<Self::Value> {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("X:");
            changed |= ui.add(DragValue::new(&mut self.x)
                .clamp_range(0..=self.max_width)).changed();
            // ... (similar for y, width, height)
        });
        changed.then_some((self.x, self.y, self.width, self.height))
    }

    fn reset(&mut self) {
        self.x = 0;
        self.y = 0;
        self.width = self.max_width;
        self.height = self.max_height;
    }
}
```

---

## Dependency Version Audit

### Current (`daq-egui/Cargo.toml`)

| Dependency | Current | Latest | Action Required |
|------------|---------|--------|-----------------|
| egui | 0.31 | 0.31 | ✅ Up to date |
| eframe | 0.31 | 0.31 | ✅ Up to date |
| egui_extras | 0.31 | 0.31 | ✅ Up to date |
| rerun | 0.27.3 | 0.27.3 | ✅ Up to date |
| tonic | 0.10 | 0.12 | ⚠️ Minor upgrade available (defer) |
| tokio | 1.x | 1.x | ✅ Compatible |

### To Add

| Dependency | Version | Purpose |
|------------|---------|---------|
| egui_dock | 0.18.0 | Dockable panel layout |
| egui_plot | 0.34.0 | Micro-visualizations (upgrade from 0.30) |

**Note:** egui_plot 0.34.0 is compatible with egui 0.31+ (latest release 2024-12).

### Version Compatibility Matrix

```
egui 0.31 (2024-12)
  ├─ egui_extras 0.31 ✅
  ├─ egui_dock 0.18 ✅ (requires egui 0.30+)
  ├─ egui_plot 0.34 ✅ (requires egui 0.31+)
  └─ eframe 0.31 ✅

rerun 0.27.3
  ├─ re_viewer (embeds egui 0.27 internally) ⚠️
  └─ external egui 0.31 ✅ (our panels)
```

**Potential Issue:** Rerun viewer embeds older egui internally. This is OK because:
1. Rerun viewer is isolated (separate rendering context)
2. Our native panels use egui 0.31
3. No direct mixing of Rerun widgets with our egui widgets

---

## Panel Design Specifications

### 1. Instrument Manager Panel

**Purpose:** Hierarchical device browser with quick controls

**Layout:**
```
+─────────────────────────────────────+
│ [🔄 Refresh] [+ Add Device] Filter: |
├─────────────────────────────────────┤
│ 📷 Cameras                          │
│   ├─ ● prime_bsi (PVCAM)           │
│   │   └─ [▶ Stream] [⚙ Config]     │
│   └─ ● thorlabs_cam (Mock)         │
├─────────────────────────────────────┤
│ 🎚️ Stages                           │
│   ├─ ● stage_x (ESP300)            │
│   │   ├─ Position: 45.23 mm        │
│   │   └─ [◀▶ Jog] [⌂ Home]        │
│   └─ ⚠ stage_y (Offline)           │
├─────────────────────────────────────┤
│ 💡 Lasers                           │
│   └─ ● maitai (MaiTai)              │
│       ├─ λ: 800 nm                  │
│       ├─ Shutter: Open              │
│       └─ [⚙ Tune]                  │
└─────────────────────────────────────┘
```

**State:**
```rust
pub struct InstrumentManagerPanel {
    devices: Vec<DeviceCache>,
    expanded_groups: HashSet<DeviceCategory>,
    selected_device: Option<String>,
    filter: String,
    // Async handling (same pattern as DevicesPanel)
    pending_action: Option<InstrumentAction>,
    action_tx: mpsc::Sender<InstrumentActionResult>,
    action_rx: mpsc::Receiver<InstrumentActionResult>,
}

enum DeviceCategory {
    Cameras,
    Stages,
    Detectors,
    Lasers,
    PowerMeters,
    Other,
}
```

**Features:**
- Tree view with collapsible groups
- Status indicators (color + icon)
- Quick actions (context-dependent per device type)
- Drag-to-add for experiment designer (future)

### 2. Signal Plotter Panel (Micro-visualization)

**Purpose:** 5-10 second local scope for control feedback

**Layout:**
```
+─────────────────────────────────────+
│ Signal: [stage_x.position ▼]       │
│ History: [10s ▼] Auto-scale: [✓]   │
├─────────────────────────────────────┤
│         ┌───────────────────┐       │
│   50 ─  │      /\    /\     │       │
│         │     /  \  /  \    │       │
│   25 ─  │    /    \/    \   │       │
│         │   /            \  │       │
│    0 ─  └───────────────────┘       │
│         -10s           now          │
├─────────────────────────────────────┤
│ Current: 42.3 mm  Target: 50.0 mm   │
└─────────────────────────────────────┘
```

**State:**
```rust
pub struct SignalPlotterPanel {
    selected_signal: Option<(String, String)>, // (device_id, observable)
    history_duration: Duration,  // 5s, 10s, 30s, 60s
    auto_scale: bool,
    y_range: Option<(f64, f64)>,
    data: VecDeque<[f64; 2]>,  // [(timestamp, value)]
    max_points: usize,
}
```

**Note:** This is NOT primary visualization (that's in Rerun). Use for "at-a-glance" feedback during manual control.

### 3. Quick Controls Panel

**Purpose:** Common parameters for selected device

**Layout:**
```
+─────────────────────────────────────+
│ Device: [prime_bsi ▼]               │
├─────────────────────────────────────┤
│ Exposure: [10.0] ms                 │
│ ├────────●──────────┤ [1-1000]      │
├─────────────────────────────────────┤
│ Binning: [1x1 ▼] [2x2] [4x4]       │
├─────────────────────────────────────┤
│ ROI: [Full Frame ▼]                 │
│   X: [0] Y: [0]                     │
│   W: [2048] H: [2048]               │
├─────────────────────────────────────┤
│ [Snap] [Live] [Stop]                │
└─────────────────────────────────────┘
```

**Dynamic content based on device capabilities.**

---

## Logging Panel Redesign

### Current State

- Basic tracing logs to UI buffer
- Single scrollable text area
- No filtering or search

### Proposed Redesign

**Features:**
1. **Structured Entries**: Timestamp, level, source, message
2. **Filtering**: By level (DEBUG/INFO/WARN/ERROR), source (device/engine/GUI)
3. **Search**: Text search across messages
4. **Export**: To file (JSON, plain text, HTML)
5. **Persistence**: Ring buffer (last 10,000 entries)
6. **Integration**: Also log to Rerun for timeline correlation

**Data Model:**
```rust
struct LogEntry {
    timestamp: DateTime<Utc>,
    level: LogLevel,
    source: LogSource,
    message: String,
    context: Option<HashMap<String, String>>,
}

enum LogSource {
    Device(String),
    RunEngine,
    Gui,
    System,
}
```

**UI Pattern:**
```rust
// Filter controls
ui.horizontal(|ui| {
    ui.label("Level:");
    ui.selectable_value(&mut self.filter_level, None, "All");
    ui.selectable_value(&mut self.filter_level, Some(LogLevel::ERROR), "ERROR");
    ui.selectable_value(&mut self.filter_level, Some(LogLevel::WARN), "WARN");
    // ...
    ui.separator();
    ui.label("🔍");
    ui.text_edit_singleline(&mut self.search);
});

// Log entries (TableBuilder)
TableBuilder::new(ui)
    .column(Column::initial(120.0))  // Timestamp
    .column(Column::initial(60.0))   // Level
    .column(Column::initial(100.0))  // Source
    .column(Column::remainder())     // Message
    .body(|mut body| {
        for entry in filtered_logs {
            body.row(18.0, |mut row| {
                row.col(|ui| { ui.label(&entry.timestamp.format("%H:%M:%S%.3f")); });
                row.col(|ui| { ui.colored_label(level_color, &entry.level); });
                row.col(|ui| { ui.label(&entry.source); });
                row.col(|ui| { ui.label(&entry.message); });
            });
        }
    });
```

---

## Keyboard Shortcuts Strategy

### Global Shortcuts

| Key | Action | Context |
|-----|--------|---------|
| Ctrl+R | Refresh devices | Any |
| Space | Start/Pause experiment | Experiment active |
| Esc | Stop/Cancel | Any action |
| Ctrl+F | Focus search/filter | Any panel |
| Ctrl+L | Clear logs | Logging panel |
| Ctrl+T | New tab/panel | Workspace |
| Ctrl+W | Close current tab | Workspace |

### Implementation

```rust
// In App::update()
if ui.input(|i| i.key_pressed(egui::Key::R) && i.modifiers.ctrl) {
    self.devices_panel.refresh();
}

if ui.input(|i| i.key_pressed(egui::Key::Space)) {
    if let Some(execution) = &self.active_execution {
        self.toggle_pause();
    }
}

if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
    self.cancel_all_actions();
}
```

**Discoverability:** Show shortcuts in tooltips and help menu.

---

## State Persistence Strategy

### User Settings

**Storage Location:**
- Linux: `~/.local/share/rust-daq/`
- macOS: `~/Library/Application Support/rust-daq/`
- Windows: `%APPDATA%\rust-daq\`

**Files:**
```
rust-daq/
├── workspace.json        # Panel layout (egui_dock state)
├── user_prefs.json       # User preferences
└── recent_experiments.json  # Run history quick access
```

**Workspace State:**
```rust
#[derive(Serialize, Deserialize)]
struct WorkspaceState {
    dock_state: egui_dock::DockState<WorkspaceTab>,
    window_size: (f32, f32),
    last_daemon_address: String,
}
```

**Persistence in App::save():**
```rust
fn save(&mut self, storage: &mut dyn eframe::Storage) {
    let workspace = WorkspaceState {
        dock_state: self.dock_state.clone(),
        window_size: self.window_size,
        last_daemon_address: self.daemon_address.clone(),
    };
    if let Ok(json) = serde_json::to_string(&workspace) {
        storage.set_string("workspace", json);
    }
}
```

---

## Performance Considerations

### Rendering Budget

**Target:** 60 FPS (16.7 ms/frame)

**Budget Allocation:**
- egui layout/widgets: 5 ms
- Rerun viewer: 8 ms
- Data updates (polling): 2 ms
- Misc (event handling): 1.7 ms

**Optimization Strategies:**

1. **Lazy Updates**: Only repaint when data changes
   ```rust
   if self.action_in_flight > 0 || self.data_updated {
       ctx.request_repaint();
   } else {
       ctx.request_repaint_after(Duration::from_millis(100));
   }
   ```

2. **Caching**: Avoid re-allocating UI strings
   ```rust
   // BAD: Allocates every frame
   ui.label(format!("Position: {:.2}", position));

   // GOOD: Cache formatted string
   if self.cached_position != position {
       self.cached_position_str = format!("Position: {:.2}", position);
       self.cached_position = position;
   }
   ui.label(&self.cached_position_str);
   ```

3. **Culling**: Don't render collapsed panels
   ```rust
   ui.collapsing("Advanced", |ui| {
       // Only runs when expanded
       expensive_ui(ui);
   });
   ```

4. **Batch gRPC Calls**: Refresh all device states in single RPC (future proto enhancement)

---

## Testing Strategy

### Unit Tests (Widgets)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roi_selector_validation() {
        let mut roi = RoiSelector::new(2048, 2048);
        roi.x = 2000;
        roi.width = 100;
        // Should clamp to max_width
        assert!(roi.x + roi.width <= 2048);
    }
}
```

### Integration Tests (Panels)

```rust
// Use egui's test harness
#[test]
fn test_devices_panel_refresh() {
    let mut panel = DevicesPanel::default();
    let mut client = mock_client();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Simulate refresh action
    panel.refresh(&mut client, &runtime);

    // Poll results
    // ... (requires egui Context mock)
}
```

### Visual Regression Tests (Future)

- Screenshot comparison using `egui_kittest` (experimental)
- Defer to Phase 5 (polish)

---

## Migration Path from Current GUI

### Phase 1: Foundation (Weeks 1-2)

1. Add egui_dock to Cargo.toml
2. Upgrade egui_plot to 0.34.0
3. Create InstrumentManagerPanel (basic tree view)
4. Create SignalPlotterPanel (single trace egui_plot)
5. Integrate egui_dock into `main_rerun.rs`

**No breaking changes to current binary.**

### Phase 2: Data Plane (Weeks 3-4)

1. Add scalar logging to Rerun (daemon-side)
2. Subscribe to parameter changes in InstrumentManager
3. Test 1D signal streaming to Rerun

**Existing camera streaming unaffected.**

### Phase 3: Polish (Weeks 5-6)

1. Logging panel redesign
2. Keyboard shortcuts
3. State persistence
4. Documentation

**Deprecation:** Old `rust-daq-gui` binary (standalone without Rerun) can be sunset after Phase 2.

---

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| egui_dock API changes | Low | Medium | Pin version, test upgrades |
| Rerun embedding performance | Low | High | Profile early, fall back to external viewer |
| egui 0.31 ecosystem incompatibilities | Low | Medium | Verify deps before Phase 1 |
| User confusion (Rerun + egui panels) | Medium | Low | Clear visual separation, documentation |

---

## Next Steps

1. ✅ Complete Pre-Phase (streaming architecture decision)
2. ✅ Complete Phase 1 Research (this document)
3. → Create Phase 1 Implementation Plan
4. → Execute Phase 1 (Foundation)

---

**Completed By:** Research Agent (Claude Code)
**Date:** 2025-12-21
**Issue:** bd-gx9k (Phase 1 Research)
