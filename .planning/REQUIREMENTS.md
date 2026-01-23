# Requirements: Experiment Design Module

**Defined:** 2025-01-22
**Core Value:** Scientists can design and interactively run experiments without writing code, while power users retain full programmatic control.

## v1 Requirements

Requirements for initial release. Each maps to roadmap phases.

### Scan Building

- [ ] **SCAN-01**: User can create 1D line scans (sweep single parameter with configurable start/stop/points)
- [ ] **SCAN-02**: User can create 2D grid scans (sweep two parameters in raster pattern)
- [ ] **SCAN-03**: User can create nested scans (outer/inner loops, e.g., wavelength at each position)
- [ ] **SCAN-04**: User can create adaptive scans (adjust scan based on acquired data, e.g., find peak and zoom)

### Sequences

- [x] **SEQ-01**: User can add move commands (position actuators with absolute or relative moves)
- [x] **SEQ-02**: User can add wait/delay steps (pause for settling time, configurable duration)
- [x] **SEQ-03**: User can add acquire commands (trigger detectors, capture data points)
- [x] **SEQ-04**: User can create loops (repeat sequences N times or until condition met)

### Visual Editor

- [ ] **EDIT-01**: User can drag-drop nodes onto canvas and connect with wires
- [ ] **EDIT-02**: User can configure node parameters via property inspector panel
- [ ] **EDIT-03**: User can group nodes into subgraphs (collapse complex sections)
- [ ] **EDIT-04**: User can undo/redo edits with full history (Ctrl+Z/Ctrl+Y)
- [ ] **EDIT-05**: Editor validates connections and shows errors visually

### Execution Control

- [ ] **EXEC-01**: User can start experiment execution from the editor
- [ ] **EXEC-02**: User can stop/abort a running experiment immediately
- [ ] **EXEC-03**: User can pause a running experiment at checkpoints
- [ ] **EXEC-04**: User can resume a paused experiment
- [ ] **EXEC-05**: User can modify device parameters while paused (exposure, etc.)
- [ ] **EXEC-06**: User can see progress (current step, completion percentage, estimated time)

### Live Visualization

- [ ] **VIZ-01**: User sees live line plots updating as scalar data is acquired
- [x] **VIZ-02**: User sees live image display for camera frames during acquisition
- [x] **VIZ-03**: Plots auto-scale to data range with manual override option

### Data Management

- [ ] **DATA-01**: Data auto-saves to disk (HDF5 or CSV) during acquisition
- [ ] **DATA-02**: Metadata captured with each run (device settings, timestamps, scan parameters)
- [ ] **DATA-03**: User can add custom notes/tags to runs
- [ ] **DATA-04**: User can browse run history and view previous results
- [ ] **DATA-05**: User can compare data from multiple runs

### Experiment Library

- [ ] **LIB-01**: User can save experiment design to file (JSON format)
- [ ] **LIB-02**: User can load experiment design from file
- [ ] **LIB-03**: User can access template library with common experiment patterns
- [ ] **LIB-04**: User can create custom templates from their experiments
- [ ] **LIB-05**: Experiment designs have version history (track changes over time)

### Code Integration

- [ ] **CODE-01**: User sees live code preview pane showing generated Rhai script
- [ ] **CODE-02**: User can export experiment as standalone Rhai script file
- [ ] **CODE-03**: User can switch to script editor mode (eject from visual, edit code directly)
- [ ] **CODE-04**: Generated code is readable and well-formatted

## v2 Requirements

Deferred to future release. Tracked but not in current roadmap.

### Advanced Automation

- **AUTO-01**: User can schedule experiments to run at specific times
- **AUTO-02**: User can chain multiple experiments with conditional logic
- **AUTO-03**: User can define custom trigger conditions across devices

### Collaboration

- **COLLAB-01**: User can share experiment designs via URL/link
- **COLLAB-02**: Multiple users can view experiment progress remotely

### Analysis Integration

- **ANAL-01**: User can add inline analysis nodes (fitting, filtering)
- **ANAL-02**: User can define derived quantities computed during acquisition

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Bidirectional code↔graph sync | Extremely difficult, fragile — research confirms anti-pattern |
| Hardware timing compilation | Not needed for current instruments, labscript-style timing adds massive complexity |
| Multi-user real-time collaboration | Single-operator system, adds sync complexity |
| Cloud storage for experiments | Local filesystem sufficient, security concerns |
| AI-driven experiment design | Research shows high failure modes, premature |
| Heavy in-acquisition analysis | Keep acquisition loop lean, defer to post-processing |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| SCAN-01 | Phase 1 | Pending |
| SCAN-02 | Phase 1 | Pending |
| SCAN-03 | Phase 8 | Pending |
| SCAN-04 | Phase 8 | Pending |
| SEQ-01 | Phase 4 | Pending |
| SEQ-02 | Phase 4 | Pending |
| SEQ-03 | Phase 4 | Pending |
| SEQ-04 | Phase 4 | Pending |
| EDIT-01 | Phase 2 | Pending |
| EDIT-02 | Phase 2 | Pending |
| EDIT-03 | Phase 2 | Pending |
| EDIT-04 | Phase 2 | Pending |
| EDIT-05 | Phase 2 | Pending |
| EXEC-01 | Phase 1 | Pending |
| EXEC-02 | Phase 1 | Pending |
| EXEC-03 | Phase 3 | Pending |
| EXEC-04 | Phase 3 | Pending |
| EXEC-05 | Phase 3 | Pending |
| EXEC-06 | Phase 3 | Pending |
| VIZ-01 | Phase 1 | Pending |
| VIZ-02 | Phase 5 | Complete |
| VIZ-03 | Phase 5 | Complete |
| DATA-01 | Phase 1 | Pending |
| DATA-02 | Phase 6 | Pending |
| DATA-03 | Phase 6 | Pending |
| DATA-04 | Phase 6 | Pending |
| DATA-05 | Phase 6 | Pending |
| LIB-01 | Phase 2 | Pending |
| LIB-02 | Phase 2 | Pending |
| LIB-03 | Phase 9 | Pending |
| LIB-04 | Phase 9 | Pending |
| LIB-05 | Phase 9 | Pending |
| CODE-01 | Phase 7 | Pending |
| CODE-02 | Phase 7 | Pending |
| CODE-03 | Phase 7 | Pending |
| CODE-04 | Phase 7 | Pending |

**Coverage:**
- v1 requirements: 35 total
- Mapped to phases: 35
- Unmapped: 0 ✓

---
*Requirements defined: 2025-01-22*
*Last updated: 2025-01-22 after roadmap creation*
