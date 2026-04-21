//! DH3P (Deuterium-Halogen) flat-field blaze correction + variance-weighted
//! order merging.
//!
//! The Mechelle 5000's matched calibration lamp is a Deuterium + Tungsten-
//! Halogen hybrid (DH3P) whose continuum spans the 200–975 nm bandpass.
//! Unlike a pure Tungsten-Halogen lamp (approximately Planckian, flat over
//! ~10 nm — a per-order peak normalisation is enough), the DH3P continuum
//! has a pronounced crossover near 350–400 nm where the Deuterium peak
//! transits into the Halogen roll-off. A naïve peak normalisation entangles
//! the instrumental blaze with the lamp's intrinsic SED and yields biased
//! blaze curves at the UV/visible boundary.
//!
//! # Algorithm
//!
//! 1. Extract the DH3P flat through the same pipeline as science,
//!    yielding per-order `F_i(x)` and `λ_i(x)`.
//! 2. Assemble a global `(λ, F)` cloud across all orders and fit a smoothed
//!    continuum `C_lamp(λ)` via iterative positive-sigma-clip against a
//!    piecewise-linear knot interpolation (rejects Deuterium Balmer emission
//!    at 434/486/656 nm, cosmic rays, saturated pixels).
//! 3. Per order: `B_i(x) = F_i(x) / C_lamp(λ_i(x))`, peak-normalised, then
//!    masked below 15 % of that per-order peak — aggressive edge cutoff
//!    against optical aberrations at the detector boundary.
//! 4. At extraction time: divide each order's flux by `B_i(x)`.
//! 5. Merge with weights `W_i(λ) = B_i(λ)² / σ_i(λ)²` ≡ `1 / σ_corr²`, which
//!    penalises order edges (where `B → 0`) in proportion to their squared
//!    SNR loss — matches CERES / PypeIt / ESO MIDAS practice.
//!
//! The Mechelle's "no moving parts" design means a single high-SNR master
//! DH3P flat can be reused across many science runs; the derived blaze
//! curves are cached in [`crate::types::EchelleCorrections::blaze_curves`].

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

/// Wavelength sanity window (nm) for DH3P sample intake.
///
/// Generous around the Mechelle's 200–975 nm physical bandpass: accepts mild
/// edge extrapolation while rejecting pathological near-zero or >2 μm values
/// that degenerate orders sometimes emit.
const SAMPLE_WAVELENGTH_MIN_NM: f64 = 100.0;
const SAMPLE_WAVELENGTH_MAX_NM: f64 = 2000.0;

/// Single `(λ, F)` sample from the assembled DH3P flat.
#[derive(Debug, Clone, Copy)]
struct FlatSample {
    wavelength: f64,
    flux: f64,
}

/// Smoothed global continuum fit `C_lamp(λ)` to the DH3P lamp SED.
///
/// Stored as a piecewise-linear interpolation over uniformly-spaced knots.
/// Mathematically equivalent to a heavily-smoothed variable-knot spline when
/// knot spacing sits well above the echelle line-spread function yet below
/// the lamp SED's intrinsic ~20 nm correlation length.
///
/// The 64-knot default (~12 nm spacing over 200–975 nm) resolves the Deuterium
/// peak and Halogen roll-off without chasing local noise.
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
    /// Evaluate `C_lamp(λ)` by linear interpolation; clamps at the knot range.
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
        let pos = ks.partition_point(|&k| k <= wavelength);
        let lo = pos.saturating_sub(1).min(ks.len() - 2);
        let hi = lo + 1;
        let (x0, x1) = (ks[lo], ks[hi]);
        let (y0, y1) = (fs[lo], fs[hi]);
        let t = (wavelength - x0) / (x1 - x0);
        y0 + t * (y1 - y0)
    }
}

/// Configuration for [`fit_dh3p_continuum`].
#[derive(Debug, Clone)]
pub struct Dh3pContinuumConfig {
    /// Number of spline knots. Default 64 (~12 nm spacing on 200–975 nm).
    /// Too few → lamp SED features leak into the blaze correction; too many
    /// → the fit chases Deuterium Balmer emission even after sigma-clipping.
    pub n_knots: usize,
    /// Positive-outlier sigma-clip threshold (emission lines, CRs, saturation).
    pub sigma_threshold: f64,
    /// Maximum sigma-clip iterations.
    pub max_iters: usize,
}

impl Default for Dh3pContinuumConfig {
    fn default() -> Self {
        Self {
            n_knots: 64,
            sigma_threshold: 3.0,
            max_iters: 5,
        }
    }
}

/// Fit the global DH3P lamp continuum `C_lamp(λ)` from per-order flats.
///
/// `orders_flat` — one entry per order: `(wavelengths_nm, extracted_flux)`.
/// Wavelengths need not be globally sorted; the function assembles and sorts
/// all samples internally.
///
/// # Precondition — in-illumination samples only
///
/// Pass ONLY flat-flux samples from pixels where the order is actually
/// illuminated on the detector. Synthesised-profile orders span the full
/// detector width in pixels, but the order occupies a narrow illuminated
/// strip; off-strip pixels aperture-sum to the near-zero inter-order
/// background. Including them drags the median-per-knot continuum estimator
/// down to the noise floor, inflating per-order peak blaze efficiencies by
/// 10–100×.
///
/// Recommended caller filter: keep pixels where `F_i(x) > 10 × p10(F_i)`
/// (10× the 10th-percentile flux for that order) — this reliably separates
/// in-illumination pixels from background.
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
            if !w.is_finite()
                || !f.is_finite()
                || f <= 0.0
                || !(SAMPLE_WAVELENGTH_MIN_NM..=SAMPLE_WAVELENGTH_MAX_NM).contains(&w)
            {
                continue;
            }
            samples.push(FlatSample {
                wavelength: w,
                flux: f,
            });
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

    // Uniform (not log) knot spacing: DH3P SED correlation is smooth in linear
    // λ across the Mechelle bandpass.
    let knot_wavelengths: Vec<f64> = (0..config.n_knots)
        .map(|i| {
            let t = i as f64 / (config.n_knots - 1) as f64;
            lambda_lo + t * (lambda_hi - lambda_lo)
        })
        .collect();

    let mut kept_mask: Vec<bool> = vec![true; samples.len()];
    let mut knot_fluxes = vec![0.0; config.n_knots];
    let mut rms = 0.0;
    for _ in 0..config.max_iters.max(1) {
        knot_fluxes = compute_knot_fluxes(&samples, &kept_mask, &knot_wavelengths);
        let fit = Dh3pContinuum {
            knot_wavelengths: knot_wavelengths.clone(),
            knot_fluxes: knot_fluxes.clone(),
            rms_residual: 0.0,
            n_samples_kept: 0,
            n_samples_rejected: 0,
        };
        let (mean, std) = residual_stats(&samples, &kept_mask, &fit);
        if std <= 0.0 {
            rms = 0.0;
            break;
        }
        let threshold = config.sigma_threshold * std;
        let mut any_rejected = false;
        for (i, sample) in samples.iter().enumerate() {
            if !kept_mask[i] {
                continue;
            }
            // Reject only positive residuals: emission lines / CRs sit above
            // the continuum; absorption dips belong to the envelope.
            let r = sample.flux - fit.eval(sample.wavelength) - mean;
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
    let n_rejected = samples.len() - n_kept;
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

/// Per-order DH3P blaze correction: peak-normalised curve plus usable mask.
#[derive(Debug, Clone)]
pub struct OrderBlaze {
    /// Instrumental blaze per dispersion pixel, peak-normalised to 1.0.
    pub blaze: Vec<f64>,
    /// Per-pixel mask: `true` iff `blaze[i] ≥ blaze_threshold_frac`.
    pub usable_mask: Vec<bool>,
    /// Per-order raw peak `max(F/C_lamp)` before normalisation (diagnostic).
    pub raw_peak: f64,
}

/// Compute an order's instrumental blaze curve `B_i(x) = F_i(x) / C_lamp(λ_i(x))`.
///
/// Peak-normalised to 1.0, with a mask of pixels exceeding `blaze_threshold_frac`
/// of that peak (SOTA §Absolute Thresholding — 0.15 is the Mechelle default).
/// Returns an empty result if `flat_flux.len() != wavelengths.len()`.
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

    let mut blaze_raw = vec![0.0; n];
    for i in 0..n {
        let f = flat_flux[i];
        let c = continuum.eval(wavelengths[i]);
        blaze_raw[i] = if c > 1e-12 && f.is_finite() {
            f / c
        } else {
            0.0
        };
    }

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
    let usable_mask: Vec<bool> = blaze.iter().map(|&b| b >= blaze_threshold_frac).collect();

    OrderBlaze {
        blaze,
        usable_mask,
        raw_peak,
    }
}

/// Per-order input to [`variance_weighted_merge`].
pub struct MergeOrderInput<'a> {
    /// Wavelength axis (nm).
    pub wavelengths: &'a [f64],
    /// Blaze-corrected flux `S_corr_i(x) = S_i(x) / B_i(x)`.
    pub flux: &'a [f64],
    /// Pixel variance `σ_i(x)²` of the **raw** (pre-blaze-correction) flux;
    /// the merge recovers `σ_corr² = σ² / B²` internally.
    pub variance: &'a [f64],
    /// Per-pixel blaze efficiency `B_i(x) ∈ [0, 1]`, peak-normalised.
    pub blaze: &'a [f64],
    /// Per-pixel usability mask (from [`OrderBlaze::usable_mask`]).
    pub usable_mask: &'a [bool],
}

/// Variance-weighted merged echelle spectrum on a uniform wavelength grid.
#[derive(Debug, Clone)]
pub struct MergedSpectrum {
    /// Wavelength grid (nm).
    pub wavelengths: Vec<f64>,
    /// Merged flux.
    pub flux: Vec<f64>,
    /// Propagated merged variance: `1 / Σ_i W_i(λ)` where `W_i = B²/σ²`.
    pub variance: Vec<f64>,
    /// Number of contributing orders per wavelength bin (diagnostic).
    pub n_orders_per_bin: Vec<u32>,
}

/// Merge blaze-corrected orders via variance-weighted averaging on a uniform grid.
///
/// Per-pixel weight `W_i(λ) = B_i(λ)² / σ_i(λ)² ≡ 1 / σ_corr_i²`, so
/// order-edge pixels (`B → 0`) are penalised by their squared SNR loss.
///
/// Each input pixel is assigned to its nearest bin — adequate for Mechelle
/// grids (~0.01–0.05 nm). Use a higher-order resampler if flux conservation
/// across very wide bins matters.
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

/// Median retained sample flux in a 1.5× knot-spacing window per knot.
///
/// 1.5× overlap means neighbouring windows cover ≥50 % of each other, so
/// each knot retains ≥3 samples even at sparsely-sampled bandpass edges.
fn compute_knot_fluxes(samples: &[FlatSample], kept: &[bool], knots: &[f64]) -> Vec<f64> {
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
            // Fallback when the window is empty at a sparsely-sampled edge:
            // use the nearest retained sample regardless of window.
            let nearest =
                samples
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
#[allow(clippy::cast_lossless)] // test-only i32 iterator indices → f64
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
        // Three synthetic orders (UV, blue, visible) with small noise: a
        // heavily-smoothed fit should recover shape to within ~30 %.
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
        let refs: Vec<(&[f64], &[f64])> = orders
            .iter()
            .map(|(w, f)| (w.as_slice(), f.as_slice()))
            .collect();
        let cfg = Dh3pContinuumConfig {
            n_knots: 24,
            ..Default::default()
        };
        let fit = fit_dh3p_continuum(&refs, &cfg).expect("fit should succeed");
        assert!(fit.n_samples_kept >= 12);

        for w in [300.0, 450.0, 700.0] {
            let truth = synth_dh3p(w);
            let est = fit.eval(w);
            let rel = (est - truth).abs() / truth;
            assert!(
                rel < 0.3,
                "at {w} nm: truth={truth:.1}, est={est:.1}, rel={rel:.3}"
            );
        }
    }

    #[test]
    fn test_continuum_fit_rejects_deuterium_balmer_emission_spikes() {
        let n = 1500;
        let wl: Vec<f64> = (0..n)
            .map(|i| 200.0 + 600.0 * (i as f64) / (n as f64 - 1.0))
            .collect();
        let mut flux: Vec<f64> = wl.iter().map(|&w| synth_dh3p(w)).collect();
        // Positive spikes at Balmer lines 434 / 486 / 656 nm — sigma-clip
        // must reject them, else the fit chases the spike peak.
        for &lambda_spike in &[434.05_f64, 486.13, 656.28] {
            for (i, &w) in wl.iter().enumerate() {
                let dx = (w - lambda_spike).abs();
                if dx < 0.5 {
                    flux[i] += 8000.0 * (1.0 - dx / 0.5);
                }
            }
        }
        let orders = vec![(wl.as_slice(), flux.as_slice())];
        let fit = fit_dh3p_continuum(&orders, &Dh3pContinuumConfig::default())
            .expect("fit should succeed");
        assert!(fit.n_samples_rejected > 0, "got {}", fit.n_samples_rejected);

        let est = fit.eval(486.0);
        let truth = synth_dh3p(486.0);
        let rel = (est - truth).abs() / truth;
        assert!(rel < 0.3, "486 nm: est={est:.1}, truth={truth:.1}");
    }

    #[test]
    fn test_blaze_from_dh3p_is_peak_one_and_masks_15_percent() {
        // Synthetic order: sinc² blaze × DH3P continuum, FSR 8 nm over 20 nm
        // so edges reach past the first sinc null (sinc² < 0.15).
        let n = 256;
        let wl: Vec<f64> = (0..n)
            .map(|i| 400.0 + 20.0 * (i as f64) / (n - 1) as f64)
            .collect();
        let order_center = 410.0;
        let fsr = 8.0;
        let flux: Vec<f64> = wl
            .iter()
            .map(|&w| {
                let x = std::f64::consts::PI * (w - order_center) / fsr;
                let sinc2 = if x.abs() < 1e-12 {
                    1.0
                } else {
                    (x.sin() / x).powi(2)
                };
                synth_dh3p(w) * sinc2
            })
            .collect();

        let smooth: Vec<f64> = wl.iter().map(|&w| synth_dh3p(w)).collect();
        let refs = [(wl.as_slice(), smooth.as_slice())];
        let continuum = fit_dh3p_continuum(
            &refs,
            &Dh3pContinuumConfig {
                n_knots: 12,
                ..Default::default()
            },
        )
        .expect("continuum fit");

        let blaze = compute_blaze_from_dh3p_flat(&flux, &wl, &continuum, 0.15);
        let peak = blaze.blaze.iter().copied().fold(0.0_f64, f64::max);
        assert!((peak - 1.0).abs() < 1e-6, "got {peak}");

        assert!(blaze.usable_mask[n / 2], "order centre must be usable");
        assert!(
            !blaze.usable_mask[0] || !blaze.usable_mask[n - 1],
            "at least one edge must fall below 15% threshold"
        );
    }

    #[test]
    fn test_variance_weighted_merge_down_weights_low_blaze_regions() {
        // Order A: flat high blaze (0.9) → dominates. Order B: ramped 0.1→0.8
        // with deliberately wrong flux, masked out below 0.15 at its edge.
        // Merged flux in the overlap must equal A's flux (100) within noise.
        let wl_a: Vec<f64> = (0..100).map(|i| 500.0 + 0.05 * i as f64).collect();
        let wl_b: Vec<f64> = (0..100).map(|i| 502.5 + 0.05 * i as f64).collect();
        let flux_a = vec![100.0; 100];
        let flux_b = vec![200.0; 100]; // deliberately wrong
        let var_a = vec![1.0; 100];
        let var_b = vec![1.0; 100];
        let blaze_a = vec![0.9; 100];
        let mask_a = vec![true; 100];
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
        let overlap_bin = merged
            .wavelengths
            .iter()
            .position(|&w| (w - 502.6).abs() < 0.03)
            .expect("found overlap bin");
        let merged_flux = merged.flux[overlap_bin];
        assert!(
            (merged_flux - 100.0).abs() < 1.0,
            "merge at overlap start must favour high-blaze order A (100); got {merged_flux}"
        );
    }
}
