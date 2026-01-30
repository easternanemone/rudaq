# Phase 2: Node Graph Editor Core - Research

**Researched:** 2026-01-22
**Domain:** Node graph editors in Rust with egui immediate-mode GUI
**Confidence:** MEDIUM

## Summary

Research focused on identifying the standard stack for building node graph editors in Rust with egui, specifically for scientific experiment workflow design. The egui ecosystem offers several mature node graph libraries, with **egui-snarl** and **egui-graph-edit** emerging as the primary candidates for 2026. The original egui_node_graph library is now archived, but active forks exist.

Key findings:
- **egui-snarl** (v0.9.0, actively maintained) offers typed data-only nodes, flexible UI customization via traits, built-in serialization, and beautiful wire rendering
- **egui-graph-edit** provides semantic-agnostic graph presentation with public API for maximum flexibility
- **undo crate** (standard Rust command pattern library) provides Record and History structures for implementing undo/redo
- Immediate-mode GUI + node editors present specific challenges: coordinate transformations for pan/zoom, performance with large graphs, click-through issues with overlapping nodes

**Primary recommendation:** Use egui-snarl for core node graph UI + undo crate for command pattern, with custom SnarlViewer implementation for experiment-specific nodes.

## Standard Stack

The established libraries/tools for node graph editors in egui:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| egui-snarl | 0.9.0 (2026-01) | Node graph UI with trait-based customization | Actively maintained, typed nodes, serde support, beautiful wires |
| undo | 3.x | Command pattern for undo/redo | Standard Rust undo library, Record + History structures |
| egui | 0.33 | Immediate-mode GUI framework | Already used in ui |
| serde/serde_json | 1.0 | Graph serialization | De facto Rust serialization standard |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| egui-graph-edit | Latest | Alternative node graph library | If need more public API control than egui-snarl |
| egui_plot | 0.34 | Plotting within nodes | Node property visualizations (already in project) |
| petgraph | Latest | Graph algorithms | If need pathfinding, cycle detection, topological sort |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| egui-snarl | egui_node_graph2 (fork) | Fork has less frequent updates, but similar API |
| egui-snarl | egui-graph-edit | More public API exposure, less opinionated, but newer |
| undo crate | Custom command stack | Reinventing wheel, undo crate handles merging, branches, checkpoints |

**Installation:**
```bash
cargo add egui-snarl --features serde
cargo add undo
# egui, serde already present in ui
```

## Architecture Patterns

### Recommended Project Structure
```
crates/ui/src/
├── panels/
│   └── experiment_designer.rs    # Main panel with egui-snarl integration
├── graph/
│   ├── mod.rs                    # Graph module exports
│   ├── nodes.rs                  # Node data types (enum NodeData)
│   ├── viewer.rs                 # SnarlViewer implementation
│   ├── validation.rs             # Connection validation logic
│   ├── commands.rs               # Edit implementations for undo/redo
│   ├── serialization.rs          # JSON save/load
│   └── subgraph.rs               # Grouped node handling
└── widgets/
    └── node_palette.rs           # Drag-and-drop node library
```

### Pattern 1: Trait-Based Viewer (egui-snarl)
**What:** Separate graph data from presentation using SnarlViewer trait
**When to use:** Always — decouples node data from UI rendering
**Example:**
```rust
// Source: https://github.com/zakarumych/egui-snarl (README)
use egui_snarl::{Snarl, SnarlViewer, ui::SnarlStyle};

// Node data (enum or struct)
#[derive(Serialize, Deserialize)]
enum ExperimentNode {
    Scan { actuator: String, start: f64, stop: f64, points: u32 },
    Acquire { detector: String, duration: f64 },
    Move { device: String, position: f64 },
    Wait { duration: f64 },
    Loop { iterations: u32 },
}

// Viewer defines appearance
struct ExperimentViewer;

impl SnarlViewer<ExperimentNode> for ExperimentViewer {
    fn title(&self, node: &ExperimentNode) -> String {
        match node {
            ExperimentNode::Scan { .. } => "Scan".to_string(),
            ExperimentNode::Acquire { .. } => "Acquire".to_string(),
            // ... etc
        }
    }

    fn inputs(&self, node: &ExperimentNode) -> usize {
        match node {
            ExperimentNode::Scan { .. } => 0, // Entry point
            ExperimentNode::Loop { .. } => 1, // Body input
            _ => 1, // Sequence input
        }
    }

    fn outputs(&self, node: &ExperimentNode) -> usize {
        match node {
            ExperimentNode::Loop { .. } => 2, // Next + loop body
            _ => 1, // Sequence output
        }
    }

    fn show_input(&mut self, pin: &InPin, ui: &mut Ui, _scale: f32, snarl: &mut Snarl<ExperimentNode>) -> PinInfo {
        ui.label("⏵"); // Flow control input
        PinInfo::default()
    }

    fn show_output(&mut self, pin: &OutPin, ui: &mut Ui, _scale: f32, snarl: &mut Snarl<ExperimentNode>) -> PinInfo {
        ui.label("⏴"); // Flow control output
        PinInfo::default()
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ExperimentNode>) {
        // Validate connection type compatibility here
        // For sequential flow, always allow
        snarl.connect(from.id.node, from.id.output, to.id.node, to.id.input);
    }
}

// Usage in panel
let mut snarl = Snarl::new();
snarl.insert_node(pos, ExperimentNode::Scan { /* ... */ });

// Rendering
snarl.show(&mut ExperimentViewer, &SnarlStyle::default(), id, ui);
```

### Pattern 2: Command Pattern Undo/Redo
**What:** Wrap graph modifications in Edit implementations
**When to use:** All user-initiated graph changes (add node, delete, connect, disconnect, modify parameters)
**Example:**
```rust
// Source: https://docs.rs/undo (Edit trait documentation)
use undo::{Record, Edit, Merged};

struct AddNodeCommand {
    node_id: NodeId,
    node_data: ExperimentNode,
    position: egui::Pos2,
}

impl Edit for AddNodeCommand {
    type Target = Snarl<ExperimentNode>;
    type Output = ();

    fn edit(&mut self, snarl: &mut Self::Target) -> Self::Output {
        snarl.insert_node(self.position, self.node_data.clone());
    }

    fn undo(&mut self, snarl: &mut Self::Target) -> Self::Output {
        snarl.remove_node(self.node_id);
    }

    fn merge(&mut self, other: Self) -> Merged<Self> {
        Merged::No // Don't merge add operations
    }
}

struct ModifyNodeCommand {
    node_id: NodeId,
    old_data: ExperimentNode,
    new_data: ExperimentNode,
}

impl Edit for ModifyNodeCommand {
    type Target = Snarl<ExperimentNode>;
    type Output = ();

    fn edit(&mut self, snarl: &mut Self::Target) -> Self::Output {
        *snarl.get_node_mut(self.node_id).unwrap() = self.new_data.clone();
    }

    fn undo(&mut self, snarl: &mut Self::Target) -> Self::Output {
        *snarl.get_node_mut(self.node_id).unwrap() = self.old_data.clone();
    }

    fn merge(&mut self, other: Self) -> Merged<Self> {
        if self.node_id == other.node_id {
            // Merge consecutive modifications to same node
            self.new_data = other.new_data;
            Merged::Yes
        } else {
            Merged::No
        }
    }
}

// Usage
struct GraphEditor {
    snarl: Snarl<ExperimentNode>,
    history: Record<Snarl<ExperimentNode>>,
}

impl GraphEditor {
    fn add_node(&mut self, data: ExperimentNode, pos: Pos2) {
        let cmd = AddNodeCommand {
            node_id: next_id(),
            node_data: data,
            position: pos,
        };
        self.history.edit(&mut self.snarl, cmd);
    }

    fn undo(&mut self) {
        if let Some(_) = self.history.undo(&mut self.snarl) {
            // Undo successful
        }
    }

    fn redo(&mut self) {
        if let Some(_) = self.history.redo(&mut self.snarl) {
            // Redo successful
        }
    }
}
```

### Pattern 3: Connection Validation
**What:** Type-check connections before allowing in SnarlViewer::connect
**When to use:** Prevent invalid connections (incompatible data types, cycles where not allowed)
**Example:**
```rust
// In SnarlViewer implementation
fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ExperimentNode>) {
    let from_node = snarl.get_node(from.id.node).unwrap();
    let to_node = snarl.get_node(to.id.node).unwrap();

    // Rule 1: Sequential flow nodes always compatible
    if is_flow_pin(from) && is_flow_pin(to) {
        snarl.connect(from.id.node, from.id.output, to.id.node, to.id.input);
        return;
    }

    // Rule 2: Data pins must match types
    let from_type = pin_data_type(from_node, from.id.output);
    let to_type = pin_data_type(to_node, to.id.input);
    if from_type == to_type {
        snarl.connect(from.id.node, from.id.output, to.id.node, to.id.input);
    } else {
        // Show error toast or highlight in red
        self.validation_error = Some(format!(
            "Type mismatch: {} → {}",
            from_type, to_type
        ));
    }
}

fn has_error(&self, node_id: NodeId, snarl: &Snarl<ExperimentNode>) -> Option<String> {
    // Return error message to show red border
    // Called by egui-snarl during rendering
    self.validation_error.clone()
}
```

### Pattern 4: Subgraph Grouping
**What:** Group nodes into collapsible subgraphs (e.g., "Setup" group, "Scan Loop" group)
**When to use:** Scientists want to hide complexity, reuse patterns
**Example (conceptual, egui-snarl may not directly support):**
```rust
enum ExperimentNode {
    // ... regular nodes
    Subgraph {
        name: String,
        collapsed: bool,
        inner_graph: Snarl<ExperimentNode>, // Nested graph
        // External pins map to internal nodes
        input_mapping: HashMap<usize, (NodeId, usize)>,
        output_mapping: HashMap<usize, (NodeId, usize)>,
    },
}

// In viewer:
fn show_body(&mut self, node: &ExperimentNode, ui: &mut Ui) {
    if let ExperimentNode::Subgraph { name, collapsed, .. } = node {
        if ui.button(if *collapsed { "▶" } else { "▼" }).clicked() {
            // Toggle collapsed state
        }
        if !collapsed {
            // Render inner graph (requires nested SnarlViewer)
        }
    }
}
```
**Note:** Subgraph support may require custom implementation or waiting for library feature.

### Pattern 5: Pan/Zoom Coordinate Transform
**What:** Transform between screen space and graph space for pan/zoom
**When to use:** User wants to zoom into complex graphs, pan around large designs
**Example:**
```rust
// Source: https://www.sunshine2k.de/articles/algorithm/panzoom/panzoom.html
struct GraphTransform {
    offset: egui::Vec2,  // Pan offset
    scale: f32,          // Zoom scale
}

impl GraphTransform {
    fn screen_to_graph(&self, screen_pos: Pos2) -> Pos2 {
        // global = (screen / scale) + offset
        Pos2::new(
            screen_pos.x / self.scale + self.offset.x,
            screen_pos.y / self.scale + self.offset.y,
        )
    }

    fn graph_to_screen(&self, graph_pos: Pos2) -> Pos2 {
        // screen = (global - offset) * scale
        Pos2::new(
            (graph_pos.x - self.offset.x) * self.scale,
            (graph_pos.y - self.offset.y) * self.scale,
        )
    }
}

// egui-snarl handles this internally via SnarlStyle scaling
// Application may need to expose zoom controls in toolbar
```
**Note:** egui-snarl v0.9.0 includes UI scaling support, may not need manual implementation.

### Anti-Patterns to Avoid
- **Storing UI state in node data:** Node data should be pure (experiment parameters), not UI state (selection, hover). Use separate `GraphEditorState` for UI.
- **Blocking operations in graph updates:** All device queries, validation checks that hit network should be async. Don't freeze UI during graph manipulation.
- **Hand-rolling graph algorithms:** Use petgraph for topological sort (execution order), cycle detection, reachability analysis.
- **Ignoring coordinate transforms:** Screen-space and graph-space are different. Always transform before position checks.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Node graph UI | Custom canvas + dragging | egui-snarl | Wire routing, connection hit-testing, node layout are complex; egui-snarl handles serde, styling, context menus |
| Undo/redo stack | Vec<Box<dyn Command>> | undo crate (Record/History) | Command merging, checkpoints, memory limits, undo tree branches handled |
| Graph algorithms | DFS traversal by hand | petgraph | Topological sort (execution order), cycle detection, strongly connected components tested |
| JSON graph schema | Custom format | JSON Graph Format (JGF) or adapt | Standard schema, existing tools, well-documented |
| Pan/zoom transforms | Manual matrix math | egui-snarl's built-in scaling or egui layer transforms | Coordinate transform bugs are subtle; libraries handle edge cases |

**Key insight:** Node graph editors are deceptively complex. Immediate-mode GUI + stateful graph = impedance mismatch. Use libraries that bridge this gap (egui-snarl does this well). The "simple" parts (drawing boxes, lines) are easy; the hard parts are connection validation, undo across async operations, serialization that preserves visual layout, and coordinate transforms under zoom.

## Common Pitfalls

### Pitfall 1: Immediate-Mode GUI vs. Stateful Graph Mismatch
**What goes wrong:** egui redraws every frame with no persistent widgets. Node graphs are inherently stateful (nodes, connections, positions). This creates friction: every frame you're reconstructing UI from data, but user interactions (drag, hover, click) need to update that data correctly.

**Why it happens:** Immediate-mode GUI philosophy is "UI is a function of state," but node graphs have complex two-way interactions. Dragging a node updates position, but position affects rendering, which affects hit-testing, which affects next drag.

**How to avoid:**
- Store ALL graph state outside egui (in Snarl, or custom struct). Never rely on egui's widget IDs to track nodes.
- Use egui-snarl or similar library that handles this impedance mismatch internally.
- Separate data (Snarl) from UI concerns (selection, hover, drag state) in different structures.

**Warning signs:** Nodes jump around when dragging, connections don't update until mouse release, hover state "sticks" after mouse moves away.

### Pitfall 2: Lock-Across-Await in Async Validation
**What goes wrong:** You query device metadata to validate connections (e.g., "Can this actuator output connect to this detector?"). If you hold a graph lock during async device query, GUI freezes.

**Why it happens:** egui runs on main thread. Any `.await` that blocks must not hold mutexes/locks. But graph modifications need exclusive access.

**How to avoid:**
- Perform validation asynchronously BEFORE committing connection.
- Show "connecting..." spinner, spawn task to validate, update graph on callback.
- Or: optimistic UI (connect immediately, show red border if validation fails later).
- Never: `let guard = graph.lock().await; device.query().await; // DEADLOCK`

**Warning signs:** GUI freezes when dragging wire to pin, entire app unresponsive during connection attempts.

### Pitfall 3: Coordinate Space Confusion
**What goes wrong:** Node positions stored in graph space, but hit-testing uses screen space. After zooming, clicks land on wrong nodes, dragging offsets are incorrect.

**Why it happens:** Pan/zoom changes screen-to-graph mapping. Code mixes screen coords (from egui mouse events) with graph coords (node.position) without transforming.

**How to avoid:**
- Always transform coordinates explicitly: `screen_to_graph(mouse_pos)` before hit-testing nodes.
- egui-snarl handles this internally — trust the library.
- If custom implementation: track `GraphTransform { offset, scale }` and transform all position comparisons.

**Warning signs:** Nodes can't be clicked after zooming, dragging moves nodes by wrong amount, connections render at wrong positions.

### Pitfall 4: Undo Stack Explosion with Auto-Merge
**What goes wrong:** User drags node, undo stack accumulates 100+ move commands (one per pixel), undo becomes unusable ("undo 100 times to revert one drag").

**Why it happens:** Immediate-mode GUI calls update every frame. If you push command on every frame, stack explodes.

**How to avoid:**
- Use undo crate's `merge()` functionality: consecutive moves to same node merge into single command.
- Or: Push to undo stack only on mouse release (drag end), not per-frame.
- Record-based undo libraries handle this with `Edit::merge` returning `Merged::Yes`.

**Warning signs:** Undo requires many key presses to revert one action, memory usage grows during dragging.

### Pitfall 5: Missing Connection Validation
**What goes wrong:** User connects incompatible nodes (e.g., "Scan" output to "Move" input that expects position, not scan results), graph looks valid but execution crashes.

**Why it happens:** Node graph libraries are semantic-agnostic — they allow ANY connection. Application must add validation logic.

**How to avoid:**
- Implement `SnarlViewer::connect()` with type checking before calling `snarl.connect()`.
- Define pin types (Flow, DeviceReference, NumericData) and validate compatibility.
- Show validation errors visually: red border on node, error tooltip, or refuse to create connection.
- Add `validate_graph()` method that checks entire graph before execution (detect cycles, unreachable nodes, type mismatches).

**Warning signs:** Graph runs but crashes mid-execution with "type error," no visual feedback when invalid connections made.

### Pitfall 6: Subgraph Reference Invalidation
**What goes wrong:** User groups nodes into subgraph, deletes original nodes, then tries to expand subgraph → crashes or shows stale data.

**Why it happens:** Subgraphs store NodeId references to internal nodes. If those nodes are deleted from parent graph, IDs become invalid.

**How to avoid:**
- Subgraphs should OWN their nodes (nested Snarl), not reference parent nodes.
- When creating subgraph: move nodes out of parent, insert into child graph.
- When expanding subgraph: move nodes back to parent, remove child graph.
- Or: Use SlotMap-style generational IDs that detect use-after-free.

**Warning signs:** Expanding subgraph crashes, subgraph shows old data after modification, NodeId lookup returns None.

## Code Examples

Verified patterns from official sources:

### Creating and Rendering Graph (egui-snarl)
```rust
// Source: https://github.com/zakarumych/egui-snarl (README example)
use egui_snarl::{Snarl, SnarlViewer, ui::SnarlStyle, InPin, OutPin, InPinId, OutPinId, NodeId};
use egui::{Ui, Response};

// 1. Define node data
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum MyNode {
    Input { value: f64 },
    Add,
    Output,
}

// 2. Implement viewer
struct MyViewer;

impl SnarlViewer<MyNode> for MyViewer {
    fn title(&self, node: &MyNode) -> String {
        match node {
            MyNode::Input { .. } => "Input".into(),
            MyNode::Add => "Add".into(),
            MyNode::Output => "Output".into(),
        }
    }

    fn inputs(&self, node: &MyNode) -> usize {
        match node {
            MyNode::Input { .. } => 0,
            MyNode::Add => 2,
            MyNode::Output => 1,
        }
    }

    fn outputs(&self, node: &MyNode) -> usize {
        match node {
            MyNode::Input { .. } => 1,
            MyNode::Add => 1,
            MyNode::Output => 0,
        }
    }

    fn show_input(&mut self, pin: &InPin, ui: &mut Ui, _scale: f32, _snarl: &mut Snarl<MyNode>) -> egui_snarl::ui::PinInfo {
        ui.label("In");
        Default::default()
    }

    fn show_output(&mut self, pin: &OutPin, ui: &mut Ui, _scale: f32, _snarl: &mut Snarl<MyNode>) -> egui_snarl::ui::PinInfo {
        ui.label("Out");
        Default::default()
    }
}

// 3. Create and render
fn show_graph_editor(ui: &mut Ui, snarl: &mut Snarl<MyNode>) {
    snarl.show(&mut MyViewer, &SnarlStyle::default(), egui::Id::new("my_graph"), ui);
}
```

### Undo/Redo with Command Pattern
```rust
// Source: https://docs.rs/undo (documentation examples)
use undo::{Record, Edit};

struct MyGraph {
    nodes: Vec<String>,
}

struct AddNode {
    node: String,
    index: usize,
}

impl Edit for AddNode {
    type Target = MyGraph;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        target.nodes.insert(self.index, self.node.clone());
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        target.nodes.remove(self.index);
    }
}

// Usage
let mut graph = MyGraph { nodes: vec![] };
let mut history = Record::new();

// Add node with undo support
history.edit(&mut graph, AddNode {
    node: "Node1".into(),
    index: 0,
});

// Undo
history.undo(&mut graph);

// Redo
history.redo(&mut graph);
```

### JSON Serialization (Serde)
```rust
// Source: egui-snarl serde support + https://jsongraphformat.info/
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct GraphState {
    snarl: Snarl<ExperimentNode>,
    metadata: GraphMetadata,
}

#[derive(Serialize, Deserialize)]
struct GraphMetadata {
    name: String,
    created: String,
    version: u32,
}

fn save_graph(state: &GraphState, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn load_graph(path: &Path) -> Result<GraphState> {
    let json = std::fs::read_to_string(path)?;
    let state = serde_json::from_str(&json)?;
    Ok(state)
}
```

### Keyboard Shortcuts (Ctrl+Z/Ctrl+Y)
```rust
// In panel UI code
fn handle_input(&mut self, ui: &mut Ui) {
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
        if ui.input(|i| i.modifiers.shift) {
            // Ctrl+Shift+Z = Redo (Windows)
            self.redo();
        } else {
            // Ctrl+Z = Undo
            self.undo();
        }
    }

    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y)) {
        // Ctrl+Y = Redo (alternative)
        self.redo();
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| egui_node_graph (setzer22) | egui-snarl or forks (egui_node_graph2) | 2024 (archive) | Original library archived; active forks or egui-snarl recommended |
| Hand-rolled undo stack | undo crate (Record/History) | Mature since 2018 | Standard command pattern library with merging, branches |
| Custom JSON schema | JSON Graph Format (JGF) | 2016+ (stable) | Standardized graph serialization, but may need adaptation for visual layout |
| Manual coordinate transforms | egui layer transforms | egui 0.20+ (2023) | `Ui::with_visual_transform` for pan/zoom, but text rendering issues reported |

**Deprecated/outdated:**
- **egui_node_graph (setzer22/egui_node_graph):** Archived in 2024. Use forks (trevyn/egui_node_graph2, philpax/egui_node_graph2) or migrate to egui-snarl.
- **imnodes-rs:** C++ imnodes bindings for ImGui, not egui. Outdated for Rust-native egui projects.

## Open Questions

Things that couldn't be fully resolved:

1. **Subgraph/grouping support in egui-snarl**
   - What we know: egui-snarl supports custom node types, nested Snarl possible conceptually
   - What's unclear: No official subgraph example in docs, may require custom implementation
   - Recommendation: Start without subgraphs, add in Phase 3 if needed. Use node naming/color-coding for grouping initially.

2. **Undo/redo integration with async validation**
   - What we know: undo crate works with synchronous Edit trait, async validation needs separate handling
   - What's unclear: Best pattern for "undo connection that was validated asynchronously"
   - Recommendation: Optimistic undo (allow connection immediately, show error if validation fails, add undo entry after validation). Or: validation happens synchronously using cached device metadata.

3. **Performance with large graphs (100+ nodes)**
   - What we know: egui immediate-mode redraws every frame, large graphs may slow down
   - What's unclear: At what size does performance degrade? egui-snarl performance benchmarks not published.
   - Recommendation: Start with assumption of <50 nodes for Phase 2. If performance issues, investigate culling (only render visible nodes) or switch to retained-mode rendering for graph.

4. **Property inspector integration**
   - What we know: egui-snarl shows node body UI, can embed widgets
   - What's unclear: Best UX for editing complex properties — inline in node or separate panel?
   - Recommendation: Use separate "Properties" panel (like Blender node editor) with detail view when node selected. Inline UI for simple fields (sliders, checkboxes), panel for complex (device selection, multi-line text).

## Sources

### Primary (HIGH confidence)
- [egui-snarl GitHub](https://github.com/zakarumych/egui-snarl) - README, API design, examples
- [undo crate docs.rs](https://docs.rs/undo) - Edit trait, Record/History API
- [egui-graph-edit GitHub](https://github.com/kamirr/egui-graph-edit) - Alternative library architecture
- [egui official](https://github.com/emilk/egui) - Coordinate systems, transform APIs

### Secondary (MEDIUM confidence)
- [egui_node_graph2 fork](https://github.com/trevyn/egui_node_graph2) - Maintained fork of archived library
- [JSON Graph Format spec](https://jsongraphformat.info/) - Graph serialization standard
- [Pan/Zoom algorithm article](https://www.sunshine2k.de/articles/algorithm/panzoom/panzoom.html) - Coordinate transform math
- [imnodes article](https://nelari.us/post/imnodes/) - Immediate-mode node editor design philosophy

### Tertiary (LOW confidence)
- Hacker News discussions on egui performance - Anecdotal reports of slowness, not quantified
- GitHub issue #1811 (egui pan/zoom container request) - Feature request, not implemented
- Material Maker subgraph docs - Different framework (Godot), patterns may not translate

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - egui-snarl and undo crate are well-documented, actively maintained, used in production
- Architecture: MEDIUM - Patterns verified in docs, but no rust-daq-specific implementation yet; subgraph approach uncertain
- Pitfalls: MEDIUM - Based on documented issues (GitHub, articles), but not all tested in egui context
- Performance: LOW - No benchmarks found for egui-snarl at scale; egui performance concerns anecdotal

**Research date:** 2026-01-22
**Valid until:** ~60 days (stable ecosystem; egui-snarl releases infrequently but steadily)

**Sources:**
- [egui-graph-edit on crates.io](https://crates.io/crates/egui-graph-edit)
- [egui_node_graph on crates.io](https://crates.io/crates/egui_node_graph)
- [egui-snarl on crates.io](https://crates.io/crates/egui-snarl)
- [egui-graph-edit GitHub](https://github.com/kamirr/egui-graph-edit)
- [egui_node_graph2 GitHub (fork)](https://github.com/trevyn/egui_node_graph2)
- [imnodes: immediate mode node editor library article](https://nelari.us/post/imnodes/)
- [undo crate on docs.rs](https://docs.rs/undo)
- [egui-snarl on docs.rs](https://docs.rs/egui-snarl)
- [JSON Graph Format specification](https://jsongraphformat.info/)
- [GitHub: jsongraph/json-graph-specification](https://github.com/jsongraph/json-graph-specification)
- [Panning and Zooming algorithm](https://www.sunshine2k.de/articles/algorithm/panzoom/panzoom.html)
- [egui Issue #1811: Panning and zooming container](https://github.com/emilk/egui/issues/1811)
- [Material Maker subgraph nodes documentation](https://rodzill4.github.io/material-maker/doc/subgraph_nodes.html)
