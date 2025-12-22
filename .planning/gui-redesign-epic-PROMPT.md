# rust-daq GUI Redesign: DynExp-Inspired Scientific Workbench

## Meta-Prompt for Claude-to-Claude Pipeline

**Epic ID:** bd-gui-redesign
**Created:** 2025-12-21
**Status:** Research → Plan → Implement
**Tracking:** Use `bd` issue tracker throughout

---

## Executive Context

Transform rust-daq's current device-centric egui GUI into a comprehensive scientific experiment workbench inspired by DynExp, PyMoDAQ, and ScopeFoundry. The backend remains pure Rust with the headless-first V5 architecture. The GUI evolution focuses on laboratory researcher usability without sacrificing power-user capabilities.

### Reference Platforms Analyzed

| Platform | Key Strengths to Adopt |
|----------|------------------------|
| **DynExp** | Hierarchical object model (HardwareAdapters → Instruments → Modules), HTML logging, signal/spectrum/image viewers, task-based module communication |
| **PyMoDAQ** | Dashboard unifying detectors/actuators, DAQ Viewer/DAQ Move split, DAQ Scan extension, modular plugin architecture |
| **ScopeFoundry** | HardwareComponents/Measurements separation, DataBrowser for analysis, LoggedQuantity settings synchronization, 80+ community drivers |

### Current Architecture (V5 Headless-First)

```
daq-egui (egui GUI)
    ↓ gRPC
rust-daq-daemon (daq-server)
    ↓
daq-hardware (capability-based HAL)
    ↓
Physical Instruments
```

**Current GUI Panels:** GettingStarted, Devices, Scripts, Scans, Storage, Modules, PlanRunner, DocumentViewer, Logs

**Gaps:** No live plotting, no 2D image viewer, no instrument manager hierarchy, no experiment designer, logging is basic.

---

## Target GUI Architecture

### Panel Structure (DynExp-Inspired)

```
+------------------------------------------------------------------+
|  Menu Bar: File | View | Experiment | Hardware | Help            |
+------------------------------------------------------------------+
|  Toolbar: [Connect] [New Experiment] [Run] [Pause] [Stop]        |
+------------------------------------------------------------------+
| Left Sidebar    |  Main Workspace (tabbed/tiled panels)          |
+-----------------+                                                 |
| Instrument      |  +-------------------------------------------+  |
| Manager         |  |  Live Visualization (1D/2D panels)        |  |
|                 |  |  - Signal Plotter (egui_plot)              |  |
| - Cameras       |  |  - Image Viewer (2D detector output)      |  |
| - Stages        |  |  - Spectrum Viewer (frequency domain)     |  |
| - Detectors     |  +-------------------------------------------+  |
| - Lasers        |  |  Experiment Designer / Scan Builder        |  |
|                 |  |  - Visual workflow graph                   |  |
|                 |  |  - Parameter entry forms                   |  |
+-----------------+  +-------------------------------------------+  |
| Quick Controls  |  |  Data Browser / Run History                |  |
| - Position      |  |  - Past experiments with metadata          |  |
| - Exposure      |  |  - Quick preview and comparison            |  |
| - Wavelength    |  +-------------------------------------------+  |
+-----------------+--------------------------------------------------+
|  Bottom Dock: Logging Panel (structured, filterable, exportable) |
+------------------------------------------------------------------+
|  Status Bar: Connection | Active Scan | Progress | Timestamp     |
+------------------------------------------------------------------+
```

### Key Feature Areas

#### 1. Instrument Manager (Left Sidebar)

**Inspired by:** DynExp's hierarchical object model, ScopeFoundry's HardwareComponents

**Structure:**
- Tree view of registered hardware grouped by type (Cameras, Stages, Detectors, etc.)
- Expandable nodes showing device state and quick actions
- Context menu: Configure, Test Connection, View Parameters, Remove
- Drag-and-drop to add devices to experiments
- Status indicators (online/offline/error)

**Data Model:**
```rust
struct InstrumentNode {
    device_id: String,
    device_type: DeviceCategory, // Camera, Stage, Detector, Laser, etc.
    display_name: String,
    state: InstrumentState, // Online, Offline, Error, Busy
    capabilities: Vec<Capability>,
    quick_controls: Vec<QuickControl>, // Exposed for sidebar
}

struct QuickControl {
    parameter_name: String,
    widget_type: WidgetType, // Slider, Toggle, Dropdown
    current_value: Value,
}
```

#### 2. Live Visualization Panels

**Inspired by:** DynExp's signal/spectrum/image viewers, PyMoDAQ's DAQ Viewer

##### 2.1 Signal Plotter (1D Time Series)
- Real-time line plots with configurable history depth
- Multiple traces with color coding
- Autoscale Y-axis with manual override
- Rolling window or triggered acquisition
- Export to CSV/image
- Cursor for value readout
- Math channels (derived quantities)

**Implementation:** Use `egui_plot` with gRPC streaming subscription

```rust
struct SignalPlotterState {
    traces: Vec<SignalTrace>,
    x_range: TimeRange,
    y_range: Option<(f64, f64)>, // None = autoscale
    history_depth: usize,
    cursor_position: Option<f64>,
}

struct SignalTrace {
    label: String,
    device_id: String,
    observable_name: String,
    color: Color32,
    points: VecDeque<(f64, f64)>, // (timestamp, value)
}
```

##### 2.2 Image Viewer (2D Detector Output)
- Live camera frames from gRPC stream
- Colormap selection (grayscale, viridis, inferno, etc.)
- ROI selection with statistics (mean, std, min, max)
- Zoom/pan with pixel coordinates
- Histogram overlay
- False color scaling (linear, log, sqrt)
- Snapshot to file

**Implementation:** Integrate with Rerun or custom egui texture rendering

```rust
struct ImageViewerState {
    current_frame: Option<ImageFrame>,
    colormap: Colormap,
    scale_mode: ScaleMode, // Linear, Log, Sqrt
    roi: Option<Rect>,
    roi_stats: Option<RoiStatistics>,
    zoom_level: f32,
    pan_offset: Vec2,
}
```

##### 2.3 Spectrum Viewer (Frequency Domain)
- FFT of time series data
- Peak detection with labels
- Multiple spectra overlay
- Log/linear scale toggle
- Export spectral data

#### 3. Experiment Designer Module

**Inspired by:** DynExp's Signal Designer/Trajectory1D, PyMoDAQ's DAQ Scan

##### 3.1 Scan Builder (Sequence-Based)
- Visual step sequencer (add/remove/reorder steps)
- Step types: Move, Acquire, Wait, Set Parameter, Loop, Conditional
- Parameter forms with validation and units
- Preview execution timeline
- Generate Rhai script from sequence
- Save/load experiment templates

**UI Mockup:**
```
Experiment: "Wavelength Scan"
+----------------------------------------------+
| Step 1: Loop (wavelength: 700nm → 900nm, 21) |
|   ├─ Step 1.1: Move(maitai, wavelength)      |
|   ├─ Step 1.2: Wait(0.5s)                    |
|   └─ Step 1.3: Acquire(power_meter)          |
+----------------------------------------------+
| [+ Add Step] [▶ Run] [💾 Save Template]      |
+----------------------------------------------+
| Generated Rhai Preview:                      |
| for wavelength in range(700.0, 900.0, 21) {  |
|   maitai.set_wavelength(wavelength);         |
|   sleep(0.5);                                |
|   power_meter.read();                        |
| }                                            |
+----------------------------------------------+
```

##### 3.2 Plan Integration
- Connect to daq-experiment RunEngine Plans
- GridScan, TimeSeries, ParameterSweep as built-in templates
- Custom Plan upload

#### 4. Logging Panel

**Inspired by:** DynExp's structured HTML logging

**Features:**
- Structured log entries (timestamp, level, source, message)
- Color-coded by level (DEBUG, INFO, WARN, ERROR)
- Filterable by source (device, engine, GUI)
- Searchable
- Export to file (plain text, HTML, JSON)
- Persistent across sessions
- Click-to-copy individual entries

**Data Model:**
```rust
struct LogEntry {
    timestamp: DateTime<Utc>,
    level: LogLevel,
    source: LogSource,
    message: String,
    context: Option<HashMap<String, String>>, // Extra metadata
}

enum LogSource {
    Device(String),
    RunEngine,
    Gui,
    System,
}
```

#### 5. Data Browser / Run History

**Inspired by:** ScopeFoundry's DataBrowser

**Features:**
- List past experiments with metadata (date, operator, scan type)
- Quick preview (thumbnail for images, first 100 points for scans)
- Search/filter by metadata
- Re-run from history
- Compare runs (overlay plots)
- Export subset

---

## Implementation Phases

### Phase 1: Foundation (bd-gui-phase1)

**Goal:** Instrument Manager + Basic Live Visualization

1. Create `InstrumentManagerPanel` with device tree view
2. Add device grouping by type in gRPC DeviceInfo
3. Implement `SignalPlotterPanel` with single trace
4. Add gRPC streaming subscription for observables
5. Create dockable panel infrastructure (egui_dock consideration)

**Files to Create/Modify:**
- `crates/daq-egui/src/panels/instrument_manager.rs` (new)
- `crates/daq-egui/src/panels/signal_plotter.rs` (new)
- `crates/daq-egui/src/app.rs` (panel integration)
- `crates/daq-proto/proto/daq.proto` (observable streaming)

**Acceptance Criteria:**
- [ ] Tree view shows all registered devices grouped by type
- [ ] Device state updates in real-time
- [ ] Single observable can be plotted with 1000-point history
- [ ] Auto-refresh rate configurable (100ms - 5s)

### Phase 2: Image Visualization (bd-gui-phase2)

**Goal:** 2D Image Viewer for Camera Streams

1. Create `ImageViewerPanel` with texture rendering
2. Implement colormap system
3. Add ROI selection with live statistics
4. Connect to existing PVCAM gRPC frame streaming
5. Add histogram overlay

**Files to Create/Modify:**
- `crates/daq-egui/src/panels/image_viewer.rs` (new)
- `crates/daq-egui/src/widgets/colormap.rs` (new)
- `crates/daq-egui/src/widgets/roi_selector.rs` (new)

**Acceptance Criteria:**
- [ ] Live camera frames displayed at 10+ FPS
- [ ] ROI selection shows mean/std/min/max
- [ ] Multiple colormaps selectable
- [ ] Zoom/pan functional

### Phase 3: Experiment Designer (bd-gui-phase3)

**Goal:** Visual Scan Builder

1. Create `ScanDesignerPanel` with step sequencer
2. Implement step types (Move, Acquire, Wait, Loop)
3. Add Rhai script generation from steps
4. Create template save/load system
5. Integrate with RunEngine Plans

**Files to Create/Modify:**
- `crates/daq-egui/src/panels/scan_designer.rs` (new)
- `crates/daq-egui/src/widgets/step_editor.rs` (new)
- `crates/daq-scripting/src/codegen.rs` (new - Rhai generation)

**Acceptance Criteria:**
- [ ] 5+ step types available
- [ ] Generated Rhai script is syntactically correct
- [ ] Templates save/load from JSON
- [ ] GridScan/TimeSeries templates pre-installed

### Phase 4: Logging & History (bd-gui-phase4)

**Goal:** Structured Logging + Run History

1. Create `LoggingPanel` with structured entries
2. Add log filtering and search
3. Create `RunHistoryPanel` with metadata browser
4. Implement experiment comparison view
5. Add export functionality

**Files to Create/Modify:**
- `crates/daq-egui/src/panels/logging.rs` (refactor from current Logs)
- `crates/daq-egui/src/panels/run_history.rs` (new)
- `crates/daq-storage/src/metadata_index.rs` (new - for search)

**Acceptance Criteria:**
- [ ] Logs filterable by level and source
- [ ] Past 100 runs queryable by metadata
- [ ] Quick preview shows scan summary
- [ ] Export to HTML/JSON functional

### Phase 5: Polish & Integration (bd-gui-phase5)

**Goal:** Complete Workbench Experience

1. Implement dockable/tiled panel layout
2. Add workspace save/restore
3. Create keyboard shortcuts
4. Add theme switching (light/dark)
5. Performance optimization (batch renders)
6. User documentation

**Acceptance Criteria:**
- [ ] Panel layout persists across sessions
- [ ] Common actions have keyboard shortcuts
- [ ] 60 FPS UI with 4 active panels
- [ ] User guide covers all new features

---

## CRITICAL RESEARCH: Live Data Streaming Architecture

### Research Objective

**Compare approaches for streaming live data (1D signals, 2D images, spectra) from the daemon to GUI visualization panels.** The user suspects Rerun.io may be simpler than custom gRPC streaming implementations.

### Current State Analysis

**Existing Rerun Integration (`crates/daq-egui/src/main_rerun.rs`):**
- `daq-rerun` binary embeds Rerun viewer alongside DAQ controls
- Uses `re_grpc_client::stream()` to connect to daemon's Rerun gRPC server
- Camera frames already stream via Rerun's tensor logging
- Dual-plane architecture: Control (gRPC to daemon) + Data (Rerun stream)

**Existing gRPC Streaming:**
- Frame streaming exists for PVCAM cameras via custom proto
- Observable streaming not yet implemented

---

### Option A: Rerun.io-First Architecture

**Concept:** Use Rerun as the primary data plane for all live visualization. The daemon logs data to Rerun, and the GUI embeds/connects to Rerun viewer.

**Advantages:**
| Benefit | Details |
|---------|---------|
| **Abstracted Complexity** | Rerun handles high-performance data streaming, buffering, and timeline management |
| **Rich Visualization** | Built-in support for tensors (images), scalars (1D), 3D plots, text logs |
| **Time Travel** | Scrub through historical data, compare timepoints |
| **Recording** | Save/load .rrd files for offline analysis and sharing |
| **Multi-client** | Multiple viewers can connect to same stream |
| **Proven Performance** | Designed for robotics/ML workloads with high-bandwidth sensor data |
| **Already Integrated** | `daq-rerun` binary exists, PVCAM streaming works |

**Disadvantages:**
| Concern | Details |
|---------|---------|
| **Dependency Weight** | Rerun adds ~50MB to binary, complex build with `re_viewer` |
| **Viewer Coupling** | Must embed Rerun viewer or run externally (less integrated feel) |
| **Customization Limits** | Cannot easily add custom analysis widgets (ROI stats, curve fitting) |
| **SDK Maturity** | Rerun SDK evolves rapidly, API changes between versions |
| **Learning Curve** | Team must learn Rerun's entity-path and timeline concepts |

**Architecture:**
```
┌─────────────────────────────────────────────────────────────┐
│                     rust-daq-daemon                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │ daq-hardware│───>│ Rerun SDK   │───>│ Rerun Server│     │
│  │ (sensors)   │    │ rec.log()   │    │ (gRPC)      │     │
│  └─────────────┘    └─────────────┘    └──────┬──────┘     │
└────────────────────────────────────────────────┼────────────┘
                                                 │ rerun+http://
                                                 ▼
┌─────────────────────────────────────────────────────────────┐
│                     daq-egui (GUI)                           │
│  ┌─────────────┐    ┌─────────────────────────────────┐     │
│  │ DAQ Control │    │        Embedded Rerun Viewer     │     │
│  │ Panels      │    │  - Camera frames (Tensor)        │     │
│  │ (egui)      │    │  - Signal traces (Scalar)        │     │
│  │             │    │  - Spectra (BarChart/Tensor)     │     │
│  └─────────────┘    └─────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

**Implementation Effort:** Low (extend existing `daq-rerun` pattern)

---

### Option B: Native gRPC Streaming

**Concept:** Build custom gRPC streaming services for each data type. GUI subscribes to streams and renders with egui_plot.

**Advantages:**
| Benefit | Details |
|---------|---------|
| **Full Control** | Custom proto messages, exactly what's needed |
| **Tight Integration** | Visualization widgets are native egui, deeply customizable |
| **Minimal Dependencies** | No large viewer framework to embed |
| **Simpler Build** | No re_viewer compilation (significant build time savings) |

**Disadvantages:**
| Concern | Details |
|---------|---------|
| **Implementation Burden** | Must build buffering, timeline, performance optimization |
| **No Time Travel** | Historical data requires separate storage/replay logic |
| **Frame Rate Management** | Must handle backpressure, frame dropping, synchronization |
| **Reinventing Wheel** | Rerun already solved these problems |

**Proto Extensions Required:**
```protobuf
// Observable streaming (1D signals)
message StreamObservablesRequest {
    repeated string device_ids = 1;
    repeated string observable_names = 2;
    uint32 sample_rate_hz = 3;
}

message ObservableValue {
    string device_id = 1;
    string observable_name = 2;
    double value = 3;
    string units = 4;
    google.protobuf.Timestamp timestamp = 5;
}

service DaqService {
    rpc StreamObservables(StreamObservablesRequest) returns (stream ObservableValue);
    rpc StreamFrames(StreamFramesRequest) returns (stream FrameData);  // Already exists
}
```

**Implementation Effort:** High (significant custom development)

---

### Option C: Hybrid Architecture (RECOMMENDED FOR RESEARCH)

**Concept:** Use Rerun for high-bandwidth data (images, dense time series) + native egui for control panels and lightweight visualization. Best of both worlds.

**Split by Data Type:**
| Data Type | Visualization Approach | Rationale |
|-----------|------------------------|-----------|
| **Camera Frames** | Rerun Tensor | High bandwidth (2048x2048 @ 10 FPS = 80 MB/s), needs GPU rendering |
| **Dense Time Series** | Rerun Scalar | 1000+ points/sec, benefits from Rerun's timeline |
| **Sparse Observables** | egui_plot | < 10 Hz updates, custom analysis widgets (ROI, fitting) |
| **Logging** | Native egui | Structured text, filtering, search |
| **Run History** | Native egui | Database queries, metadata search |
| **Experiment Designer** | Native egui | Forms, trees, no streaming data |

**Architecture:**
```
┌─────────────────────────────────────────────────────────────┐
│                     rust-daq-daemon                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │ daq-hardware│───>│ Rerun SDK   │───>│ Rerun Server│     │
│  │ (cameras)   │    │ (images)    │    │ :9876       │     │
│  ├─────────────┤    └─────────────┘    └──────┬──────┘     │
│  │ observables │───────────────────────────────┼───────────│
│  │ (sparse)    │                               │           │
│  └──────┬──────┘                               │           │
│         │ gRPC :50051                          │           │
└─────────┼──────────────────────────────────────┼───────────┘
          │                                      │
          ▼                                      ▼
┌─────────────────────────────────────────────────────────────┐
│                     daq-egui (GUI)                           │
│  ┌───────────────────────┬─────────────────────────────────┐│
│  │   Native egui Panels  │     Embedded Rerun Viewport     ││
│  │   - Instrument Mgr    │     - Camera live view          ││
│  │   - Quick Controls    │     - Dense signal traces       ││
│  │   - Experiment Design │     - Timeline scrubbing        ││
│  │   - Logging           │                                 ││
│  │   - Run History       │                                 ││
│  │   - Sparse plots      │                                 ││
│  └───────────────────────┴─────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

**Implementation Effort:** Medium (leverage Rerun where beneficial, custom where needed)

---

### Option D: Alternative Frameworks to Research

| Framework | Type | Pros | Cons |
|-----------|------|------|------|
| **egui_plot** | Native egui | Simple, no deps, customizable | Performance limits at high data rates |
| **plotters** | Rust plotting lib | Publication-quality, many chart types | Not interactive, backend-agnostic |
| **poloto** | Rust SVG plots | Simple API, lightweight | Static only |
| **Grafana** | External dashboard | Powerful, existing ecosystem | External process, web-based |
| **Web-based (eframe_template WASM)** | Browser rendering | Cross-platform, WebGPU | Latency, complexity |

---

### Research Tasks (Pre-Phase 1)

**Task R1: Benchmark Rerun vs egui_plot**
```bash
# Measure FPS and latency for:
# 1. 2048x2048 u16 camera frames @ 10 Hz
# 2. 1000 scalar samples/second for 60 seconds
# 3. Multiple simultaneous streams (4 cameras + 10 signals)
```

**Task R2: Evaluate Rerun Embedding Options**
- Full embedded viewer (current `daq-rerun` approach)
- Viewport-only embedding (no Rerun panels)
- External viewer + gRPC connection (separate process)
- WebViewer option (browser-based)

**Task R3: Prototype Hybrid Layout**
- Create mockup with Rerun viewport + native egui panels
- Test egui_dock with Rerun viewports
- Measure memory usage and startup time

**Task R4: Consult Gemini/Codex**
```
clink gemini "Compare Rerun.io vs custom gRPC streaming for
scientific instrument visualization. Consider 2048x2048 camera
frames at 10 Hz and 1000 scalar samples/second. What are the
performance tradeoffs and implementation complexity differences?"
```

---

### Decision Framework

**Choose Rerun-First (Option A) if:**
- Camera visualization is primary use case
- Time-travel debugging is valuable
- Recording/replay is required
- Team accepts Rerun SDK learning curve

**Choose Native gRPC (Option B) if:**
- Binary size critical (embedded deployment)
- Minimal dependencies required
- Deep customization of visualization widgets needed
- Simple data patterns (< 100 Hz, < 1MB/s)

**Choose Hybrid (Option C) if:**
- Mixed data patterns (cameras + sparse observables)
- Want best-in-class for each data type
- Willing to maintain two visualization systems
- Value both Rerun features AND custom egui widgets

---

### Preliminary Recommendation

**Start with Option C (Hybrid)** for research phase:

1. **Phase 1:** Extend existing `daq-rerun` for camera + dense signals
2. **Phase 1:** Build native `InstrumentManagerPanel` and `SignalPlotterPanel` for sparse data
3. **Phase 2:** Evaluate if Rerun viewport can replace custom `ImageViewerPanel`
4. **Phase 3-5:** Use native egui for experiment design, logging, history

This approach:
- Leverages existing Rerun integration (working PVCAM streaming)
- Allows comparison during development (which approach works better?)
- Doesn't commit fully to either extreme
- Produces usable GUI incrementally

---

## Technical Considerations

### egui Ecosystem

- **Current:** egui 0.31, eframe 0.31
- **Consider:** egui_dock for dockable panels, egui_extras for tables
- **Alternative:** Evaluate iced or Dioxus for declarative UI (major rewrite, defer to v0.8.0)

### gRPC Streaming (if pursuing Option B/C)

- **Observable Streaming:** Add `StreamObservables` RPC for real-time data
- **Frame Streaming:** Already exists for PVCAM, generalize for all FrameProducers

### Rerun Integration (if pursuing Option A/C)

- **Current Version:** rerun 0.27.3
- **Daemon Logging:** Use `RecordingStreamBuilder` with gRPC server
- **GUI Connection:** Use `re_grpc_client::stream()` to subscribe
- **Entity Paths:** Define hierarchy like `/devices/{id}/frames`, `/devices/{id}/observables/{name}`

### Data Model Extensions (gRPC)

```protobuf
message ObservableValue {
    string device_id = 1;
    string observable_name = 2;
    double value = 3;
    string units = 4;
    google.protobuf.Timestamp timestamp = 5;
}

service DaqService {
    rpc StreamObservables(StreamObservablesRequest) returns (stream ObservableValue);
}
```

### State Management

- Use existing `Parameter<T>` reactive system for GUI binding
- Consider Redux-like state container for complex panel interactions
- Persist workspace configuration in user settings

---

## Python Bindings Consideration

**User Request:** "might be beneficial to include Python bindings as a high-level API for less experienced users"

**Recommendation:** Defer to Phase 6 (post-GUI redesign)

**Approach when implemented:**
1. PyO3-based bindings for core types (`Device`, `Plan`, `Document`)
2. Python client for gRPC API (auto-generated from proto)
3. Jupyter kernel integration
4. NumPy/SciPy interop for data export

**Example API (future):**
```python
from rust_daq import connect, GridScan

daq = connect("localhost:50051")
stage = daq.device("stage_x")
camera = daq.device("prime_bsi")

scan = GridScan(stage, start=0, end=100, steps=101)
scan.add_detector(camera)

run = daq.execute(scan)
data = run.to_numpy()  # NumPy array
run.to_hdf5("experiment.h5")
```

---

## External AI Consultation (Gemini/Codex)

**Allowed Per User Request:** Use `clink` tool for:

1. **Gemini CLI:** Complex UI/UX design decisions, layout optimization
2. **Codex CLI:** Performance-critical render loops, egui optimization patterns

**Example Consultation Points:**
- Panel docking library evaluation
- Color science for scientific colormaps
- Efficient texture upload strategies for live imaging
- Keyboard shortcut conventions for scientific software

---

## Success Metrics

### Quantitative
- [ ] GUI renders at 60 FPS with 4 active panels
- [ ] Live plotting handles 1000 points/second
- [ ] Image viewer handles 2048x2048 @ 10 FPS
- [ ] Startup time < 2 seconds

### Qualitative
- [ ] Laboratory researcher can design scan without code
- [ ] Experiment history discoverable within 3 clicks
- [ ] Error states clearly communicated
- [ ] Documentation covers 90% of features

---

## Issue Tracking with bd

```bash
# Create epic
bd create "GUI Redesign: DynExp-Inspired Workbench" --tag epic

# Create phase issues
bd create "Phase 1: Instrument Manager + Signal Plotter" --dep <epic-id>
bd create "Phase 2: Image Viewer" --dep <phase1-id>
bd create "Phase 3: Scan Designer" --dep <phase2-id>
bd create "Phase 4: Logging + History" --dep <phase3-id>
bd create "Phase 5: Polish + Integration" --dep <phase4-id>

# Track progress
bd update <id> --status in_progress
bd close <id> --reason "Completed with ..."
```

---

## Execution Instructions

### For Research Agent

**Pre-Phase: Streaming Architecture Decision (CRITICAL)**
1. Read this prompt completely, especially "CRITICAL RESEARCH: Live Data Streaming Architecture"
2. Run existing `daq-rerun` binary to evaluate current Rerun integration:
   ```bash
   cargo run --bin daq-rerun --features rerun_viewer
   ```
3. Execute Research Tasks R1-R4 (benchmarking, embedding options, prototype, Gemini consultation)
4. Document streaming architecture decision in `.planning/phases/00-streaming-architecture-decision.md`
5. Update this prompt with chosen approach before proceeding to Phase 1

**Phase 1+ Research:**
1. Explore existing `crates/daq-egui/src/` for current implementation
2. Research egui_dock, egui_plot patterns
3. Consult Gemini for UI/UX best practices (`clink gemini`)
4. Document findings in `.planning/phases/01-research-findings.md`

### For Planning Agent

1. Read research findings
2. Create detailed task breakdown for each phase
3. Identify dependencies and parallel work
4. Estimate effort (T-shirt sizing)
5. Write phase plans to `.planning/phases/NN-PLAN.md`

### For Implementation Agent

1. Read phase plan
2. Create bd issues for tasks
3. Implement incrementally with tests
4. Update CLAUDE.md if new patterns established
5. Commit with conventional commits

---

## Appendix: Reference Screenshots

### DynExp
- Hierarchical Modules panel (left)
- Signal plotter with multiple traces (center)
- Image viewer with camera output (center)
- Log panel with structured entries (bottom)

### PyMoDAQ
- Dashboard with unified detector/actuator views
- DAQ Scan configuration form
- Live 2D viewer with colormap

### ScopeFoundry
- DataBrowser with run list
- Settings panel with LoggedQuantity bindings
- Measurement queue

(Screenshots to be captured during research phase)

---

**End of Meta-Prompt**
