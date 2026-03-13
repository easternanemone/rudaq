//! Flat-frame trace detection for echelle order calibration.
//!
//! Detects echelle order traces from a flat-field frame using
//! PypeIt-inspired two-pass centroid refinement:
//!
//! 1. Cross-dispersion collapse → 1D spatial profile
//! 2. Peak detection for order locations
//! 3. Pass 1: uniform-weight centroid tracing + polynomial fit + sigma-clip
//! 4. Pass 2: Gaussian-weight centroid refinement + final polynomial fit
//!
//! This module is inherently numerical — pixel indices are always small enough
//! that usize→f64 casts are lossless, and f64→usize truncation is intentional
//! for converting continuous trace positions to pixel coordinates.
// Module-level cast allowances: pixel indices are always small enough for
// lossless usize→f64, and f64→usize truncation is intentional for pixel coords.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use serde::{Deserialize, Serialize};

use crate::echelle::{EchelleTraceModel, PolynomialBasis};

/// Configuration for trace detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFitConfig {
    /// Minimum SNR for a peak in the collapsed profile (default: 5.0).
    pub min_snr: f64,
    /// Step size in pixels along dispersion axis for centroid sampling (default: 5).
    pub step_pixels: u32,
    /// Half-width of the aperture for centroid computation (default: 4.0 pixels).
    pub aperture_half_width: f64,
    /// Polynomial degree for trace fitting (default: 4).
    pub poly_degree: usize,
    /// Sigma-clip threshold for outlier rejection (default: 5.0).
    pub sigma_clip_thresh: f64,
    /// Gaussian FWHM for Pass 2 weighting (default: 3.0 pixels).
    pub fwhm_gaussian: f64,
}

impl Default for TraceFitConfig {
    fn default() -> Self {
        Self {
            min_snr: 5.0,
            step_pixels: 5,
            aperture_half_width: 4.0,
            poly_degree: 4,
            sigma_clip_thresh: 5.0,
            fwhm_gaussian: 3.0,
        }
    }
}

/// A detected order trace with its polynomial fit.
#[derive(Debug, Clone)]
pub struct OrderTrace {
    /// The fitted trace model.
    pub trace: EchelleTraceModel,
    /// Estimated aperture half-width from the peak profile.
    pub aperture_half_width: f64,
    /// RMS residual of the polynomial fit (pixels).
    pub fit_rms: f64,
    /// Number of centroid samples used in the final fit.
    pub n_samples: usize,
}

/// Detect order traces from a flat-field frame.
///
/// `frame` is row-major f32 intensity data of dimensions `width × height`.
/// Dispersion axis is assumed to be X (horizontal).
///
/// Returns detected traces sorted by cross-dispersion position (Y).
pub fn detect_orders(
    frame: &[f32],
    width: u32,
    height: u32,
    config: &TraceFitConfig,
) -> Vec<OrderTrace> {
    let w = width as usize;
    let h = height as usize;

    if frame.len() < w * h || w < 3 || h < 3 {
        return Vec::new();
    }

    // Step 1: Collapse along dispersion axis (sigma-clipped mean per row).
    let spatial_profile = collapse_dispersion(frame, w, h);

    // Step 2: Find peaks in the spatial profile.
    let noise_rms = estimate_noise(&spatial_profile);
    if noise_rms <= 0.0 {
        return Vec::new();
    }
    let threshold = config.min_snr * noise_rms;
    let peak_rows = find_peaks(&spatial_profile, threshold);

    // Steps 3 & 4: Trace each peak.
    let mut traces = Vec::new();
    for &peak_row in &peak_rows {
        if let Some(trace) = trace_order(frame, w, h, peak_row, config) {
            traces.push(trace);
        }
    }

    // Sort by the trace center at the midpoint of the frame.
    let mid_x = w as f64 / 2.0;
    traces.sort_by(|a, b| {
        let ya = eval_trace_model(&a.trace, mid_x).unwrap_or(0.0);
        let yb = eval_trace_model(&b.trace, mid_x).unwrap_or(0.0);
        ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
    });

    traces
}

/// Collapse frame along dispersion axis: sigma-clipped mean per row.
fn collapse_dispersion(frame: &[f32], w: usize, h: usize) -> Vec<f64> {
    let mut profile = Vec::with_capacity(h);
    for row in 0..h {
        let row_data: Vec<f64> = frame[row * w..(row + 1) * w]
            .iter()
            .map(|&v| f64::from(v))
            .collect();
        profile.push(sigma_clipped_mean(&row_data, 3.0));
    }
    profile
}

/// Compute sigma-clipped mean (iterative 3-sigma rejection, 3 rounds).
fn sigma_clipped_mean(data: &[f64], sigma_thresh: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut mask = vec![true; data.len()];

    for _ in 0..3 {
        let (sum, count) = data
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .fold((0.0, 0usize), |(s, c), (&v, _)| (s + v, c + 1));
        if count == 0 {
            return 0.0;
        }
        let mean = sum / count as f64;

        let var = data
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(&v, _)| (v - mean).powi(2))
            .sum::<f64>()
            / count as f64;
        let std_dev = var.sqrt();

        if std_dev < 1e-10 {
            return mean;
        }

        for (m, &v) in mask.iter_mut().zip(data) {
            if *m && (v - mean).abs() > sigma_thresh * std_dev {
                *m = false;
            }
        }
    }

    let (sum, count) = data
        .iter()
        .zip(&mask)
        .filter(|(_, &m)| m)
        .fold((0.0, 0usize), |(s, c), (&v, _)| (s + v, c + 1));
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

/// Estimate noise from inter-percentile range of the spatial profile.
fn estimate_noise(profile: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = profile.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.len() < 10 {
        return 0.0;
    }
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    (sorted[n * 84 / 100] - sorted[n * 16 / 100]) / 2.0
}

/// Find local maxima in the profile above the threshold.
fn find_peaks(profile: &[f64], threshold: f64) -> Vec<usize> {
    let n = profile.len();
    if n < 3 {
        return Vec::new();
    }

    let mut peaks = Vec::new();
    for i in 1..n - 1 {
        if profile[i] > threshold && profile[i] > profile[i - 1] && profile[i] > profile[i + 1] {
            peaks.push(i);
        }
    }
    peaks
}

/// Trace a single order starting from the peak row.
///
/// Pass 1: uniform centroid + polynomial fit + sigma-clip.
/// Pass 2: Gaussian centroid + final polynomial fit.
fn trace_order(
    frame: &[f32],
    w: usize,
    h: usize,
    peak_row: usize,
    config: &TraceFitConfig,
) -> Option<OrderTrace> {
    let step = config.step_pixels.max(1) as usize;
    let half_w = config.aperture_half_width;
    let domain_end = (w - 1) as f64;

    // --- Pass 1: uniform-weight centroids ---
    let mut xs = Vec::new();
    let mut ys = Vec::new();

    let mut x = 0;
    while x < w {
        let xf = x as f64;
        if let Some(cy) = uniform_centroid(frame, w, h, x, peak_row, half_w) {
            xs.push(xf);
            ys.push(cy);
        }
        x += step;
    }

    if xs.len() < (config.poly_degree + 1).max(3) {
        return None;
    }

    // Fit polynomial (Pass 1).
    let mut coeffs = fit_polynomial(&xs, &ys, config.poly_degree, 0.0, domain_end)?;

    // Sigma-clip residuals and refit.
    let mut mask = vec![true; xs.len()];
    for _ in 0..2 {
        let residuals: Vec<f64> = xs
            .iter()
            .zip(&ys)
            .map(|(&x, &y)| y - eval_monomial(&coeffs, x))
            .collect();

        let rms = residual_rms(&residuals, &mask);
        if rms < 1e-10 {
            break;
        }

        for (i, &r) in residuals.iter().enumerate() {
            if mask[i] && r.abs() > config.sigma_clip_thresh * rms {
                mask[i] = false;
            }
        }

        // Refit with unmasked points.
        let fxs: Vec<f64> = xs
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(&x, _)| x)
            .collect();
        let fys: Vec<f64> = ys
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(&y, _)| y)
            .collect();

        if fxs.len() < (config.poly_degree + 1).max(3) {
            return None;
        }
        coeffs = fit_polynomial(&fxs, &fys, config.poly_degree, 0.0, domain_end)?;
    }

    // --- Pass 2: Gaussian-weight centroids ---
    // sigma = FWHM / (2 * sqrt(2 * ln(2))) ≈ FWHM / 2.3548
    let gauss_sigma = config.fwhm_gaussian / (2.0 * (2.0 * 2.0_f64.ln()).sqrt());
    let mut xs2 = Vec::new();
    let mut ys2 = Vec::new();

    let mut x = 0;
    while x < w {
        let xf = x as f64;
        // Use Pass 1 fit as the center estimate for Gaussian weighting.
        let center_est = eval_monomial(&coeffs, xf);
        if let Some(cy) = gaussian_centroid(frame, w, h, x, center_est, half_w, gauss_sigma) {
            xs2.push(xf);
            ys2.push(cy);
        }
        x += step;
    }

    if xs2.len() < (config.poly_degree + 1).max(3) {
        return None;
    }

    let final_coeffs = fit_polynomial(&xs2, &ys2, config.poly_degree, 0.0, domain_end)?;

    // Compute fit RMS.
    let residuals: Vec<f64> = xs2
        .iter()
        .zip(&ys2)
        .map(|(&x, &y)| y - eval_monomial(&final_coeffs, x))
        .collect();
    let all_mask = vec![true; residuals.len()];
    let rms = residual_rms(&residuals, &all_mask);

    Some(OrderTrace {
        trace: EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: final_coeffs,
            domain_start: 0.0,
            domain_end,
        },
        aperture_half_width: half_w,
        fit_rms: rms,
        n_samples: xs2.len(),
    })
}

/// Uniform-weight centroid in the cross-dispersion direction at column `col`.
fn uniform_centroid(
    frame: &[f32],
    w: usize,
    h: usize,
    col: usize,
    center_row: usize,
    half_width: f64,
) -> Option<f64> {
    let hw = half_width.ceil() as usize;
    let start = center_row.saturating_sub(hw);
    let end = (center_row + hw + 1).min(h);

    let mut sum_val = 0.0_f64;
    let mut sum_y = 0.0_f64;

    for row in start..end {
        let val = f64::from(frame[row * w + col]);
        if val > 0.0 {
            let y = row as f64;
            sum_val += val;
            sum_y += val * y;
        }
    }

    if sum_val > 0.0 {
        Some(sum_y / sum_val)
    } else {
        None
    }
}

/// Gaussian-weight centroid in the cross-dispersion direction at column `col`.
fn gaussian_centroid(
    frame: &[f32],
    w: usize,
    h: usize,
    col: usize,
    center_est: f64,
    half_width: f64,
    gauss_sigma: f64,
) -> Option<f64> {
    let hw = half_width.ceil() as usize;
    let center_row = center_est.round() as usize;
    let start = center_row.saturating_sub(hw);
    let end = (center_row + hw + 1).min(h);

    let mut sum_wv = 0.0_f64;
    let mut sum_wy = 0.0_f64;

    for row in start..end {
        let val = f64::from(frame[row * w + col]);
        if val > 0.0 {
            let y = row as f64;
            let weight = (-(y - center_est).powi(2) / (2.0 * gauss_sigma * gauss_sigma)).exp();
            let wv = val * weight;
            sum_wv += wv;
            sum_wy += wv * y;
        }
    }

    if sum_wv > 0.0 {
        Some(sum_wy / sum_wv)
    } else {
        None
    }
}

/// Evaluate a monomial polynomial: c[0] + c[1]*x + c[2]*x^2 + ...
fn eval_monomial(coeffs: &[f64], x: f64) -> f64 {
    let mut result = 0.0;
    for &c in coeffs.iter().rev() {
        result = result * x + c;
    }
    result
}

/// Evaluate an `EchelleTraceModel` at position `x`.
fn eval_trace_model(trace: &EchelleTraceModel, x: f64) -> Option<f64> {
    match trace {
        EchelleTraceModel::Polynomial {
            coefficients,
            domain_start,
            domain_end,
            ..
        } => {
            if x < *domain_start || x > *domain_end || coefficients.is_empty() {
                return None;
            }
            Some(eval_monomial(coefficients, x))
        }
    }
}

/// Compute RMS of masked residuals.
fn residual_rms(residuals: &[f64], mask: &[bool]) -> f64 {
    let (sum_sq, count) = residuals
        .iter()
        .zip(mask)
        .filter(|(_, &m)| m)
        .fold((0.0, 0usize), |(s, c), (&r, _)| (s + r * r, c + 1));
    if count == 0 {
        0.0
    } else {
        (sum_sq / count as f64).sqrt()
    }
}

/// Fit a polynomial of given degree using least-squares (normal equations).
///
/// Internally normalizes x-coordinates to [-1, 1] for numerical stability,
/// then converts coefficients back to the original domain. This avoids
/// catastrophic precision loss from monomial x^8 ≈ 10^29 on 4K frames.
///
/// Returns coefficients [c0, c1, ..., cn] in the original x domain.
fn fit_polynomial(
    xs: &[f64],
    ys: &[f64],
    degree: usize,
    _domain_start: f64,
    _domain_end: f64,
) -> Option<Vec<f64>> {
    let n = degree + 1;
    if xs.len() < n {
        return None;
    }

    // Center and scale x to [-1, 1] for conditioning.
    let x_min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let x_max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let x_range = x_max - x_min;

    // If all x-values are identical, only a constant can be fit.
    if x_range < 1e-10 {
        if degree == 0 {
            let mean = ys.iter().sum::<f64>() / ys.len() as f64;
            return Some(vec![mean]);
        }
        return None;
    }

    let alpha = 2.0 / x_range; // scale factor
    let beta = -(x_max + x_min) / x_range; // shift: t = alpha*x + beta

    // Transform x to normalized coordinates.
    let ts: Vec<f64> = xs.iter().map(|&x| alpha * x + beta).collect();

    // Build normal equations in normalized space: (V^T V) c = V^T y.
    let m = ts.len();
    let mut vtv = vec![0.0_f64; n * n];
    let mut vty = vec![0.0_f64; n];

    for i in 0..m {
        let t = ts[i];
        let y = ys[i];
        let mut ti = 1.0_f64;
        for j in 0..n {
            let mut tij = ti;
            for k in j..n {
                vtv[j * n + k] += ti * tij;
                if j != k {
                    vtv[k * n + j] += ti * tij;
                }
                tij *= t;
            }
            vty[j] += ti * y;
            ti *= t;
        }
    }

    // Gaussian elimination with partial pivoting.
    let mut aug = vec![0.0_f64; n * (n + 1)];
    for i in 0..n {
        for j in 0..n {
            aug[i * (n + 1) + j] = vtv[i * n + j];
        }
        aug[i * (n + 1) + n] = vty[i];
    }

    for col in 0..n {
        let mut max_row = col;
        let mut max_val = aug[col * (n + 1) + col].abs();
        for row in (col + 1)..n {
            let val = aug[row * (n + 1) + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        if max_val < 1e-15 {
            return None;
        }

        if max_row != col {
            for j in 0..=n {
                aug.swap(col * (n + 1) + j, max_row * (n + 1) + j);
            }
        }

        let pivot = aug[col * (n + 1) + col];
        for row in (col + 1)..n {
            let factor = aug[row * (n + 1) + col] / pivot;
            for j in col..=n {
                aug[row * (n + 1) + j] -= factor * aug[col * (n + 1) + j];
            }
        }
    }

    let mut norm_coeffs = vec![0.0_f64; n];
    for i in (0..n).rev() {
        let mut sum = aug[i * (n + 1) + n];
        for j in (i + 1)..n {
            sum -= aug[i * (n + 1) + j] * norm_coeffs[j];
        }
        norm_coeffs[i] = sum / aug[i * (n + 1) + i];
    }

    // Convert normalized coefficients back to original domain.
    // p(x) = Σ a_k (alpha*x + beta)^k
    // Expand using binomial theorem to get coefficients in x.
    Some(denormalize_coefficients(&norm_coeffs, alpha, beta))
}

/// Convert polynomial coefficients from normalized domain t = alpha*x + beta
/// back to coefficients in the original x domain.
///
/// Uses the binomial expansion: `(alpha*x + beta)^k = Σ_j C(k,j) alpha^j beta^(k-j) x^j`
fn denormalize_coefficients(norm_coeffs: &[f64], alpha: f64, beta: f64) -> Vec<f64> {
    let n = norm_coeffs.len();
    let mut orig = vec![0.0_f64; n];

    // Precompute binomial coefficients (Pascal's triangle, max degree ~10).
    let mut binom = vec![vec![0.0_f64; n]; n];
    for k in 0..n {
        binom[k][0] = 1.0;
        for j in 1..=k {
            binom[k][j] = binom[k - 1][j - 1] + if j < k { binom[k - 1][j] } else { 0.0 };
        }
    }

    for (k, &a_k) in norm_coeffs.iter().enumerate() {
        if a_k.abs() < 1e-30 {
            continue;
        }
        // Expand a_k * (alpha*x + beta)^k
        for j in 0..=k {
            // C(k,j) * alpha^j * beta^(k-j)
            let term = a_k * binom[k][j] * alpha.powi(j as i32) * beta.powi((k - j) as i32);
            orig[j] += term;
        }
    }

    orig
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a synthetic flat frame with horizontal order traces.
    ///
    /// Each trace is a horizontal band at the given Y position with a
    /// Gaussian cross-dispersion profile.
    fn synthetic_flat(
        width: usize,
        height: usize,
        traces: &[(f64, f64)], // (y_center, amplitude)
        sigma: f64,
    ) -> Vec<f32> {
        let mut frame = vec![1.0_f32; width * height]; // baseline
        for &(y_center, amplitude) in traces {
            for row in 0..height {
                let y = row as f64;
                let weight = (-(y - y_center).powi(2) / (2.0 * sigma * sigma)).exp();
                let val = (amplitude * weight) as f32;
                for col in 0..width {
                    frame[row * width + col] += val;
                }
            }
        }
        frame
    }

    #[test]
    fn test_detect_horizontal_traces() {
        // 200×300 frame with 3 horizontal traces at y=50, 150, 250.
        // Height must be large enough that peaks are a small fraction of rows,
        // otherwise the inter-percentile noise estimate is inflated by the peaks.
        let width = 200;
        let height = 300;
        let traces = vec![(50.0, 500.0), (150.0, 800.0), (250.0, 600.0)];
        let frame = synthetic_flat(width, height, &traces, 2.0);

        let config = TraceFitConfig {
            poly_degree: 2, // Horizontal → low-degree is sufficient
            step_pixels: 10,
            ..Default::default()
        };
        let detected = detect_orders(&frame, width as u32, height as u32, &config);

        assert_eq!(
            detected.len(),
            3,
            "expected 3 traces, got {}: {:?}",
            detected.len(),
            detected.iter().map(|t| &t.trace).collect::<Vec<_>>()
        );

        // Verify centers are close to true values (within 0.5 pixel).
        let mid_x = width as f64 / 2.0;
        for (trace, &(true_y, _)) in detected.iter().zip(&traces) {
            let fit_y = eval_trace_model(&trace.trace, mid_x).unwrap();
            assert!(
                (fit_y - true_y).abs() < 0.5,
                "trace center error: expected {true_y}, got {fit_y:.3}"
            );
        }
    }

    #[test]
    fn test_trace_rms_is_small() {
        // A perfectly horizontal trace should have very small fit RMS.
        let width = 100;
        let height = 200;
        let frame = synthetic_flat(width, height, &[(100.0, 1000.0)], 3.0);

        let config = TraceFitConfig::default();
        let detected = detect_orders(&frame, width as u32, height as u32, &config);

        assert_eq!(detected.len(), 1);
        assert!(
            detected[0].fit_rms < 0.1,
            "RMS should be tiny for horizontal trace, got {}",
            detected[0].fit_rms
        );
    }

    #[test]
    fn test_polynomial_fit_recovers_line() {
        // y = 2.0 + 0.01*x → slight slope.
        let xs: Vec<f64> = (0..100).map(f64::from).collect();
        let ys: Vec<f64> = xs.iter().map(|&x| 2.0 + 0.01 * x).collect();

        let coeffs = fit_polynomial(&xs, &ys, 2, 0.0, 99.0).unwrap();

        assert!(
            (coeffs[0] - 2.0).abs() < 1e-6,
            "intercept: expected 2.0, got {}",
            coeffs[0]
        );
        assert!(
            (coeffs[1] - 0.01).abs() < 1e-6,
            "slope: expected 0.01, got {}",
            coeffs[1]
        );
    }

    #[test]
    fn test_empty_frame_returns_empty() {
        let traces = detect_orders(&[], 0, 0, &TraceFitConfig::default());
        assert!(traces.is_empty());
    }

    #[test]
    fn test_sigma_clipped_mean_rejects_outlier() {
        let mut data = vec![10.0; 100];
        data[50] = 1000.0; // one big outlier
        let mean = sigma_clipped_mean(&data, 3.0);
        assert!(
            (mean - 10.0).abs() < 0.1,
            "clipped mean should be ~10.0, got {mean}"
        );
    }
}
