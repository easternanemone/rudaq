# Phase 5 Plan 1: Auto-Scale Plot Foundation Summary

**One-liner:** Grow-to-fit plot wrapper with per-axis lock controls for stable live data visualization

---

## Frontmatter

```yaml
phase: 05-live-visualization
plan: 01
subsystem: gui-widgets
status: complete
completed: 2026-01-22
duration: 3min

requires:
  - "04-03: Loop body translation (experiment execution foundation)"
  - "egui_plot 0.34 API"

provides:
  - "AutoScalePlot widget for grow-only axis scaling"
  - "Per-axis lock controls for X/Y independence"
  - "Reset functionality for bounds and locks"

affects:
  - "05-02: Live frame streaming integration"
  - "Future plot-based visualizations in GUI"

tech-stack:
  added: []
  patterns:
    - "Grow-only bounds expansion (axes never shrink automatically)"
    - "Per-axis lock state management"
    - "egui_plot wrapper pattern with auto_bounds configuration"

key-files:
  created:
    - "crates/ui/src/widgets/auto_scale_plot.rs"
  modified:
    - "crates/ui/src/widgets/mod.rs"

decisions:
  - name: "Grow-only scaling"
    rationale: "Prevents jarring visual jumps during live acquisition"
    alternatives: ["Full auto-scale (shrinks)", "Manual-only scaling"]
    impact: "Stable visualization for scientists monitoring experiments"

  - name: "Per-axis lock independence"
    rationale: "Scientists often want to lock X (scan position) while Y (signal) auto-scales"
    alternatives: ["Single lock for both axes", "No lock controls"]
    impact: "Flexible control for different experiment types"

  - name: "None for uninitialized bounds"
    rationale: "Distinguishes uninitialized (first data sets bounds) from initialized state"
    alternatives: ["Default to [0.0, 1.0]", "Require explicit initialization"]
    impact: "Clean initialization semantics, first data always visible"
```

---

## Implementation Summary

### What Was Built

Created `AutoScalePlot` widget wrapper around `egui_plot::Plot` that provides:

1. **Grow-only bounds expansion**: Axes expand to fit new data but never shrink automatically
2. **Per-axis lock controls**: X and Y axes can be locked independently via checkboxes
3. **Reset functionality**: Clears bounds and unlocks both axes
4. **Two rendering modes**:
   - `show()`: Renders plot with current settings
   - `show_with_controls()`: Adds toolbar with Lock X, Lock Y checkboxes and Reset button

### Architecture

```
AutoScalePlot
├── AxisLockState
│   ├── x_locked: bool
│   ├── y_locked: bool
│   ├── x_bounds: Option<[f64; 2]>  // None = uninitialized
│   └── y_bounds: Option<[f64; 2]>
├── update_bounds(&[[f64; 2]])       // Grow-only expansion
├── reset_bounds()                    // Clear + unlock
├── show(ui, id_salt, contents)      // Render plot
└── show_with_controls(...)          // Render with toolbar
```

**Key Logic:**
- `update_bounds()` only expands bounds if unlocked
- First call initializes bounds from data (None → Some([min, max]))
- Locked axes use `include_x`/`include_y` to enforce fixed bounds
- Unlocked axes use `auto_bounds([bool, bool])` for automatic scaling

### Test Coverage

All tests pass (3/3):

1. **test_bounds_grow_only**: Verifies bounds expand but never shrink
2. **test_axis_lock_prevents_update**: Verifies locked axes don't change
3. **test_reset_clears_bounds**: Verifies reset clears bounds and unlocks

### Files Modified

| File | Lines Added | Purpose |
|------|-------------|---------|
| `auto_scale_plot.rs` | 274 | Core widget implementation + tests |
| `widgets/mod.rs` | 2 | Module export |

---

## Deviations from Plan

**None** - Plan executed exactly as written.

---

## Verification Results

✅ **All must_haves satisfied:**

1. ✅ Plot Y-axis grows when new data exceeds current range
2. ✅ Plot Y-axis never shrinks automatically during acquisition
3. ✅ User can lock X-axis independently from Y-axis
4. ✅ User can lock Y-axis independently from X-axis
5. ✅ Reset button restores auto-scale for both axes
6. ✅ File `auto_scale_plot.rs` exists with 274 lines (>100 required)
7. ✅ Exports `AutoScalePlot` and `AxisLockState`
8. ✅ Wrapper configures `auto_bounds` via `Plot::new().auto_bounds(Vec2b)`

**Build verification:**
```bash
$ cargo check -p ui
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.05s
```

**Test verification:**
```bash
$ cargo test -p ui auto_scale_plot
test result: ok. 3 passed; 0 failed; 0 ignored
```

---

## Integration Notes

### Usage Pattern

```rust
use daq_egui::widgets::{AutoScalePlot, AxisLockState};
use egui_plot::Line;

// In panel state
let mut plot = AutoScalePlot::new(AxisLockState::default());

// Update bounds with new data (grow-only)
let points: Vec<[f64; 2]> = vec![[0.0, 1.0], [1.0, 2.0]];
plot.update_bounds(&points);

// Render with controls
plot.show_with_controls(ui, "my_plot", |plot_ui| {
    plot_ui.line(Line::new(points));
});
```

### Next Phase Readiness

**For 05-02 (Live Frame Streaming):**
- `AutoScalePlot` ready to wrap intensity profile plots
- `show_with_controls()` provides UI for locking axes during streaming
- `update_bounds()` should be called on each new frame data

**Known Integration Points:**
- Frame streaming panels will call `plot.update_bounds(profile_data)` on each frame
- Lock controls allow user to freeze X (pixel position) or Y (intensity)
- Reset button useful when changing ROI or exposure settings

---

## Performance Notes

- **Bounds calculation**: O(N) scan over data points (minimal overhead)
- **State updates**: No allocations when bounds are locked
- **UI overhead**: Standard egui_plot rendering, no additional cost

---

## Technical Debt

**None identified.** Widget is self-contained with full test coverage.

---

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1 & 2 | `18b3904d` | feat(05-01): create AutoScalePlot widget with grow-to-fit logic |

---

## Lessons Learned

1. **egui type imports**: `Vec2b` is in `egui` crate, not `egui_plot` (initially tried `.into()`)
2. **Version alignment**: egui_plot 0.34 uses emath types for `auto_bounds()`
3. **Test-first approach**: Writing tests in same commit catches API mismatches early
4. **None for initialization**: Using `Option` for bounds cleanly handles uninitialized state vs locked state

---

## Related Documentation

- **egui_plot API**: `Plot::auto_bounds(Vec2b)`, `include_x()`, `include_y()`
- **Pattern**: Widget wrapper with state management for enhanced plotting behavior
- **Testing**: Unit tests cover all state transitions (grow, lock, reset)
