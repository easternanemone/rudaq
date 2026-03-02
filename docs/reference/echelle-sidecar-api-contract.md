# Echelle Sidecar API Contract (Design)

This document defines the proposed Python sidecar contract for higher-fidelity
echelle extraction/calibration workflows (GAMSE-backed or GAMSE-inspired).

This is a design/specification document for the prototype workstream.

## Goals

- Reuse mature Python echelle-reduction logic without blocking rust-daq UI work
- Keep rust-daq responsible for orchestration/UI
- Provide deterministic request/response behavior with explicit provenance
- Allow numerical comparison against the local Rust MVP extractor

## GAMSE Reuse Assessment (Non-Pipeline Embedding)

## Minimal GAMSE Surfaces of Interest

The reusable functional areas are:

- order tracing
  - `gamse.echelle.trace` (`find_apertures(...)`)
- extraction
  - `gamse.echelle.extract` (`extract_aperset(...)`, `extract_aperset_optimal(...)`)
- wavelength calibration
  - `gamse.echelle.wlcalib` (`wlcalib(...)`)

## Embedding Constraints

- GAMSE is pipeline-oriented; direct function reuse requires adapting:
  - data structures
  - config expectations
  - file/provenance conventions
- A sidecar boundary is preferred over in-process Python embedding for MVP/phase-2:
  - easier isolation
  - simpler failure handling/restart
  - clearer licensing/dependency boundary

## Sidecar Modes

Recommended modes:

- `health`
  - sanity check, version info, dependency status
- `extract_preview`
  - fast extraction for a single frame using an existing calibration profile
- `extract_offline`
  - offline batch/captured-frame extraction with richer provenance
- `calibrate_*` (future)
  - trace/wavelength/blaze workflows

## Transport (Prototype Recommendation)

## Control Channel

- JSON lines over `stdin`/`stdout`
- one request -> one terminal response
- structured log lines on `stderr`

Why:

- trivial to prototype in Rust and Python
- no port management
- easy timeout/restart semantics

## Frame Payload Transport

Recommended prototype approach:

- control message in JSON
- bulk frame bytes in a temporary file (`.npy`, `.npz`, or raw binary + metadata)

Future options:

- JSON header + binary payload framing over stdio
- Arrow IPC / Flight (more complex, stronger for high throughput)

## Request Schema (Design)

## Common Envelope

```json
{
  "request_id": "uuid-or-seq",
  "op": "extract_preview",
  "timeout_ms": 2000,
  "client": {
    "name": "rust-daq",
    "version": "dev"
  }
}
```

## `extract_preview` Request

Fields:

- `request_id`
- `op = "extract_preview"`
- `frame`
  - `encoding`: `raw_le_u8` | `raw_le_u16` | `npy` | `npz`
  - `width`, `height`, `bit_depth`
  - `roi_x`, `roi_y` (optional but preferred)
  - `binning_x`, `binning_y` (optional but preferred)
  - `path` (for file-backed payloads)
- `profile`
  - `encoding`: `toml` | `json`
  - `path` (or embedded object for small prototypes)
- `options`
  - `mode`: `simple_sum` | `optimal` | `auto`
  - `background`: `true/false`
  - `return_orders`: `true/false`
  - `return_merged`: `true/false`
  - `max_orders` (optional debug limit)

## Response Schema (Design)

## Success Response

```json
{
  "request_id": "uuid-or-seq",
  "ok": true,
  "result": {
    "orders": [],
    "merged": null,
    "quality": {},
    "provenance": {}
  }
}
```

## Error Response

```json
{
  "request_id": "uuid-or-seq",
  "ok": false,
  "error": {
    "code": "PROFILE_INCOMPATIBLE",
    "message": "frame size mismatch ...",
    "details": {}
  }
}
```

## Result Payload Requirements

## Per-Order Spectrum

- `relative_index`
- `physical_order_number` (optional)
- `wavelengths[]`
- `flux[]`
- `wavelength_unit`
- `flux_unit`
- quality metrics:
  - `covered_samples`
  - `total_samples`
  - `saturated_samples`
  - `mean_valid_fraction`

## Merged Spectrum

- `wavelengths[]`
- `flux[]`
- units
- merge policy metadata:
  - sorting strategy
  - overlap policy
  - blaze correction applied / not applied

## Provenance Requirements

- sidecar name/version
- extraction backend (`rust_mvp`, `gamse`, `gamse+custom`)
- calibration profile ID + schema version
- source frame metadata (dims/ROI/binning/bit depth)
- processing options used
- timing metrics (wall time)

## Error Codes (Recommended)

- `INVALID_REQUEST`
- `UNSUPPORTED_ENCODING`
- `PROFILE_PARSE_ERROR`
- `PROFILE_INCOMPATIBLE`
- `FRAME_IO_ERROR`
- `EXTRACTION_FAILED`
- `TIMEOUT`
- `INTERNAL_ERROR`

## Serialization Decision Matrix (2D Frames + Profile Exchange)

## Prototype Default: `JSON + file-backed frame payload`

- Pros:
  - simple implementation
  - debuggable
  - works with NumPy/GAMSE workflows
- Cons:
  - temp file overhead

## `NPZ` for frame + artifacts

- Pros:
  - natural Python fit
  - can bundle masks/aux arrays
- Cons:
  - Rust writing/reading complexity vs raw binary

## Arrow IPC / Flight (future)

- Pros:
  - strong typed transport
  - scalable for vector/image payloads
- Cons:
  - overkill for prototype

## Python Environment Packaging Strategy (Prototype)

Recommended support policy:

- Preferred: `uv`-managed virtual environment
- Supported fallback: `venv`
- Optional lab-managed: `conda`

Requirements:

- pinned dependency lock file
- reproducible setup command
- explicit feature gate in rust-daq (sidecar path disabled by default)
- sidecar `--version` output includes dependency snapshot hash if possible

## Licensing / Compliance Guidance

Preferred dependency posture:

- Apache-2.0 / MIT / BSD compatible components
- GAMSE is preferred for algorithm reuse due Apache-2.0 licensing

Avoid for direct code vendoring without further review:

- GPL-licensed code (contamination risk for direct integration)
- repositories with unclear or missing license notices

Rules of engagement:

- referencing algorithms/papers is fine
- copying code requires license review and attribution
- record third-party dependency licenses in sidecar environment docs

## Implementation Status

- Design/spec only (no production sidecar path yet)
- Local Rust MVP extractor remains the active path for Image Viewer preview
