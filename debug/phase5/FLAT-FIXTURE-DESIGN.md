# Mechelle 5000 flat-field fixture strategy (post-slit-swap)

**Date:** 2026-04-21
**Bead:** bd-cpph3
**Related:** bd-61aej (DH3P DoE), bd-1b5lb (echelle cleanup)

## Summary

Direct comparison of three lamp configurations against the Mechelle 5000
+ Andor iStar rig at leabs-dev, HgAr arc `hgar_g500_t1000ms.tiff` as the
arc frame, and the rest of the `config/calibration/mechelle_5000.toml`
defaults (atlas 359 HgAr lines, gate 0.7 nm, `min_lines_per_order = 2`,
`allow_single_line_fallback = true`):

| Flat | Integration | Orders detected | Orders calibrated | Overall RMS | m-range | λ-range |
|---|---:|---:|---:|---:|:---:|:---:|
| **Halogen g100/acc5**   | **2.5 s**  | **142** | **37** | **0.271 nm** | 24 - 84 | **237-858 nm** |
| D2 g4095/acc30          | 900 s      | 66      | 32     | 0.359 nm     | 25 - 84 | 237-810 nm     |
| DH3P combined (bd-61aej) | 300 s      | 17      | 10     | 0.401 nm     | 25 - 48 | 418-810 nm     |

Halogen-alone wins on every measurable metric: most orders detected,
most orders calibrated, lowest per-order wavelength-fit RMS, and the
full 237-858 nm Mechelle spectral range including deep UV — all with
2.5 s effective integration.

## Why halogen alone covers the UV

A 3000 K tungsten-halogen blackbody at 237 nm is approximately 10⁻⁸ of
its NIR-peak intensity. Naive estimate says halogen should be
invisible in the UV. But the iStar MCP intensifier's combined QE ×
gain is ~10⁶ at 237 nm, so detected signal is still ~100× above the
inter-order pedestal — enough for the trace detector to resolve order
stripes. The UV floor isn't the lamp's absolute output; it's the
*detector's bottom-of-band response*, which halogen beats easily.

D2's nominal advantage (UV-preferred emission) is real at the optical
plane but does not translate to superior on-detector signal because
the same detector-QE cliff applies. In fact D2 performs WORSE at every
tested gain × integration — presumably because D2's total throughput
is lower than halogen's UV tail after 360× less integration.

## Non-obvious observation: raw-pixel contrast underreports usability

The D2 analyzer scored every frame as `uv_underexposed` (UV contrast
ratio ~1.1 vs target ≥3). Yet the pipeline calibrated 32 orders with
0.36 nm RMS from the same frame. Root cause: `p97 / p50` measures
band-wide *bright-to-median* ratio, but orders on a Mechelle are thin
(~1 px wide) stripes — they make up <5% of pixels in any row band, so
their per-pixel contrast is diluted in the percentile statistic. The
trace detector (per-column peak-finder with local neighborhood
comparison) sees a much higher contrast locally than the global
histogram implies.

**Takeaway:** raw-pixel contrast is a necessary-but-not-sufficient
filter for flat-field usability. Always confirm with a pipeline
acceptance test (`rust-daq-daemon calibrate --frame arc --flat flat
--diagnose`) — that's the metric downstream consumers actually see.

## Decision

- **Primary flat:** `halogen_g100_t500ms_acc5.tiff` (2.5 s integration)
  from `debug/phase5/halogen_matrix/`. Replaces the legacy combined
  DH3P flat everywhere the calibration pipeline consumes a flat frame.
- **Secondary fixtures:** keep the full D2 DoE (5 frames) and halogen
  DoE (6 frames) committed as reproducibility baselines and for
  diagnostics on future optical-path changes (slit swaps, fiber
  re-coupling, lamp degradation). No HDR-merge is needed for routine
  calibration — halogen alone covers the full band.
- **Archived:** the DH3P combined flat (`debug/phase5/dh3p_matrix/`
  from PR #635) stays committed for historical continuity but is no
  longer the default.

## Integration points this decision touches

- `crates/integration-tests/tests/echelle_merged_spectrum_regression.rs` —
  NOT updated in this PR. The regression test's pipeline calls
  `echelle::blaze::fit_dh3p_continuum` which hardcodes the bimodal
  DH3P shape (D2 peak UV + halogen peak NIR); pointing it at a halogen-
  only flat returns `None` / "continuum fit failed". Filed as bd-lj4g4
  (generalize blaze fitter to lamp-agnostic); that bead switches the
  regression test's fixture once the fitter supports halogen-shape
  continua.
- `debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff` — legacy
  pre-slit-swap flat, still the regression fixture until bd-lj4g4
  lands. After bd-lj4g4 it becomes unused.

## What we explicitly decided not to do

- **Combined DH3P + separate D2 + separate halogen (three-way HDR
  merge):** rejected because halogen alone covers the full Mechelle
  band, so merging adds complexity without calibration benefit. The
  D2 fixture remains available if a future workflow specifically needs
  UV-only flat characterization (e.g., for blaze envelope validation
  independent of halogen leakage), but it is not in the default path.
- **Retiring the DH3P combined fixture:** left committed for audit
  trail / reproducibility. If future work needs a historical reference
  frame under the post-slit-swap configuration, the 6-frame DH3P DoE
  in `debug/phase5/dh3p_matrix/` is that reference.
