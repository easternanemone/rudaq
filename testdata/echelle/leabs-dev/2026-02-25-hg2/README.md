# leabs-dev Hg-2 / Mechelle / iSTAR Capture Set (2026-02-25)

Real hardware capture set collected from `leabs-dev` with:

- Camera: Andor iStar sCMOS (`istar_camera`)
- Spectrograph: Mechelle 5000
- Illumination: Ocean Optics HG-2 Mercury-Argon calibration lamp

## Contents

- `manifest.json`
  - top-level capture manifest and per-exposure summaries
- `parameters_raw.json`
  - raw `GetParameter` responses for selected camera settings at capture time
- `hg2_*_frame.json`
  - first streamed `FrameData` object captured at each requested exposure
- `hg2_*_payload_lz4.bin`
  - base64-decoded `FrameData.data` payload bytes (still LZ4-compressed)
- `reference/`
  - externally generated pixel-space reference outputs, provenance, and comparison tolerances
- `SHA256SUMS.txt`
  - checksums for all committed fixture and reference artifacts
- `FIXTURE_STORAGE_POLICY.md`
  - size/compression/checksum policy for this dataset

## Exposure Set

- `hg2_001ms`
- `hg2_010ms`
- `hg2_100ms`

## Important Notes

- Stream payloads were captured as `COMPRESSION_LZ4` with `uncompressed_size=8388608`
  (expected for `2048x2048` at 2 bytes/pixel transport for 12-bit data).
- `FrameData` stream metadata reported:
  - `width=2048`, `height=2048`, `bit_depth=12`
  - `exposure_ms` matched requested exposures
- Some parameter RPC values appear inconsistent with streamed frame metadata at capture time:
  - `AOIWidth` and `AOIHeight` reported `"1"` via `GetParameter`
  - streamed frames were `2048x2048`
  - `roi_x/roi_y/binning_x/binning_y` were absent (`null`) in streamed `FrameData`

Treat the streamed `FrameData` dimensions/bit-depth as the canonical values for these captures.

## Data Quality Classification (Current)

The committed 2026-02-25 capture set is currently classified as **diagnostic ramp-like**
rather than a usable Hg-Ar echellegram:

- all three exposures decode to a deterministic additive ramp pattern (approximately `pixel = row + col`)
- `hg2_010ms` and `hg2_100ms` are bitwise-identical after decompression
- `hg2_001ms` differs from those frames by a uniform `+1` count offset

As a result, these fixtures are currently intended for:

- transport/decompression regression tests
- extractor no-crash / shape-handling tests
- reference artifact/provenance plumbing

They are **not** currently suitable for:

- wavelength calibration validation
- order tracing correctness validation
- spectroscopic intensity fidelity comparisons

See `reference/comparison_tolerances.json` and `reference/capture_diagnostics.json`.
