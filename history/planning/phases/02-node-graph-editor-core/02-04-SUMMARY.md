---
phase: 02-node-graph-editor-core
plan: 04
subsystem: ui-graph-editor
tags: [egui, serialization, json, file-dialogs, validation-display]
requires:
  - phase: 02
    plan: 02
    deliverable: Node palette and wire connections
  - phase: 02
    plan: 03
    deliverable: Property inspector and undo/redo
provides:
  - JSON serialization for experiment graphs (.expgraph files)
  - Native file dialogs for Save/Load operations
  - Validation error display in status bar and property inspector
  - Keyboard shortcuts for file operations (Ctrl+S, Ctrl+O)
affects:
  - plan: 03-xx (Plan translation will load graphs from files)
  - plan: 07-xx (Code export may extend serialization format)
tech-stack:
  added:
    - rfd: "0.15 (rusty file dialogs)"
    - chrono: "0.4 (timestamps for metadata)"
  patterns:
    - "GraphFile wrapper with version and metadata"
    - "Per-node validation with error collection"
    - "Status bar with auto-fading messages"
key-files:
  created:
    - crates/ui/src/graph/serialization.rs
  modified:
    - crates/ui/Cargo.toml (added rfd, chrono)
    - crates/ui/src/graph/mod.rs (added serialization export)
    - crates/ui/src/graph/viewer.rs (added node_errors HashMap)
    - crates/ui/src/panels/experiment_designer.rs (save/load UI, validation display)
decisions:
  - id: expgraph-extension
    summary: "Used .expgraph file extension for experiment graphs"
    rationale: "Distinct extension avoids confusion with generic JSON files"
  - id: validation-in-status-bar
    summary: "Display validation errors in bottom status bar and property inspector"
    rationale: "Non-intrusive but visible; detailed errors in inspector when node selected"
metrics:
  duration: "12m"
  completed: "2026-01-22"
  human_verified: true
---

# Phase 02 Plan 04: JSON Serialization and Validation Display Summary

**One-liner:** Added file save/load with native dialogs and validation error display in status bar and property inspector.

## What Was Built

1. **JSON Serialization:**
   - `GraphFile` struct wraps graph data with version and metadata
   - `GraphMetadata` stores name, description, author, timestamps
   - `save_graph()` / `load_graph()` functions with error handling
   - Version checking for future compatibility
   - `.expgraph` file extension for experiment graphs

2. **File Operations UI:**
   - Toolbar buttons: New, Open..., Save, Save As...
   - Native file dialogs via `rfd` crate
   - Keyboard shortcuts: Ctrl+S (save), Ctrl+O (open)
   - Current file name displayed in toolbar
   - Auto-fading status messages for save/load feedback

3. **Validation Error Display:**
   - `node_errors` HashMap in ExperimentViewer
   - Bottom status bar shows error count and summary
   - Property inspector shows detailed error for selected node
   - Color coding: red for errors, green for "Graph valid"
   - Validation runs on: load, property change, node add/remove

## Technical Decisions

**File Extension (.expgraph):**
Using a distinct extension rather than generic `.json` helps users identify experiment files and enables proper file associations.

**Validation Location:**
Chose status bar + property inspector over inline node decoration. This avoids egui-snarl API complexity while providing clear feedback.

**Status Message Auto-fade:**
Messages auto-clear after 3 seconds using `Instant::elapsed()` check. Keeps UI clean without requiring manual dismiss.

## Deviations from Plan

None significant. Implementation followed plan closely.

## Human Verification Results

**Verified by user on 2026-01-22:**

- ✓ Node creation via context menu works
- ✓ Wire connections between nodes work
- ✓ Property editing in inspector works
- ✓ Undo/redo with Ctrl+Z/Ctrl+Y works
- ✓ Save As... creates valid .expgraph file
- ✓ New clears graph, Open... restores saved state
- ✓ Validation errors appear for empty actuator field
- ✓ "Graph valid" shows when validation passes

## Files Changed

**Created (1 file):**
- `crates/ui/src/graph/serialization.rs` (65 lines) - GraphFile, save/load functions

**Modified (4 files):**
- `crates/ui/Cargo.toml` (+2 deps) - rfd, chrono
- `crates/ui/src/graph/mod.rs` (+2 lines) - serialization export
- `crates/ui/src/graph/viewer.rs` (+25 lines) - node_errors, error methods
- `crates/ui/src/panels/experiment_designer.rs` (+120 lines) - file ops, validation display

## Commits

- `f4d69beb` - feat(02-04): implement JSON serialization for graph save/load
- `86be95a4` - feat(02-04): add Save/Load UI with file dialogs
- `c666d385` - feat(02-04): add validation error display in status bar and inspector

## Phase 02 Completion

This plan completes Phase 02: Node Graph Editor Core.

**All success criteria met:**
1. ✓ User can drag node from palette onto canvas and connect nodes with wires
2. ✓ User can configure node parameters via property inspector panel
3. ✓ User can undo/redo edits with Ctrl+Z/Ctrl+Y, with full edit history
4. ✓ Editor shows validation errors visually (status bar, property inspector) when nodes invalid

**Note:** EDIT-03 (subgraph grouping) was deferred to Phase 9 as documented in ROADMAP.md.
