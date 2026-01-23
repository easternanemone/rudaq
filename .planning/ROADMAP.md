# Roadmap: Experiment Design Module

## Overview

Build a visual experiment designer for rust-daq that transforms how scientists create data acquisition experiments. Starting with a form-based scan builder to validate the core execution loop, we'll progressively add node-based visual editing, live execution control with pause/resume, real-time visualization, and complete data provenance. The journey delivers both a GUI-first workflow for novice users and code export capabilities for power users, all built on rust-daq's existing RunEngine and Plan architecture.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Form-Based Scan Builder** - Validate core execution loop with simple forms
- [x] **Phase 2: Node Graph Editor Core** - Visual editing foundation with undo/validation
- [x] **Phase 3: Plan Translation and Execution** - Connect visual graph to RunEngine
- [x] **Phase 4: Sequences and Control Flow** - Sequence composition with loops
- [x] **Phase 5: Live Visualization** - Real-time plotting during acquisition
- [x] **Phase 6: Data Management** - Auto-save, metadata, and run history
- [ ] **Phase 7: Code Export and Provenance** - Rhai generation and versioning
- [ ] **Phase 8: Advanced Scans** - Nested and adaptive scans
- [ ] **Phase 9: Templates and Library** - Reusable experiment patterns (including subgraph grouping)
- [ ] **Phase 10: Polish and Integration** - Final features and production readiness

## Phase Details

### Phase 1: Form-Based Scan Builder
**Goal**: Scientists can configure and execute 1D/2D scans using simple forms, with live plotting and auto-save
**Depends on**: Nothing (first phase)
**Requirements**: SCAN-01, SCAN-02, EXEC-01, EXEC-02, VIZ-01, DATA-01
**Success Criteria** (what must be TRUE):
  1. User can discover available devices from registry and select actuators/detectors
  2. User can configure 1D line scan (start/stop/points) and 2D grid scan via form fields
  3. User can start experiment and see live plot updating as data is acquired
  4. User can abort running experiment immediately, with data saved up to abort point
  5. Data auto-saves to HDF5 or CSV during acquisition without user intervention
**Plans**: 3 plans in 3 waves

Plans:
- [x] 01-01-PLAN.md — Device selection and form layout (ScanBuilderPanel foundation)
- [x] 01-02-PLAN.md — Execution and live 1D plotting (Start/Abort, document streaming)
- [x] 01-03-PLAN.md — 2D grid scan and completion summary (2D visualization, polish)

### Phase 2: Node Graph Editor Core
**Goal**: Scientists can visually build experiments by dragging nodes and connecting wires, with validation and undo
**Depends on**: Phase 1
**Requirements**: EDIT-01, EDIT-02, EDIT-04, EDIT-05, LIB-01, LIB-02
**Success Criteria** (what must be TRUE):
  1. User can drag node from palette onto canvas and connect nodes with wires
  2. User can configure node parameters via property inspector panel
  3. User can undo/redo edits with Ctrl+Z/Ctrl+Y, with full edit history
  4. Editor shows validation errors visually (status bar, property inspector) when nodes invalid
**Deferred to Phase 9**: EDIT-03 (subgraph grouping) - requires stable node graph foundation first
**Plans**: 4 plans in 3 waves

Plans:
- [x] 02-01-PLAN.md — Graph module foundation (egui-snarl integration, ExperimentNode, basic canvas)
- [x] 02-02-PLAN.md — Node palette and wire connections (drag-drop creation, validation)
- [x] 02-03-PLAN.md — Property inspector and undo/redo (node editing, command pattern)
- [x] 02-04-PLAN.md — Serialization and polish (JSON save/load, visual error display)

### Phase 3: Plan Translation and Execution
**Goal**: Experiments designed visually translate to executable Plans and run via RunEngine
**Depends on**: Phase 2
**Requirements**: EXEC-03, EXEC-04, EXEC-05, EXEC-06
**Success Criteria** (what must be TRUE):
  1. User can execute experiment from node graph editor, with visual feedback of running nodes
  2. User can pause running experiment at checkpoint, modify device parameters, and resume
  3. User sees current progress (step N of M, percentage, estimated time remaining)
  4. Validation errors prevent execution (missing devices, invalid parameters, cycles in graph)
**Plans**: 4 plans in 3 waves

Plans:
- [x] 03-01-PLAN.md — DaqClient engine control + graph translation with cycle detection
- [x] 03-02-PLAN.md — Execution state tracking + Run/Pause/Resume UI + progress display
- [x] 03-03-PLAN.md — Parameter editing while paused
- [x] 03-04-PLAN.md — Integration and human verification

### Phase 4: Sequences and Control Flow
**Goal**: Scientists can compose multi-step sequences with moves, waits, acquire, and loops
**Depends on**: Phase 3
**Requirements**: SEQ-01, SEQ-02, SEQ-03, SEQ-04
**Success Criteria** (what must be TRUE):
  1. User can add move command node (absolute/relative position) to sequence
  2. User can add wait/delay node with configurable duration for settling time
  3. User can add acquire node to trigger detector data capture
  4. User can create loop node to repeat sequence N times or until condition met
**Plans**: 3 plans in 2 waves

Plans:
- [x] 04-01-PLAN.md — Enhanced node types and device selector widget
- [x] 04-02-PLAN.md — Property inspector panels for Move, Wait, Acquire, Loop nodes
- [x] 04-03-PLAN.md — Loop body translation and validation

### Phase 5: Live Visualization
**Goal**: Scientists see real-time plots and images updating during acquisition
**Depends on**: Phase 1 (extends live plotting), Phase 3 (execution infrastructure)
**Requirements**: VIZ-02, VIZ-03
**Success Criteria** (what must be TRUE):
  1. User sees live camera frames displayed in image viewer during acquisition
  2. Plots auto-scale to data range automatically, with manual override option
  3. Multiple plots update simultaneously for multi-detector experiments
**Plans**: 5 plans in 3 waves

Plans:
- [x] 05-01-PLAN.md — Auto-scale plot wrapper with grow-to-fit logic and per-axis lock controls
- [x] 05-02-PLAN.md — Multi-detector grid layout panel using StripBuilder
- [x] 05-03-PLAN.md — Live visualization panel integrating cameras and plots with FPS status
- [x] 05-04-PLAN.md — Integration with experiment execution (auto-spawn visualization on run)
- [x] 05-05-PLAN.md — Gap closure: Wire camera/document streaming to visualization channels

### Phase 6: Data Management
**Goal**: Complete metadata capture, run history browsing, and comparison tools
**Depends on**: Phase 1 (auto-save), Phase 3 (execution)
**Requirements**: DATA-02, DATA-03, DATA-04, DATA-05
**Success Criteria** (what must be TRUE):
  1. Metadata captured automatically with each run (device settings, timestamps, scan parameters)
  2. User can add custom notes and tags to experiments during or after execution
  3. User can browse run history, search by metadata, and view previous results
  4. User can compare data from multiple runs with overlaid plots
**Plans**: 4 plans in 2 waves

Plans:
- [x] 06-01-PLAN.md — Metadata editor widget and StartDoc enrichment
- [x] 06-02-PLAN.md — Run history panel with filtering
- [x] 06-03-PLAN.md — HDF5 post-acquisition annotation
- [x] 06-04-PLAN.md — Multi-run comparison panel

### Phase 7: Code Export and Provenance
**Goal**: Complete provenance tracking with one-way code generation for inspection
**Depends on**: Phase 2 (node graph), Phase 3 (execution)
**Requirements**: CODE-01, CODE-02, CODE-03, CODE-04
**Success Criteria** (what must be TRUE):
  1. User sees live code preview pane showing generated Rhai script for current graph
  2. User can export experiment as standalone Rhai script file
  3. User can switch to script editor mode (eject from visual, edit code directly)
  4. Generated code is readable with comments explaining each step
  5. Every experiment run captures complete provenance (graph version, git commit, device states)
**Plans**: 4 plans in 2 waves

Plans:
- [ ] 07-01-PLAN.md — Code generation engine (ExperimentNode to Rhai translation)
- [ ] 07-02-PLAN.md — Provenance tracking (git commit, graph hash in ExperimentManifest)
- [ ] 07-03-PLAN.md — Code preview panel with syntax highlighting
- [ ] 07-04-PLAN.md — Export and script editor mode

### Phase 8: Advanced Scans
**Goal**: Nested multi-dimensional scans and adaptive plans responding to data
**Depends on**: Phase 1 (basic scans), Phase 4 (sequences)
**Requirements**: SCAN-03, SCAN-04
**Success Criteria** (what must be TRUE):
  1. User can create nested scan (outer wavelength loop, inner XY position scan)
  2. User can create adaptive scan that adjusts parameters based on acquired data (e.g., zoom into peak)
  3. Nested scans execute in correct order (outer × inner loops) with proper data dimensionality
**Plans**: TBD

Plans:
- [ ] 08-01: TBD during plan-phase
- [ ] 08-02: TBD during plan-phase

### Phase 9: Templates and Library
**Goal**: Reusable experiment patterns and subgraph grouping accelerate common tasks
**Depends on**: Phase 2 (save/load), Phase 7 (versioning)
**Requirements**: LIB-03, LIB-04, LIB-05, EDIT-03 (subgraph grouping - deferred from Phase 2)
**Success Criteria** (what must be TRUE):
  1. User can access template library with common patterns (wavelength cal, beam alignment)
  2. User can create custom template from their experiment and add to library
  3. Templates have version history tracking changes over time
  4. Templates include metadata (author, description, parameter documentation)
  5. User can group related nodes into collapsible subgraph to simplify complex experiments
**Plans**: TBD

Plans:
- [ ] 09-01: TBD during plan-phase
- [ ] 09-02: TBD during plan-phase

### Phase 10: Polish and Integration
**Goal**: Production-ready system with performance optimization and complete testing
**Depends on**: All previous phases
**Requirements**: None (integration and polish)
**Success Criteria** (what must be TRUE):
  1. Large graphs (50+ nodes) render smoothly without UI lag
  2. High-FPS camera streams display without frame drops via downsampling
  3. All validation messages are clear and actionable for users
  4. Complete user documentation and tutorial experiments available
  5. Integration tests cover all major workflows end-to-end
**Plans**: TBD

Plans:
- [ ] 10-01: TBD during plan-phase
- [ ] 10-02: TBD during plan-phase

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9 -> 10

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Form-Based Scan Builder | 3/3 | Complete | 2026-01-22 |
| 2. Node Graph Editor Core | 4/4 | Complete | 2026-01-22 |
| 3. Plan Translation and Execution | 4/4 | Complete (human approved) | 2026-01-22 |
| 4. Sequences and Control Flow | 3/3 | Complete | 2026-01-22 |
| 5. Live Visualization | 5/5 | Complete | 2026-01-22 |
| 6. Data Management | 4/4 | Complete | 2026-01-22 |
| 7. Code Export and Provenance | 0/4 | Planned | - |
| 8. Advanced Scans | 0/TBD | Not started | - |
| 9. Templates and Library | 0/TBD | Not started | - |
| 10. Polish and Integration | 0/TBD | Not started | - |
