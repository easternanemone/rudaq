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
