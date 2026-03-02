# Echelle Extraction Rollout and Troubleshooting (MVP)

This guide covers safe rollout of the current local echelle extraction preview,
runtime knobs, and troubleshooting guidance for lab use.

## Rollout Stages

## Stage 0: Offline / Developer Validation

- Use recorded/canned frames
- Load calibration profile programmatically
- Compare preview exports against reference tooling outputs
- Tune extraction cadence (`Every N frames`) for responsiveness

## Stage 1: Lab Preview (Operator-Assisted)

- Enable local extraction preview during live acquisition
- Use merged/order plots for qualitative checks
- Watch diagnostics:
  - extract errors
  - extraction time
  - decimation skips

## Stage 2: Routine Lab Usage (Preview Only)

- Keep extraction decimation > 1 for high frame-rate sessions unless needed
- Export snapshots when updating calibrations
- Treat preview as an operator aid until sidecar/protocol streaming QA is complete

## Runtime Knobs / Feature Controls (Implemented)

## Programmatic API knobs (`ImageViewerPanel`)

- `set_echelle_profile_path(path)`
- `clear_echelle_profile_path()`
- `set_echelle_extraction_enabled(bool)`
- `set_echelle_extract_every_n_frames(u32)`
- `set_echelle_preview_show_merged(bool)`

## In-Panel Controls (Echelle Spectrum MVP Preview)

- `Enabled`
- `Every N frames` cadence
- `Merged` toggle
- order selector
- x-axis mode (`Wavelength` / `Sample`)
- display-only smoothing window

## Debug Export Hooks (Implemented)

- `echelle_preview_measurements()`
- `save_echelle_preview_measurements_json(path)`
- `save_echelle_preview_merged_csv(path)`

## Planned (Not Yet Implemented) Rollout Controls

- persistent config-backed feature flag defaults
- sidecar path enable/disable toggle
- protocol-stream mode selection
- central telemetry for extraction failures/latency

## Troubleshooting Guide

## Symptom: “No echelle calibration profile loaded”

Cause:

- no profile path set yet (no UI file picker in current MVP)

Action:

- set profile path programmatically via `set_echelle_profile_path(...)`

## Symptom: Profile loads, but extraction preview shows errors

Common causes:

- frame/profile size mismatch
- bit depth mismatch
- malformed profile fields (trace domain, wavelengths, etc.)

Action:

- check the echelle preview error text
- verify profile `compatibility` matches the active acquisition mode
- validate with test fixture patterns in the schema docs

## Symptom: Plot updates too slowly / UI feels heavy

Cause:

- extraction is running too often for current frame rate and image size

Action:

- increase `Every N frames`
- disable preview temporarily during alignment/high-rate capture
- reduce frame size / ROI if acceptable

## Symptom: Spectrum looks flat or wrong order alignment

Common causes:

- incorrect orientation axes
- trace coefficients not matching current ROI/mode
- wrong wavelength model/order mapping
- stale profile after acquisition mode change

Action:

- verify orientation fields
- confirm ROI/binning
- inspect selected-order overlay and hover cross-link marker
- export preview JSON/CSV and compare to reference tool output

## Symptom: Hot-reload profile edit breaks preview temporarily

Expected behavior:

- parse/validation error is reported
- last-good profile and last-good preview are preserved

Action:

- finish editing/saving valid profile
- verify the “profile loaded” status after reload

## Logging / Observability Guidance (Current MVP)

Use a combination of:

- UI diagnostics panel in the echelle preview
- status/error bar in Image Viewer
- tracing logs from the UI panel / frame processing path

Recommended observations during rollout:

- extraction error count growth
- extraction latency vs camera FPS
- repeated profile reload errors
- mismatches after ROI/binning changes

## What to Capture When Reporting a Failure

- calibration profile file (or hash + version + `profile_id`)
- frame dimensions, bit depth, ROI, binning
- screenshot of image + spectrum preview
- echelle preview error text
- exported preview JSON/CSV (if available)
- rust-daq commit hash
