# Project Research Summary

**Project:** rust-daq Visual Experiment Designer
**Domain:** Scientific data acquisition with node-based experiment orchestration
**Researched:** 2026-01-22
**Confidence:** HIGH

## Executive Summary

Building a visual experiment designer for rust-daq requires a three-layer architecture: node graph editor (egui-snarl) as the visual source of truth, JSON intermediate representation for serialization and validation, and Plan translation layer integrating with the existing RunEngine. The project has a strong foundation with Bluesky-inspired Plan/RunEngine architecture, gRPC server, and hardware abstraction layer already in place.

The recommended approach is **GUI-first workflow with one-way code export**, avoiding the bidirectional sync trap that plagued UML tools and other visual programming systems. Research shows that interactive execution control (pause/resume/abort), live plotting, and complete metadata capture are table stakes in scientific DAQ systems. Visual node-based design is the primary differentiator separating rust-daq from code-first systems like Bluesky and labscript. The hybrid approach (visual for design, code generation as escape hatch) positions rust-daq uniquely in the market.

Key risks center around execution state management and complexity scaling. Live parameter editing during execution requires careful checkpoint coordination with RunEngine to avoid race conditions and data corruption. Visual graph complexity must be managed from day one with subgraph/grouping support, not retrofitted later. Complete provenance capture (graph version, parameters, hardware state, mid-run changes) is non-negotiable for scientific reproducibility. The Command pattern for undo/redo must be built into the foundation, as memento-based approaches don't scale beyond trivial graphs.

## Key Findings

### Recommended Stack

The core technology stack leverages rust-daq's existing egui ecosystem while adding specialized node graph and validation libraries. Three new dependencies are required (egui-snarl, daggy, undo), with two optional enhancements (egui-async, tera) for evaluation during development.

**Core technologies:**
- **egui-snarl 0.9+**: Node graph editor with built-in serde support, five-zone layout, context menus. Actively maintained (January 2025 release), integrates seamlessly with egui 0.33. Type-safe data-only nodes perfect for wrapping Plan abstractions.
- **daggy 0.9+**: DAG validation with cycle detection at construction time. Built on petgraph but enforces acyclic invariants, preventing infinite loops before execution starts. Topological sort for execution order.
- **undo 7.0+**: Command pattern implementation for undo/redo. Stores deltas rather than full state, scales to large graphs. Avoids memento pattern's deep-cloning overhead.
- **egui_plot 0.34**: Already integrated for live plotting. Handles <10K points efficiently, requires downsampling for high-FPS camera streams.
- **Rhai 1.19**: Already integrated for scripting. Generates experiment code from node graphs, provides escape hatch for complex logic. Limitation: no native pause/resume, requires step-wise generation with pause points between steps.
- **tokio 1.36+**: Already integrated async runtime. Handles RunEngine execution, progress updates via channels, background validation.

**Optional enhancements:**
- **egui-async 0.1+**: Simplifies async task management across egui frames. Young crate (2025), evaluate vs manual tokio::sync::mpsc pattern already proven in daq-egui.
- **tera 1.20+**: Template-based code generation if string interpolation becomes complex. Runtime loading supports customizable script templates.

### Expected Features

Scientific experiment design systems split between code-first (Bluesky, labscript) and GUI-first (PyMoDAQ, ScopeFoundry) workflows. rust-daq's hybrid approach targets both personas: visual builder for novices, code export for experts.

**Must have (table stakes):**
- **Parameter Scans (1D/2D)**: Core use case across all systems. Grid, linear, list-based sweeps with motor/laser/voltage control while acquiring detector data.
- **Pause/Resume/Abort**: Interactive control essential for recovering from errors. Bluesky's checkpoint-based system is gold standard. rust-daq RunEngine already has Checkpoint support.
- **Live Plotting**: Real-time visual feedback separates DAQ from batch processing. All modern systems have this. Challenge: high-FPS camera streams require downsampling.
- **Auto-Save to Disk**: Stream to HDF5/CSV during acquisition. Data loss = career catastrophe in science. daq-storage already supports this.
- **Device Discovery**: List available motors/detectors from DeviceRegistry. Capability trait filtering (Movable, Readable, FrameProducer).
- **Metadata Capture**: Reproducibility requirement. Auto-capture: timestamp, user, hostname, git commit. User-provided: sample ID, conditions, notes. REPRODUCE-ME model: Data, Agent, Activity, Plan, Step, Setting, Instrument, Material.
- **Run History**: Browse past experiments, view parameters, rerun. StartDoc/StopDoc already tracked by RunEngine.
- **Error Recovery**: Checkpoint-based resume. Hardware fails = restart without data loss.

**Should have (competitive differentiators):**
- **Visual Node-Based Builder**: Scientists think in flowcharts. Orange/LabVIEW pattern. Drag nodes, connect edges. Sets rust-daq apart from code-first Bluesky/labscript.
- **One-Way Code Export**: Visual → Rhai/Python gives escape hatch. Don't attempt bidirectional (see PITFALLS.md #1).
- **Template Library**: Reusable experiment patterns (wavelength calibration, beam alignment). Save/load partial graphs accelerates common tasks.
- **Adaptive Plans**: Experiment responds to data (zoom into peak, increase integration if power drops). Bluesky's killer feature. Complexity: predicate language design.
- **Nested Scans**: 3D/4D scans (wavelength scan at each XY position). Outer × inner loop composition.
- **Dry Run / Simulation**: Preview experiment timeline, estimated duration, data size. Prevents costly mistakes.
- **Smart Device Mapping**: Auto-suggest devices based on node requirements. "This scan needs Movable" → filter available motors by capability trait.

**Defer (v2+):**
- **Hardware-Timed Sequences**: μs-precision compiled sequences (labscript's signature feature). Out of scope: rust-daq instruments don't support compiled mode. Software timing adequate for >1ms timescales.
- **PID Feedback Loops**: Auto-stabilize laser power/temperature. Separate concern for daq-control module, not experiment design.
- **Bidirectional Code ↔ Graph Sync**: Parsing arbitrary code to graph is AI-complete problem. Stick with one-way export.
- **In-Graph Data Analysis**: Mixing acquisition and analysis creates bloated UI. Export to Jupyter/Python for complex analysis.

### Architecture Approach

Visual experiment design follows three-layer architecture with strict separation: Presentation (node graph editor), Intermediate Representation (JSON graph structure), and Execution Backend (Plan translation and RunEngine integration). Visual graph is source of truth, code is export-only for inspection.

**Major components:**
1. **Node Graph Editor (daq-egui module)**: egui-snarl for visual manipulation. Node palette with categorized plan types (0d, 1d, 2d, control flow). Connection type enforcement. Parameter inspector panels. Undo/redo via Command pattern. Serialization to JSON on save.

2. **Intermediate Representation (IR)**: JSON format with version, metadata, nodes (id, type, parameters, device_bindings, position), edges (source/target nodes and ports). Validation rules: acyclic, type compatibility, device existence, required ports connected. Human-readable diffs in version control.

3. **Plan Translator**: Interpreter pattern (not compiler). IR → Box&lt;dyn Plan&gt; at runtime. Topological sort for execution order (Kahn's algorithm). Validate device bindings against DeviceRegistry. Build composite plans (potentially nested).

4. **RunEngine Integration**: Existing architecture. Plans yield PlanCommands (MoveTo, Read, Trigger, Wait, EmitEvent). State machine: Idle → Running → Paused → Idle. Document emission: Start, Manifest, Descriptor, Event, Stop. Subscribe to document stream for live plotting.

5. **Validation Engine**: Real-time graph validation during editing. Structural (DAG check), type safety (edge compatibility), parameter (ranges, required fields), execution (device capabilities). Incremental validation with debouncing (300ms). Background validation thread. Visual feedback: red borders on invalid nodes, error panel with detailed messages.

6. **Code Preview Generator**: Export IR to Rhai/Rust code. Read-only, generated on-demand. Purpose: help users understand graph behavior, enable copy/paste for scripting, debugging. NOT editable, NOT source of truth.

**Key patterns:**
- **Command Pattern for Undo**: Store mutations as reversible commands, not full state snapshots. Scales to large graphs (Figma/TLDraw pattern).
- **Dataflow Graph Execution**: Topological sort determines execution order. Future: parallel branches for multiple detectors.
- **Type System for Ports**: PortType enum (Scalar, ScalarArray, Frame, Position, Trigger, Any). Validation at connection time prevents runtime type errors.
- **JSON Graph Specification**: Standard format with nodes as objects, edges as arrays. Benefits: human-readable diffs, standard validation via JSON Schema, tooling support (jq, schema validators).

### Critical Pitfalls

Domain research reveals five critical pitfalls that cause rewrites, data loss, or architectural failures. These must be avoided from day one, as retrofitting is expensive or impossible.

1. **Round-Trip Code Parsing (Visual ↔ Code Sync)**: Bidirectional synchronization creates fragile systems. Code formatters break visual mapping, ambiguity in representation. Prevention: ONE-WAY GENERATION ONLY. Visual graph → code export (read-only). If users want code-first, provide separate text editor workflow that doesn't sync. Document clearly: "Visual is source of truth."

2. **Live Parameter Editing Without State Isolation**: Modifying parameters mid-run causes race conditions, inconsistent metadata, corrupted data files. Prevention: Checkpoint-based parameter injection only. Immutable Plan structure during execution. Parameter change provenance (log every change as EventDoc). Atomic state snapshots via message passing channels. UI affordances: gray out structural changes during execution.

3. **Dataflow Cycle Detection Failure**: Node graphs with cycles (A → B → C → A) cause infinite loops, UI freezes, out-of-memory crashes. Prevention: Static cycle detection via topological sort before execution. Restrict to DAG (directed acyclic graph). Explicit Loop nodes for intentional iteration. Visual feedback: highlight cycles in red, block execution until resolved. Fail-safe execution depth limit.

4. **Visual Spaghetti (Unmanaged Graph Complexity)**: Large graphs become tangled "spaghetti wiring" with hundreds of crossing edges. Prevention: Group nodes/subgraphs from the start (most important node type for managing complexity). Experiment templates for reusable patterns. Auto-layout tools to minimize edge crossings. Visual organization: comment boxes, alignment, reroute nodes. Escape hatch to code for truly complex logic. Complexity budget: warn when graph exceeds threshold (50 nodes, 10 hierarchy levels).

5. **Missing Execution Provenance (Unreproducible Experiments)**: Incomplete metadata capture makes results scientifically invalid. Prevention: Graph versioning (snapshot JSON at Start document). Complete provenance: parameters, device configurations, mid-run changes, third-party resources. Parameter timeline (log EventDoc for every change). Hardware state snapshot at experiment start. Code export as provenance (human-readable backup). Template provenance if using reusable components. Graph structure checksum/hash to detect modifications.

**Moderate pitfalls (cause delays but fixable):**
- Poor type safety at node boundaries (connect incompatible ports)
- No undo/redo or branching history (users afraid to experiment)
- Execution state opacity (can't see which node is running)
- Insufficient error handling visibility (error propagation hides source)
- Checkpointing without consistency guarantees (pause mid-operation corrupts state)

## Implications for Roadmap

Based on research, experiment design should be built in iterative phases that establish foundation patterns early, then add visual complexity and advanced features. The critical insight: validate core execution loop with simple forms before investing in node editor complexity.

### Phase 1: Form-Based Scan Builder (Foundation - 2 weeks)

**Rationale:** Validate core workflow (device discovery → scan config → execution → live plot → save) with minimal complexity before investing in node graph editor. Many users comfortable with forms (PyMoDAQ, ScopeFoundry pattern). Establishes integration points with existing RunEngine, DeviceRegistry, daq-storage.

**Delivers:**
- Device discovery panel listing available Movable/Readable devices from registry
- Scan configuration form (1D/2D parameter sweeps via text fields)
- Interactive execution controls (Start/Pause/Resume/Abort buttons)
- Live plotting panel (real-time line plot updates from Event documents)
- Auto-save to HDF5/CSV during run (stream, not batch)
- Basic run history browser (list past experiments with metadata)

**Uses (from STACK.md):**
- egui_plot (already integrated)
- tokio channels for RunEngine document subscription
- serde_json for experiment metadata

**Avoids (from PITFALLS.md):**
- Checkpoint-based pause/resume established early (Pitfall #2)
- Complete metadata capture protocol (Pitfall #5)
- Simple architecture validates before complexity

**Research Flag:** Standard patterns (Bluesky integration, egui forms). No additional research needed.

---

### Phase 2: Node Graph Editor Core (3 weeks)

**Rationale:** Build visual editing foundation with undo/redo, validation, and serialization. Must establish one-way generation pattern and subgraph support immediately (Pitfalls #1, #4). These are foundational decisions that can't be retrofitted.

**Delivers:**
- egui-snarl integration with node canvas
- Basic node palette (Count, LineScan, GridScan, MoveTo, Read nodes)
- Connection validation (type checking, cycle detection via daggy)
- Undo/redo with Command pattern
- JSON serialization (save/load experiments)
- Parameter inspector panels for node configuration
- Manual device binding (dropdown per node)

**Uses (from STACK.md):**
- egui-snarl 0.9+ for node graph editor
- daggy 0.9+ for DAG validation and topological sort
- undo 7.0+ for undo/redo
- serde_json for IR serialization

**Addresses (from FEATURES.md):**
- Visual node-based builder (differentiator)
- Template library foundation (save/load graphs)

**Avoids (from PITFALLS.md):**
- #1: One-way generation established (no code import)
- #3: Cycle detection via daggy (prevent infinite loops)
- #4: Subgraph/grouping designed from start
- #7: Command pattern undo (hard to retrofit)

**Research Flag:** Moderate complexity. May need phase-specific research on egui-snarl advanced features (custom node rendering, port types). Most patterns well-documented.

---

### Phase 3: Plan Translation and Execution (2 weeks)

**Rationale:** Connect visual graph to RunEngine execution. This validates the entire pipeline: node graph → IR → Plan → RunEngine → Documents → UI updates. Critical integration point exposing design issues early.

**Delivers:**
- Plan Translator (IR → Box&lt;dyn Plan&gt; conversion)
- Topological sort execution order (Kahn's algorithm)
- Device binding validation against DeviceRegistry
- Execute experiments from node graph editor
- Visual execution state (highlight running nodes, show progress)
- Error handling with source identification

**Uses (from STACK.md):**
- Existing RunEngine and Plan trait
- PlanRegistry for Plan instantiation
- DeviceRegistry for capability validation

**Implements (from ARCHITECTURE.md):**
- Plan Translator component (IR → Plan)
- Validation Engine (full validation: types, parameters, devices, execution)
- Execution state visualization

**Addresses (from FEATURES.md):**
- Parameter scans (1D/2D) execution via translated Plans
- Error recovery through RunEngine checkpoint system

**Avoids (from PITFALLS.md):**
- #2: Checkpoint protocol coordinated with RunEngine
- #8: Execution state opacity prevented (node highlighting)
- #9: Error source identification (not just propagation target)
- #10: Checkpoint consistency guarantees

**Research Flag:** Standard patterns (dataflow execution, topological sort). Existing RunEngine documentation sufficient.

---

### Phase 4: Code Export and Provenance (1 week)

**Rationale:** Add code generation and complete metadata capture before advanced features. Provenance is non-negotiable for scientific reproducibility. Code export provides escape hatch for power users and debugging.

**Delivers:**
- Rhai script generation from node graph (one-way, read-only)
- Code preview panel with syntax highlighting
- Complete provenance capture (graph version, git commit, device states)
- Parameter change logging (EventDoc for mid-run modifications)
- Graph structure checksum for version tracking
- Enhanced run history with full metadata browsing

**Uses (from STACK.md):**
- Rhai 1.19 (already integrated) for script generation
- Optional: tera 1.20 for template-based generation if string building becomes complex

**Addresses (from FEATURES.md):**
- One-way code export (differentiator)
- Metadata capture (table stakes)
- Run history with complete provenance

**Avoids (from PITFALLS.md):**
- #1: Code export is read-only, no import (prevent bidirectional sync)
- #5: Complete execution provenance (scientific reproducibility)

**Research Flag:** Low complexity. Rhai code generation straightforward. Standard provenance patterns from Bluesky.

---

### Phase 5: Advanced Features (3 weeks)

**Rationale:** After core pipeline validated, add differentiating features: templates, nested scans, adaptive plans. These build on stable foundation and can be deprioritized if timeline pressure.

**Delivers:**
- Template library (save/load partial graphs, categorized patterns)
- Nested scan support (outer × inner loop composition)
- Adaptive plan primitives (conditional execution based on data)
- Smart device mapping (type-based suggestions using capability traits)
- Enhanced visual organization (auto-layout, minimap, search)
- Run comparison (overlay plots, metadata diff)

**Uses (from STACK.md):**
- Evaluate egui-async 0.1 vs manual tokio channels for async task management
- daggy for nested graph composition

**Addresses (from FEATURES.md):**
- Template library (differentiator)
- Adaptive plans (Bluesky killer feature)
- Nested scans (3D/4D experiments)
- Smart device mapping (UX enhancement)
- Run comparison (workflow optimization)

**Avoids (from PITFALLS.md):**
- #4: Templates reduce complexity, don't hide it
- #11: Extensible node type system for custom nodes

**Research Flag:** High complexity for adaptive plans. Needs phase-specific research on predicate language design (simple comparisons vs full expression evaluation). Nested scan patterns well-documented in COMSOL/Ansys.

---

### Phase 6: Polish and Optimization (2 weeks)

**Rationale:** Performance optimization, user testing, edge case handling. Allows time for feedback incorporation and production readiness.

**Delivers:**
- Performance optimization (large graph rendering, live plot downsampling)
- Incremental validation (only changed subgraph)
- Background validation thread (don't block UI)
- Enhanced error messages and visual feedback
- User testing and refinement
- Documentation and tutorials

**Addresses:**
- Live plotting performance with high-FPS camera streams
- Visual spaghetti mitigation (auto-layout, viewport culling)
- User experience polish based on feedback

**Research Flag:** None. Performance tuning based on profiling results.

---

### Phase Ordering Rationale

**Dependency-driven sequence:**
1. Form-based builder validates RunEngine integration before visual complexity
2. Node editor establishes foundational patterns (undo, validation, serialization) early
3. Plan translation connects visual to execution, exposing design issues
4. Code export and provenance complete core pipeline
5. Advanced features build on stable foundation
6. Polish phase allows feedback incorporation

**Pitfall-aware design:**
- Phase 1 establishes checkpoint protocol (Pitfall #2)
- Phase 2 builds Command pattern undo and subgraph support (Pitfalls #1, #4, #7)
- Phase 2-3 implement cycle detection before first execution (Pitfall #3)
- Phase 4 captures complete provenance from first real use (Pitfall #5)
- All phases coordinate on type safety and execution state visibility (Pitfalls #6, #8, #9)

**Architecture-aligned grouping:**
- Phases 1-3: Core execution loop (Presentation → IR → Execution)
- Phase 4: Metadata and escape hatches
- Phase 5: Advanced differentiators
- Phase 6: Production readiness

**Risk management:**
- High-risk items (live plotting, node editor, adaptive plans) spread across phases
- Each phase delivers working functionality, not partial features
- Early phases validate integration points before complexity increases
- Advanced features (Phase 5) can be deprioritized if timeline pressure

### Research Flags

**Needs phase-specific research during planning:**
- **Phase 2 (Node Editor):** egui-snarl advanced features (custom node rendering, port type system, context menus). Should be straightforward but worth deeper dive during implementation planning.
- **Phase 5 (Adaptive Plans):** Predicate language design. Options: simple comparisons (if power < threshold), Rhai expressions (full scripting), custom DSL. This is novel domain work, not standard patterns. Consider `/gsd:research-phase` before implementation.

**Standard patterns (skip additional research):**
- **Phase 1 (Forms):** Well-documented egui patterns, existing RunEngine integration clear
- **Phase 3 (Translation):** Topological sort and dataflow execution extensively documented
- **Phase 4 (Code Export):** Rhai AST generation straightforward, provenance patterns from Bluesky
- **Phase 6 (Polish):** Performance tuning based on profiling, standard optimization techniques

**Overall research quality:** HIGH confidence across all phases except adaptive plan predicate language (MEDIUM confidence, needs design work).

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All recommended libraries actively maintained, versions compatible with egui 0.33, egui-snarl newest dependency (January 2025 release). Rhai and tokio already integrated. Only 3 new dependencies required. |
| Features | HIGH | Table stakes verified across 5+ systems (Bluesky, PyMoDAQ, labscript, ScopeFoundry, Orange). Differentiators align with market gap (no existing Rust DAQ with visual builder). Anti-features backed by research on failure modes. |
| Architecture | HIGH | Three-layer pattern proven in LabVIEW, Orange, node editor research. Bluesky provides reference for Plan/RunEngine integration. JSON IR standard from jsongraph project. Command pattern for undo/redo established in Figma/TLDraw. |
| Pitfalls | HIGH | Cross-verified across multiple authoritative sources (LabVIEW lessons learned, academic papers on scientific workflows, visual programming best practices). Critical pitfalls #1-5 have documented failure cases in production systems. |

**Overall confidence:** HIGH

Research quality across all four files (STACK, FEATURES, ARCHITECTURE, PITFALLS) is consistently high with authoritative sources and cross-verification. The rust-daq existing architecture (RunEngine, Plan trait, DeviceRegistry, capability traits, Document model) provides strong foundation, reducing integration risk. Major unknowns are limited to adaptive plan predicate language design (Phase 5) and performance tuning specifics (Phase 6).

### Gaps to Address

**Adaptive plan predicate language (MEDIUM confidence):** Research identified the pattern (experiment responds to data) but not the optimal implementation approach. Options: simple threshold comparisons, full Rhai expressions, custom DSL. This needs design exploration during Phase 5 planning, potentially with `/gsd:research-phase` to evaluate tradeoffs. Consider starting with simple comparisons (if value < threshold then action) and adding expression support later if needed.

**egui_plot high-FPS performance (MEDIUM confidence):** Documentation states <10K points efficient, but rust-daq camera streams can exceed 30 FPS. Need empirical testing to determine downsampling requirements. Mitigation: benchmark early in Phase 1 (form-based plotting) to establish performance baseline before node editor adds complexity.

**egui-snarl custom rendering (MEDIUM confidence):** Research confirms egui-snarl supports custom node rendering via Viewer trait, but specific patterns for multi-port nodes, validation feedback, execution state highlighting not documented. Low risk: can fall back to default rendering with parameter panels if custom rendering proves complex. Verify during Phase 2 planning.

**Hardware state snapshot format (LOW confidence):** Provenance capture requires snapshotting device configurations, but optimal format unclear. Options: TOML (matches hardware_config.toml), JSON (matches IR format), custom format. Low impact: affects metadata structure but not core functionality. Decide during Phase 4 planning based on daq-storage integration.

**Nested scan data dimensionality (LOW confidence):** 3D/4D nested scans create complex data structures (wavelength × XY position). Optimal HDF5 hierarchy unclear (flat vs nested groups). Low risk: defer to Phase 5, extensive precedent in scientific HDF5 usage. Can follow existing patterns from scipy/h5py.

## Sources

### Primary (HIGH confidence)

**Stack Research:**
- [egui-snarl GitHub](https://github.com/zakarumych/egui-snarl) — Node graph editor features, serde support, version compatibility
- [Rhai official documentation](https://rhai.rs/) — Scripting capabilities, on_progress for execution control, limitations
- [daggy crates.io](https://crates.io/crates/daggy) — DAG validation, cycle detection, topological sort
- [egui_plot crates.io](https://crates.io/crates/egui_plot) — Plotting performance characteristics, already integrated
- [undo crate GitHub](https://github.com/evenorog/undo) — Command pattern implementation, undo/redo architecture

**Feature Research:**
- [Bluesky Data Collection Framework](https://nsls-ii.github.io/bluesky/) — Adaptive plans, checkpoint system, metadata capture
- [PyMoDAQ Documentation](http://pymodaq.cnrs.fr/en/latest/) — Parameter scans, detector/actuator paradigm
- [labscript suite](https://labscriptsuite.org/) — Hardware-timed sequences, compiled execution model
- [ScopeFoundry](https://scopefoundry.org/) — Form-based scan builder, live plotting patterns
- [REPRODUCE-ME model](https://sheeba-samuel.github.io/REPRODUCE-ME/) — Scientific provenance requirements

**Architecture Research:**
- [Node graph architecture - Wikipedia](https://en.wikipedia.org/wiki/Node_graph_architecture) — Dataflow execution, cycle detection, graph evaluation
- [NI LabVIEW Compiler: Under the Hood](https://www.ni.com/en/support/documentation/supplemental/10/ni-labview-compiler--under-the-hood.html) — Compilation pipeline, DFIR intermediate representation
- [Bluesky GitHub](https://github.com/bluesky/bluesky) — Plan/RunEngine architecture patterns, document model
- [jsongraph specification](https://github.com/jsongraph/json-graph-specification) — JSON graph format standards
- [egui_node_graph2 crates.io](https://crates.io/crates/egui_node_editor) — Rust node editor patterns

**Pitfall Research:**
- [Designing your own node-based visual programming language - DEV](https://dev.to/cosmomyzrailgorynych/designing-your-own-node-based-visual-programming-language-2mpg) — Visual spaghetti, type safety, complexity management
- [Synchronization Between Models and Source Code - RAD Studio](https://docwiki.embarcadero.com/RADStudio/Athens/en/Synchronization_Between_Models_and_Source_Code) — Round-trip sync failures
- [You Don't Know Undo/Redo - DEV](https://dev.to/isaachagoel/you-dont-know-undoredo-4hol) — Undo/redo patterns, branching history
- [Investigating reproducibility and tracking provenance - BMC Bioinformatics](https://bmcbioinformatics.biomedcentral.com/articles/10.1186/s12859-017-1747-0) — Scientific provenance requirements
- [CheckMate: Evaluating Checkpointing Protocols - arXiv](https://arxiv.org/html/2403.13629v1) — Checkpoint consistency, state management

### Secondary (MEDIUM confidence)

- [Rete.js documentation](https://retejs.org/) — Node graph patterns, web-based reference implementation
- [Orange Data Mining](https://orangedatamining.com/home/visual-programming/) — Visual programming workflow patterns
- [Parameter Sweeps - PyRates Documentation](https://pyrates.readthedocs.io/en/latest/auto_analysis/parameter_sweeps.html) — Nested scan patterns
- [Ansys parameter sweeps guide](https://optics.ansys.com/hc/en-us/articles/360034922913-Creating-nested-parameter-sweeps) — Industrial nested scan implementations

### Tertiary (LOW confidence)

- [Automating the Practice of Science - arXiv](https://arxiv.org/html/2409.05890v1) — AI scientist limitations (informed anti-features)
- [Unreal Engine Blueprint organization](https://uhiyama-lab.com/en/notes/ue/blueprint-spaghetti-code-prevention-techniques/) — Visual organization techniques
- Community discussions on egui_node_graph alternatives, visual programming tradeoffs

---

**Research completed:** 2026-01-22
**Ready for roadmap:** Yes

**Next steps for orchestrator:**
1. Load SUMMARY.md as context for roadmap creation
2. Use phase suggestions as roadmap starting point
3. Apply research flags to determine which phases need `/gsd:research-phase`
4. Proceed to requirements definition with complete research context
