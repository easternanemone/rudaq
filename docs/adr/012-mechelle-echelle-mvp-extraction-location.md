# ADR: Mechelle Echelle MVP Extraction Location

**Status:** Proposed
**Date:** 2026-02-25
**Author:** Codex / Workstream A (`bd-2kla.1.3`)
**Related Issues:** `bd-2kla`, `bd-2kla.1`, `bd-2kla.1.3`, `bd-2kla.4`, `bd-2kla.6`, `bd-2kla.7`

---

## Context

The Mechelle 5000 integration requires converting a 2D echellegram from the Andor
iSTAR camera into a usable 1D spectrum (and eventually calibrated/merged spectra)
inside rust-daq.

rust-daq already contains:

- a full-frame image viewer path that receives raw frames and processes them for display
- a `Measurement::Spectrum` domain type that can represent spectrum arrays with units and metadata

However, the current network/backend streaming path is not yet suitable for MVP
spectrum delivery:

- `StreamMeasurements` scalarizes spectra (currently only emits a numeric summary)
- `ModuleDataPoint` is scalar-key/value oriented and cannot transport array-valued spectra
- `LiveVisualizationPanel` line plotting is currently scalar-update based

This creates a sequencing problem: should the MVP extraction pipeline be implemented
in the backend (and force a protocol redesign first), or locally in the UI frame path
to validate the product quickly?

## Decision

For **Phase 1 (MVP)**, implement the Mechelle echelle extraction path **locally in the UI**,
integrated with `ImageViewerPanel`, using a loaded calibration profile and a
throttled execution model.

Specifically:

1. The extraction pipeline will consume raw frame bytes from the image viewer path.
2. The extraction pipeline will be calibration-profile-driven (traces, wavelength mapping, etc.).
3. The pipeline will initially implement a top-hat/simple-sum extraction strategy.
4. The UI will render the derived spectrum directly (without requiring protocol-native spectrum streaming).
5. Backend/protocol-native spectrum streaming is explicitly deferred to a later phase (Workstream G).

## Rationale

### Why local UI extraction for MVP

- **Fastest path to user value:** the user can see a real derived spectrum sooner.
- **Minimal protocol churn:** avoids redesigning gRPC payloads before the product behavior is validated.
- **Existing hook point is strong:** `ImageViewerPanel::process_frame()` already has the raw bytes and related per-frame UI logic.
- **Reduces integration uncertainty:** lets us validate calibration profile design and visualization UX before deciding on server vs sidecar ownership long-term.

### Why not backend/module-first for MVP

- Requires new transport for full spectrum arrays before any visible result.
- Adds serialization/deserialization and compatibility work before extraction behavior is proven.
- Increases scope and delays feedback on extraction correctness and UX.

### Why not sidecar-first for MVP

- Sidecar is promising for higher-fidelity extraction, but adds process orchestration,
  packaging, and API-contract work immediately.
- We can still pursue the sidecar path in parallel/next (Workstream F) after proving
  the rust-daq UX and calibration flow with local MVP extraction.

## Consequences

### Positive

- Delivers a visible spectrum workflow earlier.
- Keeps Workstream D focused and implementable with existing UI infrastructure.
- Lets Workstream B (calibration profile) and Workstream C (goldens) directly feed MVP extraction work.
- Produces concrete usage patterns to inform Workstream G protocol design.

### Negative

- Extraction logic initially lives in UI code, which is not ideal as the final long-term location.
- The UI path may require careful concurrency/performance handling to avoid frame stutter.
- Some later refactoring may be needed when backend or sidecar ownership is expanded.

### Risks

- UI responsiveness regression if extraction runs too often or allocates heavily.
- Architectural inertia: MVP implementation might be mistaken for long-term final design.
- Duplication risk if sidecar/backend implementations are added later without a shared data contract.

## Guardrails for This Decision

1. **Feature-flag the MVP extractor** so it can be disabled independently of raw image viewing.
2. **Keep extraction core modular** (separable from UI rendering code) to ease future migration.
3. **Record extraction metadata/provenance** so future backend/sidecar paths can match semantics.
4. **Add throttling/latest-frame semantics** to protect UI responsiveness.
5. **Treat Workstream G as planned follow-on**, not a replacement after-the-fact.

## Deferred Follow-up Decisions

- Whether the long-term authoritative extraction engine should live in:
  - rust-daq backend/module path
  - Python sidecar
  - hybrid (UI for preview, backend/sidecar for publish-quality reductions)
- Exact gRPC schema for streaming spectrum arrays
- Long-term ownership of calibration execution vs calibration authoring
