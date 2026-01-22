# Phase 3: Plan Translation and Execution - Research

**Researched:** 2026-01-22
**Domain:** Node graph to executable Plan translation, RunEngine integration, visual execution feedback
**Confidence:** HIGH

## Summary

This phase bridges the visual node graph editor (Phase 2) with the existing RunEngine execution system. The core challenge is translating a directed acyclic graph (DAG) of `ExperimentNode` objects into a linear sequence of `PlanCommand` values that the RunEngine can execute. The existing infrastructure is well-suited for this:

- **RunEngine** already implements pause/resume/abort with checkpoint semantics
- **gRPC proto** already defines `PauseEngineRequest`, `ResumeEngineRequest`, `GetEngineStatus`
- **DaqClient** has partial implementation but needs pause/resume/status methods
- **egui-snarl** provides `wires()` iterator for connection traversal

The key architectural decision is whether to convert the graph to an existing Plan type (like `ImperativePlan`) or create a new `GraphPlan` type. Given the complexity of nested loops and the need for progress tracking, **a new `GraphPlan` implementing the Plan trait is the recommended approach**.

**Primary recommendation:** Create a `GraphPlan` struct that walks the node graph via topological sort, translating each node to PlanCommands with embedded checkpoints for pause/resume support.

## Standard Stack

The established libraries/tools for this domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| egui-snarl | 0.9 | Graph structure | Already in use, `wires()` provides connection iteration |
| daq-experiment | internal | Plan/RunEngine | Existing Plan trait and RunEngine with pause/resume |
| daq-proto | internal | gRPC messages | RunEngineService already defined with full control messages |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| petgraph (OPTIONAL) | 0.6 | Graph algorithms | If cycle detection or complex traversal needed |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| petgraph | Manual DFS | petgraph adds dependency but provides battle-tested algorithms |
| Manual DAG walk | snarl iteration | snarl's `wires()` is sufficient for simple topological order |

**Installation:**
No new dependencies required. petgraph only if cycle detection proves complex.

## Architecture Patterns

### Recommended Project Structure
```
crates/daq-egui/src/
├── graph/
│   ├── mod.rs                    # Existing module
│   ├── nodes.rs                  # ExperimentNode types
│   ├── validation.rs             # Extended with cycle detection
│   ├── translation.rs            # NEW: Graph -> Plan translation
│   └── execution_state.rs        # NEW: Visual execution tracking
├── widgets/
│   └── execution_overlay.rs      # NEW: Running node highlighting
└── panels/
    └── experiment_designer.rs    # Extended with run button
```

### Pattern 1: Graph-to-Plan Translation
**What:** Topological traversal of node graph producing PlanCommands
**When to use:** Converting visual workflow to executable sequence
**Example:**
```rust
// Source: Design pattern based on existing Plan trait
pub struct GraphPlan {
    /// Linearized commands from graph traversal
    commands: Vec<PlanCommand>,
    /// Current execution index
    current_idx: usize,
    /// Total expected events (for progress)
    total_events: usize,
    /// Node ID -> command range mapping (for visual feedback)
    node_ranges: HashMap<NodeId, Range<usize>>,
}

impl GraphPlan {
    pub fn from_snarl(
        snarl: &Snarl<ExperimentNode>,
        device_registry: &[DeviceInfo],
    ) -> Result<Self, TranslationError> {
        // 1. Find root nodes (no incoming edges)
        // 2. Topological sort via DFS
        // 3. Translate each node to PlanCommands
        // 4. Insert checkpoints between major steps
    }
}

impl Plan for GraphPlan {
    fn next_command(&mut self) -> Option<PlanCommand> {
        // Return next command, advancing index
    }

    fn num_points(&self) -> usize {
        self.total_events
    }
}
```

### Pattern 2: Execution State Broadcasting
**What:** RunEngine status streamed to GUI for visual feedback
**When to use:** Updating node highlighting during execution
**Example:**
```rust
// Source: Existing Document streaming pattern
pub struct ExecutionState {
    /// Current engine state (from EngineStatus)
    pub state: EngineState,
    /// Currently executing node (derived from checkpoint labels)
    pub active_node: Option<NodeId>,
    /// Completed nodes (from checkpoint progression)
    pub completed_nodes: HashSet<NodeId>,
    /// Current progress
    pub current_event: u32,
    pub total_events: u32,
    /// Estimated time remaining (calculated from avg event time)
    pub estimated_remaining: Option<Duration>,
}
```

### Pattern 3: Checkpoint-Based Node Tracking
**What:** Encode node IDs in checkpoint labels for execution tracking
**When to use:** Mapping RunEngine progress back to visual nodes
**Example:**
```rust
// Checkpoint label format: "node_{node_id}_start" / "node_{node_id}_end"
PlanCommand::Checkpoint {
    label: format!("node_{:?}_start", node_id),
}
// ... node's commands ...
PlanCommand::Checkpoint {
    label: format!("node_{:?}_end", node_id),
}
```

### Anti-Patterns to Avoid
- **Direct hardware calls from GUI:** Always go through gRPC to daemon
- **Blocking GUI thread on execution:** Use async channels for status updates
- **Modifying graph during execution:** Structure is immutable; only parameters change
- **Custom pause implementation:** Use existing RunEngine checkpoint semantics

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Cycle detection | Manual visited set | petgraph::algo::toposort or validation in translate | Has error return on cycles |
| Pause/resume | Custom state machine | RunEngine.pause()/resume() | Already handles checkpoint rewinding |
| Progress percentage | Manual counting | num_points / current_event | RunEngine tracks seq_num |
| ETA calculation | Complex prediction | elapsed / progress * remaining | Simple linear extrapolation |
| Parameter editing | Custom gRPC | SetParameter in HardwareService | Already implemented |

**Key insight:** The RunEngine already implements Bluesky-style pause/resume with checkpoint rewinding. The GUI only needs to call pause()/resume() and track state via GetEngineStatus or Document streaming.

## Common Pitfalls

### Pitfall 1: Cycle Detection at Wrong Time
**What goes wrong:** Validation only checks pin types, not graph structure
**Why it happens:** Easy to connect output back to earlier node creating cycle
**How to avoid:** Add cycle detection to validation before "Run" button enables
**Warning signs:** Graph validates but translation fails or hangs

### Pitfall 2: Lost Node-to-Command Mapping
**What goes wrong:** Can't highlight which node is currently executing
**Why it happens:** Translation loses source node information
**How to avoid:** Include checkpoint commands with encoded node IDs
**Warning signs:** Progress bar moves but nodes don't highlight

### Pitfall 3: GUI Blocks on gRPC Calls
**What goes wrong:** UI freezes during pause/resume operations
**Why it happens:** Synchronous gRPC calls on GUI thread
**How to avoid:** Use async pattern like existing PlanRunnerPanel
**Warning signs:** Window becomes unresponsive during operations

### Pitfall 4: Race Condition in State Updates
**What goes wrong:** GUI shows stale state after pause/resume
**Why it happens:** Multiple sources of state (local + streamed)
**How to avoid:** Single source of truth from GetEngineStatus or Document stream
**Warning signs:** State flickers or shows incorrect values

### Pitfall 5: Parameter Modification During Wrong State
**What goes wrong:** User tries to edit parameters while running
**Why it happens:** GUI doesn't disable controls based on engine state
**How to avoid:** Disable parameter fields when state != Paused
**Warning signs:** Edits appear to work but don't affect execution

## Code Examples

Verified patterns from existing codebase:

### Node Iteration with Connections
```rust
// Source: egui-snarl docs.rs API
// Get all wires (connections) in the graph
for (out_pin, in_pin) in snarl.wires() {
    let from_node_id = out_pin.node;
    let to_node_id = in_pin.node;
    // out_pin.output = which output pin (0 for most nodes, 0/1 for Loop)
    // in_pin.input = which input pin (always 0 currently)
}

// Iterate nodes with IDs
for (node_id, node) in snarl.node_ids() {
    match node {
        ExperimentNode::Scan { actuator, start, stop, points } => { ... }
        ExperimentNode::Acquire { detector, duration_ms } => { ... }
        // etc.
    }
}
```

### Existing RunEngine Control Pattern
```rust
// Source: crates/daq-experiment/src/run_engine.rs
// Pause at next checkpoint
engine.pause().await?;
// Resume from paused state
engine.resume().await?;
// Get current progress
let events_so_far = engine.current_progress().await; // Returns Option<u32>
// Get current state
let state = engine.state().await; // Returns EngineState
```

### Async Action Pattern (from PlanRunnerPanel)
```rust
// Source: crates/daq-egui/src/panels/plan_runner.rs
// Spawn async gRPC call
let mut client = client.clone();
let tx = self.action_tx.clone();
runtime.spawn(async move {
    let result = client.queue_plan(...).await;
    let _ = tx.send(ActionResult::QueuePlan { ... }).await;
});

// Poll for results in UI
fn poll_async_results(&mut self, ctx: &egui::Context) {
    loop {
        match self.action_rx.try_recv() {
            Ok(result) => { /* handle result */ }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
}
```

### ExperimentNode to PlanCommand Translation
```rust
// Source: Design based on existing PlanCommand enum
fn translate_node(node: &ExperimentNode, node_id: NodeId) -> Vec<PlanCommand> {
    let mut commands = vec![
        PlanCommand::Checkpoint { label: format!("node_{:?}_start", node_id) },
    ];

    match node {
        ExperimentNode::Scan { actuator, start, stop, points } => {
            // Generate move + trigger + read + emit for each point
            let step = (stop - start) / (*points as f64 - 1.0);
            for i in 0..*points {
                let pos = start + step * i as f64;
                commands.push(PlanCommand::MoveTo {
                    device_id: actuator.clone(),
                    position: pos,
                });
                commands.push(PlanCommand::Checkpoint {
                    label: format!("scan_point_{}", i),
                });
                // Trigger and read would follow...
            }
        }
        ExperimentNode::Acquire { detector, duration_ms } => {
            commands.push(PlanCommand::Trigger {
                device_id: detector.clone(),
            });
            commands.push(PlanCommand::Read {
                device_id: detector.clone(),
            });
        }
        ExperimentNode::Move { device, position } => {
            commands.push(PlanCommand::MoveTo {
                device_id: device.clone(),
                position: *position,
            });
        }
        ExperimentNode::Wait { duration_ms } => {
            commands.push(PlanCommand::Wait {
                seconds: *duration_ms / 1000.0,
            });
        }
        ExperimentNode::Loop { iterations } => {
            // Loop body handled by traversal order
            // The loop node itself just marks boundaries
        }
    }

    commands.push(PlanCommand::Checkpoint { label: format!("node_{:?}_end", node_id) });
    commands
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| ScanService | RunEngineService | v0.6.0 | Pause/resume via checkpoints, Document streaming |
| Direct hardware calls | Plan-based execution | v0.6.0 | Reproducibility, structured metadata |

**Deprecated/outdated:**
- `ScanService`: Deprecated in v0.7.0, use `RunEngineService` instead

## gRPC Client Gaps

The DaqClient needs these methods added:

| Method | Proto Message | Status |
|--------|---------------|--------|
| `pause_engine()` | PauseEngineRequest | NOT IMPLEMENTED |
| `resume_engine()` | ResumeEngineRequest | NOT IMPLEMENTED |
| `get_engine_status()` | GetEngineStatusRequest | NOT IMPLEMENTED |

These are already defined in the proto and implemented server-side. Only client wrappers needed.

## Validation Extensions

Current validation checks:
- Pin type compatibility (Flow, LoopBody)
- Per-node field validation (empty device names, invalid parameters)

Needed for execution:
- **Cycle detection:** Prevent infinite loops in graph structure
- **Device availability:** Check devices exist before run
- **Connected graph:** Warn about disconnected subgraphs

## Progress Tracking Implementation

### Step Progress (Current Step N of M)
- **Source:** `EngineStatus.current_event_number` / `total_events_expected`
- **Update frequency:** Per EventDocument received
- **Display:** "Step 5 of 20" in status bar

### Percentage Calculation
```rust
let percent = (current_event as f32 / total_events as f32) * 100.0;
```

### ETA Calculation (Simple Linear)
```rust
let elapsed = now - start_time;
let avg_time_per_event = elapsed / current_event;
let remaining_events = total_events - current_event;
let eta = avg_time_per_event * remaining_events;
```

### Visual Node Highlighting
- Parse checkpoint labels: `"node_{id}_start"` sets node as "active"
- `"node_{id}_end"` moves node to "completed"
- Use egui painter to draw colored border around active node

## Open Questions

Things that couldn't be fully resolved:

1. **Loop Node Handling**
   - What we know: Loop has "next" (output 0) and "body" (output 1) pins
   - What's unclear: How to handle loop body in linear command sequence
   - Recommendation: Flatten loops by repeating body commands N times during translation

2. **Parallel Branches**
   - What we know: Graph structure allows branching
   - What's unclear: Whether parallel execution is needed
   - Recommendation: For v1, execute branches sequentially (simple topological order)

3. **Real-time Device Validation**
   - What we know: Device list available via ListDevices
   - What's unclear: How stale this can be during long experiments
   - Recommendation: Validate at "Run" click, assume stable during execution

## Sources

### Primary (HIGH confidence)
- [egui-snarl docs.rs](https://docs.rs/egui-snarl/latest/egui_snarl/struct.Snarl.html) - Snarl API: wires(), node_ids(), iteration
- `/Users/briansquires/code/rust-daq/crates/daq-experiment/src/run_engine.rs` - RunEngine implementation with pause/resume
- `/Users/briansquires/code/rust-daq/crates/daq-proto/proto/daq.proto` - RunEngineService definition (lines 1438-1733)
- `/Users/briansquires/code/rust-daq/crates/daq-egui/src/graph/nodes.rs` - ExperimentNode types

### Secondary (MEDIUM confidence)
- [Bluesky Interruptions docs](https://nsls-ii.github.io/bluesky/state-machine.html) - Checkpoint/pause/resume pattern
- [petgraph toposort](https://docs.rs/petgraph/latest/petgraph/algo/fn.toposort.html) - DAG topological sort with cycle detection

### Tertiary (LOW confidence)
- [indicatif progress bar](https://docs.rs/indicatif) - ETA calculation patterns (not directly applicable to GUI)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All components already exist in codebase
- Architecture: HIGH - Following existing Plan/RunEngine patterns
- Pitfalls: MEDIUM - Based on general GUI async patterns

**Research date:** 2026-01-22
**Valid until:** 60 days (internal architecture, stable)
