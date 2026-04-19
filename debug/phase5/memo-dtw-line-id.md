# DTW for Echelle Line Identification — Research Memo (bd-rqzvt)

**Date:** 2026-04-18
**Scope:** Evaluate Dynamic Time Warping (DTW) as a replacement or
augmentation for the current echelle-equation-seeded two-phase atlas
match in `crates/echelle/src/wavelength_fitting.rs`.
**Verdict (TL;DR):** **Do not adopt DTW as a replacement.** Consider a
**scoped, optional prototype** only for the coarse Stage-0 pixel→wavelength
seed on *continuum-source* or *densely-lined ICCD* captures where the
echelle-equation seed is suspect. For the sparse HgAr case that dominates
rust-daq's current validation load (2–6 lines/order), DTW degenerates into
something weaker than what Stage 1/2/3 already does.

---

## 1. What DTW actually does for 1D spectra

DTW is a dynamic-programming sequence aligner that finds the
monotone, continuous warp path *i(k), j(k)* minimising
Σ d(a[i(k)], b[j(k)]) between two 1D signals `a` and `b`. For
wavelength calibration, the canonical formulation (Duffy et al.
2025, arXiv 2508.05862, PyKOSMOS) is:

- `a` = observed **flux-vs-pixel** arc spectrum (intensity, not centroid list),
  median-normalised.
- `b` = **flux-vs-wavelength template** of the same lamp at
  comparable R, pre-sampled on a dense grid.
- Cost = L1/L2 distance between intensities (optionally with an
  `asymmetric` step pattern and `open_begin` / `open_end` to tolerate
  spectra that don't share endpoints).

The **warp path itself is the dispersion solution**: for each observed
pixel *i*, `j(i)` gives the template sample — hence the template
wavelength. A polynomial or GP is fit through the path as a smoothing
pass. The paper explicitly reports a two-stage design — DTW for a
*first-pass* alignment, then strong-line identification + smooth
model — which is the same split-of-labor as our Stage 1/2/3.

Echelle-specific adaptation, per the paper: "DTW could be used to align
each extracted order of an echelle spectrum, given an appropriate
template" — explicitly named **future work, not implemented**. The
paper's tests are all long-slit (KOSMOS, DIS).

## 2. Comparison to the current rust-daq method

| Aspect | Current Stage 1/2/3 | DTW |
|---|---|---|
| Seed requirement | Echelle-equation `λ(x,m)` or Cauchy `Y(m)` | Template spectrum at similar R |
| Matching granularity | Per-line window + gc consistency | Global sequence alignment |
| Tolerances hand-tuned? | Yes (`primary_window_nm=2.0`, `final_tolerance_nm=seed_tolerance_nm`) | No window; replaced by cost function + band constraint |
| Works per-order | Yes (n. physical order m is explicit) | Not natively — paper applies to 1 spectrum; echelle would be per-order with per-order template |
| Handles reversed dispersion sign | Yes (orientation flip in echelle equation) | **No** — monotone warp path requires matching line ordering; a sign-flipped seed causes catastrophic mis-alignment with no recovery |
| Handles missing lines | Yes (per-line; 1-line fallback in bd-3hlp) | Partially — paper reports success with ~25% line deficit (Duffy 2025 §5.2) but not with 80%+ deficit which is our HgAr regime |
| Handles spurious lines | Yes (gc self-consistency + RMS gate, bd-0poyt) | "Accommodates some" (paper's phrasing); no quantitative bound |
| Minimum line density | 2/order (bd-3hlp single-line fallback is physics-based) | Paper success case: 30 lines / 8900 Å ≈ 1 line / 300 Å; our Mechelle orders span ~5–30 nm each with 2–6 lines → within an order of magnitude but at the edge |
| CPU | O(orders · lines_per_order) — sub-ms per order | O(N·M) cost matrix per order; paper quotes 1.6 s for a full spectrum |

Failure modes DTW **would** fix:

- **Hand-tuned tolerance sensitivity** (bd-0poyt): global alignment
  amortises the tolerance across all features rather than needing a
  per-line window.
- **Seed drift at order extremes**: if the echelle-equation seed is
  off by >2 nm at an order edge, the current two-phase match drops
  that line; DTW would re-bind it if the rest of the order is sane.

Failure modes DTW **would introduce**:

- **Monotonicity is a hard constraint.** If the echelle-equation
  orientation sign is wrong (bd-ccer6 scenario), DTW aligns the whole
  order to the reversed region of the template. The current per-line
  search has no such global lock-in.
- **Template is a new calibration artifact.** Today the atlas is 29
  HgAr line centroids (`load_hgar_atlas`). DTW wants a full-flux
  template at matched R — we'd have to either synthesise one from the
  atlas via Gaussian convolution, or capture a high-SNR reference
  exposure per instrument/grating combination. That becomes a
  versioned asset tracked per device.
- **Sparse-regime degeneracy.** With only 2–6 lines in 5–30 nm and
  large inter-line gaps of pure noise, DTW's intensity cost is
  dominated by the noise region. Local minima in the cost matrix
  easily produce swapped assignments — the paper notes catastrophic
  failure in §5.3 when template and data have "significantly different
  emission line amplitudes and profiles," which is essentially our
  HgAr-across-bright-and-weak-orders situation.

## 3. Fit for sparse HgAr arcs — the decisive point

The Duffy et al. demonstration uses 30+ lines across 8900 Å; their
"challenging" robustness case removes ~25 % of lines. Our HgAr
regime has orders with **1–3 atlas lines in 10 nm** after blaze
attenuation — i.e. the *useful* line density is 30–100× lower. In
this regime the DTW cost matrix is dominated by the continuum/noise
cells and the band-constrained minimum path is effectively
degenerate: it will track whatever baseline gradient exists and the
"alignment" of the sparse features collapses back to a per-line
nearest-neighbour assignment — exactly what `match_lines_two_phase`
already does, but without the physics-aware gc-consistency filter
(bd-mlwvz guard).

## 4. Cost vs. benefit

**Rust implementation cost:** moderate. `dtw-rs` (shshemi, MIT,
~150 commits, v0 no crates.io release) exposes Sakoe-Chiba band,
Itakura parallelogram, FastDTW, and the warp path via
`Solution::path()`. `rustDTW` (FL33TW00D) is PyO3-targeted and
unmaintained since 2021. `augurs-dtw` is active but focused on
time-series forecasting and lacks band constraints. Plausible plan:
vendor or fork `dtw-rs`'s band-constrained DP (~200 LoC, no deps).
Memory is O(N·M); for a 2048-pixel order against a 4096-sample
template, 8 MB f64 — fine.

**Pain points DTW would actually relieve:**

- bd-0poyt (RMS gate, hand-tuned tolerance): **partially** — DTW
  shifts the knob from `seed_tolerance_nm` to `band_radius_pixels`,
  which is arguably easier to reason about (fraction of detector
  width) but is still a knob.
- bd-mlwvz (derived-m collisions): **no** — DTW operates per-order
  against a per-order template, so the cross-order collision does not
  arise, but the problem has *already been fixed* by the Stage 1/2/3
  rewrite (PR #603). Not a live pain point.
- bd-3hlp (single-line fallback for pure Hg): **no, worse** —
  DTW with 1 feature in the entire order has no alignment signal and
  would need the same physics-based fallback we already ship.

**Pain points DTW does not address:** order identification (unknown
physical m), flat-fielding, blaze correction, scattered light — all
the things that actually cost us days of validation on real captures.

## 5. Recommendation

**Do not replace** Stage 1/2/3. The current pipeline is physics-aware
(echelle equation + Cauchy Y(m) + 2D Chebyshev surface) and already
handles the failure modes DTW would marginally improve. The only
published DTW-for-spectroscopy result in astronomy (Duffy et al.
2025) explicitly avoids echelle and avoids sparse lamps.

**Do consider a prototype** scoped to one narrow use case: when
future rust-daq work switches to **dense-line arcs** (ThAr/Ne, as
used by xwavecal, HARPS, NRES) or **continuum-source wavelength
transfer** (iodine cell style), a DTW pre-alignment could replace the
echelle-equation seed and remove the orientation-sign dependency for
that lamp class only. That is not the current HgAr LIBS workload.

### If implemented — sketch

- **Location:** new module `crates/echelle/src/dtw_seed.rs`, optional feature flag `dtw_seed`.
- **Data rep:** `DtwAlignment { warp_path: Vec<(u32, u32)>, cost: f64, band_radius: usize }`; inputs `observed: &[f64]` (extracted 1-D order flux), `template: &[f64]`, `template_wavelengths: &[f64]`.
- **Algorithm:** band-constrained DP (vendor from `dtw-rs`, ~200 LoC, no runtime deps), Sakoe-Chiba band = 5 % of pixels default, L1 cost on median-normalised intensity.
- **Integration point:** replaces the *seed function* passed to `match_lines_two_phase` at `calibration_pipeline.rs:1092`. Stage 1/2/3 runs unchanged on the warp-derived λ(x) seed.
- **Fallback path:** if DTW cost exceeds a threshold or monotone-feature check fails (count of local-max alignments < N_min), fall back to the current echelle-equation seed. Gate behind `WavelengthFitConfig::seed_source: Echelle | Dtw`.
- **Template provenance:** generated from `load_hgar_atlas()` by Gaussian broadening to the instrument's R, stored as an HDF5 template asset under `data/echelle/templates/<instrument>.h5` with version + R + grating-constant tags.
- **Effort estimate:** 2–3 days implementation, 2 days validation harness against PyKOSMOS reference case, 1 day Phase-E regression on HgAr captures. Close this bead with the prototype *behind a feature flag*, not wired into the default pipeline.

---

## References

1. Duffy, A. et al. 2025. *Automated Spectroscopic Wavelength Calibration using Dynamic Time Warping.* arXiv:2508.05862. <https://arxiv.org/abs/2508.05862>, HTML: <https://arxiv.org/html/2508.05862v1>. (PyKOSMOS implementation; long-slit only; §5.2 robustness, §5.3 failure modes.)
2. Brandt, G. M. et al. 2020. *Automatic Échelle Spectrograph Wavelength Calibration* (xwavecal). AJ 160:25, arXiv:1910.08079. <https://arxiv.org/abs/1910.08079>. (Blind echelle calibration *without* DTW; uses FSR-overlap + global fit. Reference for what the alternative to DTW looks like in the echelle-specific literature.)
3. PypeIt Wavelength Calibration docs: <https://pypeit.readthedocs.io/en/1.16.0/calibrations/wave_calib.html>. `full_template` uses cross-correlation with shift+stretch on snippets; `holy-grail` uses KD-tree polygon pattern matching; `reidentify` cross-correlates against archived solutions. **No DTW**.
4. PypeIt `kdtree_generator` / `autoid` modules: <https://pypeit.readthedocs.io/en/1.17.0/_modules/pypeit/core/wavecal/kdtree_generator.html>. Polygon (trigon/tetragon/pentagon) pattern matching is PypeIt's answer to the sparse-lamp problem — *not* DTW.
5. ESO MIDAS IDENT/ECHELLE: <https://www.eso.org/sci/software/esomidas/doc/user/18NOV/volb/node472.html> and Ballester 1992, *Reduction of Echelle Spectra with MIDAS*, ESOC 41, 177. <https://adsabs.harvard.edu/full/1992ESOC...41..177B>. Physical-model line identification with tolerance window + dispersion refinement — **no DTW**; closest published kin of our current Stage 1.
6. Zou, W. et al. 2019. *Scalable calibration transfer without standards via dynamic time warping for near-infrared spectroscopy.* Chemometrics & Intelligent Lab Systems. <https://www.researchgate.net/publication/335138310>. The only prior DTW-spectroscopy work cited by Duffy 2025; NIR chemistry, not echelle.
7. `dtw-rs` crate (Shahab Shemi, MIT): <https://github.com/shshemi/dtw-rs>. Supports Sakoe-Chiba band, Itakura parallelogram, FastDTW, path extraction. Only plausible Rust dep; small enough to vendor.
