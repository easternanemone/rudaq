# Echelle Validation Plan and Hardware-in-Loop Checklist

This document defines:

- the required reference dataset matrix for Mechelle/iSTAR validation
- the hardware-in-loop (HIL) checklist for lab signoff

It is intended to unblock planning and preparation work before all lab data is
collected and before protocol/sidecar paths are finalized.

## Reference Dataset Matrix (Required)

Each calibration/acquisition mode should have a dataset set that includes the
following categories.

## Core Calibration Inputs

- Bias / zero-exposure frames (if relevant to camera mode)
- Dark frames
  - representative exposure(s)
  - same temperature / gain / readout mode
- Flat-field / blaze frames
  - stable continuum illumination
  - representative signal level (avoid hard saturation)
- Arc / wavelength calibration frames
  - line-rich source suitable for Mechelle coverage

## Science / Representative Operating Inputs

- nominal science frame(s) for the target mode
- low-signal frame(s)
- near-saturation frame(s)
- frames with intentional ROI/binning variations (negative tests)

## Failure / Robustness Cases

- incompatible profile vs frame dimensions
- ROI mismatch
- binning mismatch
- malformed profile (schema/trace/wavelength issues)
- missing/disabled order coverage case
- detector defects / masked region stress cases

## Metadata Required for Each Dataset Entry

- device ID / instrument configuration
- frame dimensions
- ROI origin + size
- binning
- bit depth
- exposure time
- gain / readout settings (if applicable)
- timestamp
- calibration profile ID/schema version used (or expected)
- operator notes

## Golden Outputs to Generate (per mode)

- per-order spectra
- merged spectrum
- provenance record (tool name/version, settings)
- comparison summary versus rust-daq local preview extraction

## Recommended Tracking Table (Template)

For each mode, track:

- `mode_id`
- `dataset_status` (`missing`, `captured`, `golden_generated`, `validated`)
- `profile_status` (`draft`, `validated`, `superseded`)
- `reference_tool` (e.g. GAMSE-based workflow)
- `notes`

## Current Automated Regression Harness (Committed)

The repository now includes a dataset-backed regression harness for the
`leabs-dev/2026-02-25-hg2` fixture set:

- test: `panels::image_viewer::echelle_extraction::tests::real_canned_hg2_reference_regression_matches_declared_tolerances`
- command: `cargo test -p ui echelle --lib -- --nocapture`
- inputs:
  - `testdata/echelle/leabs-dev/2026-02-25-hg2/reference/comparison_tolerances.json`
  - `testdata/echelle/leabs-dev/2026-02-25-hg2/reference/capture_diagnostics.json`
  - `*_reference_summary.json` and dataset frame summaries

This harness currently validates transport/decompression and numeric regression
properties for a **diagnostic-ramp-like** fixture (not spectroscopic truth).
Once a real Hg-Ar echellegram fixture is captured, extend the same harness to
assert per-order and merged spectral tolerances against calibrated golden outputs.

## Current Benchmark Harness (Committed)

The repository also includes a manual benchmark harness for extraction runtime
characterization using the same real canned Hg2 frames:

- test: `panels::image_viewer::echelle_extraction::tests::benchmark_real_canned_hg2_extraction_latency_and_live_budget`
- command:

```bash
cargo test -p ui benchmark_real_canned_hg2_extraction_latency_and_live_budget --lib -- --ignored --nocapture
```

The benchmark prints a JSON report with:

- decode+extract latency stats (`mean`, `p50`, `p95`, `p99`, `max`)
- extract-only latency stats
- per-capture timing breakdown
- throughput estimate (frames/sec)
- a live-frame budget simulation (`5`, `10`, `30` FPS) as a proxy for UI responsiveness

This harness does not yet measure end-to-end egui render latency directly; it
measures the extraction path that currently dominates the echelle-preview runtime.

## Hardware-in-Loop Validation Checklist (iSTAR + Mechelle)

Use this checklist before treating echelle preview output as lab-ready.

## Pre-Run Setup

- Camera cooling at target temperature and stable
- Spectrograph configuration recorded (slit/grating/order settings)
- ROI/binning/bit depth confirmed and documented
- Correct calibration profile loaded (ID, schema version, provenance checked)
- Arc/flat sources available and stable

## Live Functional Checks (Image Viewer)

- Raw image stream stable (no repeated disconnects)
- Echelle preview panel renders without extraction errors
- Selected-order / merged toggle updates correctly
- Hover cross-link marker tracks plausible order positions on image
- Diagnostics remain reasonable:
  - extraction error count stable / zero during nominal operation
  - extraction latency acceptable for frame rate
  - decimation setting chosen appropriately

## Calibration Consistency Checks

- Order traces visually align with echelle orders across field
- Wavelength axis units and range are plausible
- Arc features land at expected approximate wavelengths
- Saturation warnings align with visibly saturated regions
- Excluded regions/masks do not remove valid signal unexpectedly

## Robustness Checks

- Profile hot reload recovers after temporary invalid edit (last-good preserved)
- ROI or binning change triggers compatibility/extraction error (no silent misuse)
- Returning to matching mode restores extraction preview

## Snapshot / Evidence Capture

- Screenshot of raw image + spectrum preview
- Exported preview JSON (`Measurement::Spectrum` snapshots)
- Exported merged CSV
- Active profile file/hash
- Runtime commit hash and date

## Signoff Criteria (MVP Preview)

Signoff can proceed for preview usage when:

- no unexplained extraction errors under nominal conditions
- trace alignment is visually correct
- wavelength output is plausibly aligned to reference features
- exported preview numerics are consistent with reference tool within agreed tolerance
- operator workflow is documented for the specific mode

## Out of Scope for This Checklist (Future Phases)

- protocol-streamed spectrum signoff
- production sidecar deployment signoff
- calibration GUI authoring workflow signoff

Those will require separate checklists after their respective workstreams land.
