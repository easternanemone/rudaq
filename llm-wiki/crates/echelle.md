# crate: `echelle`

<!--
last-ingested: 2026-04-19
sources:
  - crates/echelle/
  - docs/explanation/echelle-extraction-architecture.md
  - docs/reference/echelle-calibration-profile-schema.md
  - docs/reference/echelle-spectrum-streaming-protocol-design.md
  - docs/how-to/echelle-calibration-development.md
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

**Extensive docs:** see the `echelle-*` references in
[`../sources.md`](../sources.md).
