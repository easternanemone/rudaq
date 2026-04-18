# Echelle Calibration Pipeline — Literature Audit

**Date:** 2026-04-18
**Notebook consulted:** NotebookLM `7f275c3a-88bb-464a-9437-ed02ca776207`
"Echelle extraction options for rust-daq" (18 sources — IRAF, PypeIt,
CERES, REDUCE, HiFLEx, ESO MIDAS, STIS, HET HRS, CAFE2, muler).

All references in `[brackets]` below are to the notebook's digested
"Pipeline Evaluation" synthesis memo (source id
`3a0a13df-ce03-46d7-becc-3b88f82e4db8`) and "Echelle Calibration
Algorithms" synthesis (id `e1f9981c-fade-4d12-9863-58e47a5319e8`).

## TL;DR

Our current three-pass pipeline (Pass 1 candidate-m search, Pass 2
quadratic `m(i)` regression, Pass 3 2D Chebyshev residual bootstrap) is
**not** a standard echelle calibration algorithm. Every canonical
pipeline (IRAF ECIDENTIFY, ESO MIDAS IDENTIFY/ECHELLE, PypeIt, CERES,
REDUCE) converges on the same structure:

1. Per-order arc line detection + 1D wavelength fit (Chebyshev/Legendre
   degree 3–5) on orders with ≥3 unambiguous atlas matches.
2. A **single global 2D Chebyshev fit** `λ(x, m)` across *all* matched
   lines from *all* orders simultaneously (typical degrees 3×5 to 4×4).
3. Iterative 3σ rejection.
4. **Recover uncalibrated orders by evaluating the 2D surface**, refit.

Our pipeline has three structural deviations, plus four additional
physical-model defects documented in the prism-echelle literature.

## Five canonical failure modes (per the literature memo)

### FM1 — 3-pass Chebyshev bootstrap breaks on prism cross-dispersers

A standard 2D Chebyshev over `(x, m)` assumes smooth continuous
behaviour on the normalized domain. The Mechelle 5000's fused-silica
prism introduces Sellmeier-law exponential dispersion (compressed in
the NIR, expanded in the UV). Low-degree Chebyshev underfits the UV;
high-degree Chebyshev Runge-oscillates in the tightly-packed NIR.

**Literature prescription** (eval memo §FM1): fit a **correction
surface** to a deterministic physical base model, not `λ(x, m)`
directly. Base model = grating equation `m·λ = gc` + known focal
length + prism Sellmeier. Residual = low-degree (3×3) 2D Chebyshev
capturing mounting / thermal residuals. Modified 3-pass:
1. Fit only central orders (pseudo-linear prism region).
2. Extrapolate to UV/NIR with SNR-weighted LSQ (down-weight <10σ lines).
3. Lock `m` coefficients, optimize only spectral-axis coefficients.

**Our code**: `calibration_pipeline.rs:611,1360` fits
`m(i) = a + b·i + c·i²` where `i` is relative trace index. The
comment at line 609 claims this "captures Cauchy dispersion Y ∝ m²",
but Cauchy dispersion is `Y(m) ≈ a + b/m² + c/m⁴` (series in `1/m²`),
and the predictor is trace-index `i`, not `Y` or `m`. Triply
mis-derived. The Pass 3 residual-only 2D Chebyshev is then used as an
extrapolator beyond the anchor set — the literature's textbook
Runge-oscillation case.

### FM2 — Catastrophic over-detection of traces

On Mechelle 5000 + iStar, naive peak/Sobel-edge tracing routinely
detects ~143 orders when only ~74 exist (ICCD MCP blooming + prism
ghost orders). Latest capture on leabs-dev detected 109 traces vs
expected ~74 — a 47% over-count.

**Literature prescription** (eval memo §FM2):
- Hough transform + CWT/Savitzky-Golay smoothed peak init.
- Monotonic Δy rule (inter-order spacing must increase toward blue).
- Gaussian FWHM band: reject peaks >40% wider than slit image.
- Continuity ≥60% of dispersion axis.
- Absolute SNR floor 3.0.

**Our code**: `trace_validation.rs` only checks optional `min_snr`.
No FWHM band, no monotonic-spacing rule, no continuity rule. (Brian:
`trace_validation.rs:42` — `min_snr: Option<f64>` is the only filter.)

### FM3 — Merged spectrum scalloped without blaze correction

Blaze transmission falls >50% at order edges (documented for Mechelle
5000 in Zarini, and confirmed by MELCHIORS). Without DH3P flat-division
the merged spectrum has cusp-shaped discontinuities at every order
boundary. HG-2 Ar lines in the NIR have intensities lower than the
edge-rolloff of the adjacent Hg order and disappear into the scallop.

**Literature prescription** (eval memo §FM3): divide each extracted
order by the DH3P flat extracted with identical aperture; merge via
1/σ² weighted ramp across overlaps.

**Our code**: no blaze correction. `plot_final.py` uses per-bin max
across orders, which is a display hack, not physics. The DH3P flat is
already ingested for trace detection but never used as a divisor.

### FM4 — 4500-count ICCD baseline anomaly

On emission-line sources, MCP electron halos bleed into the inter-order
gaps at ≈4500 ADU. Standard inter-order-minima scattered-light
subtraction (CERES `get_scat`, IRAF `apscatter`) samples the halo,
treats it as global background, and over-subtracts real continuum.

**Literature prescription** (eval memo §FM4): 2D morphological opening
(erosion→dilation) with a vertical structuring element ≈25 px
(order_width + halo). Sparse anchor points excluding 15-px radius
around saturated lines. High-stiffness B-spline or thin-plate-spline
surface.

**Our code**: `scattered_light.rs:143-170` uses sigma-clipped block
median of inter-order mask. Post P2.5 fix it's 3-pass sigma-clipped,
but still samples halo pixels. No morphological opening, no saturated-
line anchor exclusion, no B-spline/TPS surface.

### FM5 — ICCD Excess Noise Factor F=1.6

Gen II/III MCP image intensifiers (Andor iStar) have stochastic
avalanche gain with F≈1.6. Standard CCD variance `V = rn² + S/g` is
wrong for ICCD. Correct: `V = rn² + F²·S/g`. Cosmic-ray rejection at
5σ with correct F matches typical CCD rejection at 10σ.

**Our code**: F is plumbed (`optimal_extraction.rs:41`), the
`OptimalExtractionConfig::for_istar()` preset at line 72 sets `F=1.6`.
But the default `OptimalExtractionConfig::default()` (line 54) uses
`F=1.0` (CCD), and `calibration_pipeline.rs:148` hard-codes
`OptimalExtractionConfig::default()`. Additionally the Mechelle config
has `use_optimal_extraction = false`, so extraction currently uses
boxcar (no variance model). F=1.6 is wired but not invoked by default.

## Wavelength-calibration algorithm deviations (FM1 expansion)

Three specific deviations in `calibration_pipeline.rs`:

### D1 — Per-trace candidate-m search is non-canonical

Lines 482–538 loop candidate `m ∈ [expected_m ± 3]`, fit the solution
at each candidate, pick the best. Uniqueness is enforced on
`candidate_m`, but `physical_order_number` is derived from the fit's
midpoint wavelength (`round(gc / λ_center)` in
`build_order_calibration`). Two different candidate_m values can
produce fits with colliding derived_m. Filed as **bd-mlwvz** (P1,
open).

**Canonical approach** (IRAF ECIDENTIFY, PypeIt, CERES): m is
assigned once, globally, from the physical grating equation at each
trace's y-centroid. Collisions can't happen because m is deterministic
from optics. Per-trace candidate search is a band-aid for our lack of
a physical trace_Y→m model.

### D2 — Pass 2 `m(i)` quadratic regression is physically unjustified

Line 611 fits `m(relative_trace_index) = a + b·i + c·i²`. There is no
optical theory under which trace index is a polynomial predictor of
physical order. The relationship the literature uses is the Cauchy
series on (Y, m): `Y(m) = a + b/m² + c/m⁴`. We should either:

- Use `Y(m) = a + b/m² + c/m⁴` fit from successfully-calibrated
  (Y_trace_centroid, m) pairs (FM1 physical base model), OR
- Drop Pass 2 re-seeding entirely and rely on the global 2D Chebyshev
  to recover failed orders (PypeIt approach).

### D3 — Pass 3 residual bootstrap runs on a residual surface, not a physical base

Lines 1218-1370 fit a 2D Chebyshev surface to the *residuals* from
calibrated orders (`λ_observed - λ_physics_baseline`), then use that
surface to predict wavelengths for uncalibrated orders. Because the
physics baseline is `gc/m + FSR/npx·(x - w/2)` — a linear-in-x
approximation — the "residual" still absorbs the prism's exponential
curvature. When that residual surface is evaluated outside the anchor
set's (x, m) envelope, it Runge-oscillates exactly as FM1 warns.

**Canonical approach** (IRAF/CERES): the 2D Chebyshev is the full
wavelength solution, not a residual correction. Fit it jointly across
all matched lines with 3σ sigma-clipping; recover failed orders by
*interpolating* the surface inside its valid domain. No extrapolation.

## Proposed remediation plan (science-first)

Each item will be a beads issue; each passes a literature citation and
an explicit code pointer.

### Phase A — wavelength-axis correctness (blocks everything else)

- **A1**: Replace Pass 2/3 with a **global 2D Chebyshev fit** over all
  matched arc lines. Degrees 3×5 (CERES default). 3σ iterative
  rejection. Recover uncalibrated orders by interpolation only. Delete
  `quadratic_regression`, delete Pass 3 residual-surface bootstrap.
- **A2**: For trace_Y → m assignment, fit `Y(m) = a + b/m² + c/m⁴`
  from successfully-matched orders (Cauchy series, the literature's
  prism-dispersion model).
- **A3**: Delete per-trace candidate-m search (`calibration_pipeline.rs:
  482-538`). m is deterministic from the Cauchy-series Y(m) fit.
  Closes bd-mlwvz as a side effect.

### Phase B — scattered-light physics

- **B1**: Add `MorphologicalOpening` subtraction mode to
  `scattered_light.rs`. 25-px vertical structuring element, saturated-
  line anchor exclusion, high-stiffness B-spline or TPS surface. Keep
  current sigma-clipped median as a non-default fallback.
- **B2**: Default `mechelle_5000.toml` scatter mode → morphological
  opening.

### Phase C — trace validation against FM2

- **C1**: Add to `TraceValidationConfig`: `max_fwhm_excess_fraction`
  (default 0.4), `min_continuity_fraction` (default 0.6), and enable
  an inter-order-spacing monotonicity check.
- **C2**: Optional: Hough-transform-seeded trace detection path (new
  trace-finder alternative to current peak-march).

### Phase D — ICCD variance + science extraction

- **D1**: Wire `OptimalExtractionConfig::for_istar()` (F=1.6) by
  default when the detector is ICCD. Add a detector-kind hint on
  `mechelle_5000.toml`.
- **D2**: Default `use_optimal_extraction = true` once flats are
  available. Left as a follow-up until Phase B lands.

### Phase E — blaze correction + order merging

- **E1**: Divide each extracted order by the DH3P-flat extraction
  using identical aperture. Add to the profile as
  `BlazeCorrection { flat_id, smoothing }`.
- **E2**: Implement 1/σ² weighted ramp merge across overlaps. Remove
  the per-bin-max hack from `plot_final.py`; keep it only as a
  diagnostic option.

### Phase F — hardware validation

- **F1**: Re-capture 5×30s HgAr stack (have one already at
  `debug/phase4/v2/hgar-stack5x30s-median.tiff`).
- **F2**: Re-run calibration with the new pipeline. Assertions:
  - Every matched Hg / Ar atlas line in a calibrated order is within
    0.1 nm of the literature value.
  - Hg 253.652, 365.015, 404.656, 435.833, 546.074 all within 0.05 nm.
  - Ar 763.511 and ≥3 other Ar lines within 0.1 nm.
  - Merged spectrum has smooth continuum in overlaps (no scallops).

## Sources

1. NotebookLM "Echelle extraction options for rust-daq"
   (7f275c3a-88bb-464a-9437-ed02ca776207) — 18 sources incl:
   - IRAF ECIDENTIFY help (noao.imred.echelle)
   - PypeIt docs, `wave_calib.html`
   - CERES (arXiv:1609.02279)
   - REDUCE (Piskunov & Valenti 2002)
   - STIS 2D scattered-light (Valenti, STScI 2002)
   - HET HRS ThAr atlas
   - Starlink SG9 echelle intro
   - HDS IRAF echelle reduction manual (Subaru)
2. Pipeline Evaluation memo (3a0a13df, Google Docs) — 5 failure modes
3. Algorithms synthesis memo (e1f9981c) — 4 approaches summary
4. Problem statement memo (090db1b4) — our specific symptoms
5. Asta Research cross-check (c959d054) — 12-paper review
