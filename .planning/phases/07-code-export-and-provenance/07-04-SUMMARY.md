---
phase: 07-code-export-and-provenance
plan: 04
subsystem: ui
tags: [export, rhai, script-editor, egui]

# Dependency graph
requires:
  - phase: 07-01
    provides: Code generation infrastructure (graph_to_rhai_script)
provides:
  - Export to .rhai file functionality via file dialog
  - ScriptEditorPanel for editing Rhai scripts in ejected mode
  - Eject-to-script mode switching with confirmation dialog
  - Save/save-as functionality in script editor
affects:
  - Phase 8 (Script execution and testing) - script editor output ready
  - Power user workflows - can now export and version control experiments

# Tech tracking
tech-stack:
  added: []
  patterns:
    - One-way code export (visual editor is source of truth, code is export artifact)
    - Mode switching in panels (graph vs script editor modes)
    - File dialog integration with rfd

key-files:
  created:
    - crates/daq-egui/src/panels/script_editor.rs
  modified:
    - crates/daq-egui/src/panels/experiment_designer.rs
    - crates/daq-egui/src/panels/mod.rs

key-decisions:
  - "ScriptEditorPanel is separate from graph editor, not embedded"
  - "One-way eject prevents accidental loss of graph structure"
  - "Script editor doesn't sync changes back to graph"

patterns-established:
  - "File dialog integration: use rfd::FileDialog with add_filter() for file type filtering"
  - "Panel state management: use Option<ScriptEditorPanel> to toggle modes"
  - "Confirmation dialogs for destructive actions (eject is one-way)"

# Metrics
duration: 12min
completed: 2026-01-25
---

# Phase 7 Plan 4: Export and Script Editor Mode Summary

**Export Rhai button with file dialog, ScriptEditorPanel for ejected mode, eject confirmation flow**

## Performance

- **Duration:** 12 min
- **Started:** 2026-01-25T14:00:00Z
- **Completed:** 2026-01-25T14:12:00Z
- **Tasks:** 3 completed + 1 human verification
- **Files modified:** 3

## Accomplishments
- Export to .rhai file saves generated code with proper syntax
- ScriptEditorPanel provides editable code with syntax highlighting, save/save-as, and theme selection
- Eject-to-script mode with confirmation dialog prevents accidental one-way conversions
- Script editor properly displays unsaved change indicators

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Export Rhai button with file dialog** - `9ee0f73d` (feat)
   - Export button in toolbar opens rfd file dialog
   - .rhai filter pre-selected
   - Generated code written to file
   - Status message on success/failure

2. **Task 2: Create ScriptEditorPanel for ejected mode** - `ac8809c4` (feat)
   - New ScriptEditorPanel struct with code editing
   - Save/save-as functionality with file dialog
   - Theme selector (Gruvbox Dark/Light, Aura Dark)
   - Dirty flag for unsaved changes tracking
   - egui_code_editor integration with Rhai syntax

3. **Task 3: Add Eject button and mode switching** - `9a35462f` (feat)
   - Eject button in toolbar
   - Confirmation dialog warns about one-way conversion
   - script_editor field in ExperimentDesignerPanel
   - Mode switching: shows script editor when ejected
   - "New Graph" button provides escape hatch back to graph mode

4. **Human verification** - approved
   - All functionality tested and verified working
   - Code preview scrolling fixed (bd-js4b)
   - Code preview layout fixed (bd-51l7)

## Files Created/Modified

- `crates/daq-egui/src/panels/script_editor.rs` - New ScriptEditorPanel with full editor functionality
- `crates/daq-egui/src/panels/experiment_designer.rs` - Added export, eject buttons and mode switching
- `crates/daq-egui/src/panels/mod.rs` - Exported ScriptEditorPanel

## Decisions Made

- **ScriptEditorPanel is separate panel, not embedded in designer:** Cleaner separation of concerns, can be extended independently for future script features
- **One-way eject with confirmation:** Prevents accidental conversion, makes visual graph the authoritative source
- **No sync back to graph:** Enforces one-way code generation pattern established in phase 07-01
- **File dialog filtering:** Used rfd with add_filter() for .rhai files to guide users
- **Theme selector in script editor:** Gives power users visual comfort options when spending time in script mode

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Code preview scrolling not working with dynamic height**
- **Found during:** Task 4 (Human verification)
- **Issue:** Code preview panel didn't scroll when text exceeded viewport height
- **Fix:** Replaced fixed `with_rows()` with dynamic height calculation based on available viewport
- **Files modified:** crates/daq-egui/src/panels/experiment_designer.rs
- **Verification:** Code preview now scrolls properly for long scripts, no loss of content
- **Committed in:** 06b39aea (separate fix commit from verification feedback)

**2. [Rule 1 - Bug] Code preview layout breaking tab boundaries**
- **Found during:** Task 4 (Human verification)
- **Issue:** Code preview panel was overflowing outside tab boundaries, causing visual layout issues
- **Fix:** Changed to `show_inside()` for CodeEditor to respect egui panel constraints
- **Files modified:** crates/daq-egui/src/panels/experiment_designer.rs
- **Verification:** Code preview now properly contained within tab, no overflow
- **Committed in:** 06b39aea (same fix commit)

---

**Total deviations:** 2 auto-fixed (both Rule 1 layout bugs discovered during verification)
**Impact on plan:** Both fixes essential for usable UI. No scope creep - fixing visual problems found during human verification.

## Issues Encountered

None - plan executed smoothly. Verification revealed two layout issues that were immediately fixed.

## Next Phase Readiness

**CODE-02 (Export as standalone Rhai file):** ✓ Complete
- Export button opens file dialog with .rhai filter
- Generated code has proper syntax and comments
- File save works with success/failure feedback

**CODE-03 (Switch to script editor mode):** ✓ Complete
- Eject button with confirmation dialog (prevents accidental loss)
- Script editor mode shows editable code
- Save/save-as functionality implemented
- Mode switching works correctly (graph ↔ script via "New Graph" button)

**Ready for Phase 8:** Script execution engine can now:
- Accept .rhai scripts from export
- Load and execute scripts from ScriptEditorPanel output
- Full power user workflow enabled

---
*Phase: 07-code-export-and-provenance*
*Completed: 2026-01-25*
