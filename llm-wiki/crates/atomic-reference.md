# crate: `atomic-reference`

<!--
last-ingested: 2026-04-21
sources:
  - crates/atomic-reference/Cargo.toml
  - crates/atomic-reference/src/
  - docs/design/nist-asd-integration-decision.md
see-also:
  - ./echelle.md
  - ../architecture.md
-->

**Role:** Pure-data NIST ASD atomic-emission-line reference crate for
spectroscopy calibration and LIBS species identification.

**Status:** Stable workspace member. It is dependency-free and `publish = false`.

**Key API:**

- `AtomicLine` — typed line record with wavelength, element, ionization stage,
  Einstein A coefficient, energy levels, statistical weights, and NIST accuracy grade.
- `LINES` — generated const table committed in `src/lines_data.rs`.
- `lines_for_element(element, sp_num)` — iterate lines for one species.
- `lines_in_range(lo_nm, hi_nm)` — iterate lines in a wavelength interval.
- `lines_for_species(species)` — one-pass multi-species filtering.
- `atlas` module — conversions to legacy echelle atlas shapes.

**Regeneration:** `uv run scripts/data/regenerate_atomic_reference.py`.
The generated line table should not be hand-edited.
