---
phase: 01-form-based-scan-builder
plan: 01
subsystem: gui
tags: [egui, scan-builder, form-ui, device-selection]

dependency-graph:
  requires: []
  provides:
    - ScanBuilderPanel struct with form state
    - Device discovery via gRPC
    - 1D/2D scan mode toggle
    - Form validation with visual feedback
    - Scan preview calculation
  affects:
    - 01-02 (scan execution)
    - future visual node editor integration

tech-stack:
  added: []
  patterns:
    - PendingAction + mpsc async pattern (from ScansPanel)
    - offline_notice widget for disconnected state
    - Form validation with HashMap<&'static str, String> errors

key-files:
  created:
    - crates/daq-egui/src/panels/scan_builder.rs
  modified:
    - crates/daq-egui/src/panels/mod.rs
    - crates/daq-egui/src/app.rs

decisions:
  - Used string-based form fields for flexible user input (parse on validation)
  - Grouped devices by capability: Movable for actuators, Readable/FrameProducer for detectors
  - Red border + tooltip for validation errors (matches existing patterns)

metrics:
  duration: 6min
  completed: 2026-01-22
---

# Phase 01 Plan 01: ScanBuilderPanel Foundation Summary

Form-based 1D/2D scan configuration UI with device discovery and validation.

## What Was Built

### ScanBuilderPanel (`crates/daq-egui/src/panels/scan_builder.rs`)

A new egui panel providing scientists with a simplified form interface for configuring parameter scans:

**Device Selection:**
- Actuators section: ComboBox selection for movable devices
- Detectors section: Multi-select checkboxes for readable/camera devices
- Device list loaded via `client.list_devices()` async call
- "Refresh Devices" button with timestamp display

**Scan Mode Toggle:**
- 1D Line Scan: Single actuator with Start/Stop/Points
- 2D Grid Scan: X and Y axes with separate Start/Stop/Points each
- Mode toggle via `ui.selectable_value()`

**Form Validation:**
- Real-time validation on field change
- Error display: Red border stroke on invalid fields
- Tooltip on hover showing error message
- Validation rules: numeric parsing, points > 0, start != stop, required selections

**Scan Preview:**
- Total points calculation (1D: points, 2D: x_points * y_points)
- Estimated duration: points * dwell_time_ms / 1000
- Human-readable duration formatting (seconds/minutes/hours)

### Integration (`crates/daq-egui/src/app.rs`)

- Added `Panel::ScanBuilder` variant to Panel enum
- Added `scan_builder_panel` field to DaqApp
- TabViewer title() returns "Scan Builder"
- TabViewer ui() renders the panel with client and runtime
- Navigation button in "Experiment" section
- View menu entry for quick access

## Technical Details

### Async Pattern

Follows the established PendingAction + mpsc channel pattern from ScansPanel:

```rust
enum PendingAction { RefreshDevices }
enum ActionResult { DevicesLoaded(Result<Vec<DeviceInfo>, String>) }

// In ui():
self.poll_async_results(ctx);
self.pending_action = None;
// ... render UI ...
if let Some(action) = self.pending_action.take() {
    self.execute_action(action, client, runtime);
}
```

### Form State

String-based fields allow flexible user input with validation on change:

```rust
start_1d: String,    // Parsed as f64
stop_1d: String,     // Parsed as f64
points_1d: String,   // Parsed as u32
```

### Validation Error Display

```rust
fn render_text_field(ui, text, has_error, error_msg) -> bool {
    let mut frame = egui::Frame::NONE;
    if has_error {
        frame = frame.stroke(egui::Stroke::new(1.0, egui::Color32::RED));
    }
    // ... render with tooltip on hover
}
```

## Deviations from Plan

None - plan executed exactly as written.

## Commits

| Hash | Message |
|------|---------|
| 960c72c8 | feat(01-01): create ScanBuilderPanel with form state and device discovery |
| ec5e15b5 | feat(01-01): integrate ScanBuilderPanel into app dock system |
| 02033b0d | refactor(01-01): polish scan builder form with cleaner text field rendering |

## Files Changed

| File | Change |
|------|--------|
| `crates/daq-egui/src/panels/scan_builder.rs` | Created (729 lines) |
| `crates/daq-egui/src/panels/mod.rs` | Added module export |
| `crates/daq-egui/src/app.rs` | Panel integration (+16 lines) |

## Next Phase Readiness

The ScanBuilderPanel foundation is complete. For the next plan (01-02), the form state can be used to:
1. Generate scan configurations for the daemon
2. Submit scans via existing gRPC scan service
3. Add "Run Scan" button to execute configured scans

No blockers identified.
