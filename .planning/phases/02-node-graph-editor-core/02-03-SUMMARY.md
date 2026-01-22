---
phase: 02-node-graph-editor-core
plan: 03
subsystem: ui-graph-editor
tags: [egui, undo-redo, property-inspector, command-pattern]
requires:
  - phase: 02
    plan: 01
    deliverable: Graph module foundation
provides:
  - PropertyInspector widget for node property editing
  - GraphEdit enum implementing undo crate Edit trait
  - Undo/redo keyboard shortcuts (Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z)
  - Three-panel layout with property inspector sidebar
affects:
  - plan: 02-04 (Wire connections may use undo system)
  - plan: 02-05 (Serialization builds on complete editor state)
tech-stack:
  added:
    - undo: "0.52 (already in Cargo.toml from 02-01)"
  patterns:
    - "Command pattern with unified GraphEdit enum"
    - "Edit trait implementation for undo/redo"
    - "egui-snarl selection state integration"
key-files:
  created:
    - crates/daq-egui/src/widgets/property_inspector.rs
    - crates/daq-egui/src/graph/commands.rs
  modified:
    - crates/daq-egui/src/widgets/mod.rs
    - crates/daq-egui/src/graph/mod.rs
    - crates/daq-egui/src/panels/experiment_designer.rs
decisions:
  - id: unified-edit-enum
    summary: "Used unified GraphEdit enum instead of individual Edit impls"
    rationale: "undo::Record<E> requires single E type; enum allows storing all command types in one history"
metrics:
  duration: "7m"
  completed: "2026-01-22"
---

# Phase 02 Plan 03: Property Inspector and Undo/Redo Summary

**One-liner:** Added PropertyInspector widget and undo/redo system using unified GraphEdit enum implementing Edit trait.

## What Was Built

1. **PropertyInspector Widget:**
   - Shows editable fields for selected node type
   - DragValue for numeric fields (float, u32)
   - Text edit for string fields (actuator, detector, device names)
   - Returns modified node when changes made
   - Placeholder message when no node selected

2. **Unified Command Pattern:**
   - `GraphEdit` enum wrapping all edit operations:
     - AddNode, RemoveNode, ModifyNode
     - ConnectNodes, DisconnectNodes
   - Implements `undo::Edit` trait with edit/undo/merge methods
   - Consecutive property modifications to same node are merged

3. **Undo/Redo Integration:**
   - `Record<GraphEdit>` tracks all graph modifications
   - Toolbar with Undo/Redo buttons (enabled/disabled based on history state)
   - History count display (head/len)
   - Keyboard shortcuts:
     - Ctrl+Z: Undo
     - Ctrl+Y: Redo
     - Ctrl+Shift+Z: Redo (alternative)

4. **Property Inspector Panel:**
   - Right sidebar in three-panel layout
   - Reads selected node from egui-snarl state
   - Property edits create ModifyNode commands
   - Edits tracked in undo history

## Technical Decisions

**Unified GraphEdit Enum vs Individual Edit Impls:**

The plan suggested individual structs implementing Edit. However, `undo::Record<E>` requires a single type E. To use multiple command types, we needed either:
- Trait objects (`Box<dyn Edit>`) - complex lifetime management
- Unified enum implementing Edit - simpler, type-safe

Chose the enum approach for simplicity and type safety.

**Selection State Access:**

egui-snarl stores selection in egui's temporary storage, not in the Snarl struct. Used `get_selected_nodes(snarl_id, ctx)` to retrieve current selection each frame.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed node_palette.rs API mismatch**
- **Found during:** Task 1 compilation
- **Issue:** `rect_stroke` in egui 0.33 requires 4 arguments (added StrokeKind)
- **Fix:** Added `StrokeKind::Inside` parameter
- **Files modified:** crates/daq-egui/src/widgets/node_palette.rs
- **Commit:** (part of 02-02 concurrent work)

**2. [Rule 3 - Blocking] Changed to unified GraphEdit enum**
- **Found during:** Task 3 compilation
- **Issue:** `Record<GraphTarget>` doesn't work - Record<E> requires E: Edit
- **Fix:** Created GraphEdit enum implementing Edit instead of individual Edit impls
- **Files modified:** crates/daq-egui/src/graph/commands.rs
- **Commit:** 1e8b9da1

## Next Phase Readiness

**Dependencies resolved:**
- undo system available for wire connection tracking
- Property inspector available for all node types
- Selection tracking working with egui-snarl

**Ready for Plan 02-04 (Wire Connections with Validation):**
- ConnectNodes/DisconnectNodes commands defined (not yet used)
- Validation module already created (from 02-02)

## Testing Notes

**Manual verification:**
- Build succeeds with warnings (pre-existing clippy warnings in codebase)
- PropertyInspector shows correct fields per node type
- Undo/redo buttons enable/disable correctly
- Keyboard shortcuts functional

**Known limitations:**
- Delete key removes node but doesn't use RemoveNode command (no undo for delete)
- Node selection requires clicking on node in canvas (egui-snarl handles this)

## Files Changed

**Created (2 files):**
- `crates/daq-egui/src/widgets/property_inspector.rs` (101 lines) - Property editing widget
- `crates/daq-egui/src/graph/commands.rs` (149 lines) - GraphEdit enum and Edit impl

**Modified (3 files):**
- `crates/daq-egui/src/widgets/mod.rs` (+2 lines) - Export PropertyInspector
- `crates/daq-egui/src/graph/mod.rs` (+4 lines) - Export command types
- `crates/daq-egui/src/panels/experiment_designer.rs` (+144 lines) - Undo/redo integration

## Commits

- `f267c840` - feat(02-03): create PropertyInspector widget for node property editing
- `e8fc6b38` - feat(02-03): implement Edit commands for undo/redo support
- `1e8b9da1` - feat(02-02): add undo/redo system and property inspector integration

Note: Final integration commit was made as part of concurrent 02-02 execution but contains 02-03 scope work (undo/redo and property inspector integration).

## Lessons Learned

1. **undo crate API:** Record<E> requires E: Edit, not Target: Edit. Need unified wrapper type for multiple command types.

2. **Concurrent execution:** Plans 02-02 and 02-03 ran concurrently and had overlapping file modifications. The final state is correct but commit attribution is mixed.

3. **egui-snarl selection:** Selection state is stored in egui's Context temporary storage, not in Snarl struct. Must query each frame using the snarl widget's Id.
