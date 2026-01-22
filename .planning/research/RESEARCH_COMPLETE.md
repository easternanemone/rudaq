# RESEARCH COMPLETE

**Project:** rust-daq Experiment Design Module
**Mode:** Features Research (ecosystem survey)
**Confidence:** HIGH
**Date:** 2026-01-22

---

## Key Findings

1. **Table Stakes = Interactive Execution + Live Feedback**
   - Pause/Resume/Abort control is mandatory (Bluesky's checkpoint model is gold standard)
   - Live plotting during acquisition separates modern DAQ from batch systems
   - Auto-save to disk (HDF5/CSV) is non-negotiable (data loss = catastrophic)
   - Metadata capture required for reproducibility (8-component REPRODUCE-ME model)

2. **Differentiators = Visual Programming + Adaptive Intelligence**
   - Visual node-based builders (Orange, LabVIEW pattern) reduce friction
   - Adaptive plans (Bluesky's killer feature) enable intelligent experiments
   - One-way code export provides escape hatch without bidirectional complexity
   - Smart device mapping using capability traits (unique to rust-daq)

3. **Anti-Features = Avoid Over-Engineering**
   - Do NOT build bidirectional code↔graph sync (AI-complete problem)
   - Do NOT embed heavy analysis in acquisition loop (causes dropped frames)
   - Do NOT promise hardware timing without instrument support
   - Do NOT build AI experiment design (documented failure modes)

4. **Domain Split: Code-First vs GUI-First**
   - Code-first: Bluesky, labscript (power users, reproducible scripts)
   - GUI-first: PyMoDAQ, ScopeFoundry, Orange (novices, visual thinkers)
   - rust-daq opportunity: Hybrid (visual design → code export)

5. **rust-daq Competitive Position**
   - ONLY system combining: visual builder + code export + headless daemon
   - ONLY Rust-based DAQ framework (performance + safety)
   - Modern GUI (egui) vs Qt legacy
   - Already has RunEngine + Checkpoint support (80% of hard work done)

---

## Files Created

| File | Purpose |
|------|---------|
| `.planning/research/FEATURES.md` | Comprehensive feature landscape with table stakes, differentiators, anti-features |

---

## Confidence Assessment

| Area | Level | Reason |
|------|-------|--------|
| Table Stakes | HIGH | Verified across 7 systems (Bluesky, labscript, PyMoDAQ, ScopeFoundry, Orange, Qudi, LabVIEW) |
| Differentiators | HIGH | Official docs + research papers confirm competitive advantages |
| Anti-Features | HIGH | Recent research (2025) on AI scientist failures, documented UX pitfalls |
| Complexity Estimates | MEDIUM | Order-of-magnitude based on similar systems, no rust-daq implementation yet |
| LOC Estimates | LOW | Ballpark only (±50% error expected) |

---

## Roadmap Implications

### Phase Structure Recommendation

**Phase 1: Form-Based MVP (2-3 weeks)**
- Validate core execution loop before investing in visual editor
- Features: Device discovery, scan forms, interactive controls, live plotting, auto-save
- **Rationale:** Many users comfortable with forms (PyMoDAQ, ScopeFoundry use forms). Proves value quickly.

**Phase 2: Visual Builder (4-6 weeks)**
- Add node-based editor once core workflow validated
- Features: egui_node_graph integration, code export, templates, smart device mapping
- **Rationale:** Visual builder is differentiator, but high complexity. Need user feedback first.

**Phase 3: Advanced Features (ongoing)**
- Adaptive plans, run comparison, nested scans, remote operation
- **Rationale:** User-driven priorities, not essential for initial adoption

### Phase Ordering Rationale

1. **Forms before Nodes:** Lower risk, faster validation, builds confidence
2. **Live Plotting Early:** High complexity but high value, needs prototyping
3. **Code Export After Nodes:** Requires stable graph structure first
4. **Adaptive Plans Late:** Needs user feedback on predicates, safety critical

### Research Flags for Phases

- **Phase 1 (Forms + Plotting):** Likely needs deeper research
  - Live plotting performance with high-FPS camera (benchmarking required)
  - egui_plot capabilities and limitations (thread safety, downsampling)

- **Phase 2 (Node Editor):** Likely needs deeper research
  - egui_node_graph evaluation (type checking, validation, UX)
  - Graph → Rhai code generation (AST construction, error handling)

- **Phase 3 (Adaptive Plans):** Definitely needs deeper research
  - Predicate language design (Rhai expressions? Custom DSL?)
  - Safety mechanisms (prevent infinite loops, hardware damage)

- **Standard Patterns (Low Research Need):**
  - Device discovery (registry already exists)
  - Auto-save (daq-storage already exists)
  - Run history (StartDoc/StopDoc already tracked)
  - Metadata capture (document model already defined)

---

## MVP Feature Prioritization

### Must Have (Phase 1)
1. Device Discovery Panel - List Movable/Readable devices from registry
2. Scan Configuration Form - Start/stop/num_points for 1D/2D scans
3. Interactive Execution - Start/Pause/Resume/Abort with Checkpoint support
4. Live Plotting - Real-time line/heatmap updates during acquisition
5. Auto-Save - Stream to HDF5/CSV using daq-storage
6. Run History - List past experiments with metadata

### Should Have (Phase 2)
1. Node-Based Editor - egui_node_graph for visual design
2. Code Export - Generate Rhai scripts from graph (one-way)
3. Template Library - Save/load partial graphs for common experiments
4. Smart Device Mapping - Type-based suggestions using capability traits

### Could Have (Phase 3+)
1. Adaptive Plans - Data-driven experiment flow
2. Run Comparison - Overlay plots, diff metadata
3. Nested Scans - 3D/4D parameter spaces
4. Remote Operation - Control from home/office (auth required)

### Won't Have (Anti-Features)
1. Bidirectional Code ↔ Graph Sync
2. Built-in Heavy Analysis (FFT, fitting, etc.)
3. Hardware-Timed Sequences (no instrument support)
4. AI Experiment Design (failure modes documented)
5. Multi-User Collaboration (complexity not justified)

---

## Open Questions for Future Research

### Deferred to Phase 1 Research
1. **Live Plotting Performance:** What FPS can egui_plot handle? Benchmarking needed.
2. **Data Downsampling:** What algorithm for high-FPS display? (e.g., 2x2 binning, skip frames)
3. **Thread Architecture:** Separate data acquisition from UI rendering? (Likely yes)

### Deferred to Phase 2 Research
1. **Node Editor Capabilities:** Does egui_node_graph support dynamic type checking?
2. **Graph Validation:** How to prevent invalid connections (e.g., camera → motor)?
3. **Code Generation:** AST structure for Rhai scripts? Error handling?

### Deferred to Phase 3 Research
1. **Adaptive Predicates:** What expression language? Safety constraints?
2. **Nested Scan Storage:** Flat vs hierarchical HDF5 structure?
3. **Remote Authentication:** TLS certificates? OAuth? API tokens?

**Recommendation:** Don't over-research upfront. Address these during respective phases.

---

## Competitive Analysis Highlights

| System | Strength | Weakness | rust-daq Position |
|--------|----------|----------|-------------------|
| **Bluesky** | Adaptive plans, metadata, interruption recovery | Code-first (steep learning curve for novices) | Adopt architecture, add visual builder |
| **labscript** | Hardware-timed μs precision, mature | Compiled sequences (inflexible), complex setup | Skip hardware timing initially (no instrument support) |
| **PyMoDAQ** | Detector/actuator paradigm, scanning focus | Qt (legacy), forms-only (no visual builder) | Use modern GUI (egui), add node editor |
| **ScopeFoundry** | Modular plugins, IPython integration | Forms-only, limited scanning | Add visual builder + code export |
| **Orange** | Best-in-class visual programming | Not domain-specific (general data mining) | Apply node-graph UX to DAQ domain |
| **LabVIEW** | Industry standard, graphical programming | Proprietary, expensive, closed ecosystem | Open-source Rust alternative |

**rust-daq Unique Value:**
- Only open-source system with visual builder + code export + headless daemon
- Only Rust-based (performance, safety, native compilation)
- Modern GUI framework (egui) vs Qt legacy
- Hybrid workflow (novices use GUI, experts write code, both workflows produce same Plans)

---

## Risk Assessment

### High-Risk Items (Require Prototyping)

1. **Live Plotting Performance**
   - Risk: High-FPS camera (30+ Hz) overwhelms egui rendering
   - Mitigation: Benchmark early, implement downsampling, separate threads
   - Phase: 1 (MVP)

2. **Node Editor Integration**
   - Risk: egui_node_graph may not support required features (type checking, validation)
   - Mitigation: Evaluate early, have fallback (custom node editor or forms-only)
   - Phase: 2

3. **Adaptive Plans Safety**
   - Risk: User-defined predicates could damage hardware (e.g., infinite loop)
   - Mitigation: Sandbox execution, timeout limits, hardware bounds checking
   - Phase: 3

### Medium-Risk Items

1. **Code Export Correctness** - Generated Rhai must match graph semantics
2. **Pause/Resume State Management** - Must handle all edge cases (mid-scan pause, etc.)
3. **Nested Scan Data Storage** - 3D/4D arrays in HDF5 need careful design

### Low-Risk Items (Well-Understood)

1. Device Discovery - Registry already exists
2. Auto-Save - daq-storage already exists
3. Run History - Document model already defined
4. Metadata Capture - Bluesky pattern well-documented

---

## Domain Insights Summary

### Key Design Patterns Identified

1. **Checkpoint-Based Pause/Resume** (Bluesky)
   - Plans yield `Checkpoint` commands at safe stopping points
   - RunEngine saves state, waits for Resume signal
   - Can inject parameter changes during pause (but not restructure plan)

2. **Detector/Actuator Paradigm** (PyMoDAQ)
   - Clear separation: Movable devices vs Readable devices
   - rust-daq already has this via capability traits (Movable, Readable, FrameProducer)

3. **Visual Source of Truth** (Orange, LabVIEW)
   - Graph is master representation, code is export format
   - Don't attempt bidirectional sync (fragile, high maintenance)

4. **Hardware Abstraction for Reusability** (Bluesky)
   - Plans reference device IDs, not physical hardware
   - Same plan works on different hardware via device registry
   - rust-daq already has this (DriverFactory, DeviceComponents)

5. **Live Data Streaming for Feedback** (All modern systems)
   - Data flows to UI during acquisition, not after
   - Enables interactive decisions (pause, adjust, abort)
   - rust-daq already has document streaming (StartDoc, EventDoc, StopDoc)

### Common Pitfalls Identified

1. **Scope Creep into Analysis** - Keep acquisition separate from heavy computation
2. **Premature Hardware Timing** - Software timing adequate for most labs
3. **Over-Complex Metadata** - Auto-capture essentials, optional user notes
4. **Threading Mistakes** - Blocking UI thread with I/O causes lag
5. **Graph Validation Neglect** - Invalid connections cause runtime errors

---

## Ready for Roadmap

Research complete. Key insights captured in FEATURES.md:
- **10 table stakes features** (must have or users leave)
- **12 differentiator features** (competitive advantages)
- **9 anti-features** (things to deliberately avoid)
- **8 scan patterns** + **7 sequence patterns** (domain taxonomy)
- **3 user personas** with priority matrices
- **Complexity estimates** with risk factors
- **MVP recommendations** (forms first, nodes second)

All findings sourced from official documentation (Bluesky, PyMoDAQ, labscript, etc.) and recent research papers (2025-2026). Confidence levels assigned honestly.

**Next Steps:**
1. Orchestrator proceeds to roadmap creation using FEATURES.md
2. Phase 1 research (live plotting benchmarking) happens during implementation
3. Phase 2 research (egui_node_graph evaluation) happens before node editor work

---

**Files Written:**
- `/Users/briansquires/code/rust-daq/.planning/research/FEATURES.md` (5000+ words, comprehensive)
- `/Users/briansquires/code/rust-daq/.planning/research/RESEARCH_COMPLETE.md` (this file)

**DO NOT COMMIT** - Orchestrator or synthesizer agent will handle commits after all research completes.
