---
phase: 05-live-visualization
plan: 03
subsystem: ui
tags: [egui, visualization, camera-streaming, plot-display, fps-tracking]

# Dependency graph
requires:
  - phase: 05-01
    provides: AutoScalePlot widget with grow-to-fit logic
  - phase: 05-02
    provides: MultiDetectorGrid layout panel
provides:
  - LiveVisualizationPanel integrating camera and plot display
  - FrameUpdate/DataUpdate message types for async streaming
  - FPS tracking infrastructure for performance monitoring
  - Channel helpers (frame_channel, data_channel) for async integration
affects: [05-04, runtime-visualization, acquisition-ui]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Message-passing via mpsc for async frame/data delivery
    - FpsTracker with rolling window for performance monitoring
    - Dual detector type rendering (camera frames + line plots)
    - TextureHandle management for camera frame display

key-files:
  created:
    - crates/daq-egui/src/panels/live_visualization.rs
  modified:
    - crates/daq-egui/src/panels/mod.rs
    - crates/daq-egui/src/panels/multi_detector_grid.rs
    - crates/daq-egui/src/panels/experiment_designer.rs

key-decisions:
  - "FPS tracking uses 2-second rolling window for stable metrics"
  - "Camera frames displayed with aspect-preserving fit-to-panel logic"
  - "Plot FPS estimated from data rate (time span / point count)"
  - "Separate update channels for frames and data (bounded SyncSender)"

patterns-established:
  - "FrameUpdate with RGBA data (pre-converted) for texture upload"
  - "DataUpdate with timestamp for time-series plots"
  - "FpsTracker reusable component for any streaming visualization"
  - "Active flag triggers continuous repaint during acquisition"

# Metrics
duration: 4min
completed: 2026-01-23
---

# Phase 5 Plan 3: Live Visualization Integration Summary

**LiveVisualizationPanel with unified camera/plot display, FPS tracking, and async channel integration for real-time acquisition monitoring**

## Performance

- **Duration:** 4 min
- **Started:** 2026-01-23T03:59:12Z
- **Completed:** 2026-01-23T04:03:10Z
- **Tasks:** 2 (implemented in single unified solution)
- **Files modified:** 4

## Accomplishments
- Unified panel showing cameras and plots in grid layout during acquisition
- FPS tracking with rolling window for performance monitoring
- Async channel integration (frame_channel, data_channel) for non-blocking updates
- Camera frame display with aspect-preserving fit logic
- Line plot integration using AutoScalePlot with controls
- LIVE/IDLE status indicator with average FPS metrics

## Task Commits

Each task was committed atomically:

1. **Task 1-2: Create LiveVisualizationPanel** - `4489cac1` (feat)
   - Implemented frame and plot state management
   - Added show() method with grid rendering and FPS status
   - Integrated MultiDetectorGrid and AutoScalePlot
   - Created message types and channel helpers

**Plan metadata:** (included in task commit)

## Files Created/Modified
- `crates/daq-egui/src/panels/live_visualization.rs` - Main panel with camera/plot rendering
- `crates/daq-egui/src/panels/mod.rs` - Export LiveVisualizationPanel and types
- `crates/daq-egui/src/panels/multi_detector_grid.rs` - Added panels() getter for external access
- `crates/daq-egui/src/panels/experiment_designer.rs` - Updated to use new visualization types

## Decisions Made

1. **FPS tracking window:** 2-second rolling window for stable metrics without excessive lag
2. **Camera aspect ratio:** Fit-to-panel logic preserves aspect ratio (no stretching)
3. **Plot FPS estimation:** Simple time span / point count (adequate for monitoring)
4. **Bounded channels:** SyncSender with capacity limits prevents memory exhaustion
5. **Active repaint:** Panel requests continuous repaint when active for smooth updates

## Deviations from Plan

None - plan executed exactly as written. Tasks 1 and 2 were implemented in a single unified solution since the show() method naturally includes all Task 2 requirements (polling, status bar, grid rendering, AutoScalePlot integration).

## Issues Encountered

**Pre-existing integration code:** ExperimentDesignerPanel had skeleton code for LiveVisualizationPanel that was using incorrect type names (std_mpsc instead of std::sync::mpsc). Fixed by adding proper imports and field types.

## Next Phase Readiness

**Ready for 05-04 (Live Visualization Server Integration):**
- LiveVisualizationPanel complete with channel receivers
- FrameUpdate/DataUpdate message types defined
- Senders available for background streaming tasks
- FPS tracking infrastructure working

**Integration points:**
- ExperimentDesignerPanel already has visualization_panel field
- Channel senders can be passed to gRPC streaming tasks
- Panel can be shown in collapsing header during execution

---
*Phase: 05-live-visualization*
*Completed: 2026-01-23*
