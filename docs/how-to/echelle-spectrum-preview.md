# Echelle Spectrum Preview (MVP)

This guide describes the current MVP path for converting a Mechelle/iSTAR
echellegram into a preview spectrum inside the Image Viewer.

## Current MVP Scope

- Runs locally in the UI `ImageViewerPanel` (no sidecar required)
- Uses a versioned calibration profile (`.toml` / `.json`)
- Supports:
  - order trace evaluation (polynomial)
  - top-hat/simple-sum extraction
  - optional sideband background subtraction
  - per-order wavelength mapping (polynomial or sampled arrays)
  - merged wavelength-sorted preview plot
- Includes throttling (extract every Nth frame)

## What Is Not Yet Final

- Calibration GUI authoring workflow (trace editing, line ID, wavelength fitting)
- Protocol-level streaming of full spectra across gRPC
- Production-quality optimal extraction
- Full bad-pixel mask artifact loading (hook exists; integration pending)

## Calibration Profile

Use the schema documented in:

- `/Users/briansquires/.codex/worktrees/5385/rust-daq/docs/reference/echelle-calibration-profile-schema.md`

Test fixture example:

- `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/common/tests/fixtures/echelle_profile_v1.toml`

## Programmatic Setup (Current Path)

There is not yet a dedicated UI file picker for echelle calibration profiles.
For now, set the profile path programmatically:

```rust
use std::path::PathBuf;

// image_viewer: &mut ImageViewerPanel
image_viewer.set_echelle_profile_path(PathBuf::from("/path/to/mechelle_profile.toml"));
```

The profile is hot-reloaded on file modification and preserves the last-good
profile if a reload fails.

## Using the Preview in Image Viewer

When a valid profile is loaded and frames are arriving:

- The side panel shows `Echelle Spectrum (MVP Preview)`
- Controls:
  - `Enabled` toggle
  - extraction cadence (`Every N frames`)
  - `Merged` toggle (merged vs selected order plot)
  - order selector
- The panel shows:
  - spectrum plot (counts vs wavelength)
  - coverage/valid-fraction/saturation summary for the selected order

## Runtime Compatibility Checks

The extractor validates the live frame against the profile before extracting:

- frame width/height
- bit depth (when specified in profile)
- ROI/binning if runtime values are available

If incompatible, extraction errors are shown in the echelle preview panel and
the last successful preview is retained for continuity.

## Developer Export / Debug Hooks

`ImageViewerPanel` exposes developer-oriented hooks for snapshot export:

- `echelle_preview_measurements()`
  - returns `Measurement::Spectrum` values for each order plus merged preview
- `save_echelle_preview_measurements_json(path)`
  - exports order + merged spectra with metadata as JSON
- `save_echelle_preview_merged_csv(path)`
  - exports merged preview as `wavelength,flux` CSV

These hooks are intended for development validation and comparison against
reference tools while the dedicated calibration UX is still under development.

## Related Implementation Files

- `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/ui/src/panels/image_viewer/echelle_extraction.rs`
- `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/ui/src/panels/image_viewer/echelle_profile_cache.rs`
- `/Users/briansquires/.codex/worktrees/5385/rust-daq/crates/ui/src/panels/image_viewer/mod.rs`
