# Handoff: Phase 7 egui-snarl Integration Fixes

**Created**: 2026-01-22
**Context**: Phase 7 (Code Export and Provenance) plan 07-04 human verification checkpoint
**Status**: Fixes applied, awaiting user verification

## Summary

Fixed three critical bugs in the egui-snarl node graph integration in the Experiment Designer panel. All fixes compile successfully but require user testing.

## Bugs Fixed

### 1. Context Menu Disappearing Immediately

**Symptom**: Right-click context menu for adding nodes would flash and disappear instantly.

**Root Cause**: The menu dismissal logic used `any_click()` which detected the same right-click that opened the menu.

**Fix**: Added secondary click exclusion in `experiment_designer.rs`:
```rust
// Close menu when clicking elsewhere
// Ignore secondary click to prevent closing immediately on the same frame it opens
if ui.input(|i| {
    i.pointer.any_click()
        && !i.pointer.secondary_clicked()
        && i.pointer.hover_pos().is_some_and(|p| p != pos)
}) {
    self.context_menu_pos = None;
}
```

### 2. Node Selection Always Empty

**Symptom**: `get_selected_nodes()` always returned `[]` even after clicking nodes.

**Root Cause**: Two issues:
1. Event handlers (`handle_context_menu`, `handle_canvas_drop`) were rendered BEFORE `SnarlWidget`, stealing pointer events
2. ID mismatch between standalone `get_selected_nodes(id, ctx)` and widget's internal ID

**Fix**:
1. Reordered rendering: SnarlWidget first, handlers after
2. Changed to `widget.get_selected_nodes(ui)` for ID consistency

**Important Discovery**: egui-snarl 0.9 requires **Shift+Click** to select nodes (not plain click). This is by design in the library.

### 3. Drag-and-Drop Not Working

**Symptom**: Dragging nodes from palette to canvas did nothing.

**Root Cause**: `ui.available_rect_before_wrap()` was called AFTER `widget.show()`, which returns an empty/wrong rect because the widget consumed all available space.

**Fix**: Capture canvas rect BEFORE widget rendering:
```rust
// Capture canvas rect BEFORE widget consumes space (for drop detection)
let canvas_rect = ui.available_rect_before_wrap();

// Render widget
widget.show(&mut self.snarl, &mut self.viewer, ui);

// Handle drop using pre-captured rect
self.handle_canvas_drop_at(ui, canvas_rect);
```

Renamed method from `handle_canvas_drop` to `handle_canvas_drop_at(ui, canvas_rect)`.

## Key File

**`/Users/briansquires/code/rust-daq/crates/daq-egui/src/panels/experiment_designer.rs`**

Critical section in `show_main_panel()`:
```rust
// Main canvas area
egui::CentralPanel::default().show_inside(ui, |ui| {
    // Sync execution state to viewer for node highlighting
    if self.execution_state.is_active() {
        self.viewer.execution_state = Some(self.execution_state.clone());
    } else {
        self.viewer.execution_state = None;
    }

    // Capture canvas rect BEFORE widget consumes space (for drop detection)
    let canvas_rect = ui.available_rect_before_wrap();

    // Define SnarlWidget
    let snarl_id = egui::Id::new("experiment_graph");
    let widget = SnarlWidget::new()
        .id(snarl_id)
        .style(self.style.clone());

    // 1. Render Graph FIRST (so it's the background/base layer)
    widget.show(&mut self.snarl, &mut self.viewer, ui);

    // 2. Render Overlays/Handlers AFTER (so they are on top)
    self.handle_context_menu(ui);
    self.handle_canvas_drop_at(ui, canvas_rect);

    // 3. Query selection using the widget instance (ensures ID consistency)
    let selected = widget.get_selected_nodes(ui);

    // DEBUG: print on every click
    if ui.input(|i| i.pointer.any_click()) {
        eprintln!(
            "CLICK - dragging: {:?}, selected: {:?}",
            self.dragging_node.as_ref().map(|n| n.name()),
            selected
        );
    }

    self.selected_node = selected.first().copied();
});
```

## Testing Instructions

```bash
cargo run -p daq-egui
```

In the Experiment Designer tab:
1. **Right-click** on canvas → context menu should stay open until clicking elsewhere
2. **Drag** a node type from the left palette → should create node on canvas at drop position
3. **Shift+Click** a node → should select it (terminal shows `selected: [NodeId(X)]`)

## Current State

- [x] All three fixes implemented
- [x] Build compiles successfully (37 warnings, 0 errors)
- [ ] User verification pending
- [ ] Git commit pending (no commits made yet for these fixes)

## Phase 7 Context

This work is part of Phase 7 plan 07-04 (Export and script editor mode) which reached a human verification checkpoint. The checkpoint was for testing the Code Preview panel and Script Editor mode, but these egui-snarl bugs were blocking basic graph interaction.

**Phase 7 Plans**:
- [x] 07-01: Code generation engine
- [x] 07-02: Provenance tracking
- [x] 07-03: Code preview panel
- [ ] 07-04: Export and script editor mode (at checkpoint)

**Roadmap**: `/Users/briansquires/code/rust-daq/.planning/ROADMAP.md`
**Phase directory**: `/Users/briansquires/code/rust-daq/.planning/phases/07-code-export/`

## Technical References

- egui-snarl 0.9 source: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-snarl-0.9.0/`
- Selection logic in egui-snarl: `src/ui.rs` lines 1866-1871 (Shift+Click required)
- `SelectedNodes` stored via `ctx.data()` temp storage

## Next Steps After Verification

1. If fixes work: commit changes, complete 07-04 checkpoint
2. If issues remain: debug with terminal output (click logging is enabled)
3. Complete Phase 7 verification and update ROADMAP.md

## Gemini Consultation Notes

Two PAL/clink sessions with Gemini helped identify:
1. Rendering order matters in egui (earlier widgets get events first)
2. Use `widget.get_selected_nodes(ui)` not standalone function
3. Ignore `secondary_clicked()` in menu dismissal
