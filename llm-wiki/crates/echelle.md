# crate: `echelle`

<!--
last-ingested: 2026-04-22
sources:
  - crates/echelle/
  - docs/explanation/echelle-extraction-architecture.md
  - docs/reference/echelle-calibration-profile-schema.md
  - docs/reference/echelle-spectrum-streaming-protocol-design.md
  - docs/how-to/echelle-calibration-development.md
  - debug/phase5/FLAT-FIXTURE-DESIGN.md  (bd-cpph3, halogen-primary decision)
see-also:
  - ./ui.md  (echelle panels in image_viewer)
-->

**Role:** Echelle spectroscopy: calibration, order extraction, simulation.

Domain crate — not used by the generic hardware path. Consumed by the
echelle panels in `ui/src/panels/image_viewer/echelle_*`.

**Sub-areas:**

- Calibration profile schema.
- Order extraction pipeline (per-order profiles, wavelength solutions).
- Simulation (synthetic echelle frames for tests).
- Streaming protocol for derived spectra.
- Blaze correction + variance-weighted merging (`blaze` module,
  lamp-agnostic since bd-lj4g4: `LampContinuum`, `fit_lamp_continuum`,
  `compute_blaze_from_flat`). Algorithm — uniform knots, median window,
  positive sigma-clip — handles DH3P, D2-alone, or halogen-alone without
  per-lamp tuning; halogen-alone is the preferred flat post-slit-swap
  (bd-cpph3, Apr 2026).

**Fixture strategy for regression tests:**

- Raw TIFF captures are gitignored (`/debug/**/*.tiff`, Apr 2026 cleanup);
  `echelle_merged_spectrum_regression` and `echelle_hgar_doe_baseline`
  skip cleanly when fixtures are absent. Hardware runners or a maintainer's
  workstation with retained captures exercise the full pipeline.
- The golden reference for the merged-spectrum regression lives under
  `testdata/echelle/reference_merged_spectrum_halogen.hdf5` (tracked,
  ~450 KB).

**Extensive docs:** see the `echelle-*` references in
[`../sources.md`](../sources.md).
