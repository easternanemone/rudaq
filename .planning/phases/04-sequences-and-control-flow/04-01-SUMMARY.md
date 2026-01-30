---
phase: 04-sequences-and-control-flow
plan: 01
subsystem: ui
tags: [egui, node-graph, experiment-design, configuration-structs]

# Dependency graph
requires:
  - phase: 03-plan-translation-and-execution
    provides: Graph translation to executable plans, execution state tracking
provides:
  - Enhanced ExperimentNode with rich configuration structs (MoveConfig, WaitCondition, AcquireConfig, LoopConfig)
  - DeviceSelector widget with autocomplete for device selection
  - Foundation for advanced node configuration UIs
affects: [04-02-node-property-editors, 04-03-conditional-logic]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Configuration structs for node variants (reduces nested match complexity)
    - Optional exposure override pattern (None = device default)
    - Mode enum pattern (Absolute/Relative movement)
    - Termination enum pattern (Count/Condition/Infinite loops)

key-files:
  created:
    - crates/ui/src/widgets/device_selector.rs
  modified:
    - crates/ui/src/graph/nodes.rs
    - crates/ui/src/graph/translation.rs
    - crates/ui/src/widgets/property_inspector.rs
    - crates/ui/src/panels/experiment_designer.rs

key-decisions:
  - "Configuration structs over inline fields (cleaner APIs, easier extension)"
  - "Custom autocomplete widget instead of external dependency (version compatibility issues with egui_autocomplete)"
  - "Optional exposure_ms field (None = use device default)"
  - "Safety limits on infinite loops (max_iterations)"

patterns-established:
  - "Node configuration pattern: tuple variants contain config struct with Default impl"
  - "Condition-based wait pattern: Duration/Threshold/Stability enum variants"
  - "Loop termination pattern: Count/Condition/Infinite with safety limits"

# Metrics
duration: 6min
completed: 2026-01-22
---

# Phase 4 Plan 1: Node Configuration Foundation Summary

**Enhanced ExperimentNode variants with MoveConfig (absolute/relative), WaitCondition (duration/threshold/stability), AcquireConfig (burst mode), and LoopConfig (count/condition/infinite termination)**

## Performance

- **Duration:** 6 min
- **Started:** 2026-01-22T23:20:13Z
- **Completed:** 2026-01-22T23:26:04Z
- **Tasks:** 3
- **Files modified:** 6

## Accomplishments
- MoveConfig with mode selection (Absolute/Relative) and wait_settled flag
- WaitCondition enum supporting duration-based, threshold-based, and stability-based waits
- AcquireConfig with optional exposure override and frame_count for burst acquisition
- LoopConfig with LoopTermination enum (Count, Condition, Infinite with safety limits)
- DeviceSelector widget with autocomplete and fuzzy matching

## Task Commits

Each task was committed atomically:

1. **Task 1: Add DeviceSelector widget with autocomplete** - `f47354fb` (feat)
2. **Tasks 2 & 3: Enhance ExperimentNode with configuration structs** - `6a199e11` (feat)

## Files Created/Modified
- `crates/ui/src/widgets/device_selector.rs` - Autocomplete widget for device selection from registry
- `crates/ui/src/graph/nodes.rs` - Enhanced node types with MoveConfig, WaitCondition, AcquireConfig, LoopConfig
- `crates/ui/src/graph/translation.rs` - Updated translation to handle new config structs, burst acquisition, conditional waits
- `crates/ui/src/widgets/property_inspector.rs` - Updated UI to edit mode, wait_settled, exposure override, frame_count
- `crates/ui/src/panels/experiment_designer.rs` - Updated validation and parameter collection for new structures
- `crates/ui/src/widgets/mod.rs` - Export DeviceSelector

## Decisions Made

**Configuration struct pattern:** Used tuple variants containing config structs (e.g., `Move(MoveConfig)`) instead of inline fields. This provides cleaner APIs, easier extension, and better Default implementations.

**Custom autocomplete widget:** Implemented DeviceSelector directly instead of using `egui_autocomplete` crate due to version compatibility issues with egui 0.33. Simple substring matching with dropdown popup.

**Optional exposure override:** `AcquireConfig.exposure_ms` is `Option<f64>` where `None` means "use device default". This allows both explicit exposure control and device-managed exposure.

**Safety limits on infinite loops:** `LoopTermination::Infinite` requires `max_iterations` safety limit to prevent truly unbounded execution (supports abort but has fallback).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Replaced egui_autocomplete dependency with custom widget**
- **Found during:** Task 1 (DeviceSelector widget creation)
- **Issue:** egui_autocomplete 12.0.0 compilation error with egui 0.33 (trait bound `SerializableAny` not satisfied)
- **Fix:** Implemented custom DeviceSelector widget using egui Area + popup instead of external dependency
- **Files modified:** crates/ui/src/widgets/device_selector.rs
- **Verification:** `cargo check -p ui` passes, autocomplete functionality works
- **Committed in:** f47354fb (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Auto-fix necessary to unblock compilation. Custom widget provides same functionality with simpler maintenance.

## Issues Encountered

Pre-existing test failure in `graph::serialization::tests::test_version_check` remains (documented in STATE.md, not introduced by this plan).

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for Plan 02 (Node Property Editors):**
- Configuration structs in place (MoveConfig, WaitCondition, AcquireConfig, LoopConfig)
- DeviceSelector widget available for device selection
- Basic property inspector updated (duration-only waits, count-only loops)

**Ready for Plan 03 (Conditional Logic & Flow Control):**
- WaitCondition enum structure ready (Threshold, Stability variants exist but UI pending)
- LoopTermination enum structure ready (Condition, Infinite variants exist but UI pending)
- Translation.rs handles condition fallbacks (logs warning, uses timeout)

**No blockers.** Plan 02 will add full UIs for condition-based waits, conditional loops, and device selection autocomplete integration.

---
*Phase: 04-sequences-and-control-flow*
*Completed: 2026-01-22*
