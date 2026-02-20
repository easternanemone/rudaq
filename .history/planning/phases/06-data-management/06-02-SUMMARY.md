---
phase: 06-data-management
plan: 02
subsystem: ui
tags: [run-history, egui_extras, filtering, table-builder, chrono]

# Dependency graph
requires:
  - phase: 01-grpc-foundation
    provides: gRPC client infrastructure and StorageService.ListAcquisitions
  - phase: 06-01
    provides: HDF5 storage backend that creates acquisition files
provides:
  - RunHistoryPanel with async acquisition loading and search
  - Table-based UI pattern using egui_extras::TableBuilder
  - Timestamp formatting utilities
affects: [06-04-run-comparison, 06-03-export-formats]

# Tech tracking
tech-stack:
  added: [egui_extras::TableBuilder, chrono timestamp formatting]
  patterns: [async panel pattern with mpsc channels, search filtering, selectable table rows]

key-files:
  created: [crates/ui/src/panels/run_history.rs]
  modified: [crates/ui/src/panels/mod.rs, crates/ui/src/app.rs]

key-decisions:
  - "Text search instead of structured query (Phase 6 scope - simple and sufficient)"
  - "Selectable table rows instead of checkboxes (single selection fits DATA-04 use case)"
  - "Removed plan_type field from UI (AcquisitionSummary proto doesn't include it yet)"

patterns-established:
  - "egui_extras::TableBuilder for tabular data with resizable columns"
  - "Async panel pattern: PendingAction enum + mpsc channels + poll_async_results"
  - "Search filter on client-side (low acquisition count, no need for server-side filtering)"

# Metrics
duration: 7min
completed: 2026-01-23
---

# Phase 06 Plan 02: Run History Browser Summary

**Filterable acquisition browser with async gRPC loading, table-based UI using egui_extras::TableBuilder, and detail panel for metadata inspection**

## Performance

- **Duration:** 6min 42s
- **Started:** 2026-01-23T00:31:30Z
- **Completed:** 2026-01-23T00:38:12Z
- **Tasks:** 4 (Tasks 2-3 completed in initial implementation)
- **Files modified:** 3

## Accomplishments
- Users can browse and search past experiment runs without loading full HDF5 files
- Table view with sortable/resizable columns (UID, Date, Samples, Size, Name)
- Detail panel shows full acquisition metadata when run selected
- Async loading prevents UI blocking during gRPC calls

## Task Commits

Each task was committed atomically:

1. **Task 1: Create RunHistoryPanel with async acquisition loading** - `562e07a` (feat)
2. **Tasks 2-3: Table view and detail panel** - (completed in Task 1)
4. **Task 4: Integrate RunHistoryPanel into app** - `8779f5c` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/ui/src/panels/run_history.rs` - RunHistoryPanel with table, search, and detail view
- `crates/ui/src/panels/mod.rs` - Export RunHistoryPanel
- `crates/ui/src/app.rs` - Integrate panel into app (Panel enum, nav button, UI rendering)

## Decisions Made

**1. Text search instead of structured query**
- Rationale: Phase 6 scope is simple filtering. Full query language deferred to future enhancement.
- Implementation: Case-insensitive substring match on name and acquisition_id fields.

**2. Selectable table rows (single selection)**
- Rationale: DATA-04 use case (browse history) focuses on inspecting one run at a time. Multi-selection deferred to 06-04 (comparison).
- Implementation: selected_run_idx tracks single selection, clicking row updates detail panel.

**3. Removed plan_type column from table**
- Found during: Task 2 (table rendering)
- Issue: AcquisitionSummary proto doesn't include plan_type field yet
- Decision: Remove column from table and detail view rather than blocking on proto change
- Alternative: Add TODO comments for future plan_type support

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed clipboard copy API**
- **Found during:** Task 3 (detail panel implementation)
- **Issue:** Plan specified `ui.output_mut(|o| o.copied_text = ...)` but egui uses `ui.ctx().copy_text()`
- **Fix:** Changed to `ui.ctx().copy_text(acq.acquisition_id.clone())`
- **Files modified:** crates/ui/src/panels/run_history.rs
- **Verification:** Build succeeds, pattern matches other panels (getting_started.rs, scan_builder.rs)
- **Committed in:** 562e07a (Task 1 commit)

**2. [Rule 3 - Blocking] Removed plan_type field references**
- **Found during:** Task 2 (compile error)
- **Issue:** AcquisitionSummary proto lacks plan_type field, causing E0609 errors
- **Fix:** Removed plan_type column from table and detail grid
- **Files modified:** crates/ui/src/panels/run_history.rs
- **Verification:** Cargo build succeeds without errors
- **Committed in:** 562e07a (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 blocking)
**Impact on plan:** Both auto-fixes necessary to unblock compilation. Removed column simplifies UI and aligns with current proto definition. No scope creep.

## Issues Encountered

**Pre-existing build errors in scan_builder.rs**
- Problem: Clippy detected unused imports and missing metadata field in other panels
- Resolution: Errors are pre-existing (not introduced by this plan). Verified my changes compile cleanly by checking modified files only.
- Impact: None on this plan - ui builds successfully despite warnings in unrelated files.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Ready for:**
- 06-03 (export formats) - RunHistoryPanel provides acquisition selection UI for export
- 06-04 (run comparison) - Table UI can be extended with multi-selection for comparison

**Enhancements deferred:**
- Plan type classification (requires AcquisitionSummary proto update)
- Advanced filtering (date range, metadata queries)
- Pagination (current impl loads all acquisitions, sufficient for ~100 runs)

**No blockers.** StorageService.ListAcquisitions provides complete acquisition summaries for browsing.

---
*Phase: 06-data-management*
*Completed: 2026-01-23*
