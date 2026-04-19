# Memo: Hough-Transform + CWT Upstream Trace Finder

**Bead:** bd-fzbn0
**Date:** 2026-04-18
**Author:** research agent (Opus 4.7, 1M)
**Status:** research verdict — **do not implement now; defer behind bd-g22gu.3**

---

## 1. Context recap

NotebookLM FM2 (pipeline-eval memo §FM2) prescribes upstream ghost rejection via a
Hough transform over detector positions combined with a Continuous Wavelet
Transform (CWT) / Savitzky-Golay peak finder, *replacing* the PypeIt-style
peak-walker in `crates/echelle/src/trace_fitting.rs`. Motivation: on Andor
Mechelle 5000 + iStar ICCD, MCP electron halos spray secondary peaks into the
inter-order gaps (FM4); the peak-walker follows those as "orders" and the
post-detection filters in `trace_validation.rs` have to be default-off because
they also reject real NIR orders that sit in halo-contaminated regions.

The question: does Hough+CWT actually eliminate that failure mode upstream, and
— crucially — now that bd-g22gu.3 has made flat-scatter subtraction the default
on continuum frames, is the upstream fix still needed at all?

## 2. Algorithm sketch (2-stage pipeline)

**Stage A — CWT peak localization per column slice.** Following PypeIt
`find_lines_cwt`, which wraps `scipy.signal.find_peaks_cwt`
([SciPy docs](https://docs.scipy.org/doc/scipy/reference/generated/scipy.signal.find_peaks_cwt.html);
[PypeIt wavecal docs](https://pypeit.readthedocs.io/en/1.16.0/calibrations/wave_calib.html)),
you convolve each cross-dispersion column with a Ricker ("Mexican-hat") wavelet
at a bank of scales `a ∈ {2, 3, 4, 5, 6, 8}` px. Local maxima that persist
across ≥3 adjacent scales and clear a per-scale SNR floor are kept. MCP halo
splotches are broad (FWHM 15-30 px on Mechelle 5000) and therefore suppressed
at the 2-6 px scale band where real slit-image peaks dominate; isolated
cosmic-ray hot pixels are rejected because they don't survive scales ≥3 px.

**Stage B — Hough accumulator over order trajectories.** Each
`(x_col, y_peak)` from Stage A is a point in detector space. For a near-linear
echelle, parameterize by (slope `m`, intercept `b`) — peaks that lie on the
same order contribute coherent votes to the same `(m,b)` bin, while scattered
MCP halo peaks project diffusely across parameter space. This is the Ballester
(1994) MIDAS "Method Hough" approach
([ESO echelle reduction node471](https://www.eso.org/sci/software/esomidas//doc/user/98NOV/volb/node471.html)):
the accumulator produces a characteristic "butterfly" of peaks, one per order;
Stage B extracts the peaks, follows each back to the detector plane, and
polynomial-refines. Crucially, MIDAS explicitly assumes **constant order slope
in a narrow range `]0, 0.5[`**.

## 3. Cauchy Y(m) compatibility — the first serious problem

On the Mechelle 5000 prism cross-disperser, order separation follows a Cauchy
series `Y(m) = a + b/m² + c/m⁴` (see `crates/echelle/src/cauchy_dispersion.rs`),
not a linear ladder. Within a single order, the trace along dispersion is
near-parabolic. Consequences:

- **A (ρ, θ) line-Hough will systematically misvote** on curved orders. The
  `imageproc::hough` crate ([docs](https://docs.rs/imageproc/latest/imageproc/hough/index.html))
  exposes only `detect_lines(LineDetectionOptions { vote_threshold, suppression_radius })`
  and `PolarLine` — no curve parameterization. Any real use requires either a
  piecewise-linear tangent decomposition (chop the detector into 3-5 X-bands,
  run line-Hough in each, stitch) or a generalized Hough with a 3-D accumulator
  over `(c0, c1, c2)` quadratic coefficients.
- **3-D accumulator cost.** For a 2560×2160 frame with realistic quantization
  (200 bins per coefficient), a 3-D accumulator is `200³ = 8 × 10⁶` cells ×
  4 B = 32 MB, tractable but not trivial. A 4-D accumulator for cubic traces
  (current `poly_degree=4`) blows to `6.4 × 10¹⁰` cells — infeasible. A
  coordinate transform (`Y = a + b/m² + c/m⁴` with `m` as the vote axis) is
  closer to physical truth but requires a prior on `m` per column, which is
  what we're trying to find in the first place.
- **Piecewise-linear hack** (3-5 tangent bands): works, but now you have to
  merge track segments across bands, which re-introduces a peak-walker-like
  stitching stage and brings back exactly the ghost-continuity problem we were
  trying to escape. See the
  [Hough parabola voting paper](https://link.springer.com/chapter/10.1007/978-3-030-63403-2_37)
  for the general 3-D/4-D cost analysis.

## 4. Robustness to MCP ghosts

Voting *does* help, but the magnitude of the help is often overstated:

- **Real mechanism.** A Hough voter down-weights pixels that don't co-linearly
  continue across dispersion. An MCP halo blob at `(x₀, y₀)` covering ~20×20
  pixels projects to a sinusoid in (ρ, θ) space with amplitude ~√(20²+20²);
  no single bin accumulates many halo votes unless the halo happens to be
  elongated along a real-order direction. In practice this gives roughly a
  5-10× ghost-suppression factor vs. naive peak-walking — useful, but not a
  silver bullet.
- **Ballester's claim** was automated order detection on UVES-era CCDs
  (negligible ICCD halo). FM4 on a Gen-II MCP is a harder adversary: halos are
  *correlated with real orders* (they flow out of bright emission lines) and
  therefore can produce 5-10 collinear halo-centroid votes in a band parallel
  to the parent order. A pure Hough voter accepts those as "orders."
- **What actually fixes this is the CWT stage, not the Hough stage.** The
  Ricker scale band filters out broad halos before they reach the voter. So
  the leverage we gain from Hough+CWT is mostly in Stage A, and Stage A alone
  is roughly equivalent to running a 2-D Ricker matched filter on the flat
  frame and peak-walking the result — a much smaller change than the full
  Hough pipeline.

## 5. Implementation cost

| Component | Rust option | Effort | Runtime on 2560×2160 |
|---|---|---|---|
| Ricker CWT, 6 scales | hand-rolled 2-D separable conv over `ndarray` or `image` | ~300 LOC, ~1 day | ~50 ms (SIMD/rayon) |
| Line Hough (ρ, θ) | `imageproc::hough::detect_lines` | ~50 LOC | ~20 ms |
| 3-D generalized Hough (for curved traces) | hand-rolled, no crate | ~600 LOC, ~3 days | ~500 ms + 32 MB accumulator |
| Butterfly cluster finder + trajectory extraction | hand-rolled | ~400 LOC | ~10 ms |
| Polynomial refinement (re-use existing) | existing `fit_trace_polynomial` | 0 | negligible |

**Total new code:** 1200-1500 LOC for the curved-order variant, 700 LOC for
the piecewise-linear hack.

**Rayon interaction (bd-ongmw).** CWT is embarrassingly parallel per column; a
rayon `par_iter` over columns trivially saturates a multicore box. Hough
voting serializes on the accumulator, but a per-thread accumulator + reduce is
standard and adds ~30 LOC. No conflict with bd-ongmw's per-order parallelism;
different pipeline stage.

## 6. Interaction with bd-g22gu.3 — the decisive cost-benefit question

bd-g22gu.3 (closed 2026-04-18) shipped **default-on morphological-opening
flat-scatter subtraction for continuum frames**. On DH3P flats the MCP halo is
now gone before trace detection runs — the very failure mode FM2 designed
around. Concretely:

- `trace_validation.rs::mechelle_5000_istar()` currently enables FWHM band
  (40%) and continuity (60%) but disables monotonic Δy and SNR, because on a
  halo-contaminated flat those two filters misclassified real NIR orders.
- With bd-g22gu.3's default flat-scatter subtraction, the pre-detection frame
  no longer *has* MCP halos; the existing peak-walker already sees a clean
  profile. The open question becomes: on a scatter-subtracted DH3P flat, does
  the existing peak-walker yield the expected ~74 traces, and does
  `trace_validation.rs` with full FM2 filters (monotonic Δy + SNR enabled)
  survive the bd-g22gu.4 regression test?
- **If yes, bd-fzbn0 becomes obsolete.** The upstream cleanup already achieves
  what Hough+CWT would achieve downstream of the peak-walker, at far lower
  implementation cost and without the Cauchy Y(m) parameterization problem.
- **If no** (e.g., scatter subtraction leaves residual halo at 5-10σ above
  baseline and the peak-walker still reports ghosts), then Hough+CWT becomes a
  second-order mitigation, not a primary one, and should be scoped to the
  CWT-only variant (Stage A in §2) which is 1 day of work and buys 80% of the
  benefit without the Hough curvature headaches.

A separate path, DTW for wavelength-only calibration (bd-rqzvt), sidesteps
trace-level ghost rejection entirely by absorbing small ghost contributions
into elastic warps. DTW is on the research queue and has lower implementation
risk than Hough+CWT.

## 7. Recommendation

**Do not implement Hough+CWT as scoped in bd-fzbn0.** File the bead as
*superseded by bd-g22gu.3 + bd-rqzvt* unless empirical evidence contradicts
the assumption in §6.

**Condition for re-opening.** If, on the bd-g22gu.4 regression fixture
(March-18 leabs-dev DH3P flat + HgAr arc) with `scatter_on_default = true`
and `TraceValidationConfig::mechelle_5000_istar()` fully enabled (monotonic Δy
+ SNR), the merged spectrum still drops real NIR orders or retains ghost
traces, then:

1. **First**, try CWT-only Stage A (Ricker bank 2-6 px, cross-dispersion
   matched-filter) feeding the existing peak-walker. ~300 LOC, 1 day.
2. **Only if that fails**, consider piecewise-linear Hough over 3-5 X-bands.
   ~700 LOC, 2-3 days.
3. **Do not** pursue a 3-D generalized Hough with a `(c0, c1, c2)`
   accumulator — DTW (bd-rqzvt) reaches a similar robustness level at a
   fraction of the complexity.

**Acceptance criteria (if re-opened).**

- Hough+CWT trace-finder produces 74 ± 5 traces on the bd-g22gu.4 fixture,
  matching the physical Mechelle 5000 order count.
- Ghost-rejection ≥ 95% on a ghost-injection fixture (inject 40 synthetic
  halo blobs at FWHM 15-25 px into a clean synthetic flat; count rejected).
- bd-g22gu.4 regression test passes with tolerances unchanged (1% flux,
  10% variance, 1 pm wavelength, ±1 order coverage).
- Runtime on 2560×2160 ≤ 200 ms wall-clock on an 8-core box (so it does not
  become the bottleneck bd-ongmw parallelization has to route around).
- Feature-flagged behind `config.trace_finder = "hough_cwt" | "peak_walker"`
  with `peak_walker` as default until ≥ 3 independent captures show
  regression against it.

## Citations

- Ballester, P. (1994). *Hough transform for robust regression and automated
  detection.* ESO Messenger Vol. 76, June 1994. [Original paper](https://www.researchgate.net/publication/234209893_Hough_transform_for_robust_regression_and_automated_detection)
- Kelson, D. D. (2003). *Optimal Techniques in Two-dimensional Spectroscopy:
  Background Subtraction for the 21st Century.* PASP 115, 688.
  [arXiv:astro-ph/0303507](https://arxiv.org/abs/astro-ph/0303507). (B-spline
  sky subtraction — precedent for optimal extraction but does *not* use Hough.)
- PypeIt `pypeit.core.arc.detect_lines` — wraps `scipy.signal.find_peaks_cwt`.
  [PypeIt wavecal docs](https://pypeit.readthedocs.io/en/1.16.0/calibrations/wave_calib.html)
- SciPy `find_peaks_cwt` — Ricker wavelet, multi-scale persistence.
  [SciPy reference](https://docs.scipy.org/doc/scipy/reference/generated/scipy.signal.find_peaks_cwt.html)
- ESO MIDAS Volume B, §Echelle / Order definition / Method Hough.
  [node471.html](https://www.eso.org/sci/software/esomidas//doc/user/98NOV/volb/node471.html)
- `imageproc::hough` — Rust Hough line detector (lines only, no curves).
  [docs.rs](https://docs.rs/imageproc/latest/imageproc/hough/index.html)
- HiFLEx (Errmann et al. 2020) — uses binned-column peak-walker, not Hough;
  demonstrates that peak-walker handles curved + defocused orders on fiber-fed
  echelles without needing Hough. [GitHub](https://github.com/ronnyerrmann/HiFLEx)
- CERES (Brahm et al. 2016) — also peak-walker, no Hough.
  [arXiv:1609.02279](https://arxiv.org/abs/1609.02279)
- GAMSE order tracing. [DeepWiki](https://deepwiki.com/wangleon/gamse/3.1-order-tracing)
- Related bd issues: bd-g22gu.3 (scatter default-on, CLOSED), bd-vdfum
  (morphological scatter, CLOSED), bd-lpgyn (FM2 post-detection filters,
  CLOSED), bd-rqzvt (DTW research, IN_PROGRESS), bd-ongmw (rayon per-order,
  CLOSED).
