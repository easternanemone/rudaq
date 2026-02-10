---
phase: 04-sequences-and-control-flow
plan: 02
subsystem: ui
tags: [egui, rust, property-inspector, node-editor, autocomplete]

# Dependency graph
requires:
  - phase: 04-01
    provides: Configuration structs (MoveConfig, WaitCondition, AcquireConfig, LoopConfig) and DeviceSelector widget
provides:
  - Complete property inspector panels for all node types with full configuration options
  - Device autocomplete integration for Move, Wait, Acquire, Loop, and Scan nodes
  - Condition type selectors for Wait nodes (Duration/Threshold/Stability)
  - Termination type selectors for Loop nodes (Count/Condition/Infinite)
  - Mode toggles and field visibility based on node configuration
affects: [04-03, Phase 5 (runtime editing), Phase 6 (experiment execution)]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Inspector method pattern: show_{node_type}_inspector for each node variant"
    - "Graceful degradation: empty device_ids falls back to text field"
    - "Stateless DeviceSelector: created per-render with current device list"

key-files:
  created: []
  modified:
    - crates/ui/src/widgets/property_inspector.rs
    - crates/ui/src/panels/experiment_designer.rs

key-decisions:
  - "Stateless DeviceSelector usage (created per-render) instead of storing as field"
  - "Empty device_ids parameter triggers fallback to text field (graceful degradation)"
  - "Condition/termination type change creates new default variant (prevents data loss until user confirms)"

patterns-established:
  - "Pattern 1: Inspector methods receive device_ids slice for autocomplete"
  - "Pattern 2: ComboBox type selector creates new default variant on change"
  - "Pattern 3: Field labels change dynamically based on mode (Position/Distance)"

# Metrics
duration: 4min
completed: 2026-01-22
---

# Phase 04 Plan 02: Node Property Editors Summary

**Full property inspectors for Move/Wait/Acquire/Loop nodes with device autocomplete, mode toggles, and condition type selectors**

## Performance

- **Duration:** 4 min
- **Started:** 2026-01-22T23:29:43Z
- **Completed:** 2026-01-22T23:33:35Z
- **Tasks:** 1 (consolidated all functionality)
- **Files modified:** 2

## Accomplishments
- Move node inspector with DeviceSelector, Absolute/Relative mode toggle, dynamic position/distance label, and wait_settled checkbox
- Wait node inspector with condition type selector (Duration/Threshold/Stability) and appropriate fields per type
- Acquire node inspector with DeviceSelector, optional exposure override, and frame count (1-1000 range)
- Loop node inspector with termination type selector (Count/Condition/Infinite) with safety warnings
- Scan node inspector enhanced with DeviceSelector for actuator field
- All inspectors use graceful degradation when device list is empty (fallback to text field)

## Task Commits

Each task was committed atomically:

1. **Task 1-3 (consolidated): Implement all property inspectors with DeviceSelector** - `abf5f984` (feat)

**Plan metadata:** (deferred to completion)

_Note: Tasks 2 and 3 were completed as part of Task 1 since the implementation naturally included all inspectors and wiring in a single coherent change._

## Files Created/Modified
- `crates/ui/src/widgets/property_inspector.rs` - Complete refactor with per-node-type inspector methods, DeviceSelector integration, ComboBox type selectors for Wait/Loop
- `crates/ui/src/panels/experiment_designer.rs` - Added device_ids parameter to PropertyInspector::show() call with TODO for async fetch

## Decisions Made

**1. Stateless DeviceSelector pattern**
- Create DeviceSelector per-render instead of storing as field
- Simpler state management, works well with egui's immediate mode
- set_selected() before show() to populate from current config

**2. Graceful degradation for empty device list**
- When device_ids is empty, fall back to text field
- Allows UI to work before device registry is loaded
- TODO added for async device fetch in ExperimentDesignerPanel

**3. Type change creates new default variant**
- When user changes Wait condition or Loop termination type via ComboBox
- Create new default variant with sensible defaults
- Prevents confusing data carry-over between incompatible types

**4. Fixed deprecated ComboBox API**
- Changed from_id_source to from_id_salt (aligned with rest of codebase)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None - implementation was straightforward.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for 04-03 (Action Buttons and Status Visualization):**
- All node configuration UI complete
- Property inspector fully functional
- DeviceSelector integration pattern established

**Future enhancement needed:**
- Device list async fetch from DaqClient (TODO in experiment_designer.rs line 285)
- Could add refresh button in property inspector header to refetch devices
- Consider caching device list in ExperimentDesignerPanel state

**Testing notes:**
- All tests pass except pre-existing test_version_check failure
- Manual testing required for GUI functionality (no automated UI tests)

---
*Phase: 04-sequences-and-control-flow*
*Completed: 2026-01-22*
