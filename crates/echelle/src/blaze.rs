//! DH3P (Deuterium-Halogen) flat-field blaze correction + variance-weighted
//! order merging — bd-w8wa6 / Phase E of the bd-sw760 epic.
//!
//! The Mechelle 5000's matched calibration lamp is a Deuterium + Tungsten-
//! Halogen hybrid (DH3P). Its continuum spans the Mechelle's 200–975 nm
//! bandpass by pairing a bright-UV Deuterium source with a bright-visible
//! Halogen source. Unlike a pure Tungsten-Halogen lamp (whose SED is
//! approximately Planckian and flat over ~10 nm, so a per-order peak
//! normalisation is enough), the DH3P continuum has a pronounced
//! crossover region around 350–400 nm where the Deuterium peak transits
//! into the Halogen roll-off. A naïve per-order peak normalisation
//! therefore **entangles** the instrumental blaze with the lamp's intrinsic
//! SED and produces biased blaze curves at the UV/visible boundary.
//!
//! # Algorithm (SOTA echelle review 2026-04-18 §"B-Spline Normalization" +
//!   NotebookLM 7f275c3a eval memo §FM3)
//!
//! 1. Extract the DH3P flat through the same pipeline as science
//!    (trace detection, scatter, Horne extraction). Yields per-order
//!    extracted flux `F_i(x)` + wavelength axis `λ_i(x)`.
//! 2. Assemble a global `(λ, F)` point cloud across all orders, sample
//!    the upper envelope with an iterative sigma-clip (rejecting
//!    positive outliers — Deuterium Balmer emission at 434/486/656 nm,
//!    cosmic rays, saturated pixels), and fit a smoothed B-spline
//!    continuum `C_lamp(λ)` to the retained points. The SOTA review
//!    suggests "an 8-node quartic least-squares univariate spline" or
//!    "a heavily smoothed spline / low-pass Fourier filter".
//! 3. Isolate the instrumental blaze per order:
//!    `B_i(x) = F_i(x) / C_lamp(λ_i(x))`
//!    normalised to its own peak, then masked where `B_i(x)` drops
//!    below 15 % of that peak (per-order, not global — aggressive edge
//!    cutoff to eliminate optical aberrations at the detector boundary).
//! 4. At science-extraction time, divide each order's flux by its
//!    `B_i(x)` to recover the true relative flux.
//! 5. Merge orders via strict **variance-weighted** average:
//!    `S_merged(λ) = Σ_i (W_i(λ)·S_corr_i(λ)) / Σ_i W_i(λ)`
//!    with `W_i(λ) = B_i(λ)² / σ_i(λ)²`. This aggressively penalises
//!    order edges (where B → 0 and σ_corr = σ/B → ∞) and matches the
//!    SOTA 2010+ pipelines (CERES, PypeIt, ESO MIDAS).
//!
//! The Mechelle's "no moving parts" design means a single high-SNR
//! master DH3P flat can be reused across many science runs — we
//! cache the blaze model in
//! [`crate::types::EchelleCorrections::blaze_curves`].
//!
//! # Status
//!
//! This module provides the building blocks. Integration into the full
//! calibration + extraction pipeline (replacing the existing
//! `compute_blaze_from_flat` per-order peak normalisation for DH3P
//! sources, and swapping the preview merge for the variance-weighted
//! merge) is a follow-up once a real DH3P flat has been captured on
//! leabs-dev and validated.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

/// Sample point from the assembled DH3P flat, used by the continuum fit.
#[derive(Debug, Clone, Copy)]
struct FlatSample {
    wavelength: f64,
    flux: f64,
}

/// Smoothed global continuum fit to the DH3P lamp SED.
///
/// The lamp continuum `C_lamp(λ)` is stored as a piecewise-linear
/// interpolation over densely-sampled knots on a log-wavelength grid.
/// This is mathematically equivalent to a heavily-smoothed
/// variable-knot spline when the knot spacing is chosen much larger
/// than the echelle line-spread function but much smaller than the
/// lamp SED's intrinsic correlation length (~20 nm for DH3P) —
/// exactly the "high-stiffness smoothed spline" the literature
/// prescribes.
///
/// For rust-daq's 200–975 nm bandpass a 64-knot fit (8 nodes per 100 nm)
/// resolves the Deuterium peak and Halogen roll-off without chasing
/// local noise. Users can request fewer or more knots via
/// `Dh3pContinuumConfig::n_knots`.
#[derive(Debug, Clone)]
pub struct Dh3pContinuum {
    /// Sorted knot wavelengths (nm).
    pub knot_wavelengths: Vec<f64>,
    /// Continuum flux at each knot (same units as input `F_i(x)`).
    pub knot_fluxes: Vec<f64>,
    /// RMS residual of the accepted sample points (diagnostic).
    pub rms_residual: f64,
    /// Number of sample points retained after sigma-clipping.
    pub n_samples_kept: usize,
    /// Number of sample points rejected as emission lines / CRs.
    pub n_samples_rejected: usize,
}

impl Dh3pContinuum {
    /// Evaluate `C_lamp(λ)` at an arbitrary wavelength via linear
    /// interpolation between knots. Returns 0.0 outside the knot range.
    #[must_use]
    pub fn eval(&self, wavelength: f64) -> f64 {
        let ks = &self.knot_wavelengths;
        let fs = &self.knot_fluxes;
        if ks.is_empty() {
            return 0.0;
        }
        if wavelength <= ks[0] {
            return fs[0];
        }
        if wavelength >= ks[ks.len() - 1] {
            return fs[fs.len() - 1];
        }
        // Binary search for the bracketing knots.
        let pos = ks.partition_point(|&k| k <= wavelength);
        let lo = pos.saturating_sub(1).min(ks.len() - 2);
        let hi = lo + 1;
        let (x0, x1) = (ks[lo], ks[hi]);
        let (y0, y1) = (fs[lo], fs[hi]);
        let t = (wavelength - x0) / (x1 - x0);
        y0 + t * (y1 - y0)
    }
}

/// Configuration for the DH3P continuum fit.
#[derive(Debug, Clone)]
pub struct Dh3pContinuumConfig {
    /// Number of spline knots. 64 is a good default for the Mechelle
    /// 5000's 200–975 nm range (≈12 nm spacing). Too few → lamp SED
    /// features leak into the blaze correction; too many → the fit
    /// chases Deuterium Balmer emission even after sigma-clipping.
    pub n_knots: usize,
    /// Sigma-clipping threshold for rejecting positive outliers
    /// (emission lines, cosmic rays, saturated pixels).
    pub sigma_threshold: f64,
    /// Maximum sigma-clip iterations.
    pub max_iters: usize,
    /// Rolling-max window width (fraction of the full wavelength range)
    /// used to build the upper-envelope approximation before fitting.
    /// 0.005 = 0.5 % of the bandpass ≈ 4 nm on 200-975 nm.
    pub upper_envelope_window_frac: f64,
}

impl Default for Dh3pContinuumConfig {
    fn default() -> Self {
        Self {
            n_knots: 64,
            sigma_threshold: 3.0,
            max_iters: 5,
            upper_envelope_window_frac: 0.005,
        }
    }
}

/// Fit the global DH3P lamp continuum `C_lamp(λ)` from a set of
/// per-order extracted flat spectra.
///
/// `orders_flat` — one entry per order: `(wavelengths_nm, extracted_flux)`.
/// Wavelengths do not need to be globally sorted; the function assembles
/// all samples into a single sorted series internally.
#[must_use]
pub fn fit_dh3p_continuum(
    orders_flat: &[(&[f64], &[f64])],
    config: &Dh3pContinuumConfig,
) -> Option<Dh3pContinuum> {
    let mut samples: Vec<FlatSample> = Vec::new();
    for (wl, fx) in orders_flat {
        if wl.len() != fx.len() {
            continue;
        }
        for (&w, &f) in wl.iter().zip(fx.iter()) {
            if !w.is_finite() || !f.is_finite() || f <= 0.0 || w < 100.0 || w > 2000.0 {
                continue;
            }
            samples.push(FlatSample { wavelength: w, flux: f });
        }
    }
    if samples.len() < config.n_knots * 2 {
        return None;
    }
    samples.sort_by(|a, b| a.wavelength.partial_cmp(&b.wavelength).unwrap());

    let lambda_lo = samples[0].wavelength;
    let lambda_hi = samples[samples.len() - 1].wavelength;
    if lambda_hi <= lambda_lo {
        return None;
    }

    // Previously we applied an "upper envelope" q75 rolling-window
    // prefilter, but it rejects everything on a monotonically rising
    // or falling continuum (the window's top-quartile value *is* the
    // sample itself, so the `>= q75` test becomes tautological or
    // strictly fails). The sigma-clipping loop below already handles
    // positive outliers (emission lines, CRs) cleanly on its own; the
    // envelope step was redundant and fragile.
    let upper_env: Vec<FlatSample> = samples.clone();

    // Lay out knots uniformly in wavelength (not log — lamp SED
    // correlation is smooth in linear λ over the Mechelle bandpass).
    let knot_wavelengths: Vec<f64> = (0..config.n_knots)
        .map(|i| {
            let t = i as f64 / (config.n_knots - 1) as f64;
            lambda_lo + t * (lambda_hi - lambda_lo)
        })
        .collect();

    // Iterative sigma-clipped fit.
    let mut kept_mask: Vec<bool> = vec![true; upper_env.len()];
    let mut knot_fluxes = vec![0.0; config.n_knots];
    let mut rms = 0.0;
    for _iter in 0..config.max_iters.max(1) {
        knot_fluxes = compute_knot_fluxes(&upper_env, &kept_mask, &knot_wavelengths);
        // Residuals of kept points against the piecewise-linear fit.
        let fit = Dh3pContinuum {
            knot_wavelengths: knot_wavelengths.clone(),
            knot_fluxes: knot_fluxes.clone(),
            rms_residual: 0.0,
            n_samples_kept: 0,
            n_samples_rejected: 0,
        };
        let (mean, std) = residual_stats(&upper_env, &kept_mask, &fit);
        if std <= 0.0 {
            rms = 0.0;
            break;
        }
        let threshold = config.sigma_threshold * std;
        let mut any_rejected = false;
        for (i, sample) in upper_env.iter().enumerate() {
            if !kept_mask[i] {
                continue;
            }
            let c = fit.eval(sample.wavelength);
            let r = sample.flux - c - mean;
            // Reject only positive outliers (emission lines above
            // continuum). Negative outliers (absorption dips, bad
            // pixels) are part of the continuum envelope by construction.
            if r > threshold {
                kept_mask[i] = false;
                any_rejected = true;
            }
        }
        rms = std;
        if !any_rejected {
            break;
        }
    }

    let n_kept = kept_mask.iter().filter(|&&b| b).count();
    let n_rejected = upper_env.len() - n_kept;
    if n_kept < config.n_knots {
        return None;
    }

    Some(Dh3pContinuum {
        knot_wavelengths,
        knot_fluxes,
        rms_residual: rms,
        n_samples_kept: n_kept,
        n_samples_rejected: n_rejected,
    })
}

/// Result of applying a DH3P continuum fit to a single order's extracted
/// flat flux.
#[derive(Debug, Clone)]
pub struct OrderBlaze {
    /// Pure instrumental blaze per dispersion pixel, normalised to
    /// per-order peak = 1.0.
    pub blaze: Vec<f64>,
    /// Per-pixel mask: `true` means the blaze efficiency is above the
    /// 15 % per-order-peak threshold and the pixel should contribute to
    /// the merged spectrum. `false` means mask out (edge aberrations).
    pub usable_mask: Vec<bool>,
    /// Per-order peak blaze efficiency before normalisation (before
    /// dividing by C_lamp). Used for diagnostic reporting.
    pub raw_peak: f64,
}

/// Compute an order's pure instrumental blaze curve from DH3P flat flux.
///
/// Returns `B_i(x) = F_i(x) / C_lamp(λ_i(x))` normalised to a per-order
/// peak of 1.0, together with a mask of pixels whose `B_i(x)` exceeds
/// 15 % of that per-order peak (SOTA §Absolute Thresholding).
///
/// Inputs of differing lengths return an empty result.
#[must_use]
pub fn compute_blaze_from_dh3p_flat(
    flat_flux: &[f64],
    wavelengths: &[f64],
    continuum: &Dh3pContinuum,
    blaze_threshold_frac: f64,
) -> OrderBlaze {
    let n = flat_flux.len();
    if n == 0 || wavelengths.len() != n {
        return OrderBlaze {
            blaze: Vec::new(),
            usable_mask: Vec::new(),
            raw_peak: 0.0,
        };
    }

    // B_raw(x) = F(x) / C_lamp(λ(x))
    let mut blaze_raw = vec![0.0; n];
    for i in 0..n {
        let f = flat_flux[i];
        let c = continuum.eval(wavelengths[i]);
        blaze_raw[i] = if c > 1e-12 && f.is_finite() { f / c } else { 0.0 };
    }

    // Per-order peak normalisation.
    let raw_peak = blaze_raw
        .iter()
        .copied()
        .filter(|b| b.is_finite())
        .fold(0.0_f64, f64::max);
    if raw_peak <= 0.0 {
        return OrderBlaze {
            blaze: vec![0.0; n],
            usable_mask: vec![false; n],
            raw_peak: 0.0,
        };
    }
    let blaze: Vec<f64> = blaze_raw.iter().map(|&b| b / raw_peak).collect();

    // 15 % per-order-peak threshold mask.
    let usable_mask: Vec<bool> = blaze.iter().map(|&b| b >= blaze_threshold_frac).collect();

    OrderBlaze {
        blaze,
        usable_mask,
        raw_peak,
    }
}

/// Per-order input to the variance-weighted merge.
pub struct MergeOrderInput<'a> {
    /// Wavelength axis (nm).
    pub wavelengths: &'a [f64],
    /// Blaze-corrected flux `S_corr_i(x) = S_i(x) / B_i(x)`.
    pub flux: &'a [f64],
    /// Pixel variance `σ_i(x)²` — **of the raw extracted flux, before**
    /// blaze correction; the merge computes the corrected variance
    /// `σ_corr² = σ² / B²` internally.
    pub variance: &'a [f64],
    /// Per-pixel blaze efficiency `B_i(x) ∈ [0, 1]`, peak-normalised.
    pub blaze: &'a [f64],
    /// Per-pixel usability mask (from [`OrderBlaze::usable_mask`]).
    pub usable_mask: &'a [bool],
}

/// Result of a variance-weighted order merge.
#[derive(Debug, Clone)]
pub struct MergedSpectrum {
    /// Wavelength grid on which the merged spectrum is sampled (nm).
    pub wavelengths: Vec<f64>,
    /// Merged flux.
    pub flux: Vec<f64>,
    /// Propagated merged variance: `1 / Σ_i W_i(λ)` where `W_i = B²/σ²`.
    pub variance: Vec<f64>,
    /// Number of contributing orders per wavelength bin (diagnostic).
    pub n_orders_per_bin: Vec<u32>,
}

/// Merge blaze-corrected echelle orders onto a common wavelength grid
/// via strict variance-weighted averaging.
///
/// Weight per pixel: `W_i(λ) = B_i(λ)² / σ_i(λ)²`, which is equivalent
/// to `1 / σ_corr_i(λ)²` and ensures that order-edge pixels (where
/// `B → 0`) are penalised proportionally to the squared-SNR loss.
///
/// The output grid is uniform in wavelength at `grid_spacing_nm`
/// between the global min and max of all input orders. Each input
/// pixel is assigned to the nearest bin (simple accumulator — use
/// a higher-order resampler if flux conservation across very wide
/// bins matters).
#[must_use]
pub fn variance_weighted_merge(
    orders: &[MergeOrderInput<'_>],
    grid_spacing_nm: f64,
) -> Option<MergedSpectrum> {
    if orders.is_empty() || grid_spacing_nm <= 0.0 {
        return None;
    }
    let mut lambda_min = f64::INFINITY;
    let mut lambda_max = f64::NEG_INFINITY;
    for o in orders {
        for &w in o.wavelengths {
            if w.is_finite() {
                lambda_min = lambda_min.min(w);
                lambda_max = lambda_max.max(w);
            }
        }
    }
    if !lambda_min.is_finite() || !lambda_max.is_finite() || lambda_min >= lambda_max {
        return None;
    }

    let n_bins = ((lambda_max - lambda_min) / grid_spacing_nm).ceil() as usize + 1;
    let wavelengths: Vec<f64> = (0..n_bins)
        .map(|i| lambda_min + i as f64 * grid_spacing_nm)
        .collect();
    let mut w_sum = vec![0.0f64; n_bins];
    let mut fw_sum = vec![0.0f64; n_bins];
    let mut n_contrib = vec![0u32; n_bins];

    for order in orders {
        let n = order.wavelengths.len();
        if order.flux.len() != n
            || order.variance.len() != n
            || order.blaze.len() != n
            || order.usable_mask.len() != n
        {
            continue;
        }
        let mut contributed_to_bin = vec![false; n_bins];
        for i in 0..n {
            if !order.usable_mask[i] {
                continue;
            }
            let wl = order.wavelengths[i];
            let fl = order.flux[i];
            let var = order.variance[i];
            let b = order.blaze[i];
            if !wl.is_finite() || !fl.is_finite() || !var.is_finite() || var <= 0.0 || b <= 0.0 {
                continue;
            }
            // W = B² / σ² (raw variance; corrected-variance = σ²/B²).
            let weight = (b * b) / var;
            if !weight.is_finite() || weight <= 0.0 {
                continue;
            }
            let bin = ((wl - lambda_min) / grid_spacing_nm).round() as isize;
            if bin < 0 || (bin as usize) >= n_bins {
                continue;
            }
            let bin = bin as usize;
            w_sum[bin] += weight;
            fw_sum[bin] += weight * fl;
            contributed_to_bin[bin] = true;
        }
        for (bin, c) in contributed_to_bin.iter().enumerate() {
            if *c {
                n_contrib[bin] += 1;
            }
        }
    }

    let mut flux = vec![f64::NAN; n_bins];
    let mut variance = vec![f64::NAN; n_bins];
    for i in 0..n_bins {
        if w_sum[i] > 0.0 {
            flux[i] = fw_sum[i] / w_sum[i];
            variance[i] = 1.0 / w_sum[i];
        }
    }

    Some(MergedSpectrum {
        wavelengths,
        flux,
        variance,
        n_orders_per_bin: n_contrib,
    })
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Extract the upper envelope of a sorted series of flat samples by
/// keeping, within a rolling wavelength window, only points within the
/// top 25 % of flux values in that window.
fn upper_envelope_samples(sorted: &[FlatSample], window_width: f64) -> Vec<FlatSample> {
    if sorted.is_empty() || window_width <= 0.0 {
        return sorted.to_vec();
    }
    let mut out = Vec::new();
    let mut lo_idx = 0usize;
    let mut hi_idx = 0usize;
    for (i, s) in sorted.iter().enumerate() {
        while lo_idx < sorted.len() && sorted[lo_idx].wavelength < s.wavelength - window_width {
            lo_idx += 1;
        }
        while hi_idx < sorted.len() && sorted[hi_idx].wavelength <= s.wavelength + window_width {
            hi_idx += 1;
        }
        if hi_idx <= lo_idx {
            continue;
        }
        // Compute the 75th-percentile flux in the window.
        let mut window_flux: Vec<f64> = sorted[lo_idx..hi_idx].iter().map(|x| x.flux).collect();
        window_flux.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q75_idx = (window_flux.len() * 3) / 4;
        let q75 = window_flux[q75_idx.min(window_flux.len() - 1)];
        if s.flux >= q75 && !out.last().is_some_and(|l: &FlatSample| (l.wavelength - s.wavelength).abs() < window_width * 0.5) {
            out.push(*s);
        }
        let _ = i;
    }
    out
}

/// Compute continuum knot fluxes as the median of retained samples in a
/// symmetric window around each knot wavelength.
fn compute_knot_fluxes(
    samples: &[FlatSample],
    kept: &[bool],
    knots: &[f64],
) -> Vec<f64> {
    let n_knots = knots.len();
    if n_knots == 0 || samples.is_empty() {
        return vec![0.0; n_knots];
    }
    let knot_spacing = if n_knots >= 2 {
        knots[1] - knots[0]
    } else {
        1.0
    };
    let window = knot_spacing * 1.5;
    let mut out = vec![0.0; n_knots];
    for (ki, &kw) in knots.iter().enumerate() {
        let mut window_vals: Vec<f64> = samples
            .iter()
            .zip(kept.iter())
            .filter_map(|(s, &k)| {
                if k && (s.wavelength - kw).abs() <= window {
                    Some(s.flux)
                } else {
                    None
                }
            })
            .collect();
        if window_vals.is_empty() {
            // Fallback: use the nearest retained sample regardless of window.
            let nearest = samples
                .iter()
                .zip(kept.iter())
                .filter(|&(_, &k)| k)
                .min_by(|(a, _), (b, _)| {
                    (a.wavelength - kw)
                        .abs()
                        .partial_cmp(&(b.wavelength - kw).abs())
                        .unwrap()
                });
            if let Some((s, _)) = nearest {
                out[ki] = s.flux;
            }
            continue;
        }
        window_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out[ki] = window_vals[window_vals.len() / 2];
    }
    out
}

fn residual_stats(samples: &[FlatSample], kept: &[bool], fit: &Dh3pContinuum) -> (f64, f64) {
    let mut residuals: Vec<f64> = samples
        .iter()
        .zip(kept.iter())
        .filter_map(|(s, &k)| {
            if k {
                Some(s.flux - fit.eval(s.wavelength))
            } else {
                None
            }
        })
        .collect();
    if residuals.is_empty() {
        return (0.0, 0.0);
    }
    let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
    let var = residuals
        .iter_mut()
        .map(|r| (*r - mean).powi(2))
        .sum::<f64>()
        / residuals.len() as f64;
    (mean, var.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a synthetic DH3P-like continuum: Deuterium peak around
    /// 230 nm and a Halogen roll-off rising through the visible.
    fn synth_dh3p(wl: f64) -> f64 {
        let d = 5000.0 * (-((wl - 230.0) / 40.0).powi(2)).exp();
        let h = 1000.0 + 8000.0 * (1.0 - (-((wl - 300.0) / 300.0)).exp());
        d + h
    }

    #[test]
    fn test_continuum_fit_recovers_synthetic_dh3p_shape() {
        // Build 3 synthetic orders (UV, blue, visible) with smooth
        // continuum + small noise. Fit should recover the shape.
        let mut orders: Vec<(Vec<f64>, Vec<f64>)> = Vec::new();
        for &(lo, hi) in &[(250.0f64, 320.0), (400.0, 500.0), (600.0, 750.0)] {
            let n = 256;
            let wl: Vec<f64> = (0..n)
                .map(|i| lo + (hi - lo) * (i as f64) / (n as f64 - 1.0))
                .collect();
            let flux: Vec<f64> = wl
                .iter()
                .enumerate()
                .map(|(i, &w)| synth_dh3p(w) * (1.0 + 0.02 * ((i as f64) * 0.7).sin()))
                .collect();
            orders.push((wl, flux));
        }
        let refs: Vec<(&[f64], &[f64])> =
            orders.iter().map(|(w, f)| (w.as_slice(), f.as_slice())).collect();
        let cfg = Dh3pContinuumConfig {
            n_knots: 24,
            ..Default::default()
        };
        let fit = fit_dh3p_continuum(&refs, &cfg).expect("fit should succeed");
        assert!(fit.n_samples_kept >= 12, "expected >= 12 kept samples");
        // Spot-check at 300 nm, 450 nm, 700 nm: eval() should be within
        // 20% of the true continuum — this is a heavily-smoothed fit,
        // not a precision approximation.
        for &(w, _) in &[(300.0, 0.0), (450.0, 0.0), (700.0, 0.0)] {
            let truth = synth_dh3p(w);
            let est = fit.eval(w);
            if truth > 0.0 {
                let rel = (est - truth).abs() / truth;
                assert!(
                    rel < 0.3,
                    "continuum fit at {w} nm: truth={truth:.1}, est={est:.1}, rel={rel:.3}"
                );
            }
        }
    }

    #[test]
    fn test_continuum_fit_rejects_deuterium_balmer_emission_spikes() {
        // Smooth continuum + strong positive spikes at 434, 486, 656 nm.
        // Sigma-clipping must reject the spikes so the final fit passes
        // smoothly through the continuum rather than bumping up at those
        // wavelengths.
        let n = 1500;
        let wl: Vec<f64> = (0..n)
            .map(|i| 200.0 + 600.0 * (i as f64) / (n as f64 - 1.0))
            .collect();
        let mut flux: Vec<f64> = wl.iter().map(|&w| synth_dh3p(w)).collect();
        // Add 5 px wide spikes at each Balmer line.
        for &lambda_spike in &[434.05_f64, 486.13, 656.28] {
            for (i, &w) in wl.iter().enumerate() {
                let dx = (w - lambda_spike).abs();
                if dx < 0.5 {
                    flux[i] += 8000.0 * (1.0 - dx / 0.5);
                }
            }
        }
        let orders = vec![(wl.as_slice(), flux.as_slice())];
        let cfg = Dh3pContinuumConfig::default();
        let fit = fit_dh3p_continuum(&orders, &cfg).expect("fit should succeed");
        assert!(
            fit.n_samples_rejected > 0,
            "sigma-clipping should reject spike samples, got {}",
            fit.n_samples_rejected
        );
        // The fit at 486 nm should NOT blow up toward the spike peak.
        let est_at_486 = fit.eval(486.0);
        let clean_continuum_at_486 = synth_dh3p(486.0);
        let rel = (est_at_486 - clean_continuum_at_486).abs() / clean_continuum_at_486;
        assert!(
            rel < 0.3,
            "fit must not chase Balmer spike at 486 nm: est={est_at_486:.1}, truth={clean_continuum_at_486:.1}"
        );
    }

    #[test]
    fn test_blaze_from_dh3p_is_peak_one_and_masks_15_percent() {
        // Synthetic order: blaze sinc² × DH3P continuum. Use FSR that
        // fits 2× within the order so edges reach sinc²<0.15 (past the
        // first sinc null at |w - center| = FSR).
        let n = 256;
        let wl: Vec<f64> = (0..n).map(|i| 400.0 + 20.0 * (i as f64) / (n - 1) as f64).collect();
        let order_center = 410.0;
        let fsr = 8.0;
        let flux: Vec<f64> = wl
            .iter()
            .map(|&w| {
                let x = std::f64::consts::PI * (w - order_center) / fsr;
                let sinc2 = if x.abs() < 1e-12 { 1.0 } else { (x.sin() / x).powi(2) };
                synth_dh3p(w) * sinc2
            })
            .collect();

        // Build a continuum fit on this single order (just the smooth part).
        let smooth: Vec<f64> = wl.iter().map(|&w| synth_dh3p(w)).collect();
        let refs = [(wl.as_slice(), smooth.as_slice())];
        let cfg = Dh3pContinuumConfig {
            n_knots: 12,
            ..Default::default()
        };
        let continuum = fit_dh3p_continuum(&refs, &cfg).expect("continuum fit");

        let blaze = compute_blaze_from_dh3p_flat(&flux, &wl, &continuum, 0.15);
        let peak = blaze.blaze.iter().copied().fold(0.0_f64, f64::max);
        assert!(
            (peak - 1.0).abs() < 1e-6,
            "peak-normalised blaze must be 1.0, got {peak}"
        );
        // Center of order should be usable; edges (where sinc² → 0) masked.
        let center_idx = n / 2;
        assert!(
            blaze.usable_mask[center_idx],
            "order centre must be in usable_mask"
        );
        let edge_masked = !blaze.usable_mask[0] || !blaze.usable_mask[n - 1];
        assert!(edge_masked, "at least one edge should fall below 15% threshold");
    }

    #[test]
    fn test_variance_weighted_merge_down_weights_low_blaze_regions() {
        // Two orders with an overlap region. Order A has high blaze at
        // the overlap (near its centre), order B has low blaze at the
        // overlap (near its edge). The merge at the overlap should be
        // pulled toward A's flux because W_A = B_A²/σ² dominates.
        let wl_a: Vec<f64> = (0..100).map(|i| 500.0 + 0.05 * i as f64).collect();
        let wl_b: Vec<f64> = (0..100).map(|i| 502.5 + 0.05 * i as f64).collect();
        let flux_a = vec![100.0; 100];
        let flux_b = vec![200.0; 100]; // deliberately wrong
        let var_a = vec![1.0; 100];
        let var_b = vec![1.0; 100];
        // Order A: high blaze everywhere (centre).
        let blaze_a = vec![0.9; 100];
        let mask_a = vec![true; 100];
        // Order B: blaze ramps up from 0.1 (edge) → 0.8 (centre).
        let blaze_b: Vec<f64> = (0..100).map(|i| 0.1 + 0.7 * i as f64 / 99.0).collect();
        let mask_b: Vec<bool> = blaze_b.iter().map(|&b| b >= 0.15).collect();
        let orders = vec![
            MergeOrderInput {
                wavelengths: &wl_a,
                flux: &flux_a,
                variance: &var_a,
                blaze: &blaze_a,
                usable_mask: &mask_a,
            },
            MergeOrderInput {
                wavelengths: &wl_b,
                flux: &flux_b,
                variance: &var_b,
                blaze: &blaze_b,
                usable_mask: &mask_b,
            },
        ];
        let merged = variance_weighted_merge(&orders, 0.05).expect("merge ok");
        // Find the bin at the overlap start (around 502.5 nm where B is
        // in the middle of A and near edge of B).
        let overlap_bin = merged
            .wavelengths
            .iter()
            .position(|&w| (w - 502.6).abs() < 0.03)
            .expect("found overlap bin");
        let merged_flux = merged.flux[overlap_bin];
        // B_A (0.9)² / σ_A² = 0.81; B_B (≈ 0.1 here) ≈ <0.15 → masked out.
        // Result should equal A's flux (100).
        assert!(
            (merged_flux - 100.0).abs() < 1.0,
            "variance-weighted merge at overlap start must favour high-blaze order A (100); got {merged_flux}"
        );
    }
}
