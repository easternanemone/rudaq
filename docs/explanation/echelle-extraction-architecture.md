# Echelle Extraction Architecture (MVP to Future)

This document explains the current and planned architecture for converting
Mechelle/iSTAR echellegrams into spectra in rust-daq.

## Current State (Implemented)

## Data Flow

1. Camera frame arrives in `ImageViewerPanel` as `FrameUpdate` with `Arc<[u8]>`.
2. Image Viewer:
   - updates ROI statistics and histogram
   - submits RGBA conversion to the background converter thread
   - optionally runs local echelle extraction preview (decimated cadence)
3. Echelle preview panel renders:
   - per-order or merged spectrum
   - calibration profile status/provenance
   - diagnostics and export hooks

## Key Design Decisions

- Calibration profile is versioned and validated in `common`:
  - `crates/common/src/echelle.rs`
- MVP extraction runs locally in `ImageViewerPanel` (UI-owned) for fastest iteration:
  - `docs/adr/012-mechelle-echelle-mvp-extraction-location.md`
- Calibration profile ownership/versioning policy is defined in:
  - `docs/adr/013-mechelle-calibration-profile-ownership-and-compatibility.md`

## Implemented MVP Components

- Profile cache / hot reload:
  - `crates/ui/src/panels/image_viewer/echelle_profile_cache.rs`
- Extraction kernel:
  - `crates/ui/src/panels/image_viewer/echelle_extraction.rs`
- UI integration + spectrum preview:
  - `crates/ui/src/panels/image_viewer/mod.rs`

## Extraction Algorithm (MVP)

- Decode raw frame bytes (8-bit, 12/16-bit little-endian storage)
- Validate frame/profile compatibility (dims, bit depth; ROI/binning when available)
- Evaluate per-order trace centerline
- Gather aperture pixels with clipping and excluded-region masking
- Apply simple-sum (top-hat) extraction
- Optional sideband background subtraction
- Map samples to wavelength (polynomial or sampled arrays)
- Produce:
  - per-order preview spectra
  - merged wavelength-sorted preview
  - `Measurement::Spectrum` debug/export objects

## Why the MVP Lives in the UI (for now)

- Fastest path to a visible, testable result
- Avoids protocol changes while the calibration schema and extraction behavior are still evolving
- Keeps calibration iteration tight (hot-reload profile file -> immediate visual result)

Trade-off:

- Extraction currently consumes UI-side CPU (mitigated with decimation controls)

## Planned Architecture Evolution

## Phase 1 (current)

- Local UI extraction preview (implemented)
- Debug exports for comparison against external tools

## Phase 2 (planned sidecar)

- Python sidecar process for:
  - reference extraction comparison
  - richer calibration/reduction reuse (GAMSE-inspired/backed workflows)
- rust-daq remains UI/orchestration layer
- Sidecar runner adds timeout/restart/error handling

## Phase 3 (planned protocol streaming)

- gRPC support for real spectrum/vector payload streaming
- Server/client preserve `Measurement::Spectrum` arrays (no scalarization)
- Live visualization panels consume streamed spectra directly

## Protocol Evolution Notes (Design Target)

Current limitation:

- existing streaming paths are scalar-oriented for live visualization updates

Future protocol additions should include:

- x-axis array (wavelength/frequency)
- y-axis array (amplitude/flux)
- units
- metadata:
  - order index / physical order number
  - merged flag
  - calibration profile ID
  - quality flags (coverage, saturation)
  - provenance/tool version

## Validation and Test Strategy (Current)

- Schema validation + compatibility tests in `common`
- UI extraction unit tests for:
  - decode parity with existing pixel helper
  - simple-sum extraction
  - excluded-region masking
  - optional background subtraction

## Operational Safety Notes

- Last-good calibration profile is preserved on hot-reload parse/validation failures
- Last-good extracted preview is preserved on extraction errors
- Extraction cadence is user-configurable to limit UI CPU impact
