# Project Development History

> Consolidated from `.history/planning/` (112 files, 1.4 MB).
> Generated 2026-02-20 from phase summaries, research docs, verification reports, and handoffs.
> Historical snapshot only: this file is useful for project history, not as the source of truth for current binaries, feature flags, config filenames, or crate counts.

## Table of Contents

- [Project Overview](#project-overview)
- [Architecture](#architecture)
- [Research Findings](#research-findings)
- [Development Phases](#development-phases)
  - [Phase 0: Streaming Architecture Decision](#phase-0-streaming-architecture-decision)
  - [Phase 1: Form-Based Scan Builder](#phase-1-form-based-scan-builder)
  - [Phase 2: Node Graph Editor Core](#phase-2-node-graph-editor-core)
  - [Phase 3: Plan Translation and Execution](#phase-3-plan-translation-and-execution)
  - [Phase 4: Sequences and Control Flow](#phase-4-sequences-and-control-flow)
  - [Phase 5: Live Visualization](#phase-5-live-visualization)
  - [Phase 6: Data Management](#phase-6-data-management)
  - [Phase 7: Code Export and Provenance](#phase-7-code-export-and-provenance)
  - [Phase 8: Advanced Scans](#phase-8-advanced-scans)
  - [Phases 9–10: Remaining Work](#phases-910-remaining-work)
- [Key Architectural Decisions](#key-architectural-decisions)
- [Technical Debt and Known Gaps](#technical-debt-and-known-gaps)
- [Handoffs](#handoffs)

---

## Project Overview

**Epic**: Experiment Design Module for rust-daq (bd-yu38, GUI Redesign Epic)

**Core Value**: Scientists can design and interactively run experiments without writing code, while power users retain full programmatic control — both workflows produce the same executable Plans.

**Status**: 80% complete (8/10 phases). 29 plans executed across 8 phases. Phases 9 (Templates and Library) and 10 (Polish and Integration) remain.

**Explicitly Out of Scope**: Round-trip code parsing (code → visual graph), hardware timing compilation (labscript-style), multi-user collaboration, cloud storage, AI-driven experiment design.

---

## Architecture

### Three-Layer Pattern

```
Presentation        →  Node Graph Editor (egui-snarl), visual source of truth
Intermediate Rep.   →  JSON graph structure (.expgraph), canonical serialization format
Execution Backend   →  Plan translation via topological sort → RunEngine integration
```

### GUI Architecture (Rerun-First)

Adopted **Rerun-First** visualization with selective egui_plot for micro-visualizations (5–10 second local scopes). Dual-plane: control via gRPC (:50051), data via Rerun (:9876).

**Panel layout** (egui_dock 0.18.0): Tab-based docking with drag-and-drop. Default: left sidebar (Instrument Manager, Quick Controls) + central Rerun viewer + bottom dock (Run History, Micro-plots).

### Stack Additions (from this epic)

| Dependency | Version | Purpose |
|---|---|---|
| egui-snarl | 0.9 | Node graph UI with serde support |
| undo | 0.52 | Command pattern undo/redo |
| rfd | 0.15 | Native file dialogs (save/load) |
| chrono | 0.4 | Timestamps for metadata |
| egui_code_editor | — | Syntax-highlighted Rhai preview |
| zarrs | 0.22.10 | Zarr V3 storage writer |
| object_store | — | Cloud storage backend for Zarr |
| find_peaks | 0.1.5 | Peak detection for adaptive scans |

### Key Patterns Established

- **AsyncExt pattern**: `PendingAction` + mpsc channels + `poll_async_results()` for non-blocking gRPC from UI
- **Graph-to-Plan translation**: Topological sort (Kahn's algorithm) → `PlanCommand` linearization
- **Checkpoint-based progress**: Node IDs embedded in checkpoint labels (`node_{id}_start`)
- **Configuration struct pattern**: Rich node variants with `Default` impls and tuple config structs
- **Unified command pattern**: Single `GraphEdit` enum for undo/redo (undo crate limitation)
- **Texture reuse**: `.set()` on existing `TextureHandle` instead of allocating new textures per frame
- **spawn_blocking**: All HDF5 I/O and RGBA conversion offloaded from GUI thread

---

## Research Findings

### Competitive Analysis

- **Code-first** tools (Bluesky, labscript): powerful but require programming
- **GUI-first** tools (PyMoDAQ, ScopeFoundry): accessible but limited
- **rust-daq unique position**: hybrid visual builder + code export + headless daemon, only Rust-based DAQ framework

### Feature Prioritization

**Must Have (v1)**: Parameter scans (1D/2D), pause/resume/abort, live plotting, auto-save to disk, metadata capture, run history, error recovery

**Differentiators**: Visual node-based builder (Orange/LabVIEW pattern), adaptive plans (Bluesky's killer feature), one-way code export, smart device mapping via capability traits, nested scans, dry run/simulation

**Anti-Features**: Bidirectional code↔graph sync (AI-complete problem), in-graph data analysis (causes bloated UI), hardware timing compilation, AI experiment design (documented failure modes), multi-user collaboration

### Critical Pitfalls Identified (Pre-Implementation)

1. **Round-trip code parsing** → ONE-WAY GENERATION ONLY. Visual is source of truth.
2. **Live parameter editing without state isolation** → Checkpoint-based parameter injection only. Log every change.
3. **Dataflow cycle detection failure** → Static cycle detection (topological sort) before execution. DAG-only.
4. **Visual spaghetti** → Group nodes/subgraphs from the start. Auto-layout. Escape hatch to code.
5. **Missing execution provenance** → Graph versioning (snapshot JSON at Start). Complete metadata.

---

## Development Phases

### Phase 0: Streaming Architecture Decision

**Decision**: Adopted Rerun-First architecture for all primary visualization (cameras, 1D signals, spectra, logs). egui_plot reserved for micro-visualizations only.

**Rationale**: Rerun already integrated for PVCAM camera streaming at 80 MB/s. Lowest marginal complexity.

---

### Phase 1: Form-Based Scan Builder

**Goal**: Scientists can configure and execute 1D/2D scans using simple forms, with live plotting and auto-save.

**Status**: ✅ PASSED (12/12 truths verified)

**What was built** (3 plans):
- `ScanBuilderPanel` (1565 lines) with device discovery, 1D/2D mode toggle, form validation (red borders + tooltips), scan preview
- Execution controls: `ExecutionState` enum (Idle/Running/Aborting), Start/Abort buttons, progress bar with ETA
- Document streaming via gRPC `client.stream_documents()` with mpsc relay
- Live egui_plot: line plot + scatter mode for 1D, colored scatter for 2D
- Completion summary modal (run ID, duration, points, saved path)

**Files created**: `crates/ui/src/panels/scan_builder.rs`
**Files modified**: `crates/ui/src/panels/mod.rs`, `crates/ui/src/app.rs`, `crates/ui/src/client.rs`

---

### Phase 2: Node Graph Editor Core

**Goal**: Visual node-based experiment designer with drag-and-drop, wire connections, property editing, undo/redo, and save/load.

**Status**: ✅ PASSED (4/4 success criteria)

**What was built** (4 plans):
- egui-snarl 0.9 integration with `ExperimentNode` enum (Scan, Acquire, Move, Wait, Loop)
- `ExperimentViewer` implementing `SnarlViewer` trait for custom node rendering
- `NodePalette` widget with 5 draggable node types + context menu creation
- Wire connection validation (Flow vs LoopBody pin types)
- `PropertyInspector` widget with per-node property editing
- `GraphEdit` unified enum implementing `undo::Edit` trait (Ctrl+Z / Ctrl+Y)
- `GraphFile` wrapper with JSON serialization, native file dialogs (.expgraph), version metadata
- Validation error display in status bar + property inspector

**Cycle detection**: Kahn's algorithm (topological sort)

**Files created**: `crates/ui/src/graph/{mod,nodes,viewer,validation,commands,serialization}.rs`, `crates/ui/src/widgets/{node_palette,property_inspector}.rs`, `crates/ui/src/panels/experiment_designer.rs`

---

### Phase 3: Plan Translation and Execution

**Goal**: Visual experiments translate to executable Plans and run via RunEngine with pause/resume and progress tracking.

**Status**: ⚠️ GAPS_FOUND (3/4 truths verified, human approved)

**What was built** (4 plans):
- `GraphPlan` struct implementing `Plan` trait — walks node graph via topological sort
- `DaqClient` engine control methods: `pause_engine`, `resume_engine`, `get_engine_status`
- `ExecutionState` module with engine state, active node, progress, ETA
- Run/Pause/Resume/Abort toolbar with 500ms status polling
- `RuntimeParameterEditor` widget for modifying parameters while paused
- Pre-run validation (cycle detection, device availability)

**Known gaps**:
1. `GraphPlan` not sent to server (TODO at `experiment_designer.rs:795`) — translation works, UI tracks state, but plan never queued via gRPC
2. Visual node highlighting not activated — egui-snarl API doesn't expose header color customization

**Files created**: `crates/ui/src/graph/{translation,execution_state}.rs`, `crates/ui/src/widgets/runtime_parameter_editor.rs`

---

### Phase 4: Sequences and Control Flow

**Goal**: Multi-step sequences with moves, waits, acquire, and loops.

**Status**: ✅ PASSED (4/4 success criteria)

**What was built** (3 plans):
- `MoveConfig` (Absolute/Relative mode, wait_settled flag)
- `WaitCondition` enum (Duration/Threshold/Stability)
- `AcquireConfig` (optional exposure override, frame_count for burst)
- `LoopConfig` (Count/Condition/Infinite termination with safety limits)
- `DeviceSelector` widget with autocomplete
- Loop body sub-graph detection via BFS from body output pin
- Loop unrolling for count-based loops (N iterations)

**Minor limitations**: WaitSettled command is checkpoint-only stub. Threshold/Stability wait runtime evaluation deferred. Condition-based loops use max_iterations as safety fallback.

**Files created**: `crates/ui/src/widgets/device_selector.rs`
**Files modified**: `crates/ui/src/graph/{nodes,translation,validation}.rs`, `crates/ui/src/widgets/property_inspector.rs` (grew to 495 lines)

---

### Phase 5: Live Visualization

**Goal**: Real-time plots and images updating during acquisition with auto-scaling and multi-detector support.

**Status**: ✅ PASSED (3/3 truths verified)

**What was built** (5 plans):
- `AutoScalePlot` widget (304 lines): grow-to-fit scaling, per-axis lock controls, reset (6 tests)
- `MultiDetectorGrid` (269 lines): automatic layout via `egui_extras::StripBuilder`, mixed camera/plot panels, dynamic sizing (1×1, 1×2, 2×2)
- `LiveVisualizationPanel` (558 lines): FPS tracking (2s rolling window), camera frame display with aspect-preserving fit, line plot integration
- Streaming integration: `StreamFrames` → `FrameUpdate` → `frame_tx` for cameras; `StreamDocuments` → `Event` filter → `DataUpdate` → `data_tx` for plots
- Background RGBA conversion offloaded to dedicated thread with buffer recycling

**Files created**: `crates/ui/src/widgets/auto_scale_plot.rs`, `crates/ui/src/panels/{multi_detector_grid,live_visualization}.rs`

---

### Phase 6: Data Management

**Goal**: Complete metadata capture, run history browsing, and comparison tools.

**Status**: ⚠️ GAPS_FOUND (3/4 truths verified)

**What was built** (4 plans):
- `MetadataEditor` widget (242 lines): sample_id, operator, purpose, notes, comma-separated tags, extensible custom key-value fields
- `RunHistoryPanel` (467 lines): table view via `egui_extras::TableBuilder`, search filter, detail view, async gRPC loading
- `hdf5_annotation.rs` (109 lines): post-acquisition annotation as HDF5 attributes in `/start` group
- `RunComparisonPanel` (420 lines): multi-run overlay plotting, 8-color distinct palette (matplotlib tab10), visibility toggles, HDF5 data loading via spawn_blocking

**Known gap**: `ExperimentDesignerPanel` extracts metadata but doesn't send to server (TODO at line 878). Root cause: Graph-based plans don't integrate with `QueuePlanRequest` gRPC. `ScanBuilderPanel` metadata flow works correctly.

**Files created**: `crates/ui/src/widgets/metadata_editor.rs`, `crates/ui/src/panels/{run_history,run_comparison}.rs`, `crates/storage/src/hdf5_annotation.rs`

---

### Phase 7: Code Export and Provenance

**Goal**: Complete provenance tracking with one-way code generation for reproducibility.

**Status**: ✅ PASSED (5/5 truths verified)

**What was built** (4 plans):
- `codegen.rs` (705 lines): `graph_to_rhai_script()` with topological sort, per-node `to_rhai()`, readable comments, proper indentation for nested loops (15 tests)
- Git provenance via `common/build.rs` (48 lines): build-time git metadata capture (SHA, dirty flag, commit date). `ExperimentManifest` fields: `git_commit`, `git_dirty`, `graph_hash`, `graph_file`. vergen rejected due to version conflicts — uses manual git commands.
- `CodePreviewPanel` (211 lines): syntax-highlighted Rhai via `egui_code_editor`, toggle in toolbar, real-time regeneration on graph edits, copy to clipboard, theme selector
- `ScriptEditorPanel` (139 lines): export to .rhai file, editable script, "Eject to Script" mode with confirmation

**One-way export**: Visual editor is source of truth. Code is a read-only artifact. No round-trip.

**Files created**: `crates/ui/src/graph/codegen.rs`, `crates/common/build.rs`, `crates/ui/src/panels/{code_preview,script_editor}.rs`

---

### Phase 8: Advanced Scans

**Goal**: Nested multi-dimensional scans and adaptive scans responding to acquired data, with Zarr V3 storage.

**Status**: ✅ COMPLETE (all features built, pending RunEngine integration for adaptive triggers)

**What was built** (7 plans):
- **Zarr V3 storage** (`zarr_writer.rs`, ~600 lines): `zarrs` crate, `ZarrArrayBuilder` fluent API, Xarray-compatible encoding (`_ARRAY_DIMENSIONS`), `object_store` integration, feature flag `storage_zarr` (7 tests)
- **NestedScan node**: `NestedScanConfig` with outer/inner `ScanDimension`, body output pin, translation via outer×inner iteration, purple/violet palette color
- **AdaptiveScan node**: `TriggerCondition` (Threshold, PeakDetection), `AdaptiveAction` (Zoom2x/4x, MoveToPeak, AcquireAtPeak, MarkAndContinue), `TriggerLogic` (Any/All), `require_approval` flag, dark orange palette color
- **Trigger evaluation** (`adaptive.rs`): `detect_peaks()` using find_peaks prominence filtering, `evaluate_triggers()` with AND/OR logic, checkpoint-based translation
- **Nested progress display**: `DimensionProgress`/`NestedProgress` structs, format as "wavelength 3/10, position 45/100" or flat "345/1000 (34.5%)", dimensional indices (`_outer_idx`, `_inner_idx`) in EmitEvent for Zarr coordinates
- **Adaptive alert modal**: Modal dialog with trigger info, peak details, approve/cancel, 3-second auto-proceed for non-approval triggers

**Zarr V3 rationale**: Better Rust ecosystem than HDF5, cloud-native, chunked storage, Xarray Python interop.

**Files created**: `crates/storage/src/zarr_writer.rs`, `crates/ui/src/graph/adaptive.rs`, `crates/ui/src/widgets/adaptive_alert.rs`
**Files modified**: `crates/ui/src/graph/{nodes,translation,validation,codegen,execution_state}.rs`

---

### Phases 9–10: Remaining Work

**Phase 9 — Templates and Library** (planning stage):
- Requirements: LIB-03, LIB-04, LIB-05, EDIT-03
- Template library, custom templates, version history, subgraph grouping (deferred from Phase 2)

**Phase 10 — Polish and Integration** (not started):
- Performance optimization, complete testing, documentation
- Cloud storage (S3/GCS) via object_store
- Sharding codec configuration for Zarr

---

## Key Architectural Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Visual editor library | egui-snarl 0.9 | Actively maintained, serde support, five-zone layout |
| DAG validation | daggy + Kahn's algorithm | Cycle detection at construction time |
| Undo/redo pattern | Command (not Memento) | Scales to large graphs; Memento deep-clones don't scale |
| Code generation | ONE-WAY ONLY (visual → Rhai) | Round-trip parsing is AI-complete, fragile |
| Validation | Real-time with 300ms debounce | Incremental checking, background thread |
| Execution control | Checkpoint-based pause/resume | Provenance logging at every checkpoint |
| Context menu | Primary node-add UX | More reliable than drag-drop (egui-snarl limitation) |
| Storage format (advanced) | Zarr V3 over HDF5 | Better Rust ecosystem, cloud-native, chunked |
| Live streaming | Rerun-First architecture | Already integrated for PVCAM at 80 MB/s |
| Metadata schema | HashMap<String, String> | Extensible without schema migrations |
| Git provenance | Manual build.rs (not vergen) | vergen-gitcl version conflicts |

---

## Technical Debt and Known Gaps

### Gaps From This Epic

1. **GraphPlan not sent to server** (Phase 3) — translation works, UI tracks state, but plan never queued via gRPC. TODO at `experiment_designer.rs:795`.
2. **ExperimentDesignerPanel metadata not sent** (Phase 6) — extracts metadata but doesn't integrate with `QueuePlanRequest`. ScanBuilderPanel works correctly.
3. **Visual node highlighting** (Phase 3) — egui-snarl doesn't expose header color API. Custom painter overlay needed.
4. **Adaptive trigger runtime evaluation** (Phase 8) — falls back to basic scan; RunEngine integration needed.
5. **Condition-based loop evaluation** (Phase 4) — uses max_iterations safety fallback; requires RunEngine enhancement.
6. **Device list async fetch** (Phase 4) — empty list triggers text field fallback.
7. **AcquisitionSummary proto** missing `plan_type` field.

### Pre-Existing Technical Debt

- **Server unwrap()/expect() panics**: gRPC services crash on mutex poisoning (`health_service.rs`, `server.rs`, `scan_service.rs`, `plugin_service.rs`)
- **PVCAM global static callback**: Only 1 camera instance supported; multi-camera causes silent data corruption
- **Large files**: `acquisition.rs` (3802 LOC), `image_viewer.rs` (2709 LOC), `instrument_manager` (1743 LOC)
- **Unsafe code**: 539+ unsafe blocks in ring buffer
- **PVCAM frame loss**: i32 overflow after ~5–6 hours at 100 FPS
- **gRPC binds 0.0.0.0** by default (no auth)
- **ELL14 multi-rotator**: One stuck device deadlocks all on RS-485 bus
- **GenericSerialDriver**: Silent timeout failures — returns `Ok("")` instead of error
- **MaiTai wavelength tuning**: Race condition — returns after 50ms without verifying settle
- **Memory leak**: `Box::leak` in factory introspection

### Pre-Existing Test Failure

- `graph::serialization::tests::test_version_check` — known failure, not addressed

---

## Handoffs

### egui-snarl Polish (Post-Phase 7)

Three fixes applied to `crates/ui/src/graph/viewer.rs` and `crates/ui/src/panels/experiment_designer.rs`:
1. Widget ID collision fix: `ui.push_id(node_id)` scopes ComboBox IDs per node
2. Header validation colors: red (error `#782828`), green (executing `#286428`), blue (completed `#283C50`)
3. Custom SnarlStyle: orthogonal wires, no grid, larger pins (8.0), blue selection stroke

### Phase 7 egui-snarl Critical Bugs

Three bugs fixed during 07-04 human verification:
1. **Context menu disappearing**: `any_click()` detected same right-click; fix: exclude `secondary_clicked()` from dismissal
2. **Node selection always empty**: Event handlers rendered before SnarlWidget; fix: reorder rendering (widget first), use `widget.get_selected_nodes(ui)`. Note: egui-snarl 0.9 requires **Shift+Click** to select.
3. **Drag-and-drop broken**: `available_rect_before_wrap()` called after widget consumed space; fix: capture canvas rect before widget

**Critical rendering order**:
```rust
// 1. Render Graph FIRST
widget.show(&mut self.snarl, &mut self.viewer, ui);
// 2. Render Overlays AFTER
self.handle_context_menu(ui);
self.handle_canvas_drop_at(ui, canvas_rect);
// 3. Query selection LAST
let selected = widget.get_selected_nodes(ui);
```

### Technical Debt Epic (bd-4asi)

15 issues across driver and scripting infrastructure. Critical dependency chain: H1 (timeout fix) → R1 (refactor), H3 (capability gating) → M4 (config params). See beads for full tracking.

---

## Requirements Coverage

| ID | Requirement | Status |
|---|---|---|
| SCAN-01–04 | 1D, 2D, nested, adaptive scans | ✅ Complete |
| SEQ-01–04 | Move, wait, acquire, loops | ✅ Complete |
| EDIT-01–02 | Drag-drop nodes, property inspector | ✅ Complete |
| EDIT-03 | Subgraph grouping | 📋 Deferred to Phase 9 |
| EDIT-04–05 | Undo/redo, validation | ✅ Complete |
| EXEC-01–06 | Start, stop, pause, resume, modify params, progress | ✅ Complete |
| VIZ-01 | Live line plots | ⏳ Pending (framework built) |
| VIZ-02–03 | Live image display, auto-scale plots | ✅ Complete |
| DATA-01 | Auto-save | ⏳ Pending |
| DATA-02 | Metadata capture | ⚠️ Partial (ScanBuilder OK, ExperimentDesigner gap) |
| DATA-03–05 | Notes/tags, run history, multi-run comparison | ✅ Complete |
| LIB-01–05 | Save/load, templates, version history | 📋 Phase 9 |
| CODE-01–04 | Code preview, Rhai export, script editor, readable gen | ✅ Complete |

---

*This file consolidates all planning artifacts from `.history/planning/`. The original 112 files are preserved in the `.history/` directory for reference but excluded from semantic indexing via the `**/.*` dot-directory rule.*
