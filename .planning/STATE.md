# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2025-01-22)

**Core value:** Scientists can design and interactively run experiments without writing code, while power users retain full programmatic control
**Current focus:** Phase 6 complete and verified, ready for Phase 7

## Current Position

Phase: 8 of 10 (Advanced Scans)
Plan: 1 of 7 (08-01 complete)
Status: In progress
Last activity: 2026-01-25 - Completed 08-01-PLAN.md (Zarr V3 Storage Foundation)

Progress: [███████░░░] 71%

## Performance Metrics

**Velocity:**
- Total plans completed: 26
- Average duration: 5.7min
- Total execution time: 2.5 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan | Status |
|-------|-------|----------|----------|--------|
| 01 | 3 | 17min | 5.7min | ✓ Complete |
| 02 | 4 | 36min | 9.0min | ✓ Complete |
| 03 | 4 | 28min | 7.0min | ✓ Complete |
| 04 | 3 | 14min | 4.7min | ✓ Complete |
| 05 | 5 | 14min | 2.8min | ✓ Complete |
| 06 | 4 | 34min | 8.5min | ✓ Complete |
| 07 | 4 | 31min | 7.8min | ✓ Complete |

**Recent Trend:**
- Last 5 plans: 07-01 (8min), 07-02 (4min), 07-03 (7min), 07-04 (12min)
- Trend: Phase 7 completed at 7.8min/plan average (full code export pipeline)

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Node-based visual editor chosen as primary interface (standard scientific workflow paradigm)
- One-way code generation established (visual is source of truth, code is export only)
- Parameter injection for live edits (RunEngine Checkpoint-based, structure immutable during execution)
- Context menu as primary node-add UX (more reliable than drag-drop with coordinate transforms)
- Unified GraphEdit enum for undo/redo (undo::Record<E> requires single E type)
- .expgraph file extension for experiment graphs (distinct from generic JSON)
- Validation errors in status bar + property inspector (avoids egui-snarl API complexity)
- Kahn's algorithm for cycle detection (standard, efficient O(V+E))
- Display cycle errors on first node (avoids overwhelming user with multiple errors)
- Skip per-node validation when cycle detected (cycles make execution impossible)
- Channel-based async communication for gRPC calls (non-blocking UI)
- ExecutionState cloned to viewer before render (cheap, avoids lifetime issues)
- Visual highlighting infrastructure ready (pending egui-snarl API support)
- Separate RuntimeParameterEditor from parameter_editor.rs (different use case: runtime vs device introspection)
- Configuration structs for node variants (cleaner than inline fields, easier to extend)
- Custom autocomplete widget instead of egui_autocomplete (version compatibility issues)
- Optional exposure_ms field in AcquireConfig (None = use device default)
- Safety limits on infinite loops via max_iterations (prevents truly unbounded execution)
- Stateless DeviceSelector usage (created per-render) instead of storing as field
- Empty device_ids parameter triggers fallback to text field (graceful degradation)
- Condition/termination type change creates new default variant (prevents data loss until user confirms)
- Loop body nodes detected via body output (pin 1) BFS traversal
- Body nodes excluded from main topological traversal to prevent double-translation
- Condition-based loops use max_iterations as safety fallback with warning
- Relative moves in loop bodies generate warnings (position compounds each iteration)
- Back-edges from loop body to ancestors detected as errors (prevent infinite recursion)
- Grow-only bounds for plot axes (prevents jarring visual jumps during live acquisition)
- Per-axis lock independence (X and Y axes lockable separately)
- Grid dimensions via cols = ceil(sqrt(n)), rows = ceil(n/cols) (roughly square layouts)
- Nested StripBuilder for responsive grids (vertical rows, horizontal columns)
- DetectorType enum for mixed detector support (Camera, LinePlot in same grid)
- FPS tracking uses 2-second rolling window for stable metrics
- Camera frames displayed with aspect-preserving fit-to-panel logic
- Separate update channels for frames and data (bounded SyncSender)
- Simple heuristic for detector classification: device_id containing 'camera'/'cam' = camera, else plot
- Visualization panel created on execution start, marked inactive on stop (panel persists for review)
- Collapsing header UI pattern for live visualization (non-intrusive, user-collapsible)
- Camera streams use Preview quality (30 FPS) for live visualization bandwidth optimization
- Document stream subscribes to all documents, filters Event payloads client-side
- Stream tasks aborted on stop_visualization() for proper cleanup
- Comma-separated tags field instead of structured selector (simplicity for MVP)
- All metadata fields optional to avoid workflow friction
- Auto-enrichment pattern: UI adds system provenance before queueing
- Tags serialized as JSON array for future structured filtering
- HDF5 attributes for post-acquisition metadata (user_notes, tags, annotated_at_ns)
- spawn_blocking pattern for HDF5 file I/O in async GUI context
- Auto-load annotations on run selection in detail view
- Checkbox selection for multi-run comparison (simpler than drag-drop)
- Matplotlib tab10 color palette for distinct run colors
- Visibility toggles via checkboxes (independent of selection state)
- HDF5 data loading on checkbox select (eager loading for responsive plotting)
- hdf5-metno 0.11.0 to match workspace version (prevents native library conflict)
- Manual git commands in build.rs instead of vergen-gitcl (avoids version conflicts, simpler)
- Optional provenance fields with serde(default) for backwards compatibility (ExperimentManifest)
- SHA256 hash of entire .expgraph file for graph provenance (simplest, most reliable)
- One-way Rhai code generation established (visual editor is source of truth, code is export artifact)
- Public topological_sort functions for code reuse between translation and codegen modules
- Float formatting with .1 precision for Rhai code readability
- egui_code_editor for syntax highlighting (compatible with egui 0.33)
- Graph version counter for change detection (prevents unnecessary regeneration)
- Code preview regenerated only when visible (performance optimization)
- ScriptEditorPanel for ejected mode (separate from graph editor, one-way conversion)
- Export to .rhai file with rfd file dialog integration
- Mode switching in ExperimentDesignerPanel (graph vs script editor via Option<ScriptEditorPanel>)
- Eject confirmation dialog prevents accidental one-way conversion
- Theme selector in script editor (Gruvbox Dark/Light, Aura Dark)
- Code preview uses show_inside() for proper tab layout (not window-level SidePanel)
- zarrs 0.22 with filesystem feature for Zarr V3 storage (08-01)
- _ARRAY_DIMENSIONS attribute for Xarray compatibility (08-01)
- ReadableWritableListableStorage Arc pattern since FilesystemStore not Clone (08-01)

### Pending Todos

- Device list async fetch from DaqClient in ExperimentDesignerPanel (TODO added in 04-02)

### Blockers/Concerns

- Pre-existing test failure in graph::serialization::tests::test_version_check (not introduced by 03-01)
- egui-snarl lacks custom header color API (visual node highlighting infrastructure ready but not applied)
- GraphPlan not queued to server yet (run_experiment TODO on line 878) - Phase 5 complete but awaits server integration
- ExperimentDesignerPanel metadata prepared but not sent (TODO at line 878)
- AcquisitionSummary proto missing plan_type field - needed for run history filtering

### Phase 3 Verification Notes

Human approved with 3/4 success criteria fully verified. Known gaps documented for future work:
1. GraphPlan not sent to server (UI-side translation complete, server integration deferred)
2. Visual node highlighting not activated (egui-snarl API limitation)

See: .planning/phases/03-plan-translation-and-execution/03-VERIFICATION.md

### Phase 5 Verification Notes

All 3/3 success criteria verified after gap closure (05-05):
1. ✓ User sees live camera frames in image viewer during acquisition
2. ✓ Plots auto-scale to data range with manual override option
3. ✓ Multiple plots update simultaneously for multi-detector experiments

VIZ-02 and VIZ-03 requirements satisfied.

See: .planning/phases/05-live-visualization/05-VERIFICATION.md

### Phase 6 Verification Notes

3/4 success criteria verified (1 partial due to inherited Phase 3 limitation):
1. ⚠️ Metadata captured with ScanBuilder runs (ExperimentDesigner metadata prepared but not sent - awaits Phase 3 server integration)
2. ✓ User can add custom notes/tags (MetadataEditor pre-execution + HDF5 annotations post-execution)
3. ✓ User can browse run history with search and view details (RunHistoryPanel)
4. ✓ User can compare multiple runs with overlaid plots (RunComparisonPanel)

DATA-03, DATA-04, DATA-05 requirements satisfied. DATA-02 partial (ScanBuilder ✓, ExperimentDesigner deferred).

See: .planning/phases/06-data-management/06-VERIFICATION.md

## Session Continuity

Last session: 2026-01-25
Stopped at: Completed 08-01-PLAN.md (Zarr V3 Storage Foundation)
Resume file: None
Next action: Execute 08-02-PLAN.md (Nested Scan Nodes)
