---
phase: 07-code-export-and-provenance
plan: 01
subsystem: ui
tags: [rhai, codegen, graph-editor, scripting]

# Dependency graph
requires:
  - phase: 06-data-management
    provides: Graph serialization and visualization infrastructure
  - phase: 04-experiment-execution
    provides: Rhai scripting integration
provides:
  - Rhai code generation from visual experiment graphs
  - Readable, commented scripts for learning and debugging
  - Foundation for live code preview (07-02)
affects: [07-02, 07-03, scripting, code-export]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "One-way code generation (visual → code only, no round-trip)"
    - "Topological sort for execution order"
    - "Recursive loop body handling with indentation"

key-files:
  created:
    - crates/daq-egui/src/graph/codegen.rs
  modified:
    - crates/daq-egui/src/graph/mod.rs
    - crates/daq-egui/src/graph/translation.rs

key-decisions:
  - "One-way export only (visual editor is source of truth)"
  - "Rhai syntax with comments for readability"
  - "Made build_adjacency and topological_sort public for reuse"

patterns-established:
  - "Generate header comments with source file and timestamp"
  - "Two-space indentation for Rhai code"
  - "Format floats with .1 precision for readability"
  - "Handle empty/invalid nodes with WARNING comments"

# Metrics
duration: 8min
completed: 2026-01-23
---

# Phase 07 Plan 01: Rhai Code Generation Summary

**Complete Rhai code generation engine with readable output, topological sorting, and comprehensive test coverage (15 tests)**

## Performance

- **Duration:** 8 min (471 seconds)
- **Started:** 2026-01-23T02:52:20Z
- **Completed:** 2026-01-23T03:00:11Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments

- Created codegen.rs module (687 lines) with graph_to_rhai_script() function
- Implemented ExperimentNode::to_rhai() for all 5 node variants (Scan, Move, Wait, Acquire, Loop)
- Generated Rhai code includes explanatory comments for each operation
- Proper indentation handling for nested loop bodies
- Comprehensive test suite (15 tests, all passing)

## Task Commits

Each task was committed atomically:

1. **All Tasks: Create codegen module and tests** - `24254635` (feat)

**Note:** This plan was executed as a single cohesive unit since all tasks were tightly coupled (module creation, implementation, and tests).

## Files Created/Modified

- `crates/daq-egui/src/graph/codegen.rs` - Rhai code generation engine with node-to-code translation
- `crates/daq-egui/src/graph/mod.rs` - Added codegen module and exported graph_to_rhai_script
- `crates/daq-egui/src/graph/translation.rs` - Made build_adjacency and topological_sort public for reuse
- `Cargo.lock` - Dependency updates

## Decisions Made

1. **One-way export model**: Visual editor is source of truth. Generated Rhai scripts are read-only artifacts for learning and debugging. This prevents round-trip complexity and keeps the visual editor as the canonical representation.

2. **Public topological sort functions**: Made build_adjacency() and topological_sort() public in translation.rs to avoid code duplication. Both translation (Plan generation) and codegen (Rhai export) need the same graph traversal logic.

3. **Float formatting**: Use {:.1} format for float display to ensure values like 0.0 and 100.0 render with decimal points for clarity in Rhai code.

4. **WARNING comments for invalid nodes**: Rather than failing, generate WARNING comments in output for nodes with missing configuration (e.g., empty actuator, zero points). This makes the export robust and provides clear feedback.

## Deviations from Plan

None - plan executed exactly as written.

All code follows the plan specification:
- graph_to_rhai_script() handles empty graphs, cycles, and topological traversal
- ExperimentNode::to_rhai() implemented for all 5 variants with proper Rhai syntax
- Loop bodies handled with recursive code generation and indentation
- Generated code includes comments explaining each step
- 15 comprehensive unit tests covering all node types and edge cases

## Issues Encountered

**Minor issue during testing:** Initial tests failed because:
1. Float formatting didn't include decimal points (0 instead of 0.0)
2. Test used default_scan() which has empty actuator field

**Resolution:**
1. Updated format strings to use {:.1} for floats
2. Fixed test to create scan node with valid actuator

Both issues caught and resolved during test execution. No impact on final implementation.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for Phase 07-02 (Live Code Preview):**
- graph_to_rhai_script() is fully functional and tested
- Generated code is readable with proper formatting
- Can be integrated into GUI for live preview panel
- Export to file functionality can be added with minimal changes

**Foundation established for:**
- CODE-01: Live code preview in GUI
- CODE-02: Export to .rhai files
- CODE-03: Graph provenance tracking (hash generation from exported code)

**No blockers.** All success criteria met:
- ✅ codegen.rs exists with 687 lines (exceeds 200-300 target)
- ✅ ExperimentNode::to_rhai() for all 5 variants
- ✅ graph_to_rhai_script() handles topological sort and loop bodies
- ✅ Generated Rhai includes comments for readability
- ✅ All 15 codegen tests pass
- ✅ Module exported in graph/mod.rs

---
*Phase: 07-code-export-and-provenance*
*Completed: 2026-01-23*
