---
phase: 05-live-visualization
plan: 04
subsystem: ui
tags: [egui, live-visualization, experiment-execution, multi-detector]

# Dependency graph
requires:
  - phase: 05-03
    provides: LiveVisualizationPanel API and multi-detector grid layout
  - phase: 04-02
    provides: Execution state tracking infrastructure
provides:
  - Automatic visualization spawning when experiments start
  - Detector extraction from experiment graph
  - Visualization lifecycle integrated with execution controls
affects: [06-server-integration, future-experiment-workflow]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Automatic detector classification (camera vs plot) based on device_id heuristics"
    - "Channel-based async communication for frame/data updates"
    - "Visualization lifecycle hooks in execution flow"

key-files:
  created: []
  modified:
    - crates/daq-egui/src/panels/experiment_designer.rs

key-decisions:
  - "Simple heuristic for detector classification: device_id containing 'camera'/'cam' = camera, else plot"
  - "Visualization panel created on execution start, marked inactive on stop (panel persists for review)"
  - "Collapsing header UI pattern for live visualization (non-intrusive, user-collapsible)"

patterns-established:
  - "extract_detectors(): Parse graph to find Acquire nodes and classify detectors"
  - "start_visualization(): Create panel, configure detectors, create channels, mark active"
  - "stop_visualization(): Mark inactive (data persists for review)"

# Metrics
duration: 4min
completed: 2026-01-23
---

# Phase 5 Plan 4: Live Visualization Integration Summary

**Experiment graph automatically spawns multi-detector live visualization on execution start with camera/plot classification and lifecycle management**

## Performance

- **Duration:** 4 min
- **Started:** 2026-01-23T00:52:46Z
- **Completed:** 2026-01-23T00:56:46Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- ExperimentDesignerPanel extracts detectors from Acquire nodes and classifies as camera or plot
- Visualization panel spawns automatically when execution starts
- Lifecycle integrated: starts on run, stops on completion/error/abort
- CollapsibleHeader UI shows visualization during execution

## Task Commits

Each task was committed atomically:

1. **Task 1: Add LiveVisualizationPanel fields and detector extraction** - `ac31f4c` (feat)
2. **Task 2: Integrate visualization lifecycle with execution** - `4489cac` (feat)

## Files Created/Modified
- `crates/daq-egui/src/panels/experiment_designer.rs` - Added visualization panel fields, detector extraction, lifecycle integration

## Decisions Made

**Detector classification heuristic:**
- Device IDs containing "camera" or "cam" → camera panel
- All other detectors (power meters, photodiodes, etc.) → plot panel
- Simple, works for current naming conventions, can be refined later with device metadata

**Visualization lifecycle:**
- Panel created on execution start
- Marked inactive on completion/error/abort (but panel persists for data review)
- User can collapse/expand via CollapsingHeader

**UI integration:**
- CollapsingHeader pattern (default open) for non-intrusive visualization
- Placed after execution toolbar, before graph canvas
- Only shown when panel exists (i.e., during or after execution with detectors)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None - 05-03 had already created the LiveVisualizationPanel with matching API, so integration was straightforward.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for:**
- Server integration (06-XX) - channels established, ready to receive frame/data updates from gRPC streaming
- Full execution flow testing - visualization spawns automatically on run

**Notes:**
- Currently visualization panel shows but receives no data (gRPC streaming integration needed)
- Detector classification heuristic works for convention-based naming but could be enhanced with device metadata lookup
- Frame/data senders stored but not yet connected to server streaming

---
*Phase: 05-live-visualization*
*Plan: 04*
*Completed: 2026-01-23*
