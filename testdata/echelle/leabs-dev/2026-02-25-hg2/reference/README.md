# Reference Outputs (External Tooling)

This directory contains externally generated reference artifacts for the
`leabs-dev/2026-02-25-hg2` fixture set.

## Generator

- Script: `scripts/echelle/reference_extract_hg2.py`
- Purpose: create reproducible, non-UI "golden" outputs plus provenance for regression testing

## Artifact Types

- `*_reference_orders_pixel.npz`
  - compressed numpy arrays (pixel axis, per-order flux arrays, flags)
- `*_reference_orders_pixel.csv.gz`
  - gzipped tabular export of the same per-order extraction results
- `*_reference_summary.json`
  - per-capture summary including order detection, extractor config, and image diagnostics
- `dataset_reference_index.json`
  - index of per-capture reference summaries
- `capture_diagnostics.json`
  - dataset-level quality diagnostics and pairwise frame-difference stats
- `provenance.json`
  - generator/runtime provenance and source fixture hashes
- `comparison_tolerances.json`
  - comparison policy + caveats for this dataset

## Interpretation

For this fixture revision, the frames are classified as `diagnostic_ramp_like` rather than
usable Hg-Ar echellegrams. The reference outputs remain useful for:

- decompression + decoding regression
- extractor pipeline shape/metadata handling
- provenance and artifact generation plumbing

They are not a spectral-truth baseline for wavelength calibration or order tracing validation.
