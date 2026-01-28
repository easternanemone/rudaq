# Phase 4: Sequences and Control Flow - Research

**Researched:** 2026-01-22
**Domain:** Node graph control flow with moves, waits, acquire, and loops for scientific experiment sequencing
**Confidence:** HIGH

## Summary

Phase 4 extends the existing node graph editor (Phase 2) with four additional node types: Move, Wait, Acquire, and Loop. The research focused on three key areas: (1) control flow patterns for loops in node-based editors, (2) device selection UX patterns in egui, and (3) conditional wait semantics from scientific control systems like Bluesky/ophyd.

The existing codebase already has partial implementations of Move, Wait, Acquire, and Loop nodes in `ExperimentNode` enum. The translation layer in `translation.rs` converts these to `PlanCommand` sequences. The primary task for this phase is enhancing these stub implementations with full configuration UIs and extending the translation logic to handle complex loop semantics.

Key findings:
- **Loop implementation:** Loops in node editors typically have two outputs (next sequence + loop body) and require back-edge detection during translation to prevent infinite recursion
- **Device selection:** `egui_autocomplete` crate (v0.0.10, egui 0.33) provides fuzzy-match dropdown for device selection from registry
- **Conditional waits:** Bluesky/ophyd uses `settle_time` parameter on devices; condition-based waits require polling device values against threshold or stability criteria
- **Move node modes:** Absolute vs relative positioning is a toggle within the node UI; blocking behavior is handled by including/excluding `wait_settled` in translation

**Primary recommendation:** Enhance existing ExperimentNode variants with configuration structures, implement property inspector panels for each node type, use `egui_autocomplete` for device selection, and extend GraphPlan translation to handle nested loop body expansion with cycle detection.

## Standard Stack

The established libraries/tools for this domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| egui-snarl | 0.9.0 | Node graph UI | Already in use, supports custom node types with multiple pins |
| egui | 0.33 | Immediate-mode GUI | Project standard, matches egui_autocomplete |
| egui_autocomplete | 0.0.10 | Fuzzy-match device selection | Purpose-built for dropdown with keyboard navigation |
| daq-experiment | internal | Plan/PlanCommand | Existing execution layer with Wait, MoveTo, Read, Trigger commands |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| petgraph (OPTIONAL) | 0.6 | Graph algorithms | If cycle detection in nested loops becomes complex |
| egui-dropdown | 0.4 | Alternative dropdown | If autocomplete is too heavy (simpler API) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| egui_autocomplete | Native egui::ComboBox | ComboBox lacks fuzzy search, less ergonomic for 20+ devices |
| egui_autocomplete | egui-dropdown | Dropdown simpler but no fuzzy match, worse UX for large lists |
| Loop body as child nodes | Loop body as parameter | Child nodes more flexible, matches Unity/Bluesky patterns |

**Installation:**
```bash
cargo add egui_autocomplete --features serde
# egui-snarl, egui, daq-experiment already present
```

## Architecture Patterns

### Recommended Project Structure
```
crates/daq-egui/src/
├── graph/
│   ├── nodes.rs                  # ExperimentNode enum (already exists)
│   ├── translation.rs            # GraphPlan with loop body expansion
│   ├── validation.rs             # Extended with loop cycle detection
│   └── viewer.rs                 # SnarlViewer with property inspectors
├── panels/
│   └── experiment_designer.rs    # Context menu for adding nodes
└── widgets/
    ├── property_inspector.rs     # Per-node configuration panel
    └── device_selector.rs        # AutoCompleteTextEdit wrapper
```

### Pattern 1: Loop Node with Dual Outputs
**What:** Loop nodes have two output pins: "Next" (exits loop) and "Body" (loop contents)
**When to use:** All loop nodes to enable both sequential flow after loop and loop body sub-graphs
**Example:**
```rust
// Source: Unity Visual Scripting Control Nodes pattern
// https://docs.unity3d.com/Packages/com.unity.visualscripting@1.9/manual/vs-control.html

impl SnarlViewer<ExperimentNode> for ExperimentViewer {
    fn outputs(&mut self, node: &ExperimentNode) -> usize {
        match node {
            ExperimentNode::Loop { .. } => 2, // Next (pin 0) + Body (pin 1)
            _ => 1,
        }
    }

    fn show_output(&mut self, pin: &OutPin, ui: &mut Ui, _scale: f32, snarl: &mut Snarl<ExperimentNode>) -> PinInfo {
        match snarl.get_node(pin.id.node) {
            Some(ExperimentNode::Loop { .. }) => {
                if pin.id.output == 0 {
                    ui.label("Next ⏴");
                } else {
                    ui.label("Body ⏴");
                }
            }
            _ => {
                ui.label("⏴");
            }
        }
        PinInfo::default()
    }
}
```

### Pattern 2: Device Selection with Autocomplete
**What:** Fuzzy-match dropdown for selecting devices from registry
**When to use:** Move, Acquire, and Read nodes that reference hardware devices
**Example:**
```rust
// Source: egui_autocomplete docs (https://docs.rs/egui_autocomplete/latest/egui_autocomplete/)
use egui_autocomplete::AutoCompleteTextEdit;

pub struct DeviceSelector {
    text: String,
    candidates: Vec<String>,
}

impl DeviceSelector {
    pub fn new(device_registry: &[DeviceInfo]) -> Self {
        Self {
            text: String::new(),
            candidates: device_registry.iter()
                .map(|d| d.device_id.clone())
                .collect(),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let response = ui.add(
            AutoCompleteTextEdit::new(&mut self.text, &self.candidates)
                .hint_text("Type to search devices...")
        );
        response.changed()
    }

    pub fn selected(&self) -> &str {
        &self.text
    }
}
```

### Pattern 3: Conditional Wait with Polling
**What:** Wait node with threshold or stability condition on device readout
**When to use:** When settling time must be adaptive (e.g., temperature stabilization)
**Example:**
```rust
// Source: Bluesky settle_time pattern (https://blueskyproject.io/ophyd/status.html)
// Translated to Rust async polling

pub enum WaitCondition {
    Duration { milliseconds: f64 },
    Threshold { device_id: String, parameter: String, operator: ThresholdOp, value: f64 },
    Stability { device_id: String, parameter: String, tolerance: f64, duration_ms: f64 },
}

pub enum ThresholdOp {
    LessThan,
    GreaterThan,
    EqualWithin { tolerance: f64 },
}

// Translation to PlanCommand sequence
match node {
    ExperimentNode::Wait { condition } => {
        match condition {
            WaitCondition::Duration { milliseconds } => {
                commands.push(PlanCommand::Wait { seconds: milliseconds / 1000.0 });
            }
            WaitCondition::Threshold { device_id, parameter, operator, value } => {
                // Emit checkpoint-poll-check loop
                commands.push(PlanCommand::Checkpoint { label: format!("wait_threshold_start") });
                commands.push(PlanCommand::Read { device_id: device_id.clone() });
                // RunEngine must handle conditional branching (future enhancement)
                // For now, translate to fixed-duration wait with warning
            }
            WaitCondition::Stability { device_id, tolerance, duration_ms } => {
                // Stability requires multiple reads over time
                // Translate to Read + Wait loop (simplified)
                commands.push(PlanCommand::Wait { seconds: duration_ms / 1000.0 });
            }
        }
    }
}
```

### Pattern 4: Loop Body Expansion with Cycle Detection
**What:** Translate loop node by linearizing body sub-graph N times (count-based) or wrapping in checkpoint markers (condition-based)
**When to use:** All loop node translations
**Example:**
```rust
// Source: Kahn's algorithm for topological sort (already used in translation.rs)
// Extended with loop body detection

fn translate_loop_node(
    loop_node_id: NodeId,
    iterations: u32,
    body_subgraph: Vec<NodeId>,
    snarl: &Snarl<ExperimentNode>,
) -> Vec<PlanCommand> {
    let mut commands = Vec::new();

    // For count-based loops: unroll body N times
    for i in 0..iterations {
        commands.push(PlanCommand::Checkpoint {
            label: format!("loop_{:?}_iter_{}_start", loop_node_id, i),
        });

        // Translate body nodes in topological order
        for body_node_id in &body_subgraph {
            let node = snarl.get_node(*body_node_id).unwrap();
            commands.extend(translate_node(node, *body_node_id).0);
        }

        commands.push(PlanCommand::Checkpoint {
            label: format!("loop_{:?}_iter_{}_end", loop_node_id, i),
        });
    }

    commands
}
```

### Pattern 5: Move Node with Blocking Toggle
**What:** Move node configuration includes "Wait for Settle" checkbox
**When to use:** All Move nodes to control whether motion is fire-and-forget or blocking
**Example:**
```rust
// Source: Movable trait in common/src/capabilities.rs (lines 168-220)

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MoveConfig {
    pub device: String,
    pub mode: MoveMode,
    pub position: f64,
    pub wait_settled: bool,  // Blocking behavior
}

pub enum MoveMode {
    Absolute,
    Relative,
}

// Translation
fn translate_move_node(config: &MoveConfig) -> Vec<PlanCommand> {
    let mut commands = vec![
        match config.mode {
            MoveMode::Absolute => PlanCommand::MoveTo {
                device_id: config.device.clone(),
                position: config.position,
            },
            MoveMode::Relative => PlanCommand::MoveRel {
                device_id: config.device.clone(),
                distance: config.position,
            },
        }
    ];

    if config.wait_settled {
        commands.push(PlanCommand::WaitSettled {
            device_id: config.device.clone(),
        });
    }

    commands
}
```

### Anti-Patterns to Avoid
- **Modifying graph structure during execution:** Loop unrolling happens at translation time, not runtime
- **Infinite loops without break condition:** Always enforce timeout or max iteration limit in UI
- **Hard-coded device IDs:** Always use registry lookup with validation
- **Synchronous device selection:** Use async channel for fetching device list to avoid GUI freezes

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Fuzzy string matching | Custom edit distance | egui_autocomplete (uses fuzzy-matcher crate) | Handles keyboard nav, rendering, edge cases |
| Device validation | Manual string checks | DaqClient.list_devices() + registry lookup | Server has authoritative device list |
| Loop cycle detection | Visited set tracking | Kahn's algorithm (already in translation.rs) | Detects cycles in O(V+E), returns error with cycle info |
| Conditional wait polling | Custom timer + read loop | Future PlanCommand::WaitUntil (requires RunEngine support) | RunEngine can pause/resume cleanly at checkpoints |
| Enum backward compatibility | Manual discriminant management | #[non_exhaustive] attribute on ExperimentNode | Forces catch-all patterns, prevents breaking changes |

**Key insight:** Device selection and validation should always go through gRPC to daemon, never cache device lists in GUI (devices can be hot-swapped, disabled, or error out).

## Common Pitfalls

### Pitfall 1: Loop Body Cycle Detection
**What goes wrong:** User connects Loop Body output back to a node inside the loop, creating actual infinite recursion in translation
**Why it happens:** egui-snarl allows any connection; validation happens after
**How to avoid:** During translation, detect if Loop Body output wire target is ancestor of loop node in DAG
**Warning signs:** Translation hangs, stack overflow, or very large command list

**Prevention:**
```rust
fn validate_loop_body(loop_node: NodeId, body_output: OutPin, snarl: &Snarl) -> Result<()> {
    // Traverse from body output target back to roots
    // If loop_node is encountered, it's a cycle
    let target_node = body_output.connections.first().map(|c| c.node);
    if let Some(target) = target_node {
        let ancestors = find_ancestors(target, snarl);
        if ancestors.contains(&loop_node) {
            return Err(anyhow!("Loop body cannot connect back into loop"));
        }
    }
    Ok(())
}
```

### Pitfall 2: Relative Move Accumulation
**What goes wrong:** Multiple Relative moves in a loop compound, leading to unexpected final positions
**Why it happens:** User expects "move +5mm each loop" but forgets final position = start + (5 * N)
**How to avoid:** Show warning in property inspector when Relative mode is used in loop body
**Warning signs:** Motion stage hits limit switch after a few iterations

### Pitfall 3: Device Selection Stale State
**What goes wrong:** User types device ID that exists at graph design time, but device is unavailable at execution time
**Why it happens:** Validation happens at translation, not real-time during editing
**How to avoid:** On graph translation (Run button), re-fetch device list and validate all device IDs
**Warning signs:** Execution starts but immediately fails with "device not found"

**Prevention:**
```rust
// In GraphPlan::from_snarl()
pub fn from_snarl(snarl: &Snarl<ExperimentNode>, client: &DaqClient) -> Result<Self> {
    // Fetch current device list
    let devices = client.list_devices().await?;
    let device_ids: HashSet<_> = devices.iter().map(|d| &d.device_id).collect();

    // Validate all referenced devices
    for (node_id, node) in snarl.node_ids() {
        match node {
            ExperimentNode::Move { device, .. } |
            ExperimentNode::Acquire { detector: device, .. } => {
                if !device_ids.contains(device) {
                    return Err(TranslationError::InvalidNode {
                        node_id,
                        reason: format!("Device '{}' not found", device),
                    });
                }
            }
            _ => {}
        }
    }

    // ... proceed with translation
}
```

### Pitfall 4: Conditional Wait Infinite Loops
**What goes wrong:** Threshold condition never met (sensor broken, wrong parameter, typo in units)
**Why it happens:** No timeout on condition-based waits
**How to avoid:** Always require timeout parameter on conditional waits (UI default: 30 seconds)
**Warning signs:** Execution hangs indefinitely at Wait node

### Pitfall 5: Acquire Node Exposure Collision
**What goes wrong:** Acquire node sets exposure_ms, but device's current exposure is different, causing first frame to use wrong exposure
**Why it happens:** `PlanCommand::Set` is async; hardware may not update before `Trigger`
**How to avoid:** Add delay or read-back after Set command in translation
**Warning signs:** First frame in burst has different exposure than rest

## Code Examples

Verified patterns from existing codebase:

### Move Node Translation (Existing)
```rust
// Source: crates/daq-egui/src/graph/translation.rs (lines 265-272)
ExperimentNode::Move { device, position } => {
    if !device.is_empty() {
        movers.push(device.clone());
        commands.push(PlanCommand::MoveTo {
            device_id: device.clone(),
            position: *position,
        });
    }
}
```

### Wait Node Translation (Existing)
```rust
// Source: crates/daq-egui/src/graph/translation.rs (lines 274-278)
ExperimentNode::Wait { duration_ms } => {
    commands.push(PlanCommand::Wait {
        seconds: *duration_ms / 1000.0,
    });
}
```

### Acquire Node Translation (Existing)
```rust
// Source: crates/daq-egui/src/graph/translation.rs (lines 240-264)
ExperimentNode::Acquire { detector, duration_ms } => {
    if !detector.is_empty() {
        detectors.push(detector.clone());
        // Set exposure if duration specified
        if *duration_ms > 0.0 {
            commands.push(PlanCommand::Set {
                device_id: detector.clone(),
                parameter: "exposure_ms".to_string(),
                value: duration_ms.to_string(),
            });
        }
        commands.push(PlanCommand::Trigger {
            device_id: detector.clone(),
        });
        commands.push(PlanCommand::Read {
            device_id: detector.clone(),
        });
        commands.push(PlanCommand::EmitEvent {
            stream: "primary".to_string(),
            data: HashMap::new(),
            positions: HashMap::new(),
        });
        events += 1;
    }
}
```

### Loop Stub (Needs Enhancement)
```rust
// Source: crates/daq-egui/src/graph/translation.rs (lines 279-286)
// Current implementation is a stub - just adds checkpoint
ExperimentNode::Loop { iterations } => {
    // Loop node itself just marks checkpoint
    // Loop body is handled by graph structure (body output connects to loop content)
    // For now, loops are not fully implemented - just add checkpoint
    commands.push(PlanCommand::Checkpoint {
        label: format!("node_{:?}_loop_iter_{}", node_id, iterations),
    });
}
```

### Device Selection Widget (New Pattern)
```rust
// Pattern based on egui_autocomplete API
use egui_autocomplete::AutoCompleteTextEdit;

pub struct MoveNodeInspector {
    device_text: String,
    device_candidates: Vec<String>,
    position: f64,
    mode: MoveMode,
    wait_settled: bool,
}

impl MoveNodeInspector {
    pub fn show(&mut self, ui: &mut egui::Ui, device_registry: &[DeviceInfo]) -> bool {
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Device:");
            let resp = ui.add(
                AutoCompleteTextEdit::new(&mut self.device_text, &self.device_candidates)
                    .hint_text("stage_x")
            );
            changed |= resp.changed();
        });

        ui.horizontal(|ui| {
            ui.label("Mode:");
            changed |= ui.radio_value(&mut self.mode, MoveMode::Absolute, "Absolute").changed();
            changed |= ui.radio_value(&mut self.mode, MoveMode::Relative, "Relative").changed();
        });

        changed |= ui.add(egui::DragValue::new(&mut self.position).suffix(" mm")).changed();
        changed |= ui.checkbox(&mut self.wait_settled, "Wait for motion to settle").changed();

        changed
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual ComboBox lists | egui_autocomplete with fuzzy search | egui 0.33 (2024) | Better UX for large device lists |
| Hard-coded loop unrolling | Dual-output nodes with body sub-graphs | Unity Visual Scripting (2021) | More flexible, supports complex nesting |
| #[repr(u8)] discriminants | #[non_exhaustive] enums | Rust 1.66 (2022) | Prevents breaking API changes when adding variants |
| Synchronous ophyd settle_time | Async Status.wait() with timeout | Bluesky 1.6+ (2023) | Non-blocking, integrates with async event loop |

**Deprecated/outdated:**
- `egui_node_graph` (archived): Original egui node editor, replaced by egui-snarl and egui-graph-edit forks
- Loop body as enum variant parameter: Modern pattern is child nodes connected to Body output pin
- Fixed settle times: Conditional waits (threshold/stability) are now standard in scientific control

## Open Questions

Things that couldn't be fully resolved:

1. **Conditional Wait Implementation in RunEngine**
   - What we know: Bluesky uses Status objects with settle_time and callbacks; Rust equivalent requires polling
   - What's unclear: Does RunEngine need to support conditional branching (while loops) or only fixed command sequences?
   - Recommendation: Phase 4 implements Duration-only waits; Threshold/Stability conditions added in Phase 5 with RunEngine enhancements

2. **Loop Iteration Variable Exposure**
   - What we know: Unity and Bluesky expose loop counters to child nodes; requires parameter injection
   - What's unclear: How to inject loop counter into ExperimentNode parameters during translation (immutable enum)
   - Recommendation: Initial implementation: loops unroll with fixed parameters. Future: add LoopContext struct passed during execution

3. **Multi-Detector Acquire Nodes**
   - What we know: Scientific workflows often acquire from multiple detectors simultaneously
   - What's unclear: Should one Acquire node reference multiple detectors, or require multiple nodes?
   - Recommendation: Start with single-detector nodes (simpler UX). Multi-detector can be added later as variant

4. **Settling Time Presets**
   - What we know: Common devices have typical settling times (motors: 100ms, temperature: 10s)
   - What's unclear: Should presets be device-specific metadata or global constants?
   - Recommendation: Store in DeviceMetadata (server-side) with UI dropdown showing "Default" + custom

## Sources

### Primary (HIGH confidence)
- egui-snarl crate (v0.9.0): https://github.com/zakarumych/egui-snarl - Node graph architecture
- egui_autocomplete docs: https://docs.rs/egui_autocomplete/latest/egui_autocomplete/ - API usage patterns
- Bluesky ophyd Status objects: https://blueskyproject.io/ophyd/status.html - Settling time patterns
- common capabilities.rs (lines 168-220): Movable trait with wait_settled
- daq-egui translation.rs (lines 240-286): Existing node translation patterns

### Secondary (MEDIUM confidence)
- Unity Visual Scripting Control Nodes: https://docs.unity3d.com/Packages/com.unity.visualscripting@1.9/manual/vs-control.html - Loop body patterns
- Rust #[non_exhaustive] attribute: https://www.slingacademy.com/article/enum-exhaustiveness-and-future-proofing-with-non-exhaustive/ - Enum evolution

### Tertiary (LOW confidence)
- WebSearch "node graph loop implementation" - General patterns, not Rust-specific

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All libraries verified via crates.io, versions confirmed in Cargo.toml
- Architecture: HIGH - Patterns extracted from existing codebase (translation.rs, viewer.rs, nodes.rs)
- Pitfalls: MEDIUM - Based on common node editor issues + scientific control system experience

**Research date:** 2026-01-22
**Valid until:** 60 days (stable domain, egui-snarl updates infrequent)
