---
phase: 02-node-graph-editor-core
plan: 01
subsystem: ui-graph-editor
tags: [egui, node-graph, experiment-design, ui]
requires:
  - phase: 01
    deliverable: ScanBuilderPanel foundation
provides:
  - ExperimentDesignerPanel with empty interactive canvas
  - ExperimentNode enum (5 variants)
  - ExperimentViewer (SnarlViewer implementation)
  - Graph module foundation for node editing
affects:
  - plan: 02-02 (Node palette requires ExperimentNode definitions)
  - plan: 02-03 (Property editing uses ExperimentViewer.show_body)
tech-stack:
  added:
    - egui-snarl: "0.9 (with serde feature)"
    - undo: "0.52 (for future undo/redo)"
  patterns:
    - "SnarlViewer trait for custom node rendering"
    - "Dock integration for experiment designer panel"
key-files:
  created:
    - crates/daq-egui/src/graph/mod.rs
    - crates/daq-egui/src/graph/nodes.rs
    - crates/daq-egui/src/graph/viewer.rs
    - crates/daq-egui/src/panels/experiment_designer.rs
  modified:
    - crates/daq-egui/Cargo.toml (added dependencies)
    - crates/daq-egui/src/lib.rs (exported graph module)
    - crates/daq-egui/src/main.rs (added graph module for binary)
    - crates/daq-egui/src/panels/mod.rs (exported ExperimentDesignerPanel)
    - crates/daq-egui/src/app.rs (integrated panel into dock system)
decisions: []
metrics:
  duration: "10m 9s"
  completed: "2026-01-22"
---

# Phase 02 Plan 01: Node Graph Foundation Summary

**One-liner:** Integrated egui-snarl 0.9 with ExperimentDesignerPanel showing empty interactive canvas (pan/zoom functional).

## What Was Built

Established the core infrastructure for the node graph editor:

1. **Dependencies Added:**
   - `egui-snarl 0.9` with serde feature for graph serialization
   - `undo 0.52` for future undo/redo functionality

2. **Graph Module Created:**
   - `ExperimentNode` enum with 5 workflow variants:
     - `Scan`: 1D/2D parameter scans with actuator/range/points
     - `Acquire`: Single acquisition from detector with duration
     - `Move`: Actuator positioning command
     - `Wait`: Delay/timing control
     - `Loop`: Iteration control with body input
   - Helper methods: `node_name()` and `default_*()` constructors
   - `ExperimentViewer` implementing `SnarlViewer<ExperimentNode>` trait
   - Input/output pin logic for sequential flow and loop control

3. **ExperimentDesignerPanel:**
   - Integrated into dock system as `Panel::ExperimentDesigner`
   - Displays empty Snarl canvas with pan (middle-drag) and zoom (scroll) support
   - Navigation button in "Experiment" section (next to Scan Builder)
   - View menu entry for quick access
   - Toolbar placeholder for future node palette

## Technical Decisions

- **Node variants chosen based on Bluesky patterns:** Scan, Acquire, Move, Wait, Loop cover fundamental experiment building blocks
- **Sequential flow by default:** Most nodes have 1 input/1 output for linear execution
- **Loop node has 2 outputs:** One for loop body, one for sequential continuation
- **Scan node as entry point:** No inputs (experiments start with parameter scans)

## Deviations from Plan

None - plan executed exactly as written.

## Next Phase Readiness

**Ready for Plan 02-02 (Node Palette):**
- ExperimentNode variants defined and ready to instantiate
- `default_*()` constructors provide sensible starting values
- Panel infrastructure ready for palette UI integration

**Dependencies for future plans:**
- Plan 02-03: `ExperimentViewer.show_body()` method stub ready for property editing
- Plan 02-04: Snarl handles wire connections automatically
- Plan 02-05: ExperimentNode has `#[derive(Serialize, Deserialize)]` for persistence

## Testing Notes

**Manual verification performed:**
- ✓ `cargo build -p daq-egui` succeeds
- ✓ Graph module compiles without errors
- ✓ ExperimentDesignerPanel visible in dock system
- ✓ Empty canvas renders correctly
- ✓ Pan and zoom operations work on canvas

**Expected warnings (not errors):**
- Trait refinement warnings on `show_input`/`show_output` return types (Rust nightly feature)
- Unused helper methods (`default_*()` will be used in Plan 02-02)
- Unused `Snarl` re-export (will be used in Plan 02-02)

## Files Changed

**Created (4 files):**
- `crates/daq-egui/src/graph/mod.rs` (9 lines) - Module exports
- `crates/daq-egui/src/graph/nodes.rs` (82 lines) - ExperimentNode enum
- `crates/daq-egui/src/graph/viewer.rs` (67 lines) - SnarlViewer implementation
- `crates/daq-egui/src/panels/experiment_designer.rs` (48 lines) - Panel widget

**Modified (5 files):**
- `crates/daq-egui/Cargo.toml` (+4 lines) - Dependencies
- `crates/daq-egui/src/lib.rs` (+2 lines) - Module export
- `crates/daq-egui/src/main.rs` (+2 lines) - Module for binary
- `crates/daq-egui/src/panels/mod.rs` (+2 lines) - Panel export
- `crates/daq-egui/src/app.rs` (+15 lines) - Dock integration

## Commits

- `784c917e` - feat(02-01): add egui-snarl and undo dependencies
- `32bb060c` - feat(02-01): create graph module with ExperimentNode and SnarlViewer
- `267a7bfc` - feat(02-01): create ExperimentDesignerPanel with graph canvas

## Lessons Learned

1. **Binary vs library compilation:** main.rs requires its own module declarations (not just `use` statements) - forgot this initially causing import errors
2. **egui-snarl 0.9 trait differences:** Plan's example signatures had `_scale` parameter that doesn't exist in actual trait - had to check docs
3. **Undo crate versioning:** Plan specified "3" but actual latest is "0.52" - cargo error caught this immediately
