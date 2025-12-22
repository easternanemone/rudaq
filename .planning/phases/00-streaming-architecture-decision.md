# Pre-Phase: Streaming Architecture Decision

**Issue:** bd-gx9k
**Created:** 2025-12-21
**Status:** Complete

## Executive Summary

**DECISION: Adopt Rerun-First Architecture (Option A) with selective egui_plot for micro-visualizations**

### Rationale

1. **Lowest Marginal Complexity**: Rerun already integrated and working for PVCAM camera streaming (2048x2048 @ 10 Hz). Extending to 1D signals and spectra is cheaper than building parallel visualization infrastructure.

2. **Data Rate Analysis**: All proposed data streams are trivial compared to existing camera load:
   - Camera: ~80 MB/s (2048x2048 u16 @ 10 Hz)
   - Dense 1D signals: ~4 MB/s upper bound (1000 Hz f32)
   - Sparse observables: <1 KB/s (10 Hz)

   Rerun is not performance-constrained by these additions.

3. **Feature Alignment**: Recording/replay, time-travel debugging, and multi-client viewing are valuable for scientific instrument workflows (experiment post-mortems, correlation analysis).

4. **Binary Size Already Committed**: Shipping Rerun viewer for cameras (~50MB). Adding 1D/spectra visualization doesn't change this constraint.

5. **Unified Timeline Semantics**: Single time/history system across all data types (cameras, signals, logs) simplifies correlation and debugging.

---

## Architecture Analysis

### Option A: Rerun-First (SELECTED)

**Use Rerun for:**
- Camera frames (existing)
- Dense time series (>100 Hz signals)
- Sparse observables (~10 Hz)
- Spectra (as curves or images)
- Structured logging

**Use egui_plot for:**
- Micro-visualizations: 5-10 second local scopes tied to parameter panels
- "UI affordances" not requiring long history or replay
- Populated by small local buffers (not a general-purpose visualization system)

**Architecture:**
```
┌─────────────────────────────────────────────────────────────┐
│                     rust-daq-daemon                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │ daq-hardware│───>│ Rerun SDK   │───>│ Rerun Server│     │
│  │ (all sensors│    │ rec.log()   │    │ (gRPC :9876)│     │
│  │  & actuators│    │ - cameras   │    │              │     │
│  │             │    │ - signals   │    │              │     │
│  │             │    │ - spectra   │    │              │     │
│  │             │    │ - logs      │    │              │     │
│  └─────────────┘    └─────────────┘    └──────┬──────┘     │
│                                                 │            │
│  ┌─────────────┐ gRPC :50051                   │            │
│  │ HardwareService (control plane)             │            │
│  └──────┬──────┘                               │            │
└─────────┼──────────────────────────────────────┼────────────┘
          │                                      │
          ▼                                      ▼
┌─────────────────────────────────────────────────────────────┐
│                     daq-egui (GUI)                           │
│  ┌───────────────────────┬─────────────────────────────────┐│
│  │   Native egui Panels  │     Embedded Rerun Viewer       ││
│  │   - Instrument Mgr    │     - Camera live view          ││
│  │   - Quick Controls    │     - Dense signal traces       ││
│  │   - Experiment Design │     - Sparse observables        ││
│  │   - Logging           │     - Spectra                   ││
│  │   - Run History       │     - Timeline scrubbing        ││
│  │   - Micro-plots       │     - Recording/replay          ││
│  │     (egui_plot)       │                                 ││
│  └───────────────────────┴─────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

**Pros:**
- Single visualization stack (Rerun viewer)
- Single timeline/recording semantics
- Replay and multi-client for all data types
- No parallel time/history systems to maintain
- Consistent operator experience
- egui remains focused on control interface

**Cons:**
- UX customization ceiling defined by Rerun
- If deeply integrated control+plot widgets needed, may bump into Rerun boundaries
- Binary size dominated by Rerun (~50MB viewer)

**Mitigation for Cons:**
- Use egui_plot for tightly-coupled control+plot micro-visualizations (e.g., "live beam current + threshold slider in same panel")
- Keep these as local 5-10 second buffers, not competing with Rerun's primary visualization role

---

### Option B: Native gRPC + egui_plot (REJECTED)

**Use native gRPC streaming and egui_plot for all visualization**

**Why Rejected:**
1. **Significantly more engineering effort** than Rerun-first
2. Must re-implement:
   - Data buffering (ring buffers per signal)
   - Timeline alignment/timestamps
   - History window management
   - Decimation/downsampling
   - Recording/replay infrastructure
   - Threading/backpressure handling
3. **No time-travel or multi-client** features without major development
4. **Binary size savings marginal**: Still shipping gRPC stack, only removes Rerun viewer
5. **Performance adequate but no better** than Rerun for stated data rates

**When to Reconsider:**
- If binary size becomes critical constraint (embedded target <100MB storage)
- If zero large external dependencies becomes top priority
- If team rejects Rerun learning curve despite already using it

---

### Option C: Hybrid (REJECTED for primary visualization)

**Use Rerun for high-bandwidth, egui_plot for sparse data**

**Why Rejected:**
1. **Cognitive overhead**: Two mental models ("check Rerun for X, GUI for Y")
2. **Cross-system correlation difficult**: Overlaying 10 Hz control signal with camera timestamps requires data duplication or manual alignment
3. **Duplicated infrastructure**: Maintain timeline/buffering in both systems
4. **Binary size unchanged**: Still shipping Rerun viewer
5. **Maintenance burden**: Two visualization systems to evolve and debug

**Note:** Hybrid ACCEPTED for micro-visualizations (see Option A), but REJECTED as architecture principle for primary data visualization.

---

## Research Task Results

### R1: Benchmark Concept Analysis (Analytical)

**Camera Streaming (Existing):**
- 2048×2048 u16 frames @ 10 Hz = ~80 MB/s
- Rerun handles this well in current `main_rerun.rs` implementation
- No reported performance issues

**Dense 1D Signals (Proposed):**
- 1000 Hz f32 samples = 4 KB/s per channel
- Upper bound: 1000 channels = 4 MB/s (unrealistic, expect <100 channels)
- Rerun scalar logging overhead: negligible vs camera bandwidth
- egui_plot capable of 100k+ points/frame with proper decimation

**Sparse Observables (Proposed):**
- 10 Hz updates = <1 KB/s
- Trivial for both Rerun and egui_plot

**Conclusion:** Both systems are performance-adequate. Rerun not constrained by adding these streams.

### R2: Rerun Embedding Options Evaluated

**Full Embedded Viewer (Current `daq-rerun` binary):**
- Pros: Self-contained, no external processes, works as demonstrated
- Cons: Large binary (~50MB viewer), full Rerun UI may be overwhelming
- **Status:** Currently working, recommend keeping

**Viewport-Only Embedding:**
- Requires Rerun API for selective panel hiding
- Not readily available in Rerun 0.27.3
- **Status:** Future consideration for cleaner integration

**External Viewer + gRPC:**
- Separate `rerun` process connects to daemon
- Pros: Smaller main GUI binary, Rerun updates independent
- Cons: Requires user to manage two processes, connection setup complexity
- **Status:** Not recommended for operator workflow

**WebViewer Option:**
- Browser-based Rerun viewer
- Pros: Cross-platform, no binary distribution
- Cons: Latency, requires web server, network security considerations
- **Status:** Defer to future remote viewing feature

**Recommendation:** Keep full embedded viewer (current approach), explore viewport-only when Rerun API matures.

### R3: Prototype Hybrid Layout (Deferred)

**Decision:** Not needed given selection of Rerun-First.

If Hybrid were pursued, would require:
- egui_dock integration to tile Rerun viewport alongside native panels
- Memory usage baseline: ~200MB (Rerun viewer + camera buffer)
- Startup time: ~1-2 seconds (acceptable)

### R4: External AI Consultation (Gemini via PAL MCP)

**Consultation Model:** gpt-5.1 (Gemini not available)

**Key Insights:**
1. **"Overkill" question resolved**: Rerun not overkill when already integrated for cameras. Marginal cost to add 1D signals is low.

2. **Complexity comparison**:
   - egui_plot path: Own buffering, timeline, decimation, replay, threading
   - Rerun path: Marshal data to API, leverage built-in timeline/replay

3. **Binary size**: Hybrid doesn't reduce size if already shipping Rerun for cameras.

4. **Recommendation aligned**: "Go Rerun-first for all data visualization, keep egui_plot only for strictly local, low-history UI widgets."

5. **Critical insight**: For scientific instrument workflows, recording/replay and multi-client are valuable features that justify Rerun's complexity.

**Full consultation transcript:** See research process above.

---

## Implementation Strategy

### Phase 1: Extend Rerun Integration

1. **Add Scalar Logging to Daemon:**
   ```rust
   // In daq-server/src/rerun_logger.rs
   pub async fn log_observable(&self, device_id: &str, observable: &str, value: f64) {
       let entity_path = format!("devices/{device_id}/observables/{observable}");
       self.rec.log(entity_path, &rerun::Scalar::new(value))?;
   }
   ```

2. **Stream Observables via Parameter System:**
   - When `Parameter<T>` broadcasts changes, also log to Rerun
   - Use `ParameterObserver` pattern to decouple Rerun from core types

3. **Spectrum Logging:**
   ```rust
   // Log FFT results as line plots
   let spectrum: Vec<[f64; 2]> = fft_result.iter()
       .enumerate()
       .map(|(i, val)| [i as f64, val.norm()])
       .collect();
   rec.log("spectra/signal_1", &rerun::LineStrips2D::new([spectrum]))?;
   ```

4. **Structured Logging Integration:**
   - Configure `tracing_subscriber` to also log to Rerun `TextLog`
   - Entity path: `logs/{level}/{source}`

### Phase 2: GUI Panel Reorganization (Native egui)

Focus native panels on:
- **Instrument Manager**: Tree view of devices (not data visualization)
- **Quick Controls**: Parameter sliders and buttons
- **Experiment Designer**: Scan builder forms
- **Run History**: Metadata browser and run selection
- **Micro-plots** (egui_plot): 5-10 second local scopes for tight control coupling

Do NOT duplicate Rerun's visualization in native panels.

### Phase 3: Workspace Layout (egui_dock)

```
+------------------------------------------------------------------+
|  Menu Bar: File | View | Experiment | Hardware | Help            |
+------------------------------------------------------------------+
|  Toolbar: [Connect] [New Experiment] [Run] [Pause] [Stop]        |
+------------------------------------------------------------------+
| Left Sidebar    |              Rerun Viewer (Embedded)            |
| (egui panels)   |                                                 |
|                 |  Timeline: [====|========] Scrubber             |
| Instrument      |                                                 |
| Manager         |  +-------------------------------------------+  |
|                 |  |  Camera View (Tensor)                      |  |
| Quick Controls  |  +-------------------------------------------+  |
|                 |  |  Signal Traces (Scalar) - multi-channel    |  |
|                 |  +-------------------------------------------+  |
|                 |  |  Spectrum (LineStrips2D)                   |  |
+-----------------+  +-------------------------------------------+  |
|  Bottom Dock    |  |  Logs (TextLog)                            |  |
|  - Run History  |  +-------------------------------------------+  |
|  - Micro-plots  |                                                 |
+-----------------+--------------------------------------------------+
|  Status Bar: Connection | Active Scan | Progress | Timestamp     |
+------------------------------------------------------------------+
```

---

## Proto Extensions (Not Required for Rerun-First)

Original plan included gRPC streaming services for observables:
```protobuf
service DaqService {
    rpc StreamObservables(StreamObservablesRequest) returns (stream ObservableValue);
}
```

**With Rerun-First, this is OPTIONAL:**
- Daemon logs directly to Rerun server
- GUI connects to Rerun gRPC (already implemented)
- Control plane remains gRPC (`HardwareService`)
- Data plane is Rerun (not custom gRPC)

**When to add custom gRPC streaming:**
- If egui_plot micro-visualizations need higher-rate data than Rerun exports
- If external non-Rerun clients need real-time data (e.g., MATLAB)
- If Rerun proves inadequate after implementation (fallback path)

---

## Dependencies and Versions

**Current:**
- egui: 0.31
- eframe: 0.31
- rerun: 0.27.3
- tokio: 1.x
- tonic: 0.10

**To Add:**
- egui_dock: 0.18 (for panel docking)
- egui_plot: 0.30 (already present in legacy codebase, verify compatibility)

**Version Alignment:**
- egui 0.31 released 2024-12
- egui_plot 0.30 works with egui 0.28-0.30 (needs upgrade to 0.31+ for compatibility)
- egui_dock 0.18 requires egui 0.30+ (compatible)

**Action:** Upgrade egui_plot to 0.31-compatible version (check for 0.34.0 which matches latest egui).

---

## Risk Assessment

### Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Rerun API changes in future versions | Medium | Medium | Pin Rerun version, test upgrades in isolation |
| UX customization limits | Medium | Low | Use egui_plot for cases needing tight integration |
| Binary size exceeds deployment limits | Low | High | If occurs, fallback to Option B (native gRPC) |
| Rerun performance degrades with many streams | Low | Medium | Implement stream selection/filtering in GUI |

### Workflow Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Team finds Rerun viewer unintuitive | Medium | Low | Training, documentation, maybe custom Rerun panel presets |
| Operators want custom derived plots not in Rerun | Medium | Medium | Use egui_plot for domain-specific visualizations |
| Multi-client access causes confusion (multiple viewers) | Low | Low | Document workflow, consider viewer-side access control |

---

## Success Metrics

### Quantitative

- [ ] All data types (camera, signals, spectra, logs) viewable in Rerun
- [ ] <2 second latency from hardware event to Rerun display
- [ ] 60 FPS GUI rendering with Rerun viewport active
- [ ] <500 MB total memory usage with 1 hour of buffered data

### Qualitative

- [ ] Operators can correlate camera frames with signal events using timeline scrubbing
- [ ] Experiment post-mortems enabled by replaying .rrd recordings
- [ ] No need for separate plotting tools (Python, MATLAB) for basic analysis
- [ ] Integration with egui control panels feels cohesive

---

## Deferred Decisions

1. **Custom Rerun spaces/blueprints**: Rerun 0.27.3 supports custom layouts. Explore pre-configured "experiment views" in Phase 5.

2. **Rerun SDK extensions**: If domain-specific visualizations needed (e.g., k-space plots), evaluate Rerun's extensibility vs egui_plot fallback.

3. **Data export from Rerun**: Operators may need CSV/HDF5 export. Check Rerun's export capabilities or build bridge.

4. **Python bindings**: Deferred to Phase 6 (post-GUI redesign). When implemented, Python can log directly to Rerun server.

---

## Next Steps

1. ✅ Document this decision (current file)
2. ✅ Update bd-gx9k with findings
3. → Proceed to Phase 1 Research (egui ecosystem, panel patterns)
4. → Create Phase 1 implementation plan

---

**Approved By:** Research Agent (Claude Code)
**Date:** 2025-12-21
**Issue:** bd-gx9k (Pre-Phase)
