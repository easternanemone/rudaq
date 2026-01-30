---
phase: 03-plan-translation-and-execution
plan: 01
subsystem: ui
tags: [egui, grpc, graph-translation, cycle-detection, experiment]

# Dependency graph
requires:
  - phase: 02-node-graph-editor-core
    provides: ExperimentNode types, Snarl-based graph editor, validation framework
provides:
  - GraphPlan struct implementing Plan trait for executable graph translation
  - DaqClient engine control methods (pause, resume, status)
  - Cycle detection in graph validation preventing invalid execution
affects: [03-02, 03-03, 04-live-editing]

# Tech tracking
tech-stack:
  added: [experiment dependency in ui]
  patterns: [topological sort with Kahn's algorithm for cycle detection, graph-to-plan translation]

key-files:
  created:
    - crates/ui/src/graph/translation.rs
  modified:
    - crates/ui/src/client.rs
    - crates/ui/src/graph/mod.rs
    - crates/ui/src/graph/validation.rs
    - crates/ui/src/panels/experiment_designer.rs
    - crates/ui/Cargo.toml

key-decisions:
  - "Use Kahn's algorithm for cycle detection (standard, efficient O(V+E))"
  - "Display cycle errors on first node (avoids overwhelming user with multiple errors)"
  - "Skip per-node validation when cycle detected (cycles make execution impossible)"

patterns-established:
  - "Graph-level validation runs before per-node validation"
  - "TranslationError enum for graph translation failures"
  - "Each node translates to checkpoint-wrapped PlanCommands"

# Metrics
duration: 7min
completed: 2026-01-22
---

# Phase 03 Plan 01: Plan Translation and Execution Foundation Summary

**DaqClient engine control via gRPC, GraphPlan translation with topological sort, and cycle detection preventing invalid graph execution**

## Performance

- **Duration:** 7 min
- **Started:** 2026-01-22T15:52:42Z
- **Completed:** 2026-01-22T15:59:25Z
- **Tasks:** 3
- **Files modified:** 6 (1 created)

## Accomplishments
- DaqClient gained pause_engine(), resume_engine(), get_engine_status() for RunEngine control
- GraphPlan struct translates Snarl<ExperimentNode> to executable Plan trait implementation
- Cycle detection integrated into graph validation using Kahn's algorithm
- Translation module with topological sort ensures execution order respects dependencies

## Task Commits

Each task was committed atomically:

1. **Task 1: Add DaqClient engine control methods** - `58b13a70` (feat)
2. **Task 2: Create graph translation module with GraphPlan** - `fccc0a6c` (feat)
3. **Task 3: Integrate cycle detection into graph validation** - `076ab1b8` (feat)

## Files Created/Modified
- `crates/ui/src/graph/translation.rs` - GraphPlan translation from Snarl to PlanCommands with cycle detection
- `crates/ui/src/client.rs` - Added pause_engine, resume_engine, get_engine_status methods
- `crates/ui/src/graph/mod.rs` - Export GraphPlan, TranslationError, detect_cycles, validate_graph_structure
- `crates/ui/src/graph/validation.rs` - Added validate_graph_structure() with Kahn's algorithm
- `crates/ui/src/panels/experiment_designer.rs` - Integrated cycle detection into validate_graph()
- `crates/ui/Cargo.toml` - Added experiment dependency for Plan trait

## Decisions Made
- **Kahn's algorithm for cycle detection:** Standard topological sort algorithm, O(V+E) complexity, clear cycle detection when sorted count != node count
- **Display cycle error on first node:** Prevents overwhelming user with multiple errors; single error communicates the issue clearly
- **Skip per-node validation on cycle:** Cycles make execution impossible, so per-node errors are secondary
- **Add experiment dependency:** Required for Plan trait; kept as direct dependency (not optional) since graph translation is core feature

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added experiment dependency**
- **Found during:** Task 2 (GraphPlan implementation)
- **Issue:** experiment not in ui dependencies, causing compilation error on Plan trait import
- **Fix:** Added `experiment = { path = "../experiment" }` to Cargo.toml
- **Files modified:** crates/ui/Cargo.toml, Cargo.lock
- **Verification:** Build succeeds, translation module compiles
- **Committed in:** fccc0a6c (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential dependency addition to unblock implementation. No scope creep.

## Issues Encountered
None - plan executed smoothly with one expected dependency addition.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- GraphPlan ready for integration with RunEngine gRPC calls
- Engine control methods available for start/pause/resume UI
- Cycle detection prevents invalid graphs from reaching execution
- Ready for 03-02: Wire up execution controls and status display

**Blockers:** None
**Concerns:** Pre-existing test failure in graph::serialization::tests::test_version_check (not introduced by this plan)

---
*Phase: 03-plan-translation-and-execution*
*Completed: 2026-01-22*
