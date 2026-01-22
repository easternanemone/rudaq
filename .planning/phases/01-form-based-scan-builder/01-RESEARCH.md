# Phase 1: Form-Based Scan Builder - Research

**Researched:** 2026-01-22
**Domain:** RunEngine integration, egui form patterns, live plotting
**Confidence:** HIGH

## Summary

Research into the rust-daq codebase reveals a well-architected foundation for building form-based scan configuration and execution:

1. **RunEngine and Plan System**: Mature, Bluesky-inspired execution engine with document streaming protocol (Start, Descriptor, Event, Stop). Plans (`LineScan`, `GridScan`, `Count`) are declarative generators that yield commands. The `PlanBuilder` trait enables runtime plan construction from string parameters and device mappings.

2. **Device Registry**: Central `DeviceRegistry` provides capability-based device discovery (`Movable`, `Readable`, `FrameProducer`). Devices are accessed via typed getters (`get_movable()`, `get_readable()`). The registry supports both static device lists and dynamic queries.

3. **egui Patterns**: Established async integration patterns using `tokio::sync::mpsc` channels with `poll_async_results()` pattern. The `PendingAction` enum + `execute_action()` pattern cleanly separates UI mutations from async operations. `ScansPanel` provides a comprehensive reference implementation.

4. **Live Plotting**: `SignalPlotterPanel` demonstrates egui_plot integration with `ObservableUpdateSender` channel pattern for thread-safe async data streaming. Document streaming via `client.stream_documents()` provides real-time access to experiment data.

5. **Data Storage**: `DocumentWriter` auto-saves documents to HDF5/CSV during acquisition. No explicit save action required—persistence is automatic via document subscription.

**Primary recommendation:** Build a standalone `ScanBuilderPanel` following the `ScansPanel` pattern, using `PlanBuilder` for runtime plan construction, `stream_documents()` for live data, and `egui_plot` for inline visualization. The foundation is already built—this is primarily UI assembly.

## Standard Stack

### Core Dependencies

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `daq-experiment` | workspace | RunEngine, Plan trait, PlanBuilder | Foundation of experiment system |
| `daq-hardware` | workspace | DeviceRegistry, capability traits | Device discovery and control |
| `daq-proto` | workspace | gRPC types (QueuePlanRequest, Document) | Client-server protocol |
| `egui` | 0.29+ | UI framework | Core GUI framework |
| `egui_plot` | 0.29+ | Inline plotting | Line/scatter plot widgets |
| `tokio::sync::mpsc` | 1.x | Async channel | Established async integration pattern |

### Supporting Libraries

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `daq-storage` | workspace | HDF5/CSV writers | Auto-save during execution (no explicit integration needed) |
| `futures::StreamExt` | 0.3 | gRPC stream handling | Document streaming (`client.stream_documents()`) |
| `chrono` | 0.4 | Timestamp formatting | Default scan names, time display |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `PlanBuilder` trait | Direct `LineScan::new()` | PlanBuilder validates params and provides error messages |
| `egui_plot` | Rerun viewer | egui_plot is better for inline preview; Rerun for deep analysis |
| `stream_documents()` | Polling `client.get_progress()` | Streaming provides real-time updates without polling overhead |

**Installation:**
All dependencies are workspace members—no new external crates needed.

## Architecture Patterns

### Recommended Panel Structure

```rust
// crates/daq-egui/src/panels/scan_builder.rs
pub struct ScanBuilderPanel {
    // Form state
    scan_mode: ScanMode,  // 1D or 2D toggle
    device_list: Vec<DeviceCache>,
    selected_actuator_1d: Option<String>,
    selected_actuator_2d_x: Option<String>,
    selected_actuator_2d_y: Option<String>,
    selected_detectors: Vec<String>,

    // 1D parameters
    start_1d: String,
    stop_1d: String,
    points_1d: String,

    // 2D parameters
    x_start: String, x_stop: String, x_points: String,
    y_start: String, y_stop: String, y_points: String,

    // Execution state
    engine_state: EngineState,
    current_run_uid: Option<String>,
    progress: Option<ProgressState>,

    // Live plotting
    plot_data: HashMap<String, Vec<(f64, f64)>>,  // detector -> (actuator_pos, value)
    document_rx: Option<mpsc::Receiver<Document>>,

    // Async integration
    pending_action: Option<PendingAction>,
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
}
```

### Pattern 1: Device Discovery

**What:** Query device registry via gRPC, group by capability
**When to use:** Panel initialization, refresh button
**Example:**

```rust
// Source: crates/daq-egui/src/panels/devices.rs
async fn refresh_devices(client: &mut DaqClient) -> Result<Vec<DeviceCache>> {
    let response = client.list_devices().await?;
    let devices = response.into_iter()
        .map(|info| DeviceCache { info, parameters: vec![] })
        .collect();
    Ok(devices)
}

// Group devices for collapsible sections
fn group_by_capability(devices: &[DeviceCache]) -> HashMap<&'static str, Vec<&DeviceCache>> {
    let mut groups = HashMap::new();
    for device in devices {
        if device.info.is_movable {
            groups.entry("Actuators").or_insert_with(Vec::new).push(device);
        }
        if device.info.is_readable || device.info.is_frame_producer {
            groups.entry("Detectors").or_insert_with(Vec::new).push(device);
        }
    }
    groups
}
```

### Pattern 2: Plan Construction via PlanBuilder

**What:** Use `PlanBuilder` trait to validate and construct plans from form strings
**When to use:** Start button click, after validating form fields
**Example:**

```rust
// Source: crates/daq-experiment/src/plans.rs (LineScanBuilder)
use daq_experiment::plans::{LineScanBuilder, PlanBuilder};

fn build_plan_from_form(panel: &ScanBuilderPanel) -> Result<Box<dyn Plan>, String> {
    let mut parameters = HashMap::new();
    parameters.insert("start".to_string(), panel.start_1d.clone());
    parameters.insert("end".to_string(), panel.stop_1d.clone());
    parameters.insert("num_points".to_string(), panel.points_1d.clone());

    let mut device_mapping = HashMap::new();
    device_mapping.insert("motor".to_string(),
        panel.selected_actuator_1d.clone().ok_or("No actuator selected")?);
    device_mapping.insert("detector".to_string(),
        panel.selected_detectors.first().ok_or("No detector selected")?.clone());

    let builder = LineScanBuilder;
    builder.build(&parameters, &device_mapping)
}
```

**Key validation:** PlanBuilder implementations validate:
- Finite numbers (not NaN/infinity)
- Range limits (`num_points <= 10_000_000`)
- Required device mappings
- Start ≠ end for scans

### Pattern 3: Live Document Streaming

**What:** Subscribe to document stream, extract Event data, update plot
**When to use:** After queuing plan and starting engine
**Example:**

```rust
// Source: crates/daq-egui/src/panels/document_viewer.rs
fn start_document_subscription(&mut self, client: &mut DaqClient, runtime: &Runtime) {
    let (tx, rx) = mpsc::channel(100);
    self.document_rx = Some(rx);

    let mut client = client.clone();
    runtime.spawn(async move {
        let stream = client.stream_documents(None, vec![]).await?;
        while let Some(result) = stream.next().await {
            match result {
                Ok(doc) => {
                    if tx.send(doc).await.is_err() { break; }
                }
                Err(e) => { /* handle error */ }
            }
        }
    });
}

// In panel.ui(), poll for documents:
fn poll_documents(&mut self) {
    while let Ok(doc) = self.document_rx.as_mut().unwrap().try_recv() {
        match doc.doc_type() {
            DocType::Event => {
                let event = doc.event.unwrap();
                // Extract actuator position from event.positions
                // Extract detector value from event.data
                // Update plot_data
            }
            DocType::Stop => {
                self.execution_complete();
            }
            _ => {}
        }
    }
}
```

### Pattern 4: Async Action Pattern

**What:** Separate UI mutations from async operations using `PendingAction` + channels
**When to use:** All gRPC operations (queue, start, abort)
**Example:**

```rust
// Source: crates/daq-egui/src/panels/scans.rs
enum PendingAction {
    QueueAndStart { plan: Box<dyn Plan> },
    AbortExecution,
}

enum ActionResult {
    QueuedAndStarted { run_uid: String, error: Option<String> },
    Aborted { success: bool },
}

// In UI rendering:
if ui.button("Start Scan").clicked() {
    self.pending_action = Some(PendingAction::QueueAndStart {
        plan: self.build_plan()?
    });
}

// After UI frame:
if let Some(action) = self.pending_action.take() {
    self.execute_action(action, client, runtime);
}

fn execute_action(&mut self, action: PendingAction, client: &mut DaqClient, runtime: &Runtime) {
    let tx = self.action_tx.clone();
    let mut client = client.clone();

    runtime.spawn(async move {
        match action {
            PendingAction::QueueAndStart { plan } => {
                // Convert Box<dyn Plan> to gRPC parameters
                let run_uid = client.queue_plan(...).await?;
                client.start_engine().await?;
                let _ = tx.send(ActionResult::QueuedAndStarted { run_uid, error: None }).await;
            }
            _ => {}
        }
    });
}
```

### Pattern 5: egui_plot Integration

**What:** Use `egui_plot::Plot` for inline line plots with live updates
**When to use:** Displaying 1D scan data as it arrives
**Example:**

```rust
// Source: crates/daq-egui/src/panels/signal_plotter.rs
use egui_plot::{Plot, Line, PlotPoints};

fn render_live_plot(ui: &mut egui::Ui, data: &HashMap<String, Vec<(f64, f64)>>) {
    Plot::new("scan_plot")
        .view_aspect(2.0)
        .show(ui, |plot_ui| {
            for (detector_id, points) in data {
                let plot_points: PlotPoints = points.iter()
                    .map(|(x, y)| [*x, *y])
                    .collect();
                plot_ui.line(Line::new(plot_points).name(detector_id));
            }
        });
}
```

### Anti-Patterns to Avoid

- **Building plans without PlanBuilder**: Direct `LineScan::new()` bypasses validation—use `LineScanBuilder.build()` instead
- **Polling for status**: Use `stream_documents()` for real-time updates instead of repeated `get_engine_status()` calls
- **Blocking async in UI thread**: Always spawn gRPC calls in `runtime.spawn()`, never `.await` in `ui()` method
- **Storing Plan in widget state**: `Box<dyn Plan>` is not `Clone`—serialize to gRPC params immediately

## Don't Hand-Roll

Problems that already have existing solutions in the codebase:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Device discovery | Custom HTTP API | `DeviceRegistry` + `client.list_devices()` | Already provides capability filtering, metadata, connection status |
| Plan construction | String parsing + manual Plan instantiation | `PlanBuilder` trait | Validates params, checks ranges, provides error messages |
| Experiment execution | Custom state machine | `RunEngine` | Handles pause/resume, checkpoints, document emission |
| Progress tracking | Custom progress struct | `stream_documents()` | Real-time event stream with positions and data |
| Data persistence | Manual file writing | `DocumentWriter` subscription | Auto-saves HDF5/CSV without explicit integration |
| Form validation | Manual string parsing | PlanBuilder validation | Already checks finite numbers, range limits |

**Key insight:** The plan system is designed for runtime introspection. Don't bypass it with hardcoded UI—use PlanBuilder to dynamically validate and construct plans based on string parameters. This keeps the UI thin and the validation logic centralized.

## Common Pitfalls

### Pitfall 1: Premature Plan Construction

**What goes wrong:** Building `Box<dyn Plan>` objects during form editing fails because plans aren't Clone/Send across frames
**Why it happens:** Natural assumption that plans should live in panel state
**How to avoid:** Store only form strings in panel state. Build plan in `execute_action()` immediately before queueing
**Warning signs:** Borrow checker errors on `Box<dyn Plan>`, inability to clone panel state

### Pitfall 2: Document Type Confusion

**What goes wrong:** Treating all documents as Event docs, crashing on Start/Stop docs
**Why it happens:** gRPC `Document` is a oneof enum—each variant has different fields
**How to avoid:** Always check `doc_type()` before accessing variant-specific fields
**Warning signs:** Panics when accessing `.event` on Start docs, empty values

```rust
// WRONG: Assumes all docs are events
let event = doc.event.unwrap();  // Panics on Start/Stop docs

// RIGHT: Check type first
match doc.doc_type() {
    DocType::Event => {
        if let Some(event) = doc.event {
            // Process event
        }
    }
    DocType::Stop => {
        if let Some(stop) = doc.stop {
            // Handle completion
        }
    }
    _ => {}
}
```

### Pitfall 3: Progress Calculation Without Descriptor

**What goes wrong:** Can't map event data to axis positions without knowing which device is the actuator
**Why it happens:** Events contain generic `HashMap<String, f64>` data—must correlate with plan hints
**How to avoid:** Store `start_doc.hints` (movers) when receiving Start doc, use to extract position from `event.positions`
**Warning signs:** Plot shows no data, unable to determine X-axis values from events

### Pitfall 4: Forgetting CSV Storage

**What goes wrong:** Assuming only HDF5 is available, requiring `storage_hdf5` feature flag
**Why it happens:** HDF5 is well-documented, CSV is not
**How to avoid:** Check `feature = "storage_csv"` (enabled by default), support both formats
**Warning signs:** Daemon fails to start without HDF5 libraries, users can't save data

### Pitfall 5: Async Task Cleanup

**What goes wrong:** Document subscription tasks keep running after panel closes, leaking memory
**Why it happens:** Spawned tasks don't automatically stop when panel is dropped
**How to avoid:** Store `JoinHandle`, call `.abort()` in panel destructor or on "Stop" button
**Warning signs:** Multiple concurrent subscriptions, memory growth, duplicate events

```rust
// Store handle
self.subscription_task = Some(runtime.spawn(async move { ... }));

// Clean up when stopping
if let Some(handle) = self.subscription_task.take() {
    handle.abort();
}
```

## Code Examples

Verified patterns from official sources:

### Device Discovery and Grouping

```rust
// Source: crates/daq-egui/src/panels/devices.rs (lines 222-300)
use daq_proto::daq::DeviceInfo;

fn render_device_selector(ui: &mut egui::Ui, devices: &[DeviceInfo], selected: &mut Option<String>) {
    egui::ComboBox::from_id_salt("actuator_selector")
        .selected_text(selected.as_deref().unwrap_or("Select actuator..."))
        .show_ui(ui, |ui| {
            for device in devices.iter().filter(|d| d.is_movable) {
                let label = format!("{} ({})", device.name, device.id);
                ui.selectable_value(selected, Some(device.id.clone()), label);
            }
        });
}

// Collapsible device groups
egui::CollapsingHeader::new("Actuators")
    .default_open(true)
    .show(ui, |ui| {
        for device in devices.iter().filter(|d| d.is_movable) {
            ui.horizontal(|ui| {
                ui.label(&device.name);
                ui.label(format!("({})", device.id));
                if device.connection_status == "connected" {
                    ui.colored_label(egui::Color32::GREEN, "●");
                } else {
                    ui.colored_label(egui::Color32::RED, "●");
                }
            });
        }
    });
```

### Progress Bar with ETA

```rust
// Source: crates/daq-egui/src/panels/scans.rs (lines 383-391)
if scan.total_points > 0 {
    let progress = scan.current_point as f32 / scan.total_points as f32;
    let progress_bar = egui::ProgressBar::new(progress).text(format!(
        "{}/{} points ({:.1}%)",
        scan.current_point,
        scan.total_points,
        scan.progress_percent
    ));
    ui.add(progress_bar);
}

// ETA calculation (from elapsed time)
fn estimate_remaining_time(current: u32, total: u32, elapsed: Duration) -> Duration {
    if current == 0 {
        return Duration::ZERO;
    }
    let avg_time_per_point = elapsed.as_secs_f64() / current as f64;
    let remaining_points = total - current;
    Duration::from_secs_f64(avg_time_per_point * remaining_points as f64)
}
```

### Form Validation with Visual Feedback

```rust
// Source: crates/daq-egui/src/panels/scans.rs (lines 288-310)
fn render_validated_field(ui: &mut egui::Ui, label: &str, buffer: &mut String, error: &Option<String>) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.text_edit_singleline(buffer);

        // Red border on error
        if error.is_some() {
            response.highlight();
        }

        // Tooltip on hover
        if let Some(err) = error {
            response.on_hover_text(err);
        }
    });
}

// Live validation
fn validate_scan_params(panel: &ScanBuilderPanel) -> HashMap<&'static str, String> {
    let mut errors = HashMap::new();

    if let Err(e) = panel.start_1d.parse::<f64>() {
        errors.insert("start", format!("Invalid number: {}", e));
    }
    if let Ok(points) = panel.points_1d.parse::<u32>() {
        if points == 0 {
            errors.insert("points", "Must be > 0".to_string());
        }
    }

    errors
}
```

### Completion Summary Panel

```rust
// After receiving Stop document
fn show_completion_summary(ui: &mut egui::Ui, stop_doc: &StopDoc) {
    egui::Window::new("Scan Complete")
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            ui.heading(format!("Scan {}", stop_doc.exit_status));
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Duration:");
                ui.monospace(format_duration(stop_doc.time_ns - start_time_ns));
            });

            ui.horizontal(|ui| {
                ui.label("Total points:");
                ui.monospace(stop_doc.num_events.to_string());
            });

            ui.horizontal(|ui| {
                ui.label("Saved to:");
                ui.monospace(&output_path);
            });

            if ui.button("Close").clicked() {
                // Dismiss summary
            }
        });
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Imperative scan execution | Declarative Plan + RunEngine | v0.4 (Dec 2025) | Pause/resume now possible at checkpoints |
| Direct plan instantiation | PlanBuilder trait | v0.6 (Jan 2026) | Form-based UIs can validate before construction |
| Polling for progress | Document streaming | v0.4 (Dec 2025) | Real-time updates without polling overhead |
| Manual HDF5 integration | DocumentWriter auto-subscribe | v0.5 (Jan 2026) | Zero-code persistence |

**Deprecated/outdated:**
- `ScanProgress` pipeline: Replaced by `stream_documents()` for live progress tracking
- `count_with_detector()` function: Use `CountBuilder` with device mapping instead
- Direct `Plan` construction in UI: Use `PlanBuilder` for validation and error handling

## Open Questions

Things that couldn't be fully resolved:

1. **Heatmap Rendering for 2D Scans**
   - What we know: `egui_plot` supports scatter plots and lines
   - What's unclear: Whether egui_plot natively supports heatmaps or if we need image-based rendering
   - Recommendation: Start with scatter plot (points colored by value), explore `egui::ColorImage` if heatmap is critical

2. **Multi-Detector Overlay Strategy**
   - What we know: `egui_plot` supports multiple Line series with legend
   - What's unclear: How to handle different Y-axis scales (normalized vs. raw units)
   - Recommendation: Start with shared axis, add per-detector normalization toggle if needed

3. **Plan Serialization for Export**
   - What we know: `PlanBuilder.build()` takes string params, could be reversed
   - What's unclear: Whether plans should export to Python/Rhai scripts or just parameter JSON
   - Recommendation: Defer to later phases—Phase 1 is execution-only

## Sources

### Primary (HIGH confidence)
- **DeviceRegistry API**: `crates/daq-hardware/src/registry.rs` (lines 1-300)
- **RunEngine execution model**: `crates/daq-experiment/src/run_engine.rs` (lines 1-1176)
- **Plan trait and builders**: `crates/daq-experiment/src/plans.rs` (lines 1-1129)
- **egui async patterns**: `crates/daq-egui/src/panels/devices.rs`, `scans.rs`, `plan_runner.rs`
- **Document streaming**: `crates/daq-egui/src/panels/document_viewer.rs` (lines 1-200)
- **Live plotting**: `crates/daq-egui/src/panels/signal_plotter.rs` (lines 1-250)
- **Storage integration**: `crates/daq-storage/src/document_writer.rs` (lines 1-200)

### Secondary (MEDIUM confidence)
- HDF5/CSV format differences: Inferred from feature flags and writer implementations
- ETA calculation approach: Extrapolated from progress display patterns, not explicitly implemented

### Tertiary (LOW confidence)
- Drag-and-drop device selection: No existing implementation found—will require custom egui interaction handler

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All dependencies are workspace members, actively used in production panels
- Architecture patterns: HIGH - Multiple reference implementations exist (ScansPanel, DevicesPanel, PlanRunnerPanel)
- Pitfalls: HIGH - Directly observed issues in existing code (Box<dyn Plan> serialization limitations)
- 2D plotting: MEDIUM - No existing 2D scan visualization in GUI, extrapolating from 1D patterns

**Research date:** 2026-01-22
**Valid until:** 60 days (stable APIs, production codebase)
