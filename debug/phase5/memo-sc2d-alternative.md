# Memo: sc2d (Lindler & Bowers 2001/2003) as an Alternative Scatter Method for Mechelle 5000 + iStar

**Bead:** bd-2qo5q
**Author:** research agent (Claude)
**Date:** 2026-04-18
**Target:** bd-3yb8 LIBS integration (Andor Mechelle 5000 + iStar ICCD, 200-700 nm, emission-line-dominant)
**Decision requested:** Do we implement sc2d as a third scatter method alongside `InterOrderMedian` and `MorphologicalOpening`?

**Verdict at the top:** **Don't implement.** sc2d is a forward-model algorithm whose correctness hinges on ~7 instrument-specific reference files (echelle scatter function, echelle cross-dispersion broadening, detector halo kernel, telescope PSF, cross-disperser scatter, ripple functions, reference-wavelength table) that STScI spent years measuring in the lab and in flight. Without those calibrations, sc2d degenerates into a glorified deconvolution with fabricated kernels. The per-call-site policy from bd-g22gu.3 already fences the known over-subtraction failure mode; if we want a better emission-frame method, building an MCP-halo–aware Howk & Sembach-style polynomial interorder fit is a 5-10x cheaper path with a much better theoretical match to our detector.

---

## 1. Algorithm description

sc2d is a **forward-model / iterative-deconvolution** method, not a surface fit. It never "identifies scatter-only pixels" the way CERES or morph-opening do. Instead it reconstructs a clean spectrum and a scatter map *simultaneously* by folding the current best guess of the 1D extracted spectrum through a calibrated, component-wise scatter kernel to produce a synthetic 2D image, and iterating until the synthetic matches the observed frame. [Valenti et al. 2002 (ISR STIS 2002-001), §"Algorithm Synopsis"](https://www.stsci.edu/files/live/sites/www/files/home/hst/instrumentation/stis/documentation/instrument-science-reports/_documents/200201.pdf).

Procedurally (paraphrasing Valenti 2002, steps 1–12):

1. Flat-field image → `IMAGE(x,y)`.
2. Standard 1D extraction → `NET(m, λ)` per order `m` at wavelength `λ`; retain the (x, y) order trace.
3. Adopt `NET(m, λ)` as initial `FLUX(m, λ)`.
4. Divide out the echelle ripple (blaze), splice orders → a single 1D continuous spectrum `F(λ)`.
5. Re-paint `F(λ)` onto an **oversized** model image using the trace (x, y) map plus extrapolated ripple — the image extends beyond detector edges so out-of-FOV orders contribute scatter.
6. Apply scatter kernels in this order:
   - **Echelle grating scatter** (`_ech`): redistribute counts along diagonal lines of constant λ using echelle line-spread functions — a sharp core plus very broad wings (at 1150 Å, 37 % of the light sits > 15 pixels from the core).
   - **Echelle cross-dispersion broadening** (`_exs`): column-by-column convolution that smears only the broad wings (the core is explicitly preserved; Valenti 2002 attributes this to an empirically determined but physically un-modelled scattering).
   - **Telescope PSF ⊗ detector halo ⊗ cross-disperser** convolution — done as a 2D kernel built from three reference files and *interpolated* along a wavelength axis via 1-3 precomputed kernels (the SRWTAB references).
   - **Window-reflection ghosts**: shifted, warped, scaled, smoothed copies of the pre-ghost model (≈0.3 % of peak for NUV).
7. Re-extract `MOD_NET(m, λ)` from the model image using identical x1d parameters as step 2.
8. Update the flux estimate with a relaxation factor:

       FLUX(m, λ) ← FLUX(m, λ) + Scale_factor · (MOD_NET − NET)

   where `Scale_factor ∈ [1.0, 1.5]` is grating-dependent.
9. Iterate steps 4–8. **Three iterations suffice for all STIS echelle modes.**
10. On the *final* iteration, rebuild the image **using only the wings** of every scatter function (cores zeroed out, 11 px window excluded around each order) → `SCAT(x,y)`.
11. `NEW_IMAGE = IMAGE − SCAT`.
12. Final standard 1D extraction; the existing 1D interorder median is *still* applied to `NEW_IMAGE` as a residual correction for "light scattered from wavelengths not visible on the detector or the variable FUV detector glow."

**Key architectural point:** the scatter surface is *not* parameterized by a polynomial or a spline. It is an *image* produced by convolving reference kernels. The kernels themselves are calibration artefacts, not fitting parameters.

## 2. Robustness across lamp types

The paper explicitly demonstrates sc2d on **both** a continuum-with-absorption source (BD+28 4211, saturated ISM Ly-α) and an emission-line lamp (Figure 14 of Valenti 2002 shows the sc2d model construction for an E140M emission-line wavecal). Figure 5 and Figure 6 of the same ISR show sc2d recovering the Ly-α emission line structure in α Cen A (G2V) where the 1D algorithm produces negative fluxes. So on paper it is lamp-type-agnostic, because the model starts from *the 1D extracted spectrum itself* rather than from interorder pixels — both absorbed continuum and pure emission are forward-modelled equally well.

There is a crucial caveat: sc2d assumes the extracted `NET(m, λ)` is a faithful first-order guess of the source. For very peaked emission (LIBS plasma lines, HgAr mercury doublets) the initial 1D extraction may underestimate the peak because some of the flux has already scattered into the wings — convergence then depends on the relaxation factor (`Scale_factor` ≤ 1.5) and the three-iteration budget. Valenti 2002 tested 47 datasets spanning "absorption and emission line spectra, and the full range of S/N ratio" and reports no convergence failures, but the emission-line test cases are narrow (wavecal lamps + one stellar chromospheric Ly-α), not the bright, dense plasma of an LIBS frame.

## 3. Fit for Mechelle 5000 + iStar MCP

This is the killer issue. sc2d's **detector halo kernel (`HALOTAB` / `_hal`)** is a 67 MB FITS reference file for STIS FUV-MAMA and 17 MB for NUV-MAMA. It was measured in flight from bright point-source calibration stars at many wavelengths, then stored as a wavelength-indexed stack of 2D kernels. For a Gen III MCP iStar, the halo is:

- **Wavelength-dependent** (varies with photocathode QE and MCP gain).
- **Gain-dependent** (phosphor persistence + ion feedback change with MCP voltage, which we vary per LIBS shot).
- **Temporally variable** — MCP halos drift as channel plates age, and ours is cooled/gated with duty cycles the reference halo measurements would not cover.

The STIS MAMAs are *photon-counting* image-intensified detectors with a halo structure fundamentally different from a phosphor-coupled iStar ICCD reading onto a CCD. There is no published `_hal` file for the Andor iStar Gen III, and there is no SOP for measuring one in-lab; the best proxy would be spatial autocorrelation of a uniform illumination at each gain setting, which is research work in its own right. Without this kernel, the 4500-count halo on the DH3P flat that morph-opening currently removes would instead be *modelled as if it were grating scatter and PSF*, and sc2d would plausibly *under*-subtract it (since the halo photons live outside the modelled kernel footprint) while simultaneously *over*-subtracting true echelle wings (because the kernel lacks the true iStar point response).

Additionally, sc2d assumes **well-measured per-order blaze functions** (`RIPTAB` / `_rip`) and **order trace (x, y) maps** that are stable across frames. Our blaze and trace come from on-the-fly DH3P fits and DTW refinement — good enough for a per-pixel correction but not the 0.1-1 % accuracy sc2d's redistribution math presupposes. Residual trace errors feed directly into redistribution-direction errors, which show up as checkerboard artefacts in the subtracted image.

## 4. Implementation cost

A faithful port would require roughly:

- **Core iteration loop** (steps 1–12): 400–600 LOC Rust. Straightforward ndarray + Tokio if we want parallelism; rayon for the convolution sweeps.
- **Kernel I/O and interpolation** (SRWTAB, HALOTAB, PSFTAB, CDSTAB, ECHSCTAB, EXSTAB, RIPTAB): 300-500 LOC plus a design for where these reference files live in `config/devices/` and how they are versioned.
- **Wavelength-dependent 2D convolution** with 1-3 stacked kernels, interpolated pixel-wise along the λ map: 200-400 LOC; numerically delicate (FFT vs. direct, edge handling, boundary extension of the oversized model image).
- **Order splicing / ripple removal / inverse-ripple re-paint**: 200-300 LOC. Needs to interoperate with our existing `blaze.rs` and trace-tracking code.
- **Test fixtures**: ~1000-2000 LOC (synthetic emission/continuum frames with injected scatter kernels, regression against a Python `stistools.x1d` or `stsdas.x1d` reference output if we can get STIS test data.
- **New dependencies**: none strictly required beyond what we already have (ndarray, rustfft for convolution); *however* we would carry a 100+ MB blob of reference kernels that have no counterpart for iStar.

**Total realistic cost:** 1500-2500 LOC production Rust + the research burden of *fabricating or measuring* five Mechelle-5000-specific reference kernels. Scale_factor tuning per grating is an empirical knob; STScI calibrates it per mode, we would need LIBS + HgAr + DH3P sweeps.

Numerical subtleties worth naming explicitly:

- The algorithm relies on the final `SCAT` having the **core zeroed** in all scatter functions. Defining "core" for an iStar halo kernel that has no clean functional form is not straightforward and will become a tuning parameter we cannot ground in theory.
- Convergence proof for the `FLUX ← FLUX + Scale_factor·(MOD_NET − NET)` iteration is empirical; STIS uses 3 iterations because their kernel has small norm. A larger effective kernel on the iStar (bigger halo, shorter focal length, denser order packing) may not converge at `Scale_factor = 1.0`.
- The existing 1D interorder median (step 12) is *still required* as a cleanup step. This means sc2d does not remove the reason `InterOrderMedian` exists; it just shifts what fraction of the scatter comes from which method.

## 5. Recommendation

**Do not implement sc2d.** One-sentence justification: sc2d's accuracy is bought with ~7 instrument-specific calibration kernels that took STScI roughly a decade to measure for HST/STIS, do not exist for the Mechelle 5000 + iStar, and would have to be fabricated with no ground truth — at which point we have replaced an empirically-tuned 2D median filter with an empirically-tuned 2D convolution filter that is *less* conservative on emission-dominated frames.

### Better alternatives (not part of this memo's scope but worth filing as beads)

1. **Keep bd-g22gu.3's per-call-site policy.** It structurally eliminates the over-subtraction failure mode for HgAr/ThAr/LIBS at the cost of not correcting scatter on emission frames — which is the right trade when the scatter contamination is sub-percent of peak line flux.
2. **Howk & Sembach (2000) polynomial-interorder fit** as a third method. Cheap (~300 LOC), well-documented, and handles the MCP halo by *excluding* saturated inter-order columns from the polynomial fit rather than trying to model the halo's physics. [Howk & Sembach 2000, AJ 119, 2481](https://ar5iv.labs.arxiv.org/html/astro-ph/9912388).
3. **Halo-aware variant of our existing InterOrderMedian**: replace the sigma-clipped median with a percentile-that-rejects-the-halo-mode (e.g., lower 25 % percentile rather than median), then a 2D Chebyshev fit. Probably 50 LOC delta; worth a half-day experiment against the March 18 DH3P frame.

---

## Sources

- [Valenti, Lindler, Bowers, Busko & Kim Quijano (2002), ISR STIS 2002-001: "2-D Algorithm for Removing Scattered Light from STIS Echelle Data"](https://www.stsci.edu/files/live/sites/www/files/home/hst/instrumentation/stis/documentation/instrument-science-reports/_documents/200201.pdf) — primary technical description of sc2d used throughout §§1–3 above.
- [ADS abstract for Valenti et al. 2002, ISR STIS 2002-001](https://ui.adsabs.harvard.edu/abs/2002stis.rept....1V/abstract).
- [Howk & Sembach (2000), AJ 119, 2481 — "Background and Scattered-Light Subtraction in the High-Resolution Echelle Modes of STIS"](https://ar5iv.labs.arxiv.org/html/astro-ph/9912388) — alternative empirical 1D polynomial method; notes Bowers & Lindler algorithm "in prep" at the time.
- [STIS Data Handbook §3.4, Descriptions of Calibration Steps (sc2dcorr)](https://hst-docs.stsci.edu/stisdhb/chapter-3-stis-calibration/3-4-descriptions-of-calibration-steps) — confirms "3 iterations → subtract → unweighted 1D extraction" pipeline behaviour.
- [Lindler & Bowers (2001), BAAS 197.1202](https://ui.adsabs.harvard.edu/abs/2000AAS...197.5305L) — original meeting-abstract reference cited by Valenti 2002.
- [Andor Mechelle 5000 specification sheet](https://andor.oxinst.com/assets/uploads/products/andor/documents/andor-mechelle-5000-specifications.pdf) and [iStar sCMOS hardware guide](https://andor.oxinst.com/downloads/uploads/iStar_sCMOS_Hardware_Guide.pdf) — wavelength range, detector architecture referenced in §3.
- Local code: `/home/brian/code/rust-daq/crates/echelle/src/scattered_light.rs` (1386 LOC, two existing methods); `/home/brian/code/rust-daq/crates/echelle/src/calibration_pipeline.rs` (`CalibrationPipelineConfig.arc_scatter` / `flat_scatter` policy from bd-g22gu.3).
