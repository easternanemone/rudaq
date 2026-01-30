---
phase: 03-plan-translation-and-execution
verified: 2026-01-22T23:45:00Z
status: gaps_found
score: 3/4 success criteria verified
gaps:
  - truth: "User can execute experiment from node graph editor, with visual feedback of running nodes"
    status: partial
    reason: "Visual highlighting infrastructure exists but not activated (egui-snarl API limitation)"
    artifacts:
      - path: "crates/ui/src/graph/viewer.rs"
        issue: "header_color() method exists but never called (dead code warning)"
      - path: "crates/ui/src/panels/experiment_designer.rs"
        issue: "run_experiment() has TODO - GraphPlan not sent to server via gRPC"
    missing:
      - "GraphPlan serialization and queueing via DaqClient.queue_plan()"
      - "Server-side GraphPlan deserialization and execution"
      - "Visual node highlighting activation (requires egui-snarl API or overlay rendering)"
---

# Phase 3: Plan Translation and Execution Verification Report

**Phase Goal:** Experiments designed visually translate to executable Plans and run via RunEngine
**Verified:** 2026-01-22T23:45:00Z
**Status:** gaps_found
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| #   | Truth                                                         | Status          | Evidence                                              |
| --- | ------------------------------------------------------------- | --------------- | ----------------------------------------------------- |
| 1   | User can execute experiment from node graph editor, with visual feedback of running nodes | ⚠️ PARTIAL | Run button exists, translation works, but visual highlighting not active and GraphPlan not sent to server |
| 2   | User can pause running experiment at checkpoint, modify device parameters, and resume | ✓ VERIFIED | Pause/Resume buttons call DaqClient methods, RuntimeParameterEditor integrated |
| 3   | User sees current progress (step N of M, percentage, estimated time remaining) | ✓ VERIFIED | ProgressBar with percentage, ETA calculation in ExecutionState |
| 4   | Validation errors prevent execution (missing devices, invalid parameters, cycles in graph) | ✓ VERIFIED | validate_graph() runs before execution, cycle detection via Kahn's algorithm |

**Score:** 3/4 truths verified (Truth 1 is partial)

### Required Artifacts

| Artifact | Expected | Status | Details |
| -------- | -------- | ------ | ------- |
| `crates/ui/src/client.rs` | pause_engine, resume_engine, get_engine_status methods | ✓ VERIFIED | Lines 866-889, all three methods implemented with correct proto types |
| `crates/ui/src/graph/translation.rs` | GraphPlan implementing Plan trait | ✓ VERIFIED | 343 lines, impl Plan at line 89, topological sort with cycle detection |
| `crates/ui/src/graph/validation.rs` | validate_graph_structure with cycle detection | ✓ VERIFIED | Function at line 62, uses Kahn's algorithm, integrated into experiment_designer.rs |
| `crates/ui/src/graph/execution_state.rs` | ExecutionState tracking | ✓ VERIFIED | 207 lines, tracks engine state, active node, progress, ETA |
| `crates/ui/src/panels/experiment_designer.rs` | Run/Pause/Resume controls | ✓ VERIFIED | show_execution_toolbar at line 670, buttons call pause_engine/resume_engine |
| `crates/ui/src/widgets/runtime_parameter_editor.rs` | Parameter editing widget | ✓ VERIFIED | 222 lines, EditableParameter struct, RuntimeParameterEditor widget |
| `crates/ui/src/graph/viewer.rs` | Visual highlighting support | ⚠️ ORPHANED | header_color() exists but never called (dead code warning) |

### Key Link Verification

| From | To | Via | Status | Details |
| ---- | --- | --- | ------ | ------- |
| ExperimentDesignerPanel | DaqClient.pause_engine | gRPC call in async spawn | ✓ WIRED | Line 814: `client.pause_engine(true).await` |
| ExperimentDesignerPanel | DaqClient.resume_engine | gRPC call in async spawn | ✓ WIRED | Line 829: `client.resume_engine().await` |
| ExperimentDesignerPanel | DaqClient.abort_plan | gRPC call in async spawn | ✓ WIRED | Line 844: `client.abort_plan(None).await` |
| RuntimeParameterEditor | DaqClient.set_parameter | gRPC call in async spawn | ✓ WIRED | Line 1050: `client.set_parameter(&device_id, &param_name, &new_value).await` |
| run_experiment | GraphPlan translation | GraphPlan::from_snarl | ✓ WIRED | Line 776: translation called before execution |
| run_experiment | DaqClient.queue_plan | GraphPlan serialization + gRPC | ✗ NOT_WIRED | Line 795: TODO comment, GraphPlan not sent to server |
| validate_graph | validate_graph_structure | Cycle detection | ✓ WIRED | Line 607: validate_graph_structure called before per-node validation |
| ExecutionState | node_state | Visual highlighting | ⚠️ PARTIAL | header_color computes colors but never applied (egui-snarl API gap) |

### Requirements Coverage

Phase 3 maps to requirements EXEC-03, EXEC-04, EXEC-05, EXEC-06:

| Requirement | Status | Blocking Issue |
| ----------- | ------ | -------------- |
| EXEC-03: Execute from graph editor | ⚠️ PARTIAL | GraphPlan not sent to server (TODO at line 795) |
| EXEC-04: Pause/resume at checkpoint | ✓ SATISFIED | Pause/resume buttons functional |
| EXEC-05: Modify parameters while paused | ✓ SATISFIED | RuntimeParameterEditor integrated |
| EXEC-06: Progress display | ✓ SATISFIED | Progress bar with ETA |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
| ---- | ---- | ------- | -------- | ------ |
| experiment_designer.rs | 795 | TODO: Queue plan via gRPC | 🛑 Blocker | Execution workflow incomplete - GraphPlan never sent to server |
| viewer.rs | 65 | Dead code warning: header_color | ⚠️ Warning | Visual feedback infrastructure unused |
| experiment_designer.rs | 257 | TODO: Visual node highlighting | ℹ️ Info | Awaiting egui-snarl API support |

### Human Verification Required

**Test 1: Execute experiment with visual node highlighting**
- **Test:** Create simple graph (Scan node), click Run, observe nodes during execution
- **Expected:** Running node shows green background, completed node shows blue
- **Why human:** Visual appearance requires GUI inspection; header_color exists but not applied
- **Status from 03-04 Summary:** User approved, but noted visual highlighting pending egui-snarl API

**Test 2: Full execution workflow with daemon**
- **Test:** Start daemon, connect GUI, run graph-based experiment end-to-end
- **Expected:** Progress updates, pause works, parameter changes apply, execution completes
- **Why human:** Integration testing requires daemon, real device interaction
- **Status from 03-04 Summary:** User approved with note about GraphPlan server integration

**Test 3: Cycle detection prevents invalid execution**
- **Test:** Create cycle (Node A → Node B → Node A), attempt to run
- **Expected:** Run button disabled, status bar shows "Graph contains a cycle"
- **Why human:** User interaction required to verify error messaging clarity
- **Status from 03-04 Summary:** User approved

### Gaps Summary

**Gap 1: GraphPlan not sent to server (Truth 1 - Execute from graph editor)**

The execution workflow stops at the UI level:
1. ✓ Graph translates to GraphPlan successfully
2. ✓ UI tracks execution state (progress, active node)
3. ✗ GraphPlan never queued via `DaqClient.queue_plan()`
4. ✗ Server doesn't deserialize/execute GraphPlan

**Why this is a gap:** User clicks Run, sees "Starting experiment with N events", but nothing actually executes on the daemon. The TODO comment at line 795 explicitly states this limitation.

**What's missing:**
- GraphPlan serialization (to proto or JSON)
- DaqClient method to queue GraphPlan
- Server-side GraphPlan deserialization
- RunEngine integration to execute graph-based plans

**Gap 2: Visual node highlighting not activated (Truth 1 - Visual feedback)**

The infrastructure exists but is unused:
1. ✓ `header_color()` computes correct colors based on execution state
2. ✓ ExecutionState synced to viewer before render
3. ✗ egui-snarl SnarlViewer trait doesn't expose header color customization
4. ✗ No overlay rendering implemented as workaround

**Why this is a gap:** Users cannot see which node is currently executing. The dead code warning confirms `header_color()` is never called.

**What's missing:**
- Either: egui-snarl API enhancement to support custom header colors
- Or: Custom painter overlay rendering after snarl.show()

**Impact:** Truth 1 is partial - execution infrastructure exists but incomplete at both ends (client doesn't send, visual feedback doesn't display).

---

## Detailed Verification

### Level 1: Existence ✓

All artifacts exist:
- `translation.rs` (343 lines)
- `execution_state.rs` (207 lines)
- `runtime_parameter_editor.rs` (222 lines)
- `viewer.rs` with header_color method
- Client methods in `client.rs`

### Level 2: Substantive ✓

**Line count check:**
- translation.rs: 343 lines ✓ (Component threshold: 15+)
- execution_state.rs: 207 lines ✓
- runtime_parameter_editor.rs: 222 lines ✓

**Stub pattern check:**
- ❌ TODO at experiment_designer.rs:795 (blocker - queue plan via gRPC)
- ✓ No empty returns in core logic
- ✓ No placeholder content in widgets

**Export check:**
- ✓ GraphPlan exported from translation module
- ✓ ExecutionState exported from graph module
- ✓ RuntimeParameterEditor exported from widgets module

**Result:** SUBSTANTIVE (with noted TODO blocker)

### Level 3: Wired

**Wiring verification:**

```bash
# DaqClient methods used?
grep -r "pause_engine\|resume_engine" crates/ui/src/panels/experiment_designer.rs
# Result: ✓ Called at lines 814, 829

# GraphPlan translation called?
grep -r "GraphPlan::from_snarl" crates/ui/src/panels/experiment_designer.rs
# Result: ✓ Called at line 776

# Validation integrated?
grep -r "validate_graph_structure" crates/ui/src/panels/experiment_designer.rs
# Result: ✓ Called at line 607

# Parameter editor integrated?
grep -r "show_parameter_editor_panel" crates/ui/src/panels/experiment_designer.rs
# Result: ✓ Called at line 306 when paused

# header_color used?
grep -r "header_color" crates/ui/src/graph/viewer.rs
# Result: ⚠️ Defined but not called (dead code warning)
```

**Result:** WIRED (except visual highlighting and GraphPlan queueing)

---

## Test Results

```bash
cargo test -p ui translation
# ✓ test_empty_graph ... ok
# ✓ test_cycle_detection ... ok
# ✓ test_single_node ... ok

cargo test -p ui execution_state
# ✓ test_checkpoint_parsing ... ok
# ✓ test_progress_calculation ... ok

cargo build -p ui
# ✓ Build succeeds with 2 warnings (dead code: header_color, variant Started)
```

---

_Verified: 2026-01-22T23:45:00Z_
_Verifier: Claude (gsd-verifier)_
