## Implementation Summary

### What Was Built

The calibration pipeline and preview renderer have been overhauled to handle sparse trace emission efficiently and without visual artifacts.

### Tickets Completed

| Ticket | Title | Status        |
| ------ | ----- | ------------- |
| bd-ai3j | Robust Echelle Calibration & Merged Visualization | closed |
| bd-ft49 | Robust Physical Order Assignment | closed |
| bd-rjhk | Prevent Render Artifacts in Merged Echelle Spectrum | closed |

### Files Changed

- `crates/echelle/src/calibration_pipeline.rs` — Replaced hardcoded order assignments via `trace_idx` with a dynamic `process_single_order` loop over candidate physical orders `m`.
- `crates/echelle/src/types.rs` — Updated `echelle_blaze_sinc_squared` to clamp to `BLAZE_FLOOR`. Modified `build_merged_preview` to inject `f64::NAN` when consecutive wavelengths jump more than `gap_threshold` (2.0 nm) apart.

### Architecture Decisions

Instead of attempting complex non-linear coordinate mappings, it was decided that cross-correlation with the atlas lines is the most robust test of a correct physical order. The pipeline now dynamically scans candidate orders `m` (`first_m - 50` to `first_m + 150`) during calibration, in the per-trace matching loop, to match sparse lines explicitly to physical order indices without depending on contiguous illumination.

### Testing

- [x] All 151 echelle unit tests pass.
- [x] Integration with `cargo test -p echelle`.
- [x] Fix verified against the `mechelle_5000_hgar_leabs.toml` fixture.

### Verification

- [x] All diagnostics clean
- [x] Tests passing
- [x] Build succeeds

### Known Limitations

If multiple `m` candidates have identical matching performance, the algorithm simply picks the first one it encounters. In practice, atomic emission patterns are highly unique across different FSRs, preventing false positives.

### Next Steps

Wait for user review and PR merge; if any regressions or follow-up work are identified, create new tickets in `bd`.
