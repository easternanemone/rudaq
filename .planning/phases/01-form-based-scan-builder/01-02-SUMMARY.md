---
phase: 01-form-based-scan-builder
plan: 02
subsystem: ui
tags: [egui, egui_plot, grpc, streaming, execution]

# Dependency graph
requires:
  - phase: 01-form-based-scan-builder
    provides: ScanBuilderPanel with form UI (01-01)
provides:
  - ExecutionState management (Idle/Running/Aborting)
  - Start/Abort control buttons with form validation
  - Document streaming via gRPC
  - Progress tracking with ETA calculation
  - Live 1D plot rendering with egui_plot
affects: [01-03, future experiment panels]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Document subscription via mpsc channel relay
    - PlotPoints collection for egui_plot Line/Points
    - ActionResult pattern for async operation feedback

key-files:
  created: []
  modified:
    - crates/ui/src/panels/scan_builder.rs
    - crates/ui/src/client.rs

key-decisions:
  - "Document streaming uses mpsc relay to avoid borrow checker issues"
  - "Plot data stored as HashMap<String, Vec<(f64, f64)>> for multi-detector support"
  - "PlotStyle enum supports LineWithMarkers and ScatterOnly modes"

patterns-established:
  - "poll_async_results returns Option<T> to signal deferred actions needing client"
  - "poll_documents collects to Vec first to avoid borrow issues"

# Metrics
duration: 5min
completed: 2026-01-22
---

# Phase 01 Plan 02: Execution Controls and Live 1D Plotting Summary

**ScanBuilderPanel now has Start/Abort buttons, progress tracking with ETA, and live egui_plot visualization updating as data arrives**

## Performance

- **Duration:** 5 min
- **Started:** 2026-01-22T19:12:30Z
- **Completed:** 2026-01-22T19:17:47Z
- **Tasks:** 4 (1, 2, 3a, 3b combined into single commit)
- **Files modified:** 2

## Accomplishments

- ExecutionState enum (Idle, Running, Aborting) with UI disabling during execution
- Start button validates form, queues plan via queue_plan, starts engine via start_engine
- Abort button calls abort_plan and cleans up subscription
- Document streaming via stream_documents gRPC with mpsc relay pattern
- Progress bar with point count and ETA calculation based on elapsed time
- Live 1D plot with egui_plot showing multiple detector traces
- PlotStyle selector (LineWithMarkers, ScatterOnly)

## Task Commits

All tasks committed atomically as single feature commit:

1. **Tasks 1-3b: Execution controls and live plotting** - `d1e2db51` (feat)

## Files Created/Modified

- `crates/ui/src/panels/scan_builder.rs` - Added ExecutionState, PlotStyle, document streaming, progress bar, live plot
- `crates/ui/src/client.rs` - Added start_engine and abort_plan methods to DaqClient

## Decisions Made

- **Document streaming pattern:** Using mpsc channel relay from async task to panel polling, following DocumentViewerPanel pattern
- **Borrow checker solution:** poll_async_results returns Option<String> (run_uid) instead of taking client parameter, allowing client to be borrowed separately for subscription
- **Plot data structure:** HashMap<String, Vec<(f64, f64)>> keyed by detector_id, supports multiple detectors with distinct colors
- **PlotPoints handling:** Create Vec<[f64; 2]> first, then convert to PlotPoints via collect() since PlotPoints doesn't implement Clone

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added missing start_engine and abort_plan methods to DaqClient**
- **Found during:** Task 1 (execution controls)
- **Issue:** Plan referenced client.start_engine() and client.abort_plan() but these methods didn't exist
- **Fix:** Added both methods to DaqClient using StartEngineRequest and AbortPlanRequest from proto
- **Files modified:** crates/ui/src/client.rs
- **Verification:** cargo check passes
- **Committed in:** d1e2db51 (combined task commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential for functionality - couldn't call start_engine/abort_plan without the methods existing

## Issues Encountered

- **PlotPoints Clone:** egui_plot's PlotPoints doesn't implement Clone, required creating point_vec first then collecting twice for line+markers in LineWithMarkers mode
- **Borrow checker with client:** poll_async_results originally took client parameter but needed to start subscription in result handler - solved by returning Option<String> and handling subscription after

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Execution controls and live plotting complete
- Ready for Plan 03: Preset templates and history
- Plot data persists after scan completes for review

---
*Phase: 01-form-based-scan-builder*
*Completed: 2026-01-22*
