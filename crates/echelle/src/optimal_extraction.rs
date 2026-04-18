//! Horne 1986 optimal extraction for echelle spectrographs.
//!
//! Implements the three-stage optimal extraction algorithm:
//! - **Stage A**: Spatial profile fitting (normalized flux distribution)
//! - **Stage B**: Inverse-variance-weighted extraction (Horne eq. 8)
//! - **Stage C**: Cosmic ray sigma-clip rejection loop
//!
//! Operates on `RectifiedOrder` sub-images from the rectification module.
//!
//! Reference: Horne, K. 1986, PASP, 98, 609

// Pixel-index casts: always small enough for lossless conversions.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::rectification::RectifiedOrder;
use std::time::Instant;

use tracing::debug;

/// Configuration for optimal extraction.
#[derive(Debug, Clone)]
pub struct OptimalExtractionConfig {
    /// Detector read noise in electrons/pixel (default: 3.0).
    pub read_noise: f64,
    /// Detector gain in electrons/ADU (default: 1.0).
    pub gain: f64,
    /// Excess noise factor for intensified detectors (default: 1.0 for CCD).
    ///
    /// For standard CCDs, F = 1.0 (no excess noise).
    /// For ICCDs (e.g., Andor iStar with Gen III MCP), F ≈ 1.6.
    /// For EMCCDs, F = √2 ≈ 1.41.
    ///
    /// The variance model becomes: V = readnoise² + F² × signal / gain.
    /// Setting F > 1 correctly downweights high-signal pixels in the
    /// inverse-variance weighting, producing a more accurate extraction
    /// for photon-counting / intensified detectors.
    pub excess_noise_factor: f64,
    /// Maximum cosmic ray rejection iterations (default: 5).
    pub max_cr_iterations: usize,
    /// Sigma threshold for cosmic ray rejection (default: 6.0).
    pub cr_sigma: f64,
    /// Minimum fractional coverage to keep a spectral pixel (default: 0.9).
    pub min_frac_use: f64,
}

impl Default for OptimalExtractionConfig {
    fn default() -> Self {
        Self {
            read_noise: 3.0,
            gain: 1.0,
            excess_noise_factor: 1.0,
            max_cr_iterations: 5,
            cr_sigma: 6.0,
            min_frac_use: 0.9,
        }
    }
}

impl OptimalExtractionConfig {
    /// Preset for Andor iStar ICCD (Gen II/III filmless MCP).
    ///
    /// Per NotebookLM 7f275c3a pipeline-eval memo §FM5 ("The ICCD Excess
    /// Noise Factor"):
    ///
    /// - `excess_noise_factor = 1.6` — manufacturer-reported Fano factor
    ///   for the Gen II/III image intensifier. Without this, the
    ///   standard-CCD variance model (F=1) underestimates noise at the
    ///   spatial-profile centre by ~2.56×, causing optimal extraction
    ///   to over-weight bright pixels and fail to reject cosmic rays /
    ///   MCP ion-feedback artifacts.
    /// - `cr_sigma = 5.0` — NotebookLM prescribes tightening the CR
    ///   rejection threshold from the default 10.0 / 6.0 to **5.0**
    ///   *because* the F² factor now makes the variance estimate
    ///   accurate; a tighter threshold can aggressively strip CRs
    ///   without falsely rejecting the cores of bright emission lines.
    /// - `read_noise = 20 e⁻` — placeholder; the MCP effectively renders
    ///   the sCMOS read noise negligible, but leaving it non-zero is
    ///   safer than relying purely on the shot-noise term when the
    ///   signal model is zero at an empty-order pixel.
    /// - `gain = 1.0` — adjust based on actual MCP gain setting.
    #[must_use]
    pub fn istar_iccd() -> Self {
        Self {
            read_noise: 20.0,
            gain: 1.0,
            excess_noise_factor: 1.6,
            cr_sigma: 5.0,
            ..Self::default()
        }
    }
}

/// Result of optimal extraction for a single order.
#[derive(Debug, Clone)]
pub struct OptimalExtractionResult {
    /// Optimally extracted flux per dispersion pixel.
    pub flux: Vec<f64>,
    /// Variance of the extracted flux.
    pub variance: Vec<f64>,
    /// Fractional spatial coverage per dispersion pixel.
    pub frac_use: Vec<f64>,
    /// Number of cosmic ray pixels rejected.
    pub n_cr_rejected: usize,
    /// Spatial profile used (n_cross × n_dispersion, row-major).
    pub spatial_profile: Vec<f64>,
}

/// Perform optimal extraction on a rectified order.
///
/// `sky` is an optional per-pixel sky estimate (same dimensions as `rect.data`).
/// If `None`, sky is assumed to be zero (appropriate after scattered light subtraction).
pub fn optimal_extract(
    rect: &RectifiedOrder,
    sky: Option<&[f32]>,
    config: &OptimalExtractionConfig,
) -> Option<OptimalExtractionResult> {
    let n_disp = rect.n_dispersion;
    let n_cross = rect.n_cross;

    if n_disp == 0 || n_cross < 3 {
        return None;
    }

    // Gate all timing instrumentation behind a single level check so there is
    // zero overhead when debug tracing is not active.
    let timing = tracing::enabled!(target: "echelle::optimal_extraction", tracing::Level::DEBUG);
    let fn_start = timing.then(Instant::now);

    // Stage A: Fit spatial profile.
    let stage_a_start = timing.then(Instant::now);
    let profile = fit_spatial_profile(rect, sky, config);
    let stage_a_elapsed = stage_a_start.map(|t| t.elapsed());

    if let Some(dur) = stage_a_elapsed {
        debug!(
            target: "echelle::optimal_extraction",
            order_index = %rect.order_index,
            stage = "A",
            duration_ms = dur.as_secs_f64() * 1000.0,
            "Stage A: Spatial profile fitting completed"
        );
    }

    // Initialize pixel mask (true = use this pixel).
    let mut pixel_mask = vec![true; n_cross * n_disp];
    // Zero out pixels where the aperture mask is zero.
    for (pm, &m) in pixel_mask.iter_mut().zip(rect.mask.iter()) {
        if m < 1e-10 {
            *pm = false;
        }
    }

    let mut n_cr_rejected = 0;

    // Outer loop: Stage B + Stage C iteration.
    let mut flux = vec![0.0f64; n_disp];
    let mut variance = vec![0.0f64; n_disp];
    let mut frac_use = vec![0.0f64; n_disp];

    let mut stage_b_total = std::time::Duration::ZERO;
    let mut stage_c_total = std::time::Duration::ZERO;
    let mut n_iterations = 0;

    for iteration in 0..=config.max_cr_iterations {
        n_iterations = iteration + 1;

        // Stage B: Optimal extraction.
        let stage_b_start = timing.then(Instant::now);
        extract_with_profile(
            rect,
            sky,
            &profile,
            &pixel_mask,
            config,
            &mut flux,
            &mut variance,
            &mut frac_use,
            iteration == 0,
        );
        if let Some(t) = stage_b_start {
            let dur = t.elapsed();
            stage_b_total += dur;
            debug!(
                target: "echelle::optimal_extraction",
                order_index = %rect.order_index,
                stage = "B",
                iteration = iteration + 1,
                duration_ms = dur.as_secs_f64() * 1000.0,
                "Stage B: Optimal extraction completed"
            );
        }

        if iteration == config.max_cr_iterations {
            break;
        }

        // Stage C: Cosmic ray rejection.
        let stage_c_start = timing.then(Instant::now);
        let n_flagged = reject_cosmic_rays(rect, sky, &profile, &flux, &mut pixel_mask, config);
        if let Some(t) = stage_c_start {
            let dur = t.elapsed();
            stage_c_total += dur;
            debug!(
                target: "echelle::optimal_extraction",
                order_index = %rect.order_index,
                stage = "C",
                iteration = iteration + 1,
                duration_ms = dur.as_secs_f64() * 1000.0,
                n_flagged = n_flagged,
                "Stage C: Cosmic ray rejection completed"
            );
        }

        n_cr_rejected += n_flagged;
        if n_flagged == 0 {
            break;
        }
    }

    if timing {
        let stage_bc_ms = (stage_b_total + stage_c_total).as_secs_f64() * 1000.0;
        debug!(
            target: "echelle::optimal_extraction",
            order_index = %rect.order_index,
            stage = "B+C",
            total_duration_ms = stage_bc_ms,
            stage_b_avg_ms = stage_b_total.as_secs_f64() * 1000.0 / n_iterations as f64,
            stage_c_avg_ms = stage_c_total.as_secs_f64() * 1000.0 / n_iterations as f64,
            iterations = n_iterations,
            "Stages B+C: Extraction and cosmic ray rejection loop completed"
        );
    }

    // Mask flux where coverage is too low.
    for col in 0..n_disp {
        if frac_use[col] < config.min_frac_use {
            flux[col] = 0.0;
            variance[col] = 0.0;
        }
    }

    // Overall timing summary
    if let Some(fn_start) = fn_start {
        let total = fn_start.elapsed();
        let stage_a_dur = stage_a_elapsed.unwrap_or_default();
        let stage_bc_dur = stage_b_total + stage_c_total;
        debug!(
            target: "echelle::optimal_extraction",
            order_index = %rect.order_index,
            total_duration_ms = total.as_secs_f64() * 1000.0,
            stage_a_duration_ms = stage_a_dur.as_secs_f64() * 1000.0,
            stage_bc_duration_ms = stage_bc_dur.as_secs_f64() * 1000.0,
            stage_a_percent = stage_a_dur.as_secs_f64() / total.as_secs_f64() * 100.0,
            stage_bc_percent = stage_bc_dur.as_secs_f64() / total.as_secs_f64() * 100.0,
            n_cr_rejected = n_cr_rejected,
            "Optimal extraction completed for order"
        );
    }

    Some(OptimalExtractionResult {
        flux,
        variance,
        frac_use,
        n_cr_rejected,
        spatial_profile: profile,
    })
}

/// Stage A: Fit the spatial profile P(x,y).
///
/// For each dispersion column, computes the normalized cross-dispersion
/// flux distribution. Uses a simple boxcar estimate smoothed along
/// the dispersion axis for stability.
fn fit_spatial_profile(
    rect: &RectifiedOrder,
    sky: Option<&[f32]>,
    config: &OptimalExtractionConfig,
) -> Vec<f64> {
    let n_disp = rect.n_dispersion;
    let n_cross = rect.n_cross;
    let mut profile = vec![0.0f64; n_cross * n_disp];

    // Step 1: Crude boxcar flux f0(x) per dispersion column. Columns whose
    // total signal is below `3 × read_noise` are treated as "profile
    // unknown" (f0 set to NaN) so Step 3 can smooth them out from their
    // neighbors rather than inventing a unit-flux profile (bd-8yjd1 P2.1).
    let low_signal_threshold = 3.0 * config.read_noise;
    let mut f0 = vec![0.0f64; n_disp];
    for (col, f0_val) in f0.iter_mut().enumerate() {
        let mut sum = 0.0;
        for row in 0..n_cross {
            let idx = row * n_disp + col;
            let data_val = f64::from(rect.data[idx]);
            let sky_val = sky.map_or(0.0, |sk| f64::from(sk[idx]));
            let weight = f64::from(rect.mask[idx]);
            sum += (data_val - sky_val) * weight;
        }
        *f0_val = if sum > low_signal_threshold {
            sum
        } else {
            f64::NAN
        };
    }

    // Step 2: Compute raw profile P_raw(x,y) = (D - sky) / f0(x).
    // Columns with f0 = NaN produce NaN entries that are skipped in the
    // smoothing pass below. Negative residuals (D < sky) are kept instead
    // of clipped at zero so downstream sigma-clipping + renormalization
    // can handle them correctly (bd-8yjd1 P2.2).
    for (col, &f0_val) in f0.iter().enumerate() {
        for row in 0..n_cross {
            let idx = row * n_disp + col;
            if f0_val.is_nan() {
                profile[idx] = f64::NAN;
                continue;
            }
            let data_val = f64::from(rect.data[idx]);
            let sky_val = sky.map_or(0.0, |sk| f64::from(sk[idx]));
            profile[idx] = (data_val - sky_val) / f0_val;
        }
    }

    // Step 3: Smooth along dispersion axis (boxcar of width 5) for each
    // spatial row, skipping NaN neighbors so missing columns are
    // interpolated from their valid neighbors.
    let smooth_half = 2;
    let mut smoothed = vec![0.0f64; n_cross * n_disp];
    for row in 0..n_cross {
        for col in 0..n_disp {
            let start = col.saturating_sub(smooth_half);
            let end = (col + smooth_half + 1).min(n_disp);
            let mut sum = 0.0;
            let mut count = 0u32;
            for c in start..end {
                let v = profile[row * n_disp + c];
                if v.is_finite() {
                    sum += v;
                    count += 1;
                }
            }
            smoothed[row * n_disp + col] = if count > 0 {
                sum / f64::from(count)
            } else {
                0.0
            };
        }
    }

    // Step 4: Enforce P >= 0 and renormalize sum_y(P) = 1 per column.
    for col in 0..n_disp {
        let mut col_sum = 0.0;
        for row in 0..n_cross {
            let idx = row * n_disp + col;
            smoothed[idx] = smoothed[idx].max(0.0);
            col_sum += smoothed[idx];
        }
        if col_sum > 1e-15 {
            for row in 0..n_cross {
                smoothed[row * n_disp + col] /= col_sum;
            }
        }
    }

    smoothed
}

/// Stage B: Extract flux using the spatial profile.
///
/// Implements Horne equation 8:
///   flux(x) = sum_y(mask * ivar * D * P) / sum_y(mask * ivar * P^2)
#[allow(clippy::too_many_arguments)]
fn extract_with_profile(
    rect: &RectifiedOrder,
    sky: Option<&[f32]>,
    profile: &[f64],
    pixel_mask: &[bool],
    config: &OptimalExtractionConfig,
    flux: &mut [f64],
    variance: &mut [f64],
    frac_use: &mut [f64],
    is_first_iteration: bool,
) {
    let n_disp = rect.n_dispersion;
    let n_cross = rect.n_cross;
    let rn2 = config.read_noise * config.read_noise;
    let f2 = config.excess_noise_factor * config.excess_noise_factor;

    // Minimum-variance floor to avoid ivar → ∞ when the signal model
    // evaluates to zero. 1e-6·rn² is small relative to real read noise but
    // large enough to keep the weighted solver numerically stable
    // (bd-8yjd1 P2.3).
    let v_floor = (rn2 * 1e-6).max(1e-10);

    for col in 0..n_disp {
        let mut num = 0.0f64;
        let mut denom = 0.0f64;
        let mut p_used = 0.0f64;
        let mut p_total = 0.0f64;

        let prev_flux = flux[col];

        for row in 0..n_cross {
            let idx = row * n_disp + col;
            let p = profile[idx];
            p_total += p;

            if !pixel_mask[idx] || rect.mask[idx] < 1e-10 {
                continue;
            }

            let d = f64::from(rect.data[idx]);
            let s = sky.map_or(0.0, |sk| f64::from(sk[idx]));

            // Variance model (bd-8yjd1 P2.3):
            //   V = readnoise² + F² × max(signal, 0) / gain
            // First iteration: variance of the raw data. Subsequent
            // iterations: variance of (sky + current-column flux × profile),
            // which drives the solver toward the Horne 1986 fixed point.
            let v_raw = if is_first_iteration {
                rn2 + f2 * d.max(0.0) / config.gain
            } else {
                rn2 + f2 * (s + prev_flux * p).max(0.0) / config.gain
            };
            let v = v_raw.max(v_floor);
            let ivar = 1.0 / v;

            num += ivar * (d - s) * p;
            denom += ivar * p * p;
            p_used += p;
        }

        flux[col] = if denom > 1e-30 { num / denom } else { 0.0 };
        variance[col] = if denom > 1e-30 { 1.0 / denom } else { 0.0 };
        frac_use[col] = if p_total > 1e-15 {
            p_used / p_total
        } else {
            0.0
        };
    }
}

/// Stage C: Flag cosmic ray pixels.
///
/// Computes residuals: r(x,y) = (D - sky - flux*P) / sqrt(V)
/// Flags pixels with |r| > cr_sigma.
/// Returns count of newly flagged pixels.
#[allow(clippy::many_single_char_names)] // Standard notation from Horne 1986: D=data, P=profile, V=variance
fn reject_cosmic_rays(
    rect: &RectifiedOrder,
    sky: Option<&[f32]>,
    profile: &[f64],
    flux: &[f64],
    pixel_mask: &mut [bool],
    config: &OptimalExtractionConfig,
) -> usize {
    let n_disp = rect.n_dispersion;
    let n_cross = rect.n_cross;
    let rn2 = config.read_noise * config.read_noise;
    let f2 = config.excess_noise_factor * config.excess_noise_factor;
    let mut n_flagged = 0;

    for (col, &flux_col) in flux.iter().enumerate().take(n_disp) {
        for row in 0..n_cross {
            let idx = row * n_disp + col;
            if !pixel_mask[idx] {
                continue;
            }

            let data_val = f64::from(rect.data[idx]);
            let sky_val = sky.map_or(0.0, |sk| f64::from(sk[idx]));
            let prof = profile[idx];

            // Variance (bd-8yjd1 P2.4): identical floor + formula as Stage B
            // so `residual/σ` is meaningful and the CR threshold bites the
            // same σ-scale used by the fit.
            let v_floor = (rn2 * 1e-6).max(1e-10);
            let var_raw = rn2 + f2 * (sky_val + flux_col * prof).max(0.0) / config.gain;
            let var = var_raw.max(v_floor);

            let residual = (data_val - sky_val - flux_col * prof) / var.sqrt();
            if residual.abs() > config.cr_sigma {
                pixel_mask[idx] = false;
                n_flagged += 1;
            }
        }
    }

    n_flagged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rectification::{OrderSpec, RectifyConfig, rectify_order};
    use crate::types::PolynomialBasis;

    /// bd-lf1bi / Phase D: the iStar preset must carry F=1.6 and
    /// cr_sigma=5.0 per NotebookLM §FM5. Regression gate against any
    /// future edit silently reverting either to the standard-CCD
    /// defaults — those values would underestimate high-flux variance
    /// by ~2.56× and re-admit cosmic rays at profile centres.
    #[test]
    fn test_istar_preset_has_excess_noise_and_tight_cr_sigma() {
        let cfg = OptimalExtractionConfig::istar_iccd();
        assert!(
            (cfg.excess_noise_factor - 1.6).abs() < 1e-12,
            "iStar excess_noise_factor must be 1.6 per NotebookLM §FM5, got {}",
            cfg.excess_noise_factor
        );
        assert!(
            (cfg.cr_sigma - 5.0).abs() < 1e-12,
            "iStar cr_sigma must be 5.0 (NotebookLM §FM5 tightens from CCD 10.0 / 6.0 \
             now that F=1.6 makes the variance accurate), got {}",
            cfg.cr_sigma
        );
        assert!(cfg.gain > 0.0, "gain must be positive, got {}", cfg.gain);
        // The F² multiplier in the variance numerator must inflate the
        // shot-noise term by exactly 2.56× vs a standard CCD (F=1).
        let f_sq = cfg.excess_noise_factor * cfg.excess_noise_factor;
        assert!(
            (f_sq - 2.56).abs() < 1e-12,
            "F² for iStar ICCD must equal 2.56 (the documented variance \
             inflation factor), got {f_sq}"
        );
    }

    fn flat_trace(center: f64) -> crate::types::EchelleTraceModel {
        crate::types::EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: vec![center],
            domain_start: 0.0,
            domain_end: 1000.0,
        }
    }

    /// Build a synthetic frame with a Gaussian spatial profile at each column.
    fn make_gaussian_frame(
        width: u32,
        height: u32,
        trace_center: f64,
        sigma: f64,
        peak_flux: f64,
        noise: f64,
    ) -> Vec<f32> {
        let w = width as usize;
        let h = height as usize;
        let mut frame = vec![0.0f32; w * h];

        for row in 0..h {
            for col in 0..w {
                let dist = row as f64 - trace_center;
                let profile = (-0.5 * (dist / sigma).powi(2)).exp();
                // Simple deterministic noise pattern.
                let n = noise * ((col as f64 * 0.73).sin() + (row as f64 * 0.37).cos());
                frame[row * w + col] = (peak_flux * profile + n) as f32;
            }
        }
        frame
    }

    #[test]
    fn test_optimal_vs_boxcar_higher_snr() {
        // Gaussian profile centered at row 25 with sigma=2, peak=1000.
        let width: u32 = 200;
        let height: u32 = 50;
        let center = 25.0;
        let sigma = 2.0;
        let peak = 1000.0;
        let noise = 5.0;

        let frame = make_gaussian_frame(width, height, center, sigma, peak, noise);
        let trace = flat_trace(center);

        // Use flat (non-Gaussian) weights so boxcar does uniform summation.
        // This is the standard Horne 1986 comparison: flat boxcar vs optimal.
        let rect_config = RectifyConfig {
            aperture_half_width: 6.0,
            gaussian_weights: false,
            fwhm: 4.0,
        };
        let spec = OrderSpec {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
            order_index: 0,
        };

        let rect =
            rectify_order(&frame, width, height, &spec, &rect_config).expect("should rectify");

        // Optimal extraction (learns profile from data).
        let opt_config = OptimalExtractionConfig {
            read_noise: noise,
            gain: 1.0,
            ..Default::default()
        };
        let opt_result = optimal_extract(&rect, None, &opt_config).expect("should extract");

        // Simple unweighted boxcar sum for comparison.
        let mut boxcar_flux = vec![0.0f64; rect.n_dispersion];
        for (col, bf) in boxcar_flux.iter_mut().enumerate() {
            for row in 0..rect.n_cross {
                *bf += f64::from(rect.data[row * rect.n_dispersion + col])
                    * f64::from(rect.mask[row * rect.n_dispersion + col]);
            }
        }

        // Both should produce positive flux.
        let opt_mean: f64 = opt_result.flux.iter().sum::<f64>() / opt_result.flux.len() as f64;
        let box_mean: f64 = boxcar_flux.iter().sum::<f64>() / boxcar_flux.len() as f64;

        assert!(
            opt_mean > 0.0,
            "optimal flux should be positive: {opt_mean}"
        );
        assert!(box_mean > 0.0, "boxcar flux should be positive: {box_mean}");

        // Optimal should have lower variance (higher SNR).
        let opt_var: f64 = opt_result
            .flux
            .iter()
            .map(|&f| (f - opt_mean).powi(2))
            .sum::<f64>()
            / opt_result.flux.len() as f64;
        let box_var: f64 = boxcar_flux
            .iter()
            .map(|&f| (f - box_mean).powi(2))
            .sum::<f64>()
            / boxcar_flux.len() as f64;

        let opt_snr = opt_mean / opt_var.sqrt();
        let box_snr = box_mean / box_var.sqrt();

        assert!(
            opt_snr > box_snr * 0.8,
            "optimal SNR ({opt_snr:.1}) should be comparable to or better than boxcar ({box_snr:.1})"
        );
    }

    #[test]
    fn test_cosmic_ray_rejection() {
        let width: u32 = 100;
        let height: u32 = 30;
        let center = 15.0;
        let sigma = 2.0;

        let mut frame = make_gaussian_frame(width, height, center, sigma, 500.0, 2.0);
        let trace = flat_trace(center);

        // Inject 3 cosmic rays: very bright pixels.
        let w = width as usize;
        frame[15 * w + 50] = 50000.0;
        frame[14 * w + 75] = 40000.0;
        frame[16 * w + 25] = 45000.0;

        let rect_config = RectifyConfig {
            aperture_half_width: 5.0,
            gaussian_weights: true,
            fwhm: 2.0 * (2.0 * 2.0_f64.ln()).sqrt() * sigma,
        };
        let spec = OrderSpec {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
            order_index: 0,
        };

        let rect =
            rectify_order(&frame, width, height, &spec, &rect_config).expect("should rectify");

        let config = OptimalExtractionConfig {
            read_noise: 2.0,
            gain: 1.0,
            cr_sigma: 5.0,
            ..Default::default()
        };

        let result = optimal_extract(&rect, None, &config).expect("should extract");

        // Should have rejected some cosmic ray pixels.
        assert!(
            result.n_cr_rejected >= 3,
            "expected at least 3 CR rejections, got {}",
            result.n_cr_rejected
        );

        // Flux at CR-affected columns should still be reasonable
        // (not dominated by the cosmic ray value).
        assert!(
            result.flux[50] < 10000.0,
            "flux at CR column 50 too high: {}",
            result.flux[50]
        );
    }

    #[test]
    fn test_spatial_profile_normalized() {
        let width: u32 = 50;
        let height: u32 = 20;
        let center = 10.0;
        let frame = make_gaussian_frame(width, height, center, 2.0, 1000.0, 0.0);
        let trace = flat_trace(center);

        let rect_config = RectifyConfig {
            aperture_half_width: 5.0,
            gaussian_weights: false,
            ..Default::default()
        };
        let spec = OrderSpec {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
            order_index: 0,
        };

        let rect =
            rectify_order(&frame, width, height, &spec, &rect_config).expect("should rectify");

        let profile = fit_spatial_profile(&rect, None, &OptimalExtractionConfig::default());

        // Profile should be normalized: sum over cross-dispersion = 1 per column.
        for col in 0..rect.n_dispersion {
            let col_sum: f64 = (0..rect.n_cross)
                .map(|row| profile[row * rect.n_dispersion + col])
                .sum();
            assert!(
                (col_sum - 1.0).abs() < 0.01,
                "profile column {col} sum = {col_sum}, expected 1.0"
            );
        }

        // Profile should be non-negative.
        for &v in &profile {
            assert!(v >= 0.0, "negative profile value: {v}");
        }
    }

    #[test]
    fn test_frac_use_low_coverage_masked() {
        // Create a narrow frame where some columns have very few valid pixels.
        let width: u32 = 50;
        let height: u32 = 10;
        let mut frame = vec![100.0f32; width as usize * height as usize];

        // Zero out most of one column to simulate bad coverage.
        for row in 0..8 {
            frame[row * width as usize + 25] = 0.0;
        }

        let trace = flat_trace(5.0);
        let rect_config = RectifyConfig {
            aperture_half_width: 4.0,
            gaussian_weights: false,
            ..Default::default()
        };
        let spec = OrderSpec {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
            order_index: 0,
        };

        let rect =
            rectify_order(&frame, width, height, &spec, &rect_config).expect("should rectify");

        let config = OptimalExtractionConfig {
            min_frac_use: 0.9,
            ..Default::default()
        };

        let result = optimal_extract(&rect, None, &config).expect("should extract");

        // Most columns should have good coverage.
        let good_cols = result.frac_use.iter().filter(|&&f| f >= 0.9).count();
        assert!(
            good_cols > width as usize / 2,
            "most columns should have good coverage, got {good_cols}"
        );
    }

    #[test]
    fn test_empty_order_returns_none() {
        let rect = RectifiedOrder {
            data: vec![],
            mask: vec![],
            n_dispersion: 0,
            n_cross: 0,
            disp_start: 0,
            cross_start: 0,
            order_index: 0,
            trace_centers: vec![],
        };

        let result = optimal_extract(&rect, None, &OptimalExtractionConfig::default());
        assert!(result.is_none(), "empty order should return None");
    }
}
