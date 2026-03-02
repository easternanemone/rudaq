# ADR: Mechelle Calibration Profile Ownership, Versioning, and Compatibility Policy

**Status:** Proposed
**Date:** 2026-02-25
**Author:** Codex / Workstream A (`bd-2kla.1.4`)
**Related Issues:** `bd-2kla`, `bd-2kla.1`, `bd-2kla.1.4`, `bd-2kla.2`, `bd-2kla.8`

---

## Context

Mechelle echelle extraction in rust-daq depends on calibration artifacts that are
more complex than a single pixel-to-wavelength mapping. At minimum, rust-daq needs
to persist and reuse:

- order trace models (per-order geometry on the detector)
- extraction aperture widths / summation settings
- wavelength mapping (per-order coefficients or sampled arrays)
- optional blaze/flat corrections
- optional masks (bad pixels, excluded regions)
- provenance metadata (how and when the calibration was generated)

The project also needs to support phased implementation:

- MVP may rely on imported/manual calibration profiles
- later phases will add in-app calibration authoring UI
- a Python sidecar may consume or produce calibration artifacts

Without a defined ownership and compatibility policy, the project risks:

- profile drift and silent mismatches (ROI/binning/orientation)
- incompatible file formats across UI, backend, and sidecar paths
- ad-hoc migrations that break old calibrations

## Decision

rust-daq will define and own a **versioned canonical calibration profile schema**
for Mechelle echelle extraction, with strict compatibility validation and fail-closed behavior.

### Policy summary

1. **Canonical schema ownership**
   - rust-daq owns the canonical schema definition and validation rules.
   - External tools (QtYETI/GAMSE-derived workflows/sidecars) are treated as producers/consumers via adapters, not as schema authorities.

2. **Explicit schema versioning**
   - Every profile must declare a `schema_version`.
   - Validation logic is version-aware and rejects unknown incompatible versions.

3. **Strict compatibility checks**
   - Profiles must encode sufficient acquisition/geometry metadata to verify applicability.
   - rust-daq must reject profiles that do not match required dimensions/ROI/binning/orientation assumptions.

4. **Fail closed, not best-effort**
   - On incompatibility or incomplete data, extraction is disabled with actionable error messages.
   - No silent coercion of geometry or units in MVP paths.

5. **Provenance is required**
   - Profiles must store origin, creation time, and enough metadata to audit how calibration was generated.

## Decision Details

### A. Canonical schema content categories (minimum)

The canonical schema must be able to represent:

#### Detector / acquisition compatibility metadata

- detector dimensions the profile was derived for
- ROI origin and dimensions (or explicit full-frame assumption)
- binning factors
- orientation / axis semantics
- bit-depth assumptions if relevant for masks or normalization

#### Order geometry / tracing

- order identifiers (relative and/or physical order numbers where known)
- trace representation (e.g., polynomial coefficients + domain)
- trace validity bounds / x-range
- optional multi-trace-per-order grouping metadata (for slicer-like setups)

#### Extraction configuration

- aperture half-widths (global and/or per-order)
- summation mode metadata (e.g., top-hat/simple-sum)
- optional background subtraction settings

#### Wavelength calibration

- per-order wavelength representation:
  - coefficients and basis metadata, or
  - sampled wavelength arrays
- wavelength unit
- ordering direction (increasing/decreasing)
- fit diagnostics summary (optional but recommended)

#### Optional calibration corrections

- blaze / flat correction references or embedded data
- bad pixel masks / exclusion regions
- validity ranges and known caveats

#### Provenance and auditability

- creator/tool identity (e.g., rust-daq UI, importer, sidecar)
- creation timestamp
- source calibration frame identifiers/checksums (where available)
- free-form notes / comments

### B. Versioning and compatibility policy

#### Version semantics

- `schema_version` is required and machine-readable.
- Major-version incompatibility is rejected by default.
- Minor-version additive changes should be handled compatibly where possible.
- Patch-level changes should not alter semantics.

#### Compatibility checks (must-pass)

At minimum, rust-daq validation must verify:

- frame dimensions match expected profile dimensions (or ROI-compatible mapping is explicit)
- ROI/binning compatibility
- order trace domains are within image bounds after mapping
- wavelength arrays / mappings are internally valid
- required profile fields for the requested extraction path are present

#### Failure behavior

- Validation failures disable extraction and surface specific reasons to the UI/logs.
- The raw image viewer remains functional.

### C. External tool interoperability policy

- rust-daq may import/export adapters for external tools (e.g., QtYETI-like trace coefficients,
  GAMSE-derived results), but imported data must be normalized into the canonical schema.
- Adapter code must record provenance indicating the original source format/tool.
- Canonical schema evolution must not be blocked by external-tool format limitations.

## Consequences

### Positive

- Establishes a stable contract for Workstream B (schema implementation) and Workstream D (MVP extraction).
- Reduces risk of silent bad spectra from profile misuse.
- Makes sidecar integration cleaner by defining a single canonical exchange target.
- Supports future calibration GUI features with a clear persistence model.

### Negative

- Requires up-front schema/validator design work before extraction can be fully integrated.
- Strict validation may initially reject partially useful profiles until adapter tooling matures.
- Migration logic will eventually be needed as the schema evolves.

### Risks

- Overdesigning schema v1 before enough real calibration data is collected.
- Encoding assumptions too narrowly for future hardware variants.
- Team confusion if profile serialization format is conflated with schema semantics.

## Guardrails

1. Keep schema v1 minimal but complete for MVP extraction.
2. Prioritize compatibility metadata and validation correctness over convenience.
3. Separate **schema semantics** from **serialization format** decisions (can be decided in Workstream B).
4. Include importer/exporter tests with explicit provenance markers.

## Follow-up Work

- Workstream B (`bd-2kla.2.*`) implements schema v1, serde IO, validator, fixtures, and migration strategy.
- Workstream H (`bd-2kla.8.*`) uses the canonical schema for calibration authoring and editing UI.
- Workstream F (`bd-2kla.6.*`) sidecar contract should target canonical schema-compatible payloads/artifacts.
