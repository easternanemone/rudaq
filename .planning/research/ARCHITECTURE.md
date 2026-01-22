# Architecture Patterns: Visual Experiment Design Systems

**Domain:** Experiment orchestration with node-based visual programming
**Researched:** 2026-01-22
**Confidence:** HIGH (verified with established patterns and rust-daq codebase)

## Recommended Architecture

Visual experiment design systems follow a three-layer architecture with strict separation of concerns:

```
┌─────────────────────────────────────────────────────────────┐
│                    PRESENTATION LAYER                        │
│  (Node Graph Editor - Visual Source of Truth)                │
│  - Node canvas with drag/drop                                │
│  - Connection validation and rendering                       │
│  - Parameter panels and inspectors                           │
│  - Real-time validation feedback                             │
└────────────────────┬────────────────────────────────────────┘
                     │ Serialization (JSON)
                     ▼
┌─────────────────────────────────────────────────────────────┐
│              INTERMEDIATE REPRESENTATION (IR)                 │
│  (Graph Data Structure - Canonical Format)                    │
│  - Nodes: ID, type, parameters, position                     │
│  - Edges: source/target nodes and ports                      │
│  - Metadata: version, author, description                    │
│  - Validation: type checking, cycle detection                │
└────────────────────┬────────────────────────────────────────┘
                     │ Code Generation / Interpretation
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                   EXECUTION BACKEND                          │
│  (Plan Execution - RunEngine Integration)                    │
│  - IR → PlanCommand translation                              │
│  - Device registry binding                                   │
│  - RunEngine orchestration                                   │
│  - Document emission and storage                             │
└─────────────────────────────────────────────────────────────┘
```

### Data Flow Direction

**Design-time (Graph Editor):**
1. User manipulates nodes/edges in visual editor
2. Editor maintains in-memory graph state
3. Changes trigger validation (types, cycles, completeness)
4. Valid graph serializes to JSON (IR format)
5. Code preview generated from IR (read-only, for reference)

**Execution-time (RunEngine):**
1. Load IR from JSON file
2. Validate IR structure and device references
3. Translate IR → Box&lt;dyn Plan&gt;
4. Queue Plan with RunEngine
5. RunEngine executes PlanCommands
6. Documents emitted to storage and subscribers

**Critical Pattern:** Visual graph is **source of truth**. Code is **export-only** for inspection.

## Component Boundaries

### 1. Node Graph Editor (daq-egui module)

**Responsibility:** Visual manipulation of experiment graph

**Technology Options:**
- `egui_node_graph2` (recommended) - Maintained fork, flexible, semantic-agnostic
- `egui-snarl` - Alternative with context menu support
- `egui-graph-edit` - Another option, similar feature set

**Communicates With:**
- Graph State Manager (owns IR in memory)
- Validation Engine (type checking, cycle detection)
- Code Preview Generator (read-only export)

**Key Features:**
- Node palette with categorized plan types (0d, 1d, 2d, control flow)
- Connection type enforcement (data types, port compatibility)
- Parameter inspector panels (inline and dedicated)
- Undo/redo via Command pattern (NOT memento - see below)
- Serialization to JSON on save

**State Management:**
```rust
pub struct GraphEditorState {
    graph: ExperimentGraph,           // IR in memory
    command_history: Vec<GraphCommand>, // Undo stack
    future_commands: Vec<GraphCommand>, // Redo stack
    selected_node: Option<NodeId>,
    validation_errors: Vec<ValidationError>,
}
```

### 2. Intermediate Representation (IR)

**Responsibility:** Canonical graph format, version-controlled, human-readable

**Format:** JSON (for diffability and tooling support)

**Schema:**
```json
{
  "version": "1.0",
  "metadata": {
    "name": "My Experiment",
    "author": "username",
    "created": "2026-01-22T10:00:00Z",
    "description": "Grid scan with power measurement"
  },
  "nodes": [
    {
      "id": "node_abc123",
      "type": "grid_scan",
      "position": {"x": 100, "y": 200},
      "parameters": {
        "x_start": 0.0,
        "x_end": 10.0,
        "x_points": 11,
        "y_start": 0.0,
        "y_end": 5.0,
        "y_points": 6,
        "snake": true
      },
      "device_bindings": {
        "x_motor": "stage_x",
        "y_motor": "stage_y",
        "detector": "power_meter"
      }
    }
  ],
  "edges": [
    {
      "id": "edge_xyz789",
      "source": {"node": "node_abc123", "port": "output"},
      "target": {"node": "node_def456", "port": "input"}
    }
  ]
}
```

**Validation Rules:**
- All node types must exist in PlanRegistry
- All parameters must satisfy PlanBuilder validation
- All device_bindings must reference devices in DeviceRegistry
- Graph must be acyclic (DAG)
- All required ports must be connected
- Type compatibility on edges (e.g., DetectorOutput → DataInput)

**Communicates With:**
- Graph Editor (read/write during editing)
- Plan Translator (read during execution)
- Version Control (saved as .json files)

### 3. Plan Translator

**Responsibility:** Convert IR → executable Plan instances

**Pattern:** Interpreter (not compiler) - translate graph to Plan at runtime

**Execution Strategy:**
```rust
pub struct PlanTranslator {
    plan_registry: Arc<PlanRegistry>,
    device_registry: Arc<DeviceRegistry>,
}

impl PlanTranslator {
    /// Translate IR graph to executable Plan
    pub fn translate(&self, ir: &ExperimentGraph) -> Result<Box<dyn Plan>> {
        // 1. Topological sort of nodes (dependency order)
        let sorted_nodes = self.topological_sort(&ir.nodes, &ir.edges)?;

        // 2. Validate device bindings against registry
        self.validate_device_bindings(ir)?;

        // 3. Build composite plan (potentially nested)
        self.build_composite_plan(&sorted_nodes)
    }

    fn topological_sort(&self, nodes: &[Node], edges: &[Edge])
        -> Result<Vec<NodeId>> {
        // Kahn's algorithm or DFS-based sort
        // Detects cycles, returns error if graph is not DAG
    }
}
```

**Communicates With:**
- IR (reads graph structure)
- PlanRegistry (creates Plan instances via builders)
- DeviceRegistry (validates device references)
- RunEngine (provides translated Plan)

### 4. RunEngine Integration

**Responsibility:** Execute Plans with hardware, emit Documents

**Existing Architecture (from rust-daq):**
- Plans yield PlanCommands (MoveTo, Read, Trigger, Wait, EmitEvent)
- RunEngine processes commands sequentially
- State machine: Idle → Running → Paused → Idle
- Document emission: Start, Manifest, Descriptor, Event, Stop

**Integration Point:**
```rust
// In GUI (plan_runner panel or new experiment_designer panel)
let ir = load_experiment_graph("my_experiment.json")?;
let plan = translator.translate(&ir)?;
let run_uid = engine.queue(plan).await;
engine.start().await?;

// Subscribe to documents
let mut docs = engine.subscribe();
while let Some(doc) = docs.recv().await {
    match doc {
        Document::Event(e) => update_live_plot(e),
        Document::Stop(_) => break,
        _ => {}
    }
}
```

**Communicates With:**
- Plan Translator (receives Plan instances)
- DeviceRegistry (executes hardware commands)
- Storage backends (emits Documents for persistence)

### 5. Validation Engine

**Responsibility:** Real-time graph validation during editing

**Validation Types:**
1. **Structural:** DAG check (no cycles), port connections complete
2. **Type Safety:** Edge type compatibility (DetectorData → AnalysisInput)
3. **Parameter:** Numeric ranges, required fields, device existence
4. **Execution:** Hardware capability checks (device supports required traits)

**Performance Pattern:**
- Incremental validation (only re-check affected subgraph)
- Debounced validation (wait 300ms after last edit)
- Background validation thread (don't block UI)

**Error Reporting:**
```rust
pub enum ValidationError {
    CycleDetected { nodes: Vec<NodeId> },
    MissingParameter { node: NodeId, param: String },
    TypeMismatch { edge: EdgeId, expected: Type, actual: Type },
    DeviceNotFound { node: NodeId, device_id: String },
    IncompatibleCapability { node: NodeId, required: Capability },
}
```

**Visual Feedback:**
- Red border on invalid nodes
- Red edge connection lines for type mismatches
- Error icon with tooltip on nodes
- Error panel listing all validation issues

**Communicates With:**
- Graph Editor (triggers validation on edits)
- PlanRegistry (checks plan type existence)
- DeviceRegistry (validates device capabilities)

### 6. Code Preview Generator

**Responsibility:** Export IR to human-readable code (Rhai or Rust)

**Pattern:** Read-only, generated on-demand, NOT source of truth

**Purpose:**
- Help users understand what the graph does
- Enable copy/paste for scripting experiments
- Debugging (compare expected vs actual behavior)

**Example Output (Rhai):**
```rhai
// Generated from: my_experiment.json
// Date: 2026-01-22T10:00:00Z

let plan = grid_scan(
    x_motor: "stage_x",
    x_start: 0.0,
    x_end: 10.0,
    x_points: 11,
    y_motor: "stage_y",
    y_start: 0.0,
    y_end: 5.0,
    y_points: 6,
    snake: true,
    detector: "power_meter"
);

run_plan(plan);
```

**NOT EDITABLE:** Code is view-only. Edits must happen in graph.

**Communicates With:**
- IR (reads graph structure)
- Code syntax highlighter (for display in GUI)

## Architecture Patterns from Research

### Pattern 1: Command Pattern for Undo/Redo

**Why NOT Memento Pattern:**
- Memento requires deep-cloning entire graph state on every change
- Doesn't scale beyond trivial apps
- Can't handle side effects (e.g., auto-layout triggered by node addition)

**Command Pattern Structure:**
```rust
pub trait GraphCommand: Send + Sync {
    fn execute(&mut self, graph: &mut ExperimentGraph) -> Result<()>;
    fn undo(&mut self, graph: &mut ExperimentGraph) -> Result<()>;
}

pub struct AddNodeCommand {
    node_id: NodeId,
    node_type: String,
    position: (f64, f64),
    // Store data needed to undo
    added: bool,
}

impl GraphCommand for AddNodeCommand {
    fn execute(&mut self, graph: &mut ExperimentGraph) -> Result<()> {
        graph.add_node(self.node_id, self.node_type.clone(), self.position)?;
        self.added = true;
        Ok(())
    }

    fn undo(&mut self, graph: &mut ExperimentGraph) -> Result<()> {
        if self.added {
            graph.remove_node(self.node_id)?;
            self.added = false;
        }
        Ok(())
    }
}
```

**Benefits:**
- Only stores deltas, not full state
- Supports complex multi-step commands
- Can apply side effects consistently on redo
- Scales to large graphs (Figma/TLDraw use this pattern)

### Pattern 2: Dataflow Graph Execution

**Execution Model:**
- Each node executes when its input data is available
- Enables parallel execution (if hardware supports)
- Natural fit for experiment orchestration

**For rust-daq:**
- Simple linear execution initially (topological sort)
- Future: parallel branches (multiple detectors simultaneously)

**Topological Sort (Kahn's Algorithm):**
```rust
fn topological_sort(nodes: &[Node], edges: &[Edge]) -> Result<Vec<NodeId>> {
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    let mut adj_list: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    // Build graph structure
    for node in nodes {
        in_degree.insert(node.id, 0);
        adj_list.insert(node.id, Vec::new());
    }

    for edge in edges {
        *in_degree.get_mut(&edge.target).unwrap() += 1;
        adj_list.get_mut(&edge.source).unwrap().push(edge.target);
    }

    // Kahn's algorithm
    let mut queue: VecDeque<NodeId> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut result = Vec::new();

    while let Some(node) = queue.pop_front() {
        result.push(node);

        for &neighbor in &adj_list[&node] {
            let deg = in_degree.get_mut(&neighbor).unwrap();
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(neighbor);
            }
        }
    }

    // Cycle detection
    if result.len() != nodes.len() {
        return Err(anyhow!("Cycle detected in experiment graph"));
    }

    Ok(result)
}
```

### Pattern 3: Type System for Ports

**Port Types (for edge validation):**
```rust
pub enum PortType {
    Scalar,           // f64 data
    ScalarArray,      // Vec<f64> data
    Frame,            // 2D image data
    Position,         // Motor position
    Trigger,          // Control flow
    Any,              // Wildcard (for debugging)
}

pub struct PortDefinition {
    pub name: String,
    pub port_type: PortType,
    pub direction: PortDirection,
}

pub enum PortDirection {
    Input,
    Output,
}
```

**Validation:**
```rust
fn validate_edge(source_port: &PortDefinition, target_port: &PortDefinition)
    -> Result<()> {
    if source_port.direction != PortDirection::Output {
        bail!("Source port must be Output");
    }
    if target_port.direction != PortDirection::Input {
        bail!("Target port must be Input");
    }
    if !types_compatible(&source_port.port_type, &target_port.port_type) {
        bail!("Type mismatch: {:?} → {:?}",
            source_port.port_type, target_port.port_type);
    }
    Ok(())
}
```

### Pattern 4: JSON Graph Specification

**Standard Format (from jsongraph project):**
- Nodes as map/object (key = node ID)
- Edges as array of {source, target} objects
- Metadata separate from graph structure

**Benefits:**
- Human-readable diffs in version control
- Standard validation via JSON Schema
- Tooling support (jq, schema validators)

**rust-daq Adaptation:**
```json
{
  "version": "1.0",
  "experiment_graph": {
    "nodes": {
      "scan_001": {
        "type": "grid_scan",
        "position": {"x": 100, "y": 200},
        "parameters": { ... },
        "device_bindings": { ... }
      }
    },
    "edges": [
      {
        "source": {"node": "scan_001", "port": "events"},
        "target": {"node": "analysis_002", "port": "data_in"}
      }
    ]
  }
}
```

### Pattern 5: LabVIEW Compilation Pipeline (Inspiration)

**LabVIEW Architecture:**
1. **Visual G code** (block diagram) is primary source
2. **Type Propagation** - resolve types and detect syntax errors
3. **DFIR (Dataflow IR)** - optimized intermediate representation
4. **LLVM IR** - lower-level compilation target
5. **Native machine code** - executed by runtime engine

**rust-daq Adaptation (Simpler):**
1. **Node Graph** (visual editor) is primary source
2. **Validation** - type checking, cycle detection
3. **Graph IR** (JSON) - intermediate representation
4. **Plan Translation** - generate Box&lt;dyn Plan&gt; at runtime
5. **PlanCommands** - executed by RunEngine

**Key Difference:** rust-daq uses **interpretation** (runtime translation), not ahead-of-time compilation. Simpler, more flexible for experiments.

## Scalability Considerations

### At 10 nodes (MVP)
- In-memory graph state
- Synchronous validation
- Single-threaded translation

### At 100 nodes (Phase 2)
- Incremental validation (only changed subgraph)
- Debounced validation (300ms delay)
- Undo history with size limit (100 commands)

### At 1000+ nodes (Future)
- Lazy graph loading (viewport culling)
- Background validation thread
- Undo history with command coalescing

## Integration with Existing rust-daq

### What Exists Today
- ✅ RunEngine with Plan execution
- ✅ PlanCommand enum (MoveTo, Read, Trigger, etc.)
- ✅ Plan trait and implementations (LineScan, GridScan, Count)
- ✅ PlanRegistry with PlanBuilder pattern
- ✅ Document emission (Start, Event, Stop)
- ✅ DeviceRegistry with capability traits
- ✅ egui GUI framework

### What Needs Building
- ❌ Node graph editor (egui_node_graph2 integration)
- ❌ Graph IR definition and JSON schema
- ❌ Plan Translator (IR → Plan)
- ❌ Validation Engine
- ❌ Code Preview Generator
- ❌ Experiment graph panel in GUI

### Build Order (Dependencies)

**Phase 1: Core IR and Basic Editor**
1. Define Graph IR schema (JSON)
2. Implement IR serialization/deserialization
3. Create basic node editor (egui_node_graph2)
4. Node palette with 1-2 plan types (Count, LineScan)
5. Manual device binding (dropdown per port)

**Phase 2: Translation and Execution**
6. Implement Plan Translator (IR → Plan)
7. Integrate with RunEngine (existing)
8. Basic validation (cycle detection only)
9. Execute experiments from graph editor

**Phase 3: Polish and Validation**
10. Full validation engine (types, parameters, devices)
11. Visual error feedback in editor
12. Undo/redo with Command pattern
13. Code preview generator (Rhai export)

**Phase 4: Advanced Features**
14. Composite plans (nested subgraphs)
15. Control flow nodes (if/while)
16. Live parameter adjustment during execution
17. Graph templates library

## Anti-Patterns to Avoid

### Anti-Pattern 1: Bidirectional Sync (Code ↔ Graph)

**What goes wrong:** Trying to keep code and graph in sync both directions creates:
- Merge conflicts (which is source of truth?)
- Round-trip conversion bugs (graph → code → graph ≠ original)
- Ambiguity in representation (multiple valid graphs for same code)

**Prevention:** Graph is ONLY source of truth. Code is export-only, for reading.

### Anti-Pattern 2: Graph Stored as Rendered Layout

**What goes wrong:** Storing visual positions as primary data structure:
- Couples logic to presentation
- Makes programmatic graph manipulation hard
- Version control diffs become unreadable

**Prevention:** Separate graph structure (nodes/edges) from layout (x/y positions). Save layout separately or as metadata.

### Anti-Pattern 3: Synchronous Validation Blocking UI

**What goes wrong:** Running full graph validation on every edit:
- Editor feels sluggish (especially on large graphs)
- Users can't work fluidly
- Feedback loop is too tight

**Prevention:** Debounce validation (300ms after last edit), show "validating..." indicator, validate incrementally.

### Anti-Pattern 4: Memento Pattern for Undo

**What goes wrong:** (See Command Pattern section above)

**Prevention:** Use Command pattern with execute/undo methods.

## Sources

**Node Graph Architecture:**
- [Node graph architecture - Wikipedia](https://en.wikipedia.org/wiki/Node_graph_architecture)
- [Designing your own node-based visual programming language - DEV Community](https://dev.to/cosmomyzrailgorynych/designing-your-own-node-based-visual-programming-language-2mpg)

**Scientific Workflow Systems:**
- [Scientific workflow system - Wikipedia](https://en.wikipedia.org/wiki/Scientific_workflow_system)
- [Bluesky Project](https://blueskyproject.io/)
- [Bluesky GitHub - experiment orchestration and data acquisition](https://github.com/bluesky/bluesky)

**Dataflow Programming:**
- [Dataflow programming - Wikipedia](https://en.wikipedia.org/wiki/Dataflow_programming)
- [Dataflow: streaming analytics | Google Cloud](https://cloud.google.com/dataflow)

**Intermediate Representation:**
- [Intermediate representation - Wikipedia](https://en.wikipedia.org/wiki/Intermediate_representation)
- [GitHub - SeaOfNodes/Simple: A Simple showcase for the Sea-of-Nodes compiler IR](https://github.com/SeaOfNodes/Simple)

**LabVIEW Compilation:**
- [NI LabVIEW Compiler: Under the Hood - NI](https://www.ni.com/en/support/documentation/supplemental/10/ni-labview-compiler--under-the-hood.html)
- [LabVIEW - Wikipedia](https://en.wikipedia.org/wiki/LabVIEW)

**Undo/Redo Patterns:**
- [Undo, Redo, and the Command Pattern | esveo](https://www.esveo.com/en/blog/undo-redo-and-the-command-pattern/)
- [You Don't Know Undo/Redo - DEV Community](https://dev.to/isaachagoel/you-dont-know-undoredo-4hol)

**Topological Sorting:**
- [Topological Sorting Explained: A Step-by-Step Guide for Dependency Resolution | Medium](https://medium.com/@amit.anjani89/topological-sorting-explained-a-step-by-step-guide-for-dependency-resolution-1a6af382b065)
- [Topological Sort In Dependency Resolution](https://heycoach.in/blog/topological-sort-in-dependency-resolution/)

**JSON Graph Specification:**
- [GitHub - jsongraph/json-graph-specification: A proposal for representing graph structure (nodes / edges) in JSON](https://github.com/jsongraph/json-graph-specification)

**Rust/egui Node Editors:**
- [egui_node_graph2 - crates.io](https://crates.io/crates/egui_node_editor)
- [GitHub - philpax/egui_node_graph2: Build your node graph applications in Rust, using egui](https://github.com/philpax/egui_node_graph2)
- [egui-snarl - crates.io](https://crates.io/crates/egui-snarl)

**Visual Programming Code Generation:**
- [Visual programming language - Wikipedia](https://en.wikipedia.org/wiki/Visual_programming_language)
- [Rete.js - JavaScript framework for visual programming](https://retejs.org/)
