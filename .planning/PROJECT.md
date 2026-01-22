# Experiment Design Module

## What This Is

A visual experiment designer for rust-daq that lets scientists design, execute, and manage data acquisition experiments through an intuitive GUI. Users can build parameter scans and sequences visually (node-based editor) or via code, with interactive execution that supports pause, mid-run adjustments, and resume. The module builds on the existing Bluesky-inspired RunEngine and Plan abstraction.

## Core Value

Scientists can design and interactively run experiments without writing code, while power users retain full programmatic control — both workflows produce the same executable Plans.

## Requirements

### Validated

- RunEngine with pause/resume/abort capabilities — existing
- Plan abstraction (GridScan, TimeSeries, VoltageScan, etc.) — existing
- Document streaming (Start, Descriptor, Event, Stop) — existing
- Device registry with capability traits — existing
- gRPC communication between GUI and daemon — existing

### Active

- [ ] Node-based visual experiment builder using egui_node_graph
- [ ] Scan builder: configure 1D/2D parameter sweeps across any actuator
- [ ] Sequence composer: order steps (move, wait, acquire, loop)
- [ ] Code editor with syntax highlighting for Rhai scripts
- [ ] One-way code generation: visual graph → Rhai export
- [ ] Interactive execution panel: start, pause, modify parameters, resume, abort
- [ ] Live plotting: real-time visualization as data is acquired
- [ ] Auto-save: stream data to disk (HDF5/CSV) during acquisition
- [ ] Metadata capture: attach arbitrary key-value pairs to runs
- [ ] Run history: browse, search, and compare previous experiments
- [ ] Experiment templates: save/load partial graphs as reusable components
- [ ] Version history: track changes to experiment designs

### Out of Scope

- Round-trip code parsing (code → visual graph) — extremely difficult, diminishing returns
- Hardware timing compilation (labscript-style) — not needed for current instruments
- Multi-user collaboration — single-operator system for now
- Cloud storage for experiments — local filesystem sufficient

## Context

**Existing Architecture:**
- `daq-experiment` crate: RunEngine, Plan trait, PlanCommand, Checkpoint support
- `daq-egui` crate: existing GUI with device panels, image viewer
- `daq-scripting` crate: Rhai integration for scriptable experiments
- Document model: StartDoc, DescriptorDoc, EventDoc, StopDoc

**Reference Systems Studied:**
- labscript: hardware-timed sequences, compilation model
- ScopeFoundry: visual sequencer, sweep patterns
- PyMoDAQ: DAQ scanning, actuator/detector paradigm
- Bluesky: RunEngine architecture (already adopted)
- Orange Data Mining: node-based UX for scientific workflows
- Qudi: modular GUI with paired logic components

**Key Insight:** Use "Visual Source of Truth, Code Read-Only Preview" pattern — the node graph is the master representation, code is an export format.

## Constraints

- **UI Framework**: egui — must integrate with existing daq-egui application
- **Execution Backend**: existing RunEngine — don't reinvent, extend
- **Mid-run Modification**: Parameter injection only — can't restructure running Plan
- **Storage**: existing daq-storage crate (HDF5, CSV, Arrow support)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Node-based visual editor | Standard paradigm for scientific workflows (Orange, LabVIEW) | — Pending |
| One-way code generation | Round-trip parsing is fragile and rarely worth the complexity | — Pending |
| egui_node_graph crate | Mature egui node editor, avoids building from scratch | — Pending |
| Parameter injection for live edits | RunEngine Checkpoints already support pause; modifying Plan structure mid-run is unsafe | — Pending |

---
*Last updated: 2025-01-22 after initialization*
