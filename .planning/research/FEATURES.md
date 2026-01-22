# Feature Landscape: Experiment Design Module

**Domain:** Scientific experiment design and DAQ scanning systems
**Researched:** 2026-01-22
**Confidence:** HIGH (verified with official documentation and multiple systems)

## Executive Summary

Experiment design systems in the scientific DAQ space exhibit a clear pattern: **interactive execution control and live data feedback are table stakes**, while **visual programming and adaptive intelligence are key differentiators**. The domain splits between code-first (Bluesky, labscript) and GUI-first (PyMoDAQ, ScopeFoundry, Orange) approaches, with modern systems increasingly supporting both workflows.

Key insight from rust-daq context: The project already has Bluesky-inspired RunEngine + Plan architecture, which provides a strong foundation. The experiment design module should focus on **GUI-first workflow** while maintaining code generation as an escape hatch for power users.

---

## Table Stakes Features

Features users expect in any experiment design system. Missing these = product feels incomplete or amateurish.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **Parameter Scans (1D/2D)** | Core use case: sweep motor/laser/voltage while acquiring | Medium | Grid, linear, list-based. All systems support this. PyMoDAQ, ScopeFoundry emphasize this. |
| **Pause/Resume/Abort** | Scientists need interactive control to save experiments from errors | Medium | Bluesky's "rewindable experiments" is gold standard. Checkpoint-based pause already in rust-daq. |
| **Live Plotting** | Immediate visual feedback separates DAQ from blind batch processing | High | Real-time updates while scanning. All modern systems have this. Handle high-FPS data carefully. |
| **Auto-Save to Disk** | Data loss = career catastrophe in science | Low | Stream to HDF5/CSV during acquisition, not after. rust-daq already has daq-storage. |
| **Device Discovery** | "What hardware is available?" must be obvious | Low | List available motors/detectors from registry. Already have device registry. |
| **Metadata Capture** | Reproducibility requirement: who/what/when/why | Medium | REPRODUCE-ME model: Data, Agent, Activity, Plan, Step, Setting, Instrument, Material. Bluesky excels here. |
| **Run History** | "What did I run yesterday?" | Medium | Browse past experiments, view parameters, rerun. StartDoc/StopDoc already tracked. |
| **Basic Sequences** | Move → Wait → Acquire → Repeat | Low | Fundamental building block. Already have PlanCommand primitives. |
| **Error Recovery** | Hardware fails, software crashes = restart without data loss | High | Checkpoint-based resume. Bluesky's interruption recovery is reference implementation. |
| **Export Data** | Get data out to analysis tools (Python, MATLAB, Origin) | Low | Multiple formats: CSV (universal), HDF5 (large data), Arrow (fast). Already in daq-storage. |

### Implementation Notes

**Pause/Resume Architecture (CRITICAL):**
- rust-daq already has `Checkpoint` in PlanCommand
- RunEngine must support: Pause → modify parameters → Resume
- **Constraint:** Cannot restructure running plan mid-execution (e.g., can't add new loop iteration)
- **Pattern:** State-based control (Idle → Running → Paused → Running → Complete/Aborted)
- **Reference:** Bluesky's checkpoint system, industrial PLCs use similar state machines

**Live Plotting Performance:**
- Challenge: High-FPS camera streams (30+ Hz) overwhelm GUI rendering
- **Solution:** Downsample for display, keep full data in storage
- **Pattern:** Separate data acquisition thread from UI update thread
- **Reference:** MATLAB's `drawnow limitrate`, ScopeFoundry's buffered updates

**Metadata Best Practices:**
- Auto-capture: timestamp, user, hostname, git commit hash
- User-provided: sample ID, experimental conditions, notes
- **Anti-pattern:** Requiring metadata entry before run starts (friction)
- **Better:** Optional notes field, can add metadata to completed runs retroactively

---

## Differentiators

Features that set a product apart. Not expected, but highly valued when present.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Visual Node-Based Builder** | Scientists think in flowcharts, not code | High | Orange, LabVIEW pattern. Drag nodes, connect edges. egui_node_graph crate available. |
| **Adaptive Plans** | Experiment responds to data (e.g., zoom into peak automatically) | High | Bluesky's killer feature. "Tailor acquisition flow based on monitored signals." |
| **One-Way Code Export** | Visual → Code gives escape hatch for complex scenarios | Medium | Generate Rhai/Python from graph. Don't attempt bidirectional (nightmare). |
| **Template Library** | Reusable experiment patterns (e.g., "wavelength calibration") | Low | Save/load partial graphs. Accelerates setup for common tasks. |
| **Multi-Detector Sync** | Coordinate camera + power meter + spectrometer simultaneously | Medium | PyMoDAQ's actuator/detector paradigm. Bluesky's device orchestration. |
| **Hardware-Timed Sequences** | μs-precision for fast experiments (pump-probe, etc.) | Very High | labscript's signature feature. **Out of scope for rust-daq initially** (no instruments support compiled sequences). |
| **PID Feedback Loops** | Auto-stabilize laser power, temperature, etc. | High | PyMoDAQ has this. Separate from experiment design (should be in daq-control module). |
| **Run Comparison** | Overlay plots from multiple runs, diff parameters | Medium | "Did changing X help?" Visual diff of metadata + plots. |
| **Smart Device Mapping** | Auto-suggest "This plan needs a Movable device" → shows available motors | Low | Type-based filtering using capability traits. Already have `Movable`, `Readable`, etc. |
| **Nested Scans** | 3D/4D scans (e.g., wavelength scan at each XY position) | Medium | Outer loop × inner loop. COMSOL, Ansys have parametric sweep nesting. |
| **Dry Run / Simulation** | Preview experiment timeline before executing | Medium | Show estimated duration, positions visited, data size. Prevents costly mistakes. |
| **Remote Operation** | Control lab from home/office | Medium | Already have gRPC. Need authentication. labscript supports distributed modules. |

### Implementation Notes

**Visual Node-Based Builder:**
- **Recommended:** `egui_node_graph` crate (mature, egui-native)
- Node types: Scan1D, Scan2D, Sequence, Loop, Conditional, MoveTo, Read, Trigger, Wait
- **Pattern:** Nodes generate PlanCommand sequences when executed
- **Complexity driver:** Graph validation (cycles, type checking, missing connections)

**Adaptive Plans:**
- **Example:** "If power drops below threshold, increase integration time automatically"
- **Implementation:** Plan yields `Conditional` command that evaluates predicate
- **Complexity driver:** Predicate language (simple comparisons? full expressions?)
- **Recommendation:** Start with simple thresholds, add scripting later

**One-Way Code Export:**
- **Why one-way:** Parsing arbitrary code into graph is AI-complete problem
- **Benefit:** Power users can manually edit generated code for edge cases
- **Format:** Rhai scripts (already in daq-scripting) or Python (Bluesky compatibility)
- **Pattern:** "Visual is source of truth, code is read-only preview"

**Nested Scans:**
- **Pattern:** Outer scan loops, inner scan at each point
- **Example:** Wavelength scan (700-900nm) at each XY grid position
- **Complexity:** Data dimensionality (2D position × 1D wavelength = 3D dataset)
- **Storage:** HDF5 hierarchical structure natural fit

---

## Anti-Features

Features to explicitly NOT build. Common mistakes in this domain.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Bidirectional Code ↔ Graph Sync** | Parsing arbitrary code to visual graph is fragile, high maintenance burden | One-way export (graph → code). Code is read-only preview. |
| **In-Graph Data Analysis** | Mixing acquisition and analysis creates bloated, slow UI | Separate analysis tool. Export to Jupyter/Python for complex analysis. Live plotting only for monitoring. |
| **Custom Hardware Timing Language** | Requires compiler + hardware support. labscript took years to mature. rust-daq instruments don't support compiled sequences. | Use software timing (adequate for >1ms timescales). Add hardware timing later if needed. |
| **Built-in Version Control** | Reinventing git poorly | Save experiments as files, let users use git. Export metadata includes git commit hash. |
| **Multi-User Collaboration** | Locking, conflict resolution, real-time sync = huge complexity | Single-operator assumption. Multiple users = multiple instances, not shared state. |
| **AI-Powered Experiment Design** | "AI scientist" systems have documented failure modes: benchmark gaming, data leakage, post-hoc selection bias | Adaptive plans with explicit predicates. Human designs experiment, computer executes. |
| **Graphical Programming for Everything** | Some tasks (complex math, conditionals) are clearer as code | Escape hatch to Rhai scripts for complex logic. Don't force flowchart metaphor where code is clearer. |
| **Real-Time OS Requirements** | Soft real-time (ms-scale) adequate for most optical/physics labs | Rely on tokio async runtime. Don't promise hard real-time (μs) guarantees. |
| **Embedded Analysis in Event Loop** | Running heavy computation (FFT, fitting) during acquisition causes lag/dropped frames | Stream raw data to disk, analyze afterward. Show simple metrics (mean, max) live, defer complex analysis. |

### Rationale Deep-Dive

**Why Not Bidirectional Sync?**
- Problem: Ambiguity (how to represent complex conditionals visually?)
- Problem: Code formatters/refactoring breaks visual mapping
- Evidence: Most visual programming tools (LabVIEW, Orange) are visual-first, code is secondary
- **rust-daq decision:** "Visual Source of Truth, Code Read-Only Preview" pattern (from PROJECT.md)

**Why Not Built-in Analysis?**
- Problem: Every user wants different analysis (Gaussian fit vs polynomial vs custom)
- Problem: Heavy computation blocks UI thread
- Evidence: Bluesky separates acquisition (bluesky) from analysis (databroker + custom scripts)
- **Better:** Export events stream, let users analyze in Jupyter/Python with full scipy/numpy stack

**Why Not AI Experiment Design?**
- Recent research (2025): AI scientist systems exhibit "data leakage, metric misuse, post-hoc selection bias"
- Problem: Scientists need to understand and justify experimental design
- **Better:** Computer-assisted (suggest parameter ranges based on history), not autonomous

---

## Feature Dependencies

```
Basic Execution Flow:
  Device Discovery → Scan Builder → Queue Plan → Execute → Live Plot → Auto-Save

Advanced Workflows:
  Template Library depends on Scan Builder
  Run Comparison depends on Run History + Auto-Save
  Adaptive Plans depends on Live Plotting (need current data to decide)
  Nested Scans depends on Basic Scans (composition)
  Remote Operation depends on Authentication (security)

Visual Builder:
  Node Editor depends on egui_node_graph crate
  Code Export depends on Node Editor
  Smart Device Mapping depends on Device Discovery + Capability Traits
```

**Critical Path for MVP:**
1. Device Discovery (already exists via registry)
2. Scan Builder (1D/2D parameter sweeps)
3. Execute + Pause/Resume (RunEngine already exists)
4. Live Plotting (high-value, high-complexity)
5. Auto-Save (already exists via daq-storage)

**Can Defer:**
- Visual node editor (start with form-based scan builder)
- Adaptive plans (advanced feature)
- Nested scans (composition of basic scans)
- Template library (nice-to-have)

---

## MVP Recommendation

For MVP, prioritize core execution loop over visual bells/whistles:

### Phase 1: Form-Based Scan Builder (no visual nodes yet)
1. ✅ **Device Discovery Panel** - List available Movable/Readable devices from registry
2. ✅ **Scan Configuration Form** - Text fields for start/stop/num_points
3. ✅ **Interactive Execution** - Start/Pause/Resume/Abort buttons
4. ✅ **Live Plotting** - Real-time line plot updates
5. ✅ **Auto-Save** - Stream to HDF5/CSV during run
6. ✅ **Run History** - List of past experiments with metadata

**Rationale:** Validate core workflow before investing in node editor. Many users comfortable with forms (PyMoDAQ, ScopeFoundry use forms heavily).

### Phase 2: Visual Experiment Builder
1. **Node-Based Editor** - egui_node_graph for drag-drop design
2. **Code Export** - Generate Rhai scripts from graph
3. **Template Library** - Save/load partial graphs
4. **Smart Device Mapping** - Type-based suggestions

**Rationale:** Once core execution proven, visual builder reduces friction for complex experiments.

### Defer to Post-MVP
- Adaptive plans (needs user feedback on predicates)
- Run comparison (needs data export solidified first)
- Nested scans (wait for user demand)
- Hardware-timed sequences (instruments don't support yet)
- PID feedback (separate module concern)

---

## Competitive Analysis Matrix

| Feature | Bluesky | labscript | PyMoDAQ | ScopeFoundry | rust-daq (Planned) |
|---------|---------|-----------|---------|--------------|-------------------|
| **Visual Builder** | ❌ Code-first | ❌ Code-first | ❌ Forms | ❌ Forms | ✅ Nodes (Phase 2) |
| **Pause/Resume** | ✅ Gold standard | ❌ Compiled sequences | ✅ Basic | ✅ Basic | ✅ Checkpoint-based |
| **Live Plotting** | ✅ Via callbacks | ❌ Post-run | ✅ Built-in | ✅ Built-in | ✅ Planned |
| **Adaptive Plans** | ✅ Killer feature | ❌ | ❌ | ❌ | ✅ Post-MVP |
| **Hardware Timing** | ❌ Software only | ✅ μs precision | ❌ Software | ❌ Software | ❌ Out of scope |
| **Multi-Detector** | ✅ Device orchestration | ✅ Multi-device | ✅ Detector/actuator | ✅ Multi-device | ✅ Already supported |
| **Metadata** | ✅ Comprehensive | ✅ Comprehensive | ⚠️ Basic | ⚠️ Basic | ✅ Document model |
| **Code Export** | N/A (code-first) | N/A (code-first) | ❌ | ❌ | ✅ One-way (Rhai) |
| **Language** | Python | Python | Python | Python | Rust + Rhai |
| **GUI Framework** | Varies (Qt, web) | Qt | Qt | Qt | egui |

**rust-daq Unique Position:**
- **Only system** combining visual builder + code export + headless daemon architecture
- **Only Rust-based** DAQ framework (performance, safety, native compilation)
- **Modern GUI** (egui) vs Qt legacy
- **Hybrid workflow** (GUI for design, code for complex scenarios)

---

## Domain-Specific Patterns

### Scan Patterns Taxonomy

| Pattern | Description | Complexity | Use Cases |
|---------|-------------|------------|-----------|
| **List Scan** | Visit explicit list of positions | Low | Wavelength calibration points, known sample positions |
| **Linear Scan** | Evenly spaced points between start/stop | Low | Most common: motor sweeps, voltage ramps |
| **Log Scan** | Logarithmically spaced points | Low | Frequency sweeps, concentration series |
| **Grid Scan (2D)** | Nested X/Y loops | Medium | Imaging, beam profiling |
| **Spiral Scan** | Outward spiral from center | Medium | Beam finding, sample centering |
| **Random Scan** | Random sampling within bounds | Medium | Statistical sampling, avoiding systematic errors |
| **Adaptive Scan** | Spacing determined by data | High | Peak finding, edge detection |
| **Snake Scan** | Zigzag pattern (no flyback) | Medium | Fast imaging, minimizes motor reversals |

**rust-daq Priority:** Linear (MVP), Grid (MVP), List (Phase 2), others (user-driven)

### Sequence Patterns Taxonomy

| Pattern | Description | Complexity | Use Cases |
|---------|-------------|------------|-----------|
| **Move-Acquire** | Position device, read detector | Low | Single-point measurements |
| **Move-Wait-Acquire** | Allow settling time | Low | Motor stabilization, thermal equilibrium |
| **Multi-Detector Acquire** | Read multiple detectors simultaneously | Medium | Correlate camera + power meter |
| **Loop-N-Times** | Repeat sequence N times | Low | Averaging, time series |
| **Loop-Until-Condition** | Repeat until predicate true | Medium | Stabilization, threshold crossing |
| **Conditional Branch** | If-then-else logic | High | Adaptive experiments |
| **Parallel Actions** | Simultaneous device control | High | Requires async coordination |

**rust-daq Priority:** First 4 (MVP), conditional (Phase 2), parallel (advanced)

---

## User Personas & Feature Priorities

### Persona 1: Graduate Student (Novice)
**Needs:** Quick start, minimal learning curve, can't afford mistakes
**Priorities:**
1. Form-based scan builder (intuitive)
2. Live plotting (immediate feedback)
3. Auto-save (prevent data loss)
4. Templates (learn from examples)

**Low Priority:**
- Code export (doesn't write code)
- Adaptive plans (too complex initially)

### Persona 2: Postdoc (Intermediate)
**Needs:** Efficiency, reproducibility, occasional custom logic
**Priorities:**
1. Visual node builder (faster than forms)
2. Run history + comparison (iterate experiments)
3. Metadata capture (publish papers)
4. Code export (escape hatch for edge cases)

**Low Priority:**
- Hardware timing (experiments are slow anyway)

### Persona 3: Lab Manager (Expert)
**Needs:** Reliability, maintainability, team collaboration
**Priorities:**
1. Template library (standardize lab procedures)
2. Error recovery (minimize downtime)
3. Remote operation (monitor overnight runs)
4. Export to multiple formats (different analysis tools)

**Low Priority:**
- Visual builder (comfortable with code)
- AI features (wants predictable behavior)

---

## Complexity Assessment

| Feature | LOC Estimate | Risk Factors | Dependencies |
|---------|--------------|--------------|--------------|
| Device Discovery Panel | 200 | Low | Registry already exists |
| Form-Based Scan Builder | 500 | Low | UI forms straightforward |
| Live Plotting | 1500 | **High** | egui_plot, threading, downsampling |
| Node Editor Integration | 2000 | **High** | egui_node_graph learning curve, graph validation |
| Code Export (Rhai) | 800 | Medium | AST generation, daq-scripting integration |
| Adaptive Plans | 1200 | **High** | Predicate evaluation engine, safety |
| Run Comparison | 600 | Medium | Data loading, overlay plotting |
| Template Library | 400 | Low | Serialization, file management |

**High-Risk Items Need Prototyping:**
- Live plotting: Test with high-FPS camera, measure UI lag
- Node editor: Evaluate egui_node_graph against requirements
- Adaptive plans: Design predicate language, safety mechanisms

---

## Open Questions for Phase-Specific Research

1. **Live Plotting:** What FPS can egui_plot handle before dropping frames? Need benchmarking.
2. **Node Editor:** Does egui_node_graph support dynamic type checking (e.g., can't connect camera to motor input)?
3. **Adaptive Plans:** What predicate language? Rhai expressions? Custom DSL?
4. **Data Storage:** How to efficiently store 3D/4D nested scan data in HDF5? Flat vs hierarchical?
5. **Remote Operation:** What authentication scheme? TLS certificates? OAuth?

**Recommendation:** Defer these until respective phases. Don't over-research upfront.

---

## Sources

### Official Documentation
- [Bluesky Data Collection Framework](https://nsls-ii.github.io/bluesky/)
- [PyMoDAQ Documentation](http://pymodaq.cnrs.fr/en/latest/)
- [labscript suite](https://labscriptsuite.org/)
- [ScopeFoundry](https://scopefoundry.org/)
- [Orange Data Mining - Visual Programming](https://orangedatamining.com/home/visual-programming/)
- [Qudi Framework](https://ulm-iqo.github.io/qudi-core/getting_started.html)
- [LabVIEW Graphical Programming - NI](https://www.ni.com/en/shop/labview.html)

### Research Papers & Technical Resources
- [Qudi: A modular python suite for experiment control](https://www.sciencedirect.com/science/article/pii/S2352711017300055)
- [The role of metadata in reproducible computational research](https://pmc.ncbi.nlm.nih.gov/articles/PMC8441584/)
- [REPRODUCE-ME: Reproducibility of Scientific Experiments](https://sheeba-samuel.github.io/REPRODUCE-ME/)
- [Automating the Practice of Science – Opportunities, Challenges, and Implications](https://arxiv.org/html/2409.05890v1)
- [The More You Automate, the Less You See: Hidden Pitfalls of AI Scientist Systems](https://arxiv.org/html/2509.08713v1)

### Community Resources
- [GitHub - bluesky/bluesky](https://github.com/bluesky/bluesky)
- [GitHub - PyMoDAQ/PyMoDAQ](https://github.com/PyMoDAQ/PyMoDAQ)
- [GitHub - labscript-suite](https://github.com/labscript-suite)
- [GitHub - ScopeFoundry/ScopeFoundry](https://github.com/ScopeFoundry/ScopeFoundry)
- [Parameter Sweeps - PyRates Documentation](https://pyrates.readthedocs.io/en/latest/auto_analysis/parameter_sweeps.html)
- [Creating nested parameter sweeps – Ansys Optics](https://optics.ansys.com/hc/en-us/articles/360034922913-Creating-nested-parameter-sweeps)

### Industrial Patterns
- [Implementing a Pause/Resume Task Engine - Industrial Monitor Direct](https://industrialmonitordirect.com/blogs/knowledgebase/implementing-a-pauseresume-task-engine-for-graceful-process-control-in-plc-ladder-logic)
- [Common experiment design pitfalls - Statsig](https://www.statsig.com/perspectives/experiment-design-pitfalls)

---

**Confidence Notes:**
- **HIGH confidence** on table stakes features (verified across 5+ systems)
- **HIGH confidence** on anti-features (backed by recent research on failures)
- **MEDIUM confidence** on complexity estimates (no rust-daq implementation yet)
- **LOW confidence** on exact LOC estimates (order-of-magnitude only)
