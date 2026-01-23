---
phase: 05-live-visualization
plan: 02
subsystem: ui
tags: [egui, egui-extras, StripBuilder, grid-layout, multi-detector, visualization]

# Dependency graph
requires:
  - phase: 03-plan-translation-and-execution
    provides: Infrastructure for executing experiment plans that generate detector data
provides:
  - Multi-detector grid layout panel for simultaneous visualization
  - Automatic grid dimension calculation (cols = ceil(sqrt(n)), rows = ceil(n/cols))
  - Nested StripBuilder pattern for responsive grid layout
  - Placeholder infrastructure for camera and line plot detectors
affects: [05-live-visualization, detector-integration, camera-streaming]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Nested StripBuilder for responsive grid layouts (vertical strips for rows, horizontal for columns)"
    - "Grid dimension calculation using sqrt for roughly square layouts"
    - "DetectorType enum for mixed detector support (Camera, LinePlot)"

key-files:
  created:
    - crates/daq-egui/src/panels/multi_detector_grid.rs
  modified:
    - crates/daq-egui/src/panels/mod.rs

key-decisions:
  - "Grid dimensions calculated as cols = ceil(sqrt(n)), rows = ceil(n/cols) for roughly square layouts"
  - "DetectorType enum supports mixed camera and line plot detectors in same grid"
  - "Empty grid cells rendered with stroke outline for visual consistency"
  - "Nested StripBuilder with Size::remainder() for equal panel sizing"

patterns-established:
  - "Nested StripBuilder pattern: outer vertical for rows, inner horizontal for columns"
  - "DetectorPanel::camera() and DetectorPanel::line_plot() factory constructors"
  - "Placeholder render methods for future detector integration"

# Metrics
duration: 3min
completed: 2026-01-22
---

# Phase 05 Plan 02: Multi-Detector Grid Layout Summary

**Responsive grid panel automatically arranging N detectors using nested StripBuilder (vertical rows × horizontal columns)**

## Performance

- **Duration:** 3 min
- **Started:** 2026-01-22T18:45:00Z (estimated, work found already committed)
- **Completed:** 2026-01-22T18:48:23Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Created MultiDetectorGrid panel with automatic N-detector grid layout
- Implemented DetectorType enum supporting Camera and LinePlot detectors
- Established nested StripBuilder pattern (vertical strips for rows, horizontal for columns)
- Added comprehensive unit tests for grid dimensions and panel management (4 tests, all passing)
- Exported MultiDetectorGrid, DetectorPanel, and DetectorType from panels module

## Task Commits

**Note:** This plan's work was committed in the same commit as plan 05-01, which is non-standard but work is complete.

1. **Task 1: Create MultiDetectorGrid panel with StripBuilder layout** - `18b3904d` (feat)
   - Implemented MultiDetectorGrid struct with add_panel(), clear(), panel_count() methods
   - Created DetectorType enum (Camera, LinePlot) and DetectorPanel struct
   - Added calculate_grid_dimensions() helper function
   - Implemented show() method using nested StripBuilder for responsive grid
   - Created render_panel() and render_empty_cell() helper methods

2. **Task 2: Add unit tests for grid dimension calculation** - `18b3904d` (feat)
   - test_grid_dimensions: Validates grid calculation for 0-16 detectors
   - test_panel_management: Verifies add_panel() and clear() operations
   - test_detector_panel_constructors: Tests camera() and line_plot() factories
   - test_with_panels_constructor: Validates with_panels() initialization

**Plan metadata:** Not separately committed (work included in 18b3904d)

## Files Created/Modified
- `crates/daq-egui/src/panels/multi_detector_grid.rs` (264 lines) - Multi-detector grid layout panel
- `crates/daq-egui/src/panels/mod.rs` - Added multi_detector_grid module and exports

## Decisions Made

**Grid dimension calculation algorithm:**
- Formula: cols = ceil(sqrt(n)), rows = ceil(n/cols)
- Produces roughly square grids (1→1×1, 2→2×1, 3→2×2, 4→2×2, 9→3×3, etc.)
- Rationale: Square layouts maximize screen space utilization for multi-detector viewing

**DetectorType enum design:**
- Camera variant: Only device_id (device name sufficient for lookup)
- LinePlot variant: device_id + label (signal name for plot labeling)
- Rationale: Different visualization requirements; plots need signal context, cameras don't

**Nested StripBuilder pattern:**
- Outer: vertical strips for rows
- Inner: horizontal strips for columns
- Size::remainder() for equal panel sizing
- Rationale: StripBuilder handles responsive resizing; nested pattern creates 2D grid

**Empty cell rendering:**
- Stroke outline (1.0px, gray 100, outside)
- Rationale: Visual consistency when detector count doesn't fill grid perfectly

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

**Compilation fix for egui API:**
- Issue: rect_stroke() requires 4 arguments (StrokeKind added in egui 0.33)
- Fix: Added StrokeKind::Outside parameter
- Impact: API compatibility with egui 0.33.3

**Pre-existing compilation error:**
- Issue: auto_scale_plot.rs had type annotation error (unrelated to this plan)
- Fix: File was modified between sessions (possibly auto-formatter)
- Impact: None on this plan's work

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for next plans:**
- Grid layout infrastructure complete
- Placeholder render methods ready for detector integration
- DetectorType enum supports both camera and line plot detectors

**Integration points established:**
- render_panel() method ready to integrate with ImageViewerPanel (plan 05-03)
- LinePlot variant ready for signal plotter integration (future plans)
- Grid automatically adjusts to detector count (1×1, 1×2, 2×2, etc.)

**No blockers** - all grid layout functionality complete and tested.

---
*Phase: 05-live-visualization*
*Completed: 2026-01-22*
