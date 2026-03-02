# Develop and Maintain Echelle Calibrations (Developer)

This guide is for developers/operators creating calibration profiles for new
Mechelle/iSTAR acquisition modes and maintaining golden datasets.

## Goals

- Create reproducible calibration profiles for specific camera/spectrograph setups
- Validate profile compatibility before lab use
- Maintain golden inputs/outputs for regression testing and numerical comparison

## Calibration Scope per Profile

A profile is specific to an acquisition mode. Treat the following as binding:

- sensor/frame dimensions
- ROI origin and frame size
- binning
- bit depth (if specified)
- detector orientation semantics
- order set and order numbering
- trace representation
- wavelength solution representation
- correction artifacts (blaze/flat/mask) and provenance

If any of these materially change, create a new profile (do not silently reuse).

## Recommended Naming and IDs

- `display_name`: human-readable instrument mode name
- `profile_id`: stable machine identifier (suggested pattern):
  - `mechelle5000-istar-<roi>-bin<bx>x<by>-<date>-v1`

## Calibration Authoring Workflow (Current)

1. Capture representative frames for the target mode.
2. Build/fit traces and wavelength solution in external tooling (or manual process).
3. Export/translate into rust-daq echelle profile schema:
   - `/Users/briansquires/.codex/worktrees/5385/rust-daq/docs/reference/echelle-calibration-profile-schema.md`
4. Load the profile into the Image Viewer (programmatic path today).
5. Validate:
   - profile loads
   - extraction preview renders
   - order traces align visually
   - wavelength mapping looks plausible
6. Save snapshot exports (`JSON` / merged `CSV`) for comparison records.

## Golden Dataset Strategy (Before Full Sidecar/QA Harness)

Maintain a dataset manifest per calibration mode with:

- raw frame files (or representative cropped frames)
- calibration profile file
- expected extracted outputs (reference tool output and/or rust-daq snapshot)
- provenance metadata:
  - source tool/version
  - date
  - operator
  - notes about preprocessing

## Recommended Golden Dataset Layout

Example layout (adapt to local lab storage):

```text
echelle-goldens/
  mechelle5000_modeA/
    profile/
      profile.toml
    raw/
      flat_0001.*
      arc_0007.*
      science_0042.*
    reference/
      order_spectra.json
      merged.csv
      provenance.md
    rust_daq/
      preview_measurements.json
      merged_preview.csv
      comparison_notes.md
```

## Golden Dataset Maintenance Rules

- Never overwrite golden outputs in place without recording why.
- Keep provenance for each regeneration (tool version + commit hash if possible).
- When schema changes:
  - keep original profile
  - add migrated profile
  - compare extracted outputs before replacing a baseline
- Use separate “known-bad” cases for incompatibility/error-path testing.

## Failure Cases to Preserve

Include at least one example for:

- ROI mismatch
- binning mismatch
- unsupported schema major version
- malformed trace domain
- sampled wavelength length mismatch
- heavy saturation / clipped aperture coverage

## Review Checklist for New Calibration Profiles

- `display_name` and `profile_id` are meaningful and unique
- compatibility dimensions/ROI/binning match acquisition mode
- orientation axes are correct
- all intended orders are present and enabled
- trace domains cover order sample ranges
- wavelength units are correct (for example `nm`)
- provenance is populated (`creator_tool`, timestamp, source frames)
- extraction preview renders without validation errors

## Useful Implementation References

- Schema and validation:
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/common/src/echelle.rs`
- Local extractor and debug exports:
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/ui/src/panels/image_viewer/echelle_extraction.rs`
  - `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/ui/src/panels/image_viewer/mod.rs`
