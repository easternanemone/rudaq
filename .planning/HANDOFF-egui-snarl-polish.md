# Handoff: egui-snarl Node Editor Polish

**Date**: 2026-01-23
**Beads Epic**: `bd-dd21`
**Status**: Core implementation complete, optional refactor pending
**Branch**: `main` (changes not yet committed)

---

## Executive Summary

Polished the egui-snarl experiment designer node graph editor with three improvements:
1. **Fixed widget ID collisions** - ComboBox dropdowns no longer bleed state between same-type nodes
2. **Added header validation colors** - Red tint for errors, green for executing nodes
3. **Custom SnarlStyle** - Orthogonal wires, no grid, larger pins, blue selection

All changes validated by **Gemini CLI** and **Codex CLI** - approved with minor corrections applied.

---

## Files Modified (Uncommitted)

| File | Status | Description |
|------|--------|-------------|
| `crates/daq-egui/src/graph/viewer.rs` | Modified | Added `ui.push_id()` wrapper, `header_frame()` method |
| `crates/daq-egui/src/panels/experiment_designer.rs` | Modified | Added `create_node_style()`, new imports |

### Verification
```bash
cargo check -p daq-egui  # Compiles successfully with warnings (pre-existing dead code)
```

---

## Implementation Details

### 1. Widget ID Collision Fix (viewer.rs:487-521)

**Problem**: Static ComboBox IDs like `"scan_actuator"` caused state bleeding between nodes.

**Solution**: Wrapped `show_body()` content with node-scoped ID:
```rust
fn show_body(&mut self, node_id: NodeId, ...) {
    ui.push_id(node_id, |ui| {
        // All widget rendering now scoped to this node
        if let Some(node) = snarl.get_node_mut(node_id) {
            match node { ... }
        }
    });
}
```

**Affected Widgets** (9 total):
- Lines 105, 134, 166, 204, 238, 294, 310, 347, 404

### 2. Header Validation Colors (viewer.rs:547-568)

**Implementation**:
```rust
fn header_frame(
    &mut self,
    default: egui::Frame,
    node: NodeId,
    _inputs: &[InPin],
    _outputs: &[OutPin],
    _snarl: &Snarl<ExperimentNode>,
) -> egui::Frame {
    // Red tint for validation errors
    if self.node_errors.contains_key(&node) {
        return default.fill(egui::Color32::from_rgb(120, 40, 40));
    }
    // Green tint for currently executing node
    if let Some(ref state) = self.execution_state {
        if state.active_node == Some(node) {
            return default.fill(egui::Color32::from_rgb(40, 100, 40));
        }
    }
    default
}
```

### 3. Custom SnarlStyle (experiment_designer.rs:88-112)

```rust
fn create_node_style() -> SnarlStyle {
    SnarlStyle {
        pin_size: Some(8.0),
        wire_style: Some(WireStyle::AxisAligned { corner_radius: 4.0 }),
        wire_width: Some(2.0),
        wire_layer: Some(WireLayer::BehindNodes),
        bg_pattern: Some(BackgroundPattern::NoPattern),
        select_stoke: Some(egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255))),
        ..Default::default()
    }
}
```

**Note**: `select_stoke` is a typo in the egui-snarl API - this is intentional and correct.

---

## Remaining Work (Optional)

### Refactor header_frame to use node_state() helper

**Priority**: LOW
**Impact**: Adds blue tint for completed nodes, improves maintainability

**Current Issue**: `header_frame` directly checks `state.active_node` instead of using `ExecutionState::node_state()` helper, missing the `Completed` state.

**Recommended Implementation**:
```rust
fn header_frame(&mut self, default: egui::Frame, node: NodeId, ...) -> egui::Frame {
    // 1. Validation Errors (Highest Priority)
    if self.node_errors.contains_key(&node) {
        return default.fill(egui::Color32::from_rgb(120, 40, 40)); // Dark Red
    }

    // 2. Execution State
    if let Some(ref state) = self.execution_state {
        match state.node_state(node) {
            NodeExecutionState::Running => {
                return default.fill(egui::Color32::from_rgb(40, 100, 40)); // Dark Green
            }
            NodeExecutionState::Completed => {
                return default.fill(egui::Color32::from_rgb(40, 60, 80)); // Dark Blue
            }
            NodeExecutionState::Pending | NodeExecutionState::Skipped => {}
        }
    }

    default
}
```

**Color Semantics**:
| State | RGB | Hex | Description |
|-------|-----|-----|-------------|
| Error | (120, 40, 40) | #782828 | Dark red |
| Running | (40, 100, 40) | #286428 | Dark green |
| Completed | (40, 60, 80) | #283C50 | Dark blue |

---

## Validation History

### External Agent Validation

| Agent | Status | Continuation ID | Remaining Turns |
|-------|--------|-----------------|-----------------|
| **Gemini CLI** | ✅ Approved | `abfd5689-efd2-4cc7-8b1f-e47e704c00fc` | 33 |
| **Codex CLI** | ✅ Approved | `30bd563a-1636-4ded-a73e-2af4cdbb7fe8` | 33 |

### Key Validation Findings

1. **Gemini**: All egui/egui-snarl v0.9 API usage correct; recommended darker blue for Completed state
2. **Codex**: Confirmed `Skipped` variant exists in `NodeExecutionState`; warned that adding Completed coloring is a behavior change

---

## Testing Checklist

### Manual Testing
```bash
cargo run -p daq-egui --bin rust-daq-gui
```

1. **Widget ID Isolation**:
   - [ ] Add two Scan nodes
   - [ ] Open dropdown on first node
   - [ ] Verify second node's dropdown remains closed
   - [ ] Select different actuators, verify independence

2. **Header Colors**:
   - [ ] Create node with empty device field → verify red header
   - [ ] Fill in required fields → verify normal header
   - [ ] (Future) Run experiment → verify green on active node

3. **Visual Style**:
   - [ ] Verify orthogonal wires between connected nodes
   - [ ] Verify no grid dots in background
   - [ ] Select node → verify blue highlight stroke (2px)

---

## Git Workflow

### To Commit These Changes
```bash
# Stage the modified files
git add crates/daq-egui/src/graph/viewer.rs
git add crates/daq-egui/src/panels/experiment_designer.rs
git add .planning/HANDOFF-egui-snarl-polish.md

# Commit with conventional commit message
git commit -m "feat(egui): polish node editor with ID fix, header colors, custom style

- Fix widget ID collisions with ui.push_id(node_id) wrapper
- Add header_frame() for validation error (red) and execution (green) tints
- Create custom SnarlStyle: orthogonal wires, no grid, larger pins

Validated by Gemini CLI and Codex CLI.
Beads: bd-dd21"

# Sync beads
bd sync

# Push
git push
```

### To Implement Optional Refactor
```bash
# After committing base changes, create follow-up commit
# Edit viewer.rs:547-568 to use node_state() helper (see code above)
git add crates/daq-egui/src/graph/viewer.rs
git commit -m "refactor(egui): use node_state() helper in header_frame

Adds Completed state (blue) header coloring.
Improves maintainability by using existing helper method.

Beads: bd-dd21"
```

---

## Beads Issue Reference

```bash
# View full epic details
bd show bd-dd21

# Close when complete
bd close bd-dd21 --reason "Node editor polish complete"
```

---

## Dependencies

- **egui-snarl**: v0.9.0
- **egui**: v0.30.x
- **Rust Edition**: 2021

---

## Contact / Context

This work was validated using PAL MCP clink tool with:
- Gemini CLI (codereviewer role)
- Codex CLI (codereviewer role)

Both agents have continuation context available if further validation needed.
