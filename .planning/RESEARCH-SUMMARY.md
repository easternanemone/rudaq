# GUI Redesign Epic: Research Phase Summary

**Epic:** bd-yu38
**Research Task:** bd-gx9k (CLOSED)
**Date:** 2025-12-21
**Status:** ✅ Research Complete → Ready for Planning Phase

---

## Executive Summary

Comprehensive research phase complete for DynExp-inspired GUI redesign. **Rerun-First architecture selected** as the optimal approach given existing integration and project requirements. All technical blockers resolved, dependency audit complete, implementation patterns validated.

**Recommendation: Proceed to Phase 1 Implementation Planning**

---

## Key Decisions

### 1. Streaming Architecture: Rerun-First (Option A)

**Decision:** Use Rerun.io for ALL primary data visualization (cameras, 1D signals, spectra, logs). Reserve egui_plot for micro-visualizations only (5-10 second local scopes).

**Rationale:**
- ✅ Rerun already integrated and working for PVCAM camera streaming (80 MB/s)
- ✅ Proposed data rates trivial compared to camera (dense 1D: 4 MB/s, sparse: <1 KB/s)
- ✅ Lowest marginal complexity: extend existing Rerun logging vs building parallel system
- ✅ Unified timeline/recording semantics across all data types
- ✅ Recording/replay valuable for scientific workflows (experiment post-mortems, correlation)
- ✅ Binary size already committed to Rerun viewer (~50MB)
- ✅ External consultation (GPT-5.1 via PAL MCP) validated approach

**Architecture:**
```
Daemon (rust-daq-daemon)
  ├─ HardwareService (gRPC :50051) ──────> DaqClient (control plane)
  └─ Rerun SDK → Rerun Server (:9876) ──> re_viewer::App (data plane)

GUI (daq-egui)
  ├─ Native egui Panels (Instrument Mgr, Controls, Designer, History)
  └─ Embedded Rerun Viewer (Cameras, Signals, Spectra, Logs with timeline)
```

**Rejected Alternatives:**
- **Option B (Native gRPC + egui_plot):** Too much engineering effort to re-implement buffering, timeline, replay
- **Option C (Hybrid):** Cognitive overhead, duplicated infrastructure, no binary size savings

**Full analysis:** `.planning/phases/00-streaming-architecture-decision.md`

---

### 2. Panel Layout: egui_dock for Docking

**Selection:** egui_dock 0.18.0

**Features:**
- Tab-based docking with drag-and-drop
- Split panes (horizontal/vertical)
- State persistence via serde
- Compatible with egui 0.31

**Default Layout:**
```
+------------------------------------------------------------------+
|  Menu Bar | Toolbar                                              |
+------------------------------------------------------------------+
| Left Sidebar    |              Rerun Viewer (Embedded)            |
| (egui panels)   |  - Timeline scrubber                            |
|                 |  - Camera view (Tensor)                         |
| Instrument      |  - Signal traces (Scalar)                       |
| Manager         |  - Spectrum (LineStrips2D)                      |
|                 |  - Logs (TextLog)                               |
| Quick Controls  |                                                 |
|                 |                                                 |
+-----------------+-------------------------------------------------+
| Bottom Dock: Run History, Micro-plots (egui_plot)                |
+------------------------------------------------------------------+
| Status Bar                                                       |
+------------------------------------------------------------------+
```

**Persistence:** Workspace layout saved to `~/.local/share/rust-daq/workspace.json`

---

### 3. Visualization: Rerun Primary, egui_plot Secondary

**Rerun (Primary):**
- Camera frames (existing, working)
- Dense time series (>100 Hz signals)
- Sparse observables (~10 Hz)
- Spectra (as curves or images)
- Structured logging

**egui_plot (Secondary - Micro-visualizations only):**
- 5-10 second local scopes for control feedback
- Examples: "Live beam current + threshold slider", "Stage settling indicator"
- NOT for long-history or replay (that's Rerun's role)

**Version:** egui_plot 0.34.0 (upgrade from 0.30.0 for egui 0.31 compatibility)

---

## Current Implementation Analysis

### Proven Patterns

**Async Action Pattern (from `DevicesPanel`):**
```rust
pub struct Panel {
    // Cached state
    devices: Vec<DeviceCache>,

    // Async operation handling
    pending_action: Option<PendingAction>,
    action_tx: mpsc::Sender<ActionResult>,
    action_rx: mpsc::Receiver<ActionResult>,
    action_in_flight: usize,

    // UI state
    error: Option<String>,
    status: Option<String>,
}

impl Panel {
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        // Non-blocking poll, update cache, trigger repaint
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, client: &mut DaqClient, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());
        // Render UI
        // Queue pending actions
        self.execute_pending_actions(client, runtime);
    }
}
```

**Benefits:**
- UI never blocks (60 FPS maintained)
- Graceful error handling
- Instant UI feedback with async backend updates

**Recommendation:** Adopt this pattern for all new panels.

---

### Existing Rerun Integration (`main_rerun.rs`)

**Working Features:**
- Embedded Rerun viewer in egui app
- Dual-plane architecture (Control via gRPC, Data via Rerun)
- PVCAM camera streaming at 2048x2048 @ 10 Hz
- Async operations (connect, refresh, move, read, stream control)

**Gaps (to address in redesign):**
- ❌ No 1D signal streaming to Rerun yet
- ❌ Control panel is basic (just device list + buttons)
- ❌ No experiment designer or run history
- ❌ Logging is console-only (not in Rerun TextLog)

---

## Dependency Audit

### Current Versions (✅ All Compatible)

| Dependency | Current | Latest | Status |
|------------|---------|--------|--------|
| egui | 0.31 | 0.31 | ✅ Up to date |
| eframe | 0.31 | 0.31 | ✅ Up to date |
| egui_extras | 0.31 | 0.31 | ✅ Up to date |
| rerun | 0.27.3 | 0.27.3 | ✅ Up to date |
| tonic | 0.10 | 0.12 | ⚠️ Minor upgrade available (defer) |
| tokio | 1.x | 1.x | ✅ Compatible |

### To Add

| Dependency | Version | Purpose | Action |
|------------|---------|---------|--------|
| egui_dock | 0.18.0 | Dockable panel layout | Add to Cargo.toml |
| egui_plot | 0.34.0 | Micro-visualizations | Upgrade from 0.30.0 |

**Version Compatibility:** ✅ All deps compatible with egui 0.31

---

## External Consultation Summary

### GPT-5.1 (via PAL MCP) - Architecture Decision

**Question:** "Compare Rerun.io vs custom gRPC streaming for scientific instrument visualization..."

**Key Insights:**

1. **Rerun not overkill when already integrated**: Marginal cost to add 1D signals is LOW given camera streaming already works

2. **Complexity comparison:**
   - Rerun path: Marshal data to API, leverage built-in timeline/replay
   - egui_plot path: Own buffering, timeline, decimation, replay, threading ← MORE WORK

3. **Binary size:** Hybrid doesn't reduce if already shipping Rerun for cameras

4. **Recommendation:** "Go Rerun-first for all data visualization, keep egui_plot only for strictly local, low-history UI widgets"

5. **Scientific workflow value:** Recording/replay and multi-client are worth Rerun's complexity

**Full transcript:** See `00-streaming-architecture-decision.md` Section R4

---

## UI/UX Best Practices (Scientific Software)

### Principles Applied

1. **Immediate Feedback:** All actions provide instant visual feedback (spinners, status)
2. **Error Visibility:** Errors prominent with color+icon (⚠ RED) and actionable messages
3. **Undo/Replay:** Rerun's time-travel supports experimental workflow requirements
4. **Contextual Controls:** Parameters grouped by function/device, not flattened
5. **Visual Hierarchy:** Status indicators use color+shape for accessibility
6. **Data Correlation:** Synchronized timelines (Rerun timeline feature)
7. **Keyboard Shortcuts:** Power user support (Space=start/pause, Esc=stop, Ctrl+R=refresh)

### Status Indicator Pattern

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

---

## Panel Specifications (Phase 1)

### 1. Instrument Manager Panel

**Purpose:** Hierarchical device browser with quick controls

**Features:**
- Tree view grouped by device type (Cameras, Stages, Detectors, Lasers)
- Status indicators (online/offline/error/busy)
- Quick actions (context-dependent per capability)
- Filter/search
- Collapsible groups

**State Pattern:** Same as DevicesPanel (async actions, cached state)

### 2. Signal Plotter Panel (Micro-visualization)

**Purpose:** 5-10 second local scope for control feedback

**Features:**
- Single signal selection dropdown
- Configurable history (5s/10s/30s/60s)
- Auto-scale or manual Y range
- Current value display
- Built with egui_plot

**NOT for:** Long-term history, multi-signal correlation (use Rerun)

### 3. Quick Controls Panel

**Purpose:** Common parameters for selected device

**Features:**
- Dynamic content based on device capabilities
- Parameter sliders with validation
- Action buttons (Snap, Live, Stop for cameras)
- Device selector dropdown

**Reuses:** `ParameterEditor` widget pattern from existing code

---

## Performance Budget

### Target: 60 FPS (16.7 ms/frame)

**Allocation:**
- egui layout/widgets: 5 ms
- Rerun viewer: 8 ms
- Data updates (polling): 2 ms
- Misc (event handling): 1.7 ms

**Optimization Strategies:**
1. **Lazy repaints:** Only repaint when data changes
2. **Caching:** Avoid re-allocating UI strings every frame
3. **Culling:** Don't render collapsed panels
4. **Batch gRPC:** Future proto enhancement for multi-device state queries

---

## State Persistence

### User Settings Storage

**Locations:**
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

---

## Implementation Phases

### Phase 1: Foundation (Weeks 1-2) ← NEXT

1. Add egui_dock to Cargo.toml
2. Upgrade egui_plot to 0.34.0
3. Create InstrumentManagerPanel (basic tree view)
4. Create SignalPlotterPanel (single trace egui_plot)
5. Integrate egui_dock into `main_rerun.rs`

**Deliverables:**
- Working dockable layout
- Instrument Manager with device tree
- Single-signal plotter (egui_plot)
- No breaking changes to existing binaries

### Phase 2: Data Plane (Weeks 3-4)

1. Add scalar logging to Rerun (daemon-side)
2. Subscribe to parameter changes in InstrumentManager
3. Test 1D signal streaming to Rerun
4. Image Viewer panel (2D detector output with colormaps)

### Phase 3: Experiment Designer (Weeks 5-6)

1. Scan Builder panel (visual step sequencer)
2. Rhai script generation
3. Template save/load

### Phase 4: Logging & History (Weeks 7-8)

1. Logging panel redesign (structured, filterable)
2. Run History panel (metadata browser)
3. Export functionality

### Phase 5: Polish & Integration (Weeks 9-10)

1. Keyboard shortcuts
2. Theme switching
3. Performance optimization
4. User documentation

---

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Rerun API changes in future | Medium | Medium | Pin version, test upgrades in isolation |
| UX customization limits | Medium | Low | Use egui_plot for tight integration cases |
| Binary size exceeds limits | Low | High | Fallback to Option B if needed |
| Rerun performance degrades | Low | Medium | Stream selection/filtering in GUI |
| Team finds Rerun unintuitive | Medium | Low | Training, documentation, custom presets |

---

## Success Metrics

### Quantitative

- [ ] All data types viewable in Rerun (cameras, signals, spectra, logs)
- [ ] <2 second latency from hardware event to Rerun display
- [ ] 60 FPS GUI rendering with Rerun viewport active
- [ ] <500 MB total memory usage with 1 hour buffered data

### Qualitative

- [ ] Operators correlate camera frames with signals using timeline
- [ ] Experiment post-mortems enabled by .rrd replay
- [ ] No need for separate plotting tools (Python/MATLAB) for basic analysis
- [ ] Integration with egui control panels feels cohesive

---

## Deliverables

1. ✅ **Architecture Decision Document**
   - File: `.planning/phases/00-streaming-architecture-decision.md`
   - Content: Detailed analysis of Rerun vs gRPC vs Hybrid
   - Decision: Rerun-First with rationale and implementation strategy

2. ✅ **Research Findings Document**
   - File: `.planning/phases/01-research-findings.md`
   - Content: egui ecosystem analysis, panel patterns, widget specifications
   - Dependencies: Version audit and compatibility matrix

3. ✅ **Issue Tracking**
   - Epic: bd-yu38 (GUI Redesign Epic)
   - Pre-Phase: bd-gx9k (CLOSED - research complete)
   - Status: Ready for Phase 1 implementation planning

---

## Next Actions

### Immediate (User/Planning Agent)

1. **Review research findings:** Read both architecture decision and research documents
2. **Create Phase 1 Plan:** Detailed task breakdown for Foundation phase
3. **Create bd issues for Phase 1 tasks:** Break down into implementable chunks
4. **Set up project board:** Track progress across phases

### Phase 1 Kickoff (Implementation Agent)

1. **Dependency updates:**
   ```bash
   cd crates/daq-egui
   cargo add egui_dock@0.18.0
   cargo add egui_plot@0.34.0
   cargo update
   cargo check --all-features
   ```

2. **Create panel stubs:**
   - `src/panels/instrument_manager.rs`
   - `src/panels/signal_plotter.rs`
   - `src/panels/quick_controls.rs`

3. **Integrate egui_dock:**
   - Modify `main_rerun.rs` to use DockArea
   - Define WorkspaceTab enum
   - Implement TabViewer trait

4. **Test build:**
   ```bash
   cargo build --bin daq-rerun --features rerun_viewer
   cargo run --bin daq-rerun --features rerun_viewer
   ```

---

## References

### Documentation Created

- `.planning/gui-redesign-epic-PROMPT.md` - Original meta-prompt
- `.planning/phases/00-streaming-architecture-decision.md` - Architecture analysis
- `.planning/phases/01-research-findings.md` - egui ecosystem research
- `.planning/RESEARCH-SUMMARY.md` - This summary

### External Resources

- [egui_dock Repository](https://github.com/Adanos020/egui_dock)
- [egui_plot Repository](https://github.com/emilk/egui_plot)
- [Rerun.io Documentation](https://www.rerun.io/docs)
- [DynExp Platform](https://github.com/DynExpPlatform/DynExp) - Inspiration
- [PyMoDAQ Platform](https://pymodaq.readthedocs.io/) - Inspiration
- [ScopeFoundry Platform](https://github.com/ScopeFoundry/ScopeFoundry) - Inspiration

### Codebase References

- `crates/daq-egui/src/main_rerun.rs` - Current Rerun integration
- `crates/daq-egui/src/panels/devices.rs` - Async pattern template
- `crates/daq-egui/src/widgets/parameter_editor.rs` - Widget pattern template
- `crates/daq-proto/proto/daq.proto` - gRPC service definitions

---

**Research Phase Complete: 2025-12-21**
**Ready for: Phase 1 Implementation Planning**
**Epic: bd-yu38**
**Pre-Phase Task: bd-gx9k (CLOSED)**
