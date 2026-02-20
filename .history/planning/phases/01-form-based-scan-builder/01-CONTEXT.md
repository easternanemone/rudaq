# Phase 1: Form-Based Scan Builder - Context

**Gathered:** 2026-01-22
**Status:** Ready for planning

<domain>
## Phase Boundary

Scientists can configure and execute 1D/2D scans using simple forms, with live plotting and auto-save. Users discover devices from registry, configure scan parameters via forms, start experiments, see live updating plots, and abort with partial data saved. Auto-saves to HDF5 or CSV during acquisition.

Node-based visual editing, pause/resume, and advanced features belong in later phases.

</domain>

<decisions>
## Implementation Decisions

### Device Selection
- Devices grouped by type in collapsible sections (Actuators, Detectors, etc.)
- Each device shows: name, current value, and units (e.g., "Stage X: 45.2 mm")
- Drag-and-drop devices into actuator/detector slots in scan configuration
- Offline devices shown grayed out with "Reconnect" button option

### Scan Configuration
- Single form with all fields visible (no wizard steps or tabs)
- 1D/2D toggle button at top — form fields update based on mode
- Validation errors shown as red border on field + tooltip on hover
- Live calculation preview: "N points, ~X minutes" updates as user fills parameters

### Live Plotting
- 1D scans: user can toggle between line plot (with markers) and scatter-only
- 2D scans: both heatmap and 3D surface views available, toggle between them
- Multiple detectors: overlay all on same axes with legend

### Experiment Control
- Start/Abort buttons at bottom of scan form (below configuration)
- Progress display: visual progress bar with estimated time remaining (ETA)
- Completion summary panel shows: scan duration, total points, file size, saved path

### Claude's Discretion
- Axis label format (device name + units is sensible default)
- Abort confirmation behavior (immediate vs dialog)
- Loading states and error message wording
- Exact progress bar styling
- Auto-scale behavior for plots

</decisions>

<specifics>
## Specific Ideas

- Drag-and-drop for device selection provides direct manipulation feel
- Live preview of scan duration helps scientists plan their time
- Completion summary panel gives scientists a record of what ran

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 01-form-based-scan-builder*
*Context gathered: 2026-01-22*
