> [!WARNING] **ARCHIVAL / HISTORICAL**
> This document is a historical snapshot and is preserved for context. It does not represent current operational guidance or source-of-truth architecture.

# Mechelle Echelle Spectrum Workstream A Plan

> Related issues: `bd-2kla`, `bd-2kla.1`, `bd-2kla.1.1`, `bd-2kla.1.2`, `bd-2kla.1.3`, `bd-2kla.1.4`, `bd-2kla.1.5`
> Date: 2026-02-25

## Purpose

This document executes Workstream A of the Mechelle/iSTAR echelle extraction epic by:

1. Capturing instrument/data assumptions and unknowns
2. Defining MVP product outcomes and acceptance criteria
3. Establishing milestone sequencing and rollout strategy
4. Providing the context for ADR-012 and ADR-013

This plan is intentionally detailed because downstream workstreams (calibration
profile schema, MVP extraction implementation, sidecar evaluation, and protocol
streaming) depend on a stable framing of scope and assumptions.

## Current Architecture Findings (rust-daq)

### Confirmed capabilities already present

- `Measurement::Spectrum` supports full x-axis + y-axis arrays, units, and metadata.
- `ImageViewerPanel` has access to raw frame bytes (`Arc<[u8]>`) at the point where
  new frames are processed for display.
- `ImageViewerPanel` already performs per-frame statistics work (ROI, histogram),
  making it a practical insertion point for an MVP extractor if runtime cost is
  controlled.

### Confirmed gaps that affect architecture

- `StreamMeasurements` currently scalarizes `Measurement::Spectrum` into a
  single numeric summary (spectrum length), so it cannot carry full spectra to the UI.
- `ModuleDataPoint` is `map<string,double>` + metadata and also cannot transport
  array-valued spectra.
- `LiveVisualizationPanel` line plots currently ingest scalar `DataUpdate` values,
  not vector/spectrum payloads.

### Hardware-model caveat (important)

- Current Andor spectrograph support in `driver-andor-sdk3` is Shamrock-oriented
  (single-axis pixel calibration helper available).
- Mechelle 5000 is a cross-dispersed echelle use case and requires:
  - order tracing
  - extraction apertures / summation strategy
  - wavelength calibration per order (and potentially merged-spectrum handling)
  - calibration artifact persistence and compatibility checks

## Instrument + Data Assumptions (Workstream A.1)

This section separates what is known from what must be confirmed on the lab system.

### A. Confirmed from rust-daq code and protocol

#### Frame transport and metadata

- Camera frame transport supports:
  - width / height
  - bit depth
  - raw bytes
  - frame number + timestamp
  - optional ROI offsets (`roi_x`, `roi_y`)
  - optional binning metadata (`binning_x`, `binning_y`)
  - extensible string metadata map

#### UI frame-processing path

- `ImageViewerPanel::process_frame()` is the canonical point where the newest frame
  is consumed after queue-drain semantics (latest frame wins).
- The panel stores raw frame bytes (`last_frame_data`) and updates histogram/ROI
  from that same raw source.

#### Existing performance constraints

- UI already uses a background RGBA conversion worker to avoid frame-rendering stalls.
- Any MVP extraction path must avoid reintroducing UI-thread stutter under live streaming.

### B. Assumptions to validate on the physical Mechelle+iSTAR setup

These are required before locking the first calibration profile(s):

#### Sensor and detector geometry assumptions

- Full sensor pixel dimensions used in the Mechelle workflow
- Typical ROI used during acquisition (full-frame vs cropped)
- Typical binning settings (if any)
- Orientation conventions:
  - detector row/column direction relative to dispersion axis
  - which axis contains the primary dispersion within each order
  - whether order numbering increases toward larger row indices or smaller row indices

#### Acquisition mode assumptions

- Exposure ranges used for:
  - science
  - flat
  - arc / line lamp
- Trigger modes used in production measurements
- Gain / readout modes that materially affect noise model or saturation behavior

#### Calibration workflow assumptions

- Which calibration frames are available in practice:
  - bias / dark (or intentionally omitted)
  - flat
  - arc lamp (e.g. ThAr or equivalent)
- Whether calibration is stable across sessions for a fixed hardware configuration
- Whether image slicer / multi-trace-per-order behavior is present in this setup

#### Output expectations

- Whether users primarily want:
  - per-order spectra
  - merged spectrum
  - both, with quick visual QA
- Whether wavelength units should default to `nm` or `Å`

### C. Assumptions we will use for MVP implementation (unless validated otherwise)

- MVP extraction will use a precomputed calibration profile with order traces and wavelength mapping.
- MVP extraction will use top-hat/simple-sum extraction (not full optimal extraction).
- MVP may run at a decimated cadence (not every frame) to preserve UI responsiveness.
- MVP visualization will initially prioritize correctness and inspectability over high-rate throughput.
- Calibration profile incompatibility (ROI/binning/orientation mismatch) will fail closed with explicit UI messaging.

## MVP Product Definition (Workstream A.2)

### MVP Goal (Phase 1)

From a live or recorded Mechelle echellegram frame, rust-daq can display a derived
spectrum in the GUI using a loaded calibration profile, with enough fidelity for
operator inspection and iteration on calibration quality.

### MVP User-visible outcomes

#### Required

- User can load/select an echelle calibration profile
- User can see raw image and extracted spectrum in the same workflow
- User can inspect at least:
  - one selected order spectrum
  - a merged spectrum view or a clearly documented placeholder if merge is deferred
- UI reports profile compatibility state (active / invalid / mismatched)
- Extraction failures are surfaced as actionable errors (not silent no-op)

#### Strongly preferred (Phase 1 if cost is reasonable)

- Per-order browsing / order selector
- Extraction cadence controls (e.g., every frame vs Nth frame)
- Developer diagnostics (extraction latency, dropped computations, invalid orders)

#### Explicitly deferred from MVP

- Full optimal extraction
- Automated arc-line identification
- Interactive calibration GUI authoring tools
- Protocol-native spectrum array streaming
- Complete laboratory-grade reduction parity with GAMSE/CERES

## MVP Acceptance Criteria (Workstream A.2)

### Functional acceptance

1. With a valid calibration profile and representative frame:
   - rust-daq displays a non-empty extracted spectrum without crashing or freezing.
2. With an incompatible calibration profile:
   - rust-daq rejects extraction and shows a specific mismatch reason
     (e.g., ROI, binning, dimensions, orientation, missing orders).
3. With no profile loaded:
   - rust-daq remains usable as raw image viewer and indicates extraction is unavailable.

### Numerical acceptance (initial)

1. Extracted per-order flux arrays are deterministic for the same frame/profile.
2. Wavelength axis is monotonic within each order (or explicitly marked if reverse-ordered).
3. Golden comparison harness (later workstream) can compare MVP results to reference outputs
   within documented tolerances.

### Performance acceptance (initial)

1. Raw image display path remains responsive during live streaming with extraction enabled.
2. Extraction cadence can be reduced / throttled if required.
3. UI does not accumulate unbounded extraction backlog (latest-frame semantics preserved).

## Milestones and Rollout Strategy (Workstream A.5)

### Milestone M0 — Planning and Decision Freeze (this workstream)

Deliverables:
- This plan document
- ADR-012 (MVP extraction location)
- ADR-013 (calibration profile ownership/versioning policy)

Exit criteria:
- Workstreams B/C/D can proceed without architectural ambiguity on MVP path.

### Milestone M1 — Calibration Profile Foundations

Primary issues:
- `bd-2kla.2.*`

Deliverables:
- Schema v1 draft + Rust types
- Validation rules
- Sample profile fixtures

Exit criteria:
- A profile can be loaded and validated independently of extraction math.

### Milestone M2 — MVP Local Extraction in Image Viewer

Primary issues:
- `bd-2kla.4.*`
- partial `bd-2kla.5.*`

Deliverables:
- Raw frame adapter
- Trace evaluation + gather indices
- Top-hat extraction
- Wavelength mapping
- Basic spectrum visualization panel

Exit criteria:
- Live/raw frame plus derived spectrum shown in UI for known-good fixtures/profile.

### Milestone M3 — Numerical Validation and Hardening

Primary issues:
- `bd-2kla.3.*`
- `bd-2kla.9.*`

Deliverables:
- Golden datasets
- Regression harness
- Benchmarks and robustness tests

Exit criteria:
- MVP outputs have repeatable validation results and documented tolerances.

### Milestone M4 — Sidecar Evaluation (Higher Fidelity Path)

Primary issues:
- `bd-2kla.6.*`

Deliverables:
- Prototype GAMSE-backed or GAMSE-inspired sidecar
- Contract and packaging strategy
- Rust-vs-sidecar comparison results

Exit criteria:
- Decision point on whether to invest in sidecar path for production-quality extraction.

### Milestone M5 — Protocol-native Spectrum Streaming

Primary issues:
- `bd-2kla.7.*`

Deliverables:
- New proto message(s) and client/server support
- UI spectrum stream consumer

Exit criteria:
- Full spectrum arrays can be streamed without scalarization.

### Milestone M6 — Calibration GUI Authoring

Primary issues:
- `bd-2kla.8.*`

Deliverables:
- Trace UI
- Arc-line ID UI
- Wavelength fit diagnostics
- Profile management UI

Exit criteria:
- Calibration profiles can be authored/edited in rust-daq (not only imported).

## Feature Flag Strategy

### Proposed flags / gating (names provisional)

- `ui_echelle_extraction_mvp`
  - Enables local extraction path in `ImageViewerPanel`
- `ui_echelle_spectrum_panel`
  - Enables spectrum visualization UI components
- `echelle_sidecar`
  - Enables Python sidecar process integration
- `grpc_spectrum_stream`
  - Enables protocol-native spectrum array streaming path
- `ui_echelle_calibration_tools`
  - Enables calibration authoring GUI workflows

### Rollout principles

- Raw camera image viewing must remain functional even if all echelle features are disabled.
- New paths should fail closed and degrade gracefully.
- Profile validation must occur before starting extraction.
- Runtime toggles should surface clear status in UI for supportability.

## Risk Register (Workstream A snapshot)

### R1. UI performance regression

- Risk: local extraction in UI frame path causes stalls/jank.
- Mitigation:
  - extraction cadence throttling
  - background worker option with latest-frame semantics
  - metrics/diagnostics in UI

### R2. Calibration profile drift / incompatibility

- Risk: users apply profiles to incompatible ROI/binning/orientation data.
- Mitigation:
  - strict validator
  - fail-closed behavior
  - explicit compatibility UI

### R3. Overfitting MVP to one data orientation

- Risk: code assumes a single detector orientation and breaks on future setups.
- Mitigation:
  - encode orientation semantics in profile schema
  - test with reversed/rotated assumptions via fixtures where possible

### R4. Premature protocol redesign

- Risk: time spent on backend streaming before validating extraction UX and data model.
- Mitigation:
  - ADR-012 explicitly chooses local UI path first for MVP
  - defer protocol work to Workstream G

### R5. External pipeline licensing/integration pitfalls

- Risk: accidental dependency on GPL/unclear-licensed code for production integration.
- Mitigation:
  - prefer Apache-2.0 (`gamse`) for reusable code paths
  - treat other repos as reference unless licensing is clarified

## Open Questions Requiring Lab Validation

1. What are the canonical Mechelle acquisition modes (ROI/binning/readout/gain) used in production?
2. Which arc lamp(s) and flat sources are available and routinely captured?
3. Is merged-spectrum output required in MVP, or is per-order browsing sufficient initially?
4. How stable are order traces and wavelength solutions between sessions for the same setup?
5. Are there image-slicer or multi-trace-per-order effects in the current Mechelle configuration?

## Immediate Next Actions (handoff to implementation workstreams)

1. Execute `bd-2kla.2.1` (schema v1 design) using the assumptions and constraints above.
2. Execute `bd-2kla.3.1`/`.3.2` to replace remaining assumptions with measured facts.
3. Start `bd-2kla.4.1` and `bd-2kla.4.2` behind a feature flag once schema draft is stable.
