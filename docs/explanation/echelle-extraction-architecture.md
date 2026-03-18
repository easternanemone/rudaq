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

## Calibration Pipeline (3-Pass, 2026-03-18)

Offline calibration uses a 3-pass pipeline to maximize order coverage (115/115 orders, 230–844nm):

**Pass 1: Echelle Equation Seed**
- Estimate physical order m from trace index using echelle grating constant: m = first_physical_order + order_step × trace_index
- Compute estimated center wavelength λ_est from m
- Match arc lines within 5nm tolerance to HgAr atlas
- Mark orders with ≥1 matched line as "arc-matched" (42 orders)

**Pass 2: Quadratic Regression Re-Seed**
- Fit quadratic m(i) = a + b*i + c*i² from Pass 1 successes
- Use fit to predict physical order m for failed orders
- Re-attempt atlas matching with predicted m values
- Typically improves coverage by anchoring marginal orders

**Pass 3: Physics Bootstrap + 2D Chebyshev Residual**
- For orders with no arc lines: assign physical_order_number via quadratic interpolation
- Compute physics baseline: λ_base(x,m) = gc/m + disp(m)*(x - w/2)
  - dispersion scales as 1/m (echelle physics)
- Fit 2D Chebyshev tensor-product surface (degree 4×3, 20 coefficients) to residuals δλ from calibrated orders
- Apply baseline + residual correction to predict wavelengths for all uncalibrated orders
- Mark as "bootstrapped" (no arc line verification; relies on physical model)

**Result**: 115/115 orders calibrated (42 arc-matched + 73 bootstrapped), full Mechelle 5000 range (m=43 to m=158)

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
