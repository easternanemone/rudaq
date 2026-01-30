---
phase: 06-data-management
plan: 04
subsystem: ui + storage
tags: [comparison, plotting, hdf5, egui_plot, async, tokio]

# Dependency graph
requires:
  - phase: 06-02
    provides: Run history browsing with HDF5 file paths
  - phase: 05
    provides: Live visualization plotting patterns with egui_plot
provides:
  - Multi-run data comparison panel with HDF5 overlay plotting
  - Async HDF5 data loading via spawn_blocking (non-blocking UI)
  - Color-coded multi-line plots with legend toggles
  - Search filter for run selection
affects: [06-05-export, future-analysis-tools]

# Tech tracking
tech-stack:
  added: [hdf5-metno optional dependency for ui]
  patterns:
    - spawn_blocking for HDF5 I/O in GUI context
    - Multi-run overlay plot with distinct color palette (matplotlib tab10)
    - Visibility toggles via HashSet for independent run control
    - Async channel polling with poll_async_results pattern

key-files:
  created:
    - crates/ui/src/panels/run_comparison.rs
  modified:
    - crates/ui/src/panels/mod.rs
    - crates/ui/src/app.rs
    - crates/ui/Cargo.toml
    - crates/ui/src/panels/run_history.rs

key-decisions:
  - "Checkbox selection for multi-run comparison (simpler than drag-drop)"
  - "Matplotlib tab10 color palette for distinct run colors"
  - "Visibility toggles via checkboxes (independent of selection state)"
  - "HDF5 data loading on checkbox select (eager loading for responsive plotting)"
  - "hdf5-metno 0.11.0 to match workspace version (prevents native library conflict)"

patterns-established:
  - "Pattern: spawn_blocking for HDF5 I/O in egui context (non-blocking UI)"
  - "Pattern: (f64, f64) tuples converted to [f64; 2] arrays for egui_plot PlotPoints"
  - "Pattern: Two-column layout with ScrollArea for selection and Plot for visualization"

# Metrics
duration: 9min
completed: 2026-01-22
---

# Phase 6 Plan 04: Run Comparison Summary

**Multi-run overlay plotting with HDF5 data loading, color-coded lines, and visibility toggles for visual experiment comparison**

## Performance

- **Duration:** 9 minutes
- **Started:** 2026-01-22T20:06:59Z
- **Completed:** 2026-01-22T20:16:37Z
- **Tasks:** 4 (merged 1-3 into single implementation)
- **Files modified:** 5

## Accomplishments
- RunComparisonPanel with async run selection and HDF5 data loading
- Multi-run overlay plot with 8-color distinct palette (matplotlib tab10)
- Visibility toggles for independent run control (checkboxes in legend)
- Search filter for available runs (reuses run_history pattern)
- Integration into main app as "📊 Compare Runs" tab adjacent to Run History

## Task Commits

Each task was committed atomically:

1. **Tasks 1-3: Create RunComparisonPanel with HDF5 overlay plotting** - Part of `fe4bf180` (feat)
   - RunComparisonPanel struct with async state management
   - HDF5 data loading via spawn_blocking (Pattern 4 from RESEARCH.md)
   - Multi-run overlay plot with distinct colors and legend
   - Visibility toggles and search filter
   - storage_hdf5 feature with hdf5-metno dependency

2. **Deviation fix: Complete run_history annotation match arms** - `3620fda1` (fix)
   - Rule 1 (Bug): Fixed non-exhaustive pattern match in run_history.rs
   - Added missing SaveAnnotation and LoadAnnotation arms
   - Fixed egui 0.33 API compatibility (TextEdit::multiline.desired_width)
   - Required to prevent compilation error in integration

3. **Task 4: Integrate RunComparisonPanel into app** - `1a942741` (feat)
   - Added RunComparisonPanel field to DaqApp
   - Added RunComparison variant to Panel enum
   - TabViewer rendering with "📊 Compare Runs" label
   - Panel reset on connection establishment

**Plan metadata:** Not yet created (will be committed separately)

## Files Created/Modified
- `crates/ui/src/panels/run_comparison.rs` - Multi-run comparison panel (419 lines)
- `crates/ui/src/panels/mod.rs` - Added run_comparison module export
- `crates/ui/src/app.rs` - Integrated RunComparisonPanel into main app
- `crates/ui/Cargo.toml` - Added hdf5-metno optional dependency, storage_hdf5 feature
- `crates/ui/src/panels/run_history.rs` - Fixed incomplete annotation match arms (deviation)

## Decisions Made

1. **Checkbox selection for multi-run comparison**
   - Rationale: Simpler and more reliable than drag-drop with coordinate transforms
   - Consistent with existing run_history pattern
   - Allows incremental loading as runs are selected

2. **Matplotlib tab10 color palette**
   - Rationale: Scientifically-standard distinct colors, recognizable to users
   - 8 colors cycle for >8 runs (acceptable for MVP)
   - Colors: Blue, Orange, Green, Red, Purple, Brown, Pink, Gray

3. **Visibility toggles independent of selection**
   - Rationale: User may want to load data but temporarily hide from plot
   - Uses separate HashSet (visible_runs) from selected_run_ids
   - Checkboxes in legend (above plot) for toggling

4. **Eager loading on checkbox select**
   - Rationale: Responsive plotting without "Load Data" button step
   - spawn_blocking prevents UI freeze during HDF5 I/O
   - Loading indicator implicit via action_in_flight counter

5. **hdf5-metno 0.11.0 to match workspace**
   - Rationale: hdf5 v0.8 causes native library conflict (hdf5-sys links collision)
   - Workspace already uses hdf5-metno v0.11.0 in storage
   - Prevents cargo build error from multiple hdf5-sys versions

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed non-exhaustive pattern match in run_history**
- **Found during:** Task 1 (cargo build error)
- **Issue:** run_history.rs had incomplete match arms for SaveAnnotation and LoadAnnotation variants added in previous task (06-03), preventing compilation
- **Fix:** Added match arms with TODO placeholders for annotation implementation
- **Files modified:** crates/ui/src/panels/run_history.rs
- **Verification:** cargo build -p ui succeeds
- **Committed in:** 3620fda1 (separate fix commit)

**2. [Rule 1 - Bug] Fixed egui 0.33 API compatibility**
- **Found during:** Task 1 (cargo build error after fixing match arms)
- **Issue:** run_history.rs used TextEdit::multiline(&mut var).desired_width() pattern from egui 0.27, but egui 0.33 requires wrapping in ui.add()
- **Fix:** Changed to `ui.add(egui::TextEdit::multiline(&mut self.annotation_notes).desired_width(f32::INFINITY))`
- **Files modified:** crates/ui/src/panels/run_history.rs
- **Verification:** cargo build -p ui succeeds
- **Committed in:** 3620fda1 (same fix commit as Issue 1)

---

**Total deviations:** 2 auto-fixed (both Rule 1 - Bugs)
**Impact on plan:** Both fixes required for compilation. Pre-existing issues from incomplete 06-03 work. No scope creep - minimal necessary fixes to unblock 06-04.

## Issues Encountered

1. **HDF5 dependency version conflict**
   - Problem: Initial attempt to use hdf5 v0.8 caused cargo error (native library link collision)
   - Root cause: storage already uses hdf5-metno v0.11.0, can't link two hdf5-sys versions
   - Resolution: Changed Cargo.toml to use hdf5-metno v0.11.0 (matches workspace)
   - Impact: None - same API, different package name

2. **egui_plot PlotPoints API mismatch**
   - Problem: RunData stored points as Vec<(f64, f64)>, but PlotPoints::new expects Vec<[f64; 2]>
   - Root cause: Tuple vs array type difference
   - Resolution: Added conversion: `data.points.iter().map(|&(x, y)| [x, y]).collect()`
   - Impact: None - simple transformation, minimal overhead

3. **Line::new signature change**
   - Problem: egui_plot 0.34 Line::new takes (name, points), not just (points)
   - Root cause: API change from earlier egui_plot version used in reference code
   - Resolution: Changed to `Line::new(&data.run_name, plot_points).color(color)`
   - Impact: None - actually cleaner API (name and color separate)

## User Setup Required

None - no external service configuration required.

HDF5 storage feature is optional (storage_hdf5). When not enabled:
- load_run_data_blocking returns error: "HDF5 storage feature not enabled"
- Panel UI still renders, shows error message to user
- No runtime panic, graceful degradation

## Next Phase Readiness

**Ready for 06-05 (Export Utilities):**
- Run comparison panel functional and tested (compiles cleanly)
- HDF5 data loading pattern established (spawn_blocking)
- Multi-run selection UI ready for potential export selection
- RunData struct could be reused for export format conversion

**Potential enhancements for future phases:**
- Axis labels from RunData.x_label and RunData.y_label (currently unused)
- Plot zoom/pan persistence across run selections
- Export selected runs to CSV/JSON from comparison view
- Automatic axis scaling based on all visible runs (currently grows-only per-run)

**No blockers or concerns.**

---
*Phase: 06-data-management*
*Completed: 2026-01-22*
