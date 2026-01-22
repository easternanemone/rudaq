# Technology Stack: Experiment Design Module

**Project:** rust-daq experiment designer
**Researched:** 2026-01-22
**Overall Confidence:** HIGH

## Executive Summary

Building a node-based experiment designer for rust-daq requires:
1. **Node Graph UI**: egui-snarl for visual editor (HIGH confidence)
2. **Plotting**: egui_plot for live data visualization (HIGH confidence)
3. **Scripting**: Existing Rhai 1.19 for code generation (HIGH confidence)
4. **Graph Data Structure**: daggy for DAG validation (MEDIUM confidence)
5. **Async Integration**: egui-async for execution control (MEDIUM confidence)
6. **Serialization**: serde for save/load (HIGH confidence)

---

## Core Technology Stack

### 1. Node Graph Editor

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **egui-snarl** | 0.9.0+ | Visual node editor UI | HIGH |

**Why egui-snarl:**
- Actively developed (last release January 2025, 499 GitHub stars)
- Built specifically for egui ecosystem (seamless integration)
- Type-safe, data-only nodes (perfect for Plan/parameter abstraction)
- Beautiful bezier wire rendering with customizable styling
- Serde serialization built-in (save/load experiment graphs)
- Five-zone node layout (header, input pins, body, output pins, footer)
- Context menus for node operations
- Multi-connection support

**Integration with rust-daq:**
```rust
// Node data wraps Plan definitions
#[derive(Serialize, Deserialize)]
struct ExperimentNode {
    plan_type: PlanType,
    parameters: HashMap<String, Value>,
}

// Viewer trait customizes UI rendering
impl Viewer<ExperimentNode> for ExperimentViewer {
    fn show_header(&mut self, node: &ExperimentNode, ui: &mut Ui) {
        ui.label(&node.plan_type.name());
    }
}
```

**Alternatives considered:**
- `egui_node_graph` (0.4.0, last update 2022) - REJECTED: Stale, no recent updates
- `egui-graph-edit` (active but less stars) - REJECTED: Fewer features, smaller community
- Custom implementation - REJECTED: Reinventing wheel, egui-snarl solves all requirements

**Installation:**
```toml
egui-snarl = { version = "0.9", features = ["serde"] }
```

---

### 2. Live Plotting

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **egui_plot** | 0.34+ | Real-time data visualization | HIGH |

**Why egui_plot:**
- Official egui plotting library (extracted to dedicated repo July 2024)
- Already in use in daq-egui (version 0.34)
- Immediate mode paradigm (rebuilt each frame, highly interactive)
- Native zoom, pan, hover support
- Handles streaming data (with downsampling for performance)

**Critical for experiment designer:**
- Plot live data during experiment execution
- Multiple plots per experiment (power, wavelength, position vs time)
- Supports line plots, scatter plots, bars, images

**Performance considerations:**
- For high-FPS streaming (>30 Hz), downsample data before plotting
- Use RingBuffer → snapshot → downsample → plot pipeline
- egui_plot renders efficiently with <10K points per frame

**Example integration:**
```rust
// Plot live power meter readings during experiment
Plot::new("power_plot")
    .allow_zoom(true)
    .view_aspect(2.0)
    .show(ui, |plot_ui| {
        let points: PlotPoints = power_readings
            .iter()
            .enumerate()
            .map(|(i, &p)| [i as f64, p])
            .collect();
        plot_ui.line(Line::new(points).color(Color32::RED));
    });
```

**Already integrated:** No new dependency needed (already in daq-egui).

**Alternatives considered:**
- plotters + egui-plotter - REJECTED: More complex, less native to egui
- Custom plotting - REJECTED: egui_plot solves all requirements

---

### 3. Script Engine

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **Rhai** | 1.19+ | Code generation target | HIGH |

**Why Rhai (already integrated):**
- Already in rust-daq (daq-scripting crate, version 1.19)
- Tight Rust integration with native types and functions
- Simple JavaScript+Rust-like syntax (accessible to scientists)
- Compile-to-AST for repeated execution
- Send + Sync with "sync" feature
- No panic guarantee (safe for untrusted scripts)

**Execution control capabilities:**
- `Engine::on_progress` for operation counting
- Force-terminate via progress callback returning `Some(Dynamic)`
- Time-based termination (check elapsed time, abort after timeout)
- Returns `EvalAltResult::ErrorTerminated` with context token

**Critical limitation:** No native pause/resume (state suspension)
- **Workaround**: Generate step-wise Rhai code from node graph, execute steps with GUI control between steps
- **Architecture**: Node graph → sequence of Rhai function calls → execute with pause points

**Hybrid visual/code approach:**
```rust
// Visual node graph generates Rhai script
let script = r#"
    // Generated from node graph
    move_stage(0.0);         // Node 1
    wait_settle();           // Node 2
    read_power_meter();      // Node 3
    move_stage(10.0);        // Node 4
"#;

// Execute with progress tracking
engine.on_progress(|ops| {
    if should_pause() {
        Some(Dynamic::from("PAUSED"))
    } else {
        None
    }
});
```

**No new dependency needed** (already in Cargo.toml).

---

### 4. Graph Data Structure

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **daggy** | 0.9.0+ | DAG validation & topological sort | MEDIUM |

**Why daggy:**
- Built on petgraph (proven, widely used)
- Enforces directed acyclic graph invariants (prevents cycles at construction time)
- `add_edge` returns `WouldCycle<E>` error if cycle detected (perfect for experiment designer)
- Serde serialization support with "serde-1" feature
- Walker trait for graph traversal without borrowing (allows mutation during traversal)
- StableDag variant (indices remain valid after node removal)

**Integration pattern:**
```rust
use daggy::{Dag, Walker};

// Experiment graph with Plan nodes
type ExperimentDag = Dag<ExperimentNode, DependencyEdge>;

// Validate no cycles when adding edge
match dag.add_edge(parent_idx, child_idx, edge_data) {
    Ok(edge_idx) => { /* Success */ },
    Err(WouldCycle(_)) => {
        ui.error("Cannot add edge: would create cycle");
    }
}

// Topological sort for execution order
let execution_order: Vec<NodeIndex> = dag
    .recursive_walk(root_idx, |g, n| g.children(n))
    .collect();
```

**Why not just petgraph:**
- petgraph allows cycles (must validate manually)
- daggy prevents cycles at construction (fail-fast, better UX)

**Confidence level: MEDIUM**
- Not as widely used as petgraph (smaller ecosystem)
- Sufficient for experiment DAG validation
- May need petgraph algorithms later (daggy wraps petgraph::Graph, can access)

**Installation:**
```toml
daggy = { version = "0.9", features = ["serde-1"] }
```

---

### 5. Async Integration

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **egui-async** | 0.1.2+ | Async task management in UI | MEDIUM |
| **tokio** | 1.36+ | Async runtime (already in stack) | HIGH |

**Why egui-async:**
- Simple, batteries-included solution for running async tasks across frames
- Supports native and wasm32
- `Bind<T, E>` struct manages task state (Idle, Pending, Finished)
- Bridges egui's immediate-mode loop with background async runtime
- Spawns futures onto runtime (tokio on native, wasm-bindgen-futures on web)
- Polls receiver each frame to check task completion

**Usage pattern:**
```rust
use egui_async::Bind;

struct ExperimentRunner {
    execution: Bind<ExperimentResult, DaqError>,
}

// Start experiment
if ui.button("Run").clicked() {
    self.execution.start(|| async {
        run_engine.execute_plan(plan).await
    });
}

// Poll status each frame
match self.execution.state() {
    State::Idle => { ui.label("Ready"); },
    State::Pending => { ui.spinner(); ui.label("Running..."); },
    State::Finished(Ok(result)) => { ui.label("Complete!"); },
    State::Finished(Err(e)) => { ui.colored_label(RED, format!("Error: {e}")); },
}
```

**Alternative approach (already in daq-egui):**
- Manual tokio runtime + channels (already used for gRPC streaming)
- Spawn task, send progress updates via `tokio::sync::mpsc`
- Poll channel in UI update loop

**Recommendation:** Try egui-async first (cleaner), fall back to manual channels if needed.

**Confidence level: MEDIUM**
- egui-async is young (released 2025), less battle-tested
- Manual channel approach is proven in daq-egui
- Both approaches viable, egui-async reduces boilerplate

**Installation:**
```toml
egui-async = "0.1"  # Optional, evaluate vs manual approach
```

---

### 6. Serialization

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **serde** | 1.0+ | Save/load experiments | HIGH |
| **serde_json** | 1.0+ | Human-readable format | HIGH |
| **toml** | 0.8+ | Alternative format (config-like) | HIGH |

**Why serde (already integrated):**
- Already throughout rust-daq codebase
- egui-snarl has built-in serde support (graphs serialize directly)
- daggy supports serde via "serde-1" feature
- Derive macros minimize boilerplate

**Serialization format options:**

| Format | Pros | Cons | Recommendation |
|--------|------|------|----------------|
| JSON | Human-readable, widely supported | Verbose | Use for experiments |
| TOML | Config-like, readable | Harder to nest deeply | Alternative |
| Bincode | Compact, fast | Not human-readable | Skip (not needed) |

**Example:**
```rust
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct Experiment {
    name: String,
    graph: Snarl<ExperimentNode>,
    metadata: ExperimentMetadata,
}

// Save
let json = serde_json::to_string_pretty(&experiment)?;
fs::write("experiment.json", json)?;

// Load
let json = fs::read_to_string("experiment.json")?;
let experiment: Experiment = serde_json::from_str(&json)?;
```

**No new dependency needed** (already in Cargo.toml).

---

### 7. Code Generation (Optional)

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **tera** | 1.20+ | Template-based Rhai generation | MEDIUM |

**Why tera (if needed):**
- Runtime template loading (can update templates without recompiling)
- Jinja2-like syntax (familiar to Python scientists)
- Rich feature set (macros, inheritance, filters)
- Good for generating structured Rhai scripts from node graphs

**When to use:**
- If Rhai generation becomes complex (nested loops, conditionals)
- If users want customizable script templates

**Alternatives:**
- **askama** (compile-time) - REJECTED: Less flexible, templates baked into binary
- **String interpolation** (format! macros) - ACCEPTABLE for simple cases
- **quote/syn** - REJECTED: Overkill for text generation

**Example template:**
```jinja
// Generated from experiment graph
{% for node in nodes %}
// {{ node.name }}
{{ node.function_call() }};
{% endfor %}
```

**Installation (if needed):**
```toml
tera = "1.20"
```

**Confidence level: MEDIUM** - May not be needed if string interpolation suffices.

---

### 8. Undo/Redo

| Technology | Version | Purpose | Confidence |
|------------|---------|---------|------------|
| **undo** | 7.0+ | Command pattern implementation | MEDIUM |

**Why undo crate:**
- Implements command pattern (all edits as objects)
- `Record` for basic linear undo-redo
- `History` for tree-based (non-linear) undo-redo with branches
- Edit merging (combine small edits into complex operations)
- Well-documented, mature crate

**Integration with node graph:**
```rust
use undo::{Record, Command};

struct AddNodeCommand {
    node: ExperimentNode,
    index: NodeIndex,
}

impl Command for AddNodeCommand {
    fn apply(&mut self, graph: &mut ExperimentDag) {
        self.index = graph.add_node(self.node.clone());
    }

    fn undo(&mut self, graph: &mut ExperimentDag) {
        graph.remove_node(self.index);
    }
}

// Usage
let mut history = Record::new();
history.apply(graph, AddNodeCommand { node, index: Default::default() });
history.undo(graph);  // Remove node
history.redo(graph);  // Re-add node
```

**Alternative: undo_2**
- Returns command sequences instead of performing undo/redo
- Easier implementation (no borrowing in commands)
- Tradeoff: More boilerplate in application code

**Recommendation:** Start with `undo` crate, switch to `undo_2` if borrowing issues arise.

**Confidence level: MEDIUM** - Standard pattern, well-supported crate.

**Installation:**
```toml
undo = "7.0"
```

---

## Supporting Libraries

### Already Integrated (No New Dependencies)

| Library | Version | Purpose |
|---------|---------|---------|
| egui | 0.33 | GUI framework |
| egui_plot | 0.34 | Plotting (already in daq-egui) |
| egui_dock | 0.18 | Docking layout |
| tokio | 1.36+ | Async runtime |
| rhai | 1.19 | Scripting engine |
| serde | 1.0+ | Serialization |
| serde_json | 1.0+ | JSON format |
| anyhow | 1.0+ | Error handling |
| tracing | 0.1+ | Logging |

### New Dependencies Required

| Library | Version | Features | Rationale |
|---------|---------|----------|-----------|
| egui-snarl | 0.9+ | serde | Node graph editor (core requirement) |
| daggy | 0.9+ | serde-1 | DAG validation (cycle prevention) |
| undo | 7.0+ | - | Undo/redo support (UX quality) |
| egui-async | 0.1+ | - | Optional (simplifies async tasks) |
| tera | 1.20+ | - | Optional (complex code generation) |

---

## Installation

### Minimum Viable Product

Add to `daq-egui/Cargo.toml`:

```toml
[dependencies]
# Node graph editor (REQUIRED)
egui-snarl = { version = "0.9", features = ["serde"] }

# DAG validation (REQUIRED)
daggy = { version = "0.9", features = ["serde-1"] }

# Undo/redo (HIGHLY RECOMMENDED)
undo = "7.0"
```

### Full Feature Set

```toml
[dependencies]
# Core
egui-snarl = { version = "0.9", features = ["serde"] }
daggy = { version = "0.9", features = ["serde-1"] }
undo = "7.0"

# Optional enhancements
egui-async = "0.1"  # Simplifies async task management
tera = "1.20"       # Template-based code generation
```

---

## Architecture Integration

### Data Flow

```
User Interaction (egui)
    ↓
Node Graph Editor (egui-snarl)
    ↓
DAG Validation (daggy) ← Prevents cycles
    ↓
Code Generation (String/Tera) → Rhai Script
    ↓
RunEngine Execution (daq-experiment)
    ↓
Progress Updates (egui-async / channels)
    ↓
Live Plotting (egui_plot)
```

### Component Boundaries

| Component | Crate | Responsibility |
|-----------|-------|---------------|
| Node Graph UI | daq-egui (new module) | Visual editing, rendering |
| DAG Logic | daq-experiment (extend) | Graph validation, execution order |
| Code Generation | daq-experiment (new) | Rhai script synthesis |
| Execution Control | daq-experiment (extend) | Pause/resume, progress tracking |
| Live Plotting | daq-egui (extend) | Real-time data visualization |

---

## Version Compatibility Matrix

Ensuring version alignment across egui ecosystem:

| Crate | Version | egui Version | Notes |
|-------|---------|--------------|-------|
| egui | 0.33.x | - | Core framework |
| egui_plot | 0.34.x | 0.33 compat | Independent versioning |
| egui-snarl | 0.9.x | 0.33 compat | Check releases for exact compat |
| egui_dock | 0.18.x | 0.33 compat | Already integrated |
| egui-async | 0.1.x | egui agnostic | Works with any egui version |

**Critical:** Verify egui-snarl supports egui 0.33 before integrating. If not, either:
1. Wait for egui-snarl update, or
2. Upgrade egui to 0.34+ (may require updating other deps)

---

## Technology Maturity Assessment

| Technology | Maturity | Community | Risk Level |
|------------|----------|-----------|------------|
| egui-snarl | Medium | Growing (499 stars) | LOW - Active dev, stable API |
| egui_plot | High | Official egui | NONE - Proven, widely used |
| Rhai | High | 3.7k stars, production use | NONE - Stable, battle-tested |
| daggy | Medium | Built on petgraph | LOW - Stable API since 2015 |
| egui-async | Low | New (2025) | MEDIUM - Young, less tested |
| undo | High | Mature (7.0) | LOW - Stable command pattern |
| tera | High | Popular template engine | LOW - Proven in web frameworks |

**Recommendation:** Prototype with core stack (egui-snarl, daggy, undo) first. Defer egui-async and tera until proven necessary.

---

## Performance Considerations

### Node Graph Rendering

- egui-snarl rebuilds UI each frame (immediate mode)
- Typical performance: 60 FPS with <100 nodes
- For large graphs (>200 nodes), consider viewport culling

### Live Plotting

- egui_plot efficient with <10K points per plot
- For high-frequency data (>30 Hz), downsample before plotting:
  ```rust
  let downsampled = ring_buffer
      .read_snapshot()
      .into_iter()
      .step_by(10)  // Plot every 10th point
      .collect();
  ```

### Async Task Overhead

- egui-async polls channel each frame (negligible overhead)
- Manual channel approach similar performance
- Tokio runtime already in daq-egui (no new overhead)

---

## Migration Path

### Phase 1: Node Graph Editor (Weeks 1-2)

```toml
egui-snarl = { version = "0.9", features = ["serde"] }
daggy = { version = "0.9", features = ["serde-1"] }
```

- Integrate egui-snarl into daq-egui
- Define ExperimentNode type wrapping Plan abstractions
- Implement Viewer trait for node rendering
- Add save/load with serde_json

### Phase 2: Execution Integration (Weeks 3-4)

```toml
undo = "7.0"
```

- Generate Rhai scripts from node graph
- Connect to existing RunEngine
- Add progress callbacks with Rhai's on_progress
- Implement undo/redo for graph editing

### Phase 3: Live Visualization (Weeks 5-6)

- Integrate egui_plot for live data (already in deps)
- Subscribe to RunEngine data streams
- Downsample for performance
- Add pause/resume UI controls

### Phase 4: Polish (Week 7+)

```toml
egui-async = "0.1"  # Optional
tera = "1.20"       # Optional
```

- Evaluate egui-async vs manual channels
- Add template-based generation if string building becomes complex
- Performance optimization
- User testing and refinement

---

## Risks & Mitigations

### Risk 1: egui-snarl Incompatibility with egui 0.33

**Likelihood:** LOW (egui-snarl actively maintained)
**Impact:** MEDIUM (delays integration)
**Mitigation:** Check GitHub releases before starting; if incompatible, upgrade egui to 0.34+ (may require updating other deps)

### Risk 2: Rhai Lacks True Pause/Resume

**Likelihood:** CERTAIN (confirmed in documentation)
**Impact:** MEDIUM (affects interactive execution)
**Mitigation:** Architect node graph to generate step-wise execution, insert pause points between steps, use on_progress for abort

### Risk 3: egui-async Immaturity

**Likelihood:** MEDIUM (released 2025, young crate)
**Impact:** LOW (fallback to manual channels)
**Mitigation:** Prototype with egui-async; if issues, revert to proven manual tokio::sync::mpsc pattern (already in daq-egui)

### Risk 4: Performance with Large Graphs

**Likelihood:** LOW (typical experiments <50 nodes)
**Impact:** MEDIUM (affects UX)
**Mitigation:** Implement viewport culling if needed; egui-snarl uses efficient rendering, unlikely to be bottleneck

---

## Alternatives Not Chosen

### Node Graph Editors

| Library | Why Not |
|---------|---------|
| egui_node_graph | Stale (last update 2022), no recent maintenance |
| egui-graph-edit | Less popular, fewer features than egui-snarl |
| Custom implementation | Reinventing wheel; egui-snarl solves all requirements |

### Plotting

| Library | Why Not |
|---------|---------|
| plotters + egui-plotter | More complex integration, less native to egui |
| rerun | Overkill for 2D plots; better for 3D/multimodal visualization |
| Custom OpenGL | Too low-level; egui_plot sufficient |

### Scripting

| Library | Why Not |
|---------|---------|
| Lua (mlua) | Rhai already integrated; switching adds no value |
| Python (PyO3) | Heavier runtime; Rhai sufficient for experiment scripts |
| Rust DSL | Requires compilation; Rhai provides runtime flexibility |

### Template Engines

| Library | Why Not |
|---------|---------|
| askama | Compile-time only; less flexible for runtime generation |
| handlebars | Less featureful than tera; no significant benefit |
| String interpolation | Acceptable for MVP; defer tera until proven necessary |

---

## Recommended Stack Summary

### Core (Required)

```toml
egui-snarl = { version = "0.9", features = ["serde"] }
daggy = { version = "0.9", features = ["serde-1"] }
undo = "7.0"
```

### Already Integrated (No Changes)

- egui 0.33
- egui_plot 0.34
- tokio 1.36
- rhai 1.19
- serde 1.0

### Optional Enhancements (Evaluate During Development)

```toml
egui-async = "0.1"  # If async task management needs simplification
tera = "1.20"       # If code generation becomes complex
```

---

## Sources

### Node Graph Libraries
- [egui-snarl GitHub](https://github.com/zakarumych/egui-snarl)
- [egui_node_graph crates.io](https://crates.io/crates/egui_node_graph)
- [egui-graph-edit crates.io](https://crates.io/crates/egui-graph-edit)

### Plotting
- [egui_plot GitHub](https://github.com/emilk/egui_plot)
- [egui_plot crates.io](https://crates.io/crates/egui_plot)
- [egui plotting discussion](https://github.com/emilk/egui/issues/1485)

### Scripting
- [Rhai official site](https://rhai.rs/)
- [Rhai GitHub](https://github.com/rhaiscript/rhai)
- [Rhai progress tracking](https://rhai.rs/book/safety/progress.html)

### Graph Data Structures
- [daggy crates.io](https://crates.io/crates/daggy)
- [daggy documentation](https://docs.rs/daggy)
- [petgraph documentation](https://docs.rs/petgraph/latest/petgraph/)
- [Graphs in Rust: Petgraph Introduction](https://depth-first.com/articles/2020/02/03/graphs-in-rust-an-introduction-to-petgraph/)

### Async Integration
- [egui-async crates.io](https://crates.io/crates/egui-async)
- [egui tokio integration discussion](https://github.com/emilk/egui/discussions/521)
- [Using egui with async functions](https://github.com/emilk/egui/discussions/2010)

### Serialization
- [petgraph serialization](https://docs.rs/petgraph/latest/src/petgraph/graph_impl/serialization.rs.html)
- [petgraph serde PR](https://github.com/petgraph/petgraph/pull/166)

### Template Engines
- [Tera vs Askama vs Handlebars comparison](https://blog.logrocket.com/top-3-templating-libraries-for-rust/)
- [Rust template engine tradeoffs](https://leapcell.io/blog/rust-template-engines-compile-time-vs-run-time-vs-macro-tradeoffs)

### Code Generation
- [quote crate documentation](https://docs.rs/quote/latest/quote/macro.quote.html)
- [proc-macro2 guide](https://generalistprogrammer.com/tutorials/proc-macro2-rust-crate-guide)
- [Rust procedural macros best practices](https://blog.logrocket.com/procedural-macros-in-rust/)

### Undo/Redo
- [undo crate GitHub](https://github.com/evenorog/undo)
- [undo crate documentation](https://docs.rs/undo)
- [Command pattern in Rust](https://refactoring.guru/design-patterns/command/rust/example)

### Async State Machines
- [Rust async functions as state machines](https://jeffmcbride.net/blog/2025/05/16/rust-async-functions-as-state-machines/)
- [async-hsm documentation](https://docs.rs/async-hsm)
- [statig hierarchical state machines](https://github.com/mdeloof/statig)
