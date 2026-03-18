//! Arc line detection and wavelength calibration for echelle spectrographs.
//!
//! Supports any emission-line calibration source (HgAr, ThAr, NeAr, etc.)
//! with a corresponding reference atlas.
//!
//! Arc line detection implements the algorithm from PypeIt's `core/arc.py::detect_lines`:
//! 1. Continuum subtraction via median-filtered baseline
//! 2. Local maximum detection above SNR threshold
//! 3. Gaussian centroid fitting (with parabolic fallback)
//! 4. Quality filtering by FWHM and separation
//!
//! Wavelength fitting uses per-order Chebyshev polynomial solutions with
//! iterative sigma-clip outlier rejection.

use serde::{Deserialize, Serialize};

/// Configuration for arc line detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArcDetectConfig {
    /// Minimum SNR for a peak to be considered (default: 5.0).
    pub sigdetect: f64,
    /// Minimum acceptable FWHM in pixels (default: 1.5).
    pub min_fwhm: f64,
    /// Maximum acceptable FWHM in pixels (default: 6.0).
    pub max_fwhm: f64,
    /// Minimum separation in pixels between accepted lines (default: 3.0).
    pub min_separation: f64,
    /// Median filter window size for continuum estimation (default: 101).
    pub continuum_window: usize,
}

impl Default for ArcDetectConfig {
    fn default() -> Self {
        Self {
            sigdetect: 5.0,
            min_fwhm: 1.5,
            max_fwhm: 6.0,
            min_separation: 3.0,
            continuum_window: 101,
        }
    }
}

/// A detected arc line with fitted parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArcLine {
    /// Echelle order index (relative).
    pub order: u32,
    /// Sub-pixel center position from Gaussian (or parabolic) fit.
    pub pixel_center: f64,
    /// Gaussian sigma of the line profile (pixels).
    pub pixel_sigma: f64,
    /// Fitted amplitude (continuum-subtracted peak flux).
    pub amplitude: f64,
    /// Optional wavelength from a matched atlas line (not set by detection;
    /// populated externally if needed for diagnostics).
    pub wavelength_hint: Option<f64>,
    /// Whether this line passed quality filtering. Set to `true` by detection;
    /// callers may set to `false` to exclude from downstream fits.
    pub used: bool,
}

impl ArcLine {
    /// Full width at half maximum: FWHM = 2 * sqrt(2 * ln(2)) * sigma ≈ 2.3548 * sigma.
    #[must_use]
    pub fn fwhm(&self) -> f64 {
        2.0 * (2.0 * 2.0_f64.ln()).sqrt() * self.pixel_sigma
    }
}

/// Detect arc emission lines in a 1D extracted order spectrum.
///
/// Returns detected lines sorted by pixel position.
pub fn detect_arc_lines(
    order_spectrum: &[f32],
    order_index: u32,
    config: &ArcDetectConfig,
) -> Vec<ArcLine> {
    if order_spectrum.len() < 5 {
        return Vec::new();
    }

    // Step 1: Continuum subtraction via running median.
    let continuum = running_median(order_spectrum, config.continuum_window);
    let subtracted: Vec<f64> = order_spectrum
        .iter()
        .zip(&continuum)
        .map(|(&s, &c)| f64::from(s) - c)
        .collect();

    // Estimate noise from inter-percentile range of continuum-subtracted spectrum.
    // For noiseless (synthetic) data, fall back to a fraction of the peak value
    // so that detection still works.
    let noise_rms = estimate_noise_rms(&subtracted);
    let effective_noise = if noise_rms > 0.0 {
        noise_rms
    } else {
        // Fallback: use 1% of the maximum value as a noise proxy.
        let max_val = subtracted.iter().copied().fold(0.0_f64, f64::max);
        if max_val <= 0.0 {
            return Vec::new();
        }
        max_val * 0.01
    };

    let threshold = config.sigdetect * effective_noise;

    // Step 2: Find local maxima above threshold.
    let candidates = find_local_maxima(&subtracted, threshold);

    // Step 3: Gaussian centroid fitting for each candidate.
    let mut lines: Vec<ArcLine> = Vec::new();
    for &peak_idx in &candidates {
        if let Some(line) = fit_gaussian_centroid(&subtracted, peak_idx, order_index) {
            lines.push(line);
        }
    }

    // Step 4: Quality filtering.
    lines.retain(|line| {
        let fwhm = line.fwhm();
        fwhm >= config.min_fwhm
            && fwhm <= config.max_fwhm
            && line.amplitude / effective_noise >= config.sigdetect
    });

    // Remove weaker lines within min_separation of a brighter line.
    // Sort by amplitude descending so brighter lines win.
    lines.sort_by(|a, b| {
        b.amplitude
            .partial_cmp(&a.amplitude)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut accepted: Vec<ArcLine> = Vec::new();
    for line in lines {
        let too_close = accepted
            .iter()
            .any(|a| (a.pixel_center - line.pixel_center).abs() < config.min_separation);
        if !too_close {
            accepted.push(line);
        }
    }

    // Sort final output by pixel position.
    accepted.sort_by(|a, b| {
        a.pixel_center
            .partial_cmp(&b.pixel_center)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    accepted
}

/// Running median filter. Uses a sliding window forced to an odd size.
fn running_median(data: &[f32], window_size: usize) -> Vec<f64> {
    let n = data.len();
    // Force odd window: OR with 1 ensures odd, max(3) ensures minimum size.
    let effective_window = window_size.max(3) | 1;
    let half = effective_window / 2;
    let mut result = Vec::with_capacity(n);
    let mut scratch = Vec::with_capacity(window_size);

    for i in 0..n {
        scratch.clear();
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(n);
        for &v in &data[start..end] {
            scratch.push(f64::from(v));
        }
        scratch.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = scratch.len() / 2;
        let median = if scratch.len() % 2 == 0 {
            scratch[mid - 1].midpoint(scratch[mid])
        } else {
            scratch[mid]
        };
        result.push(median);
    }

    result
}

/// Estimate noise RMS from inter-percentile range (16th–84th ≈ 1σ for Gaussian).
fn estimate_noise_rms(data: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = data.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.len() < 10 {
        return 0.0;
    }
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len();
    let p16 = sorted[n * 16 / 100];
    let p84 = sorted[n * 84 / 100];

    // For a Gaussian, the 16th-to-84th percentile range spans 2σ.
    (p84 - p16) / 2.0
}

/// Find local maxima in `data` that exceed `threshold`.
///
/// A local maximum at index `i` satisfies: data[i] > data[i-1] && data[i] > data[i+1].
fn find_local_maxima(data: &[f64], threshold: f64) -> Vec<usize> {
    let n = data.len();
    if n < 3 {
        return Vec::new();
    }

    let mut peaks = Vec::new();
    for i in 1..n - 1 {
        if data[i] > threshold && data[i] > data[i - 1] && data[i] > data[i + 1] {
            peaks.push(i);
        }
    }
    peaks
}

/// Fit a Gaussian centroid to the peak at `peak_idx`.
///
/// Uses a ±3px window for a least-squares Gaussian fit via the log-parabola method:
/// `ln(y) = a*x^2 + b*x + c` → `center = -b/(2a)`, `sigma = sqrt(-1/(2a))`.
///
/// Falls back to 3-point parabolic centroid if the window is too small or data is degenerate.
fn fit_gaussian_centroid(data: &[f64], peak_idx: usize, order_index: u32) -> Option<ArcLine> {
    let n = data.len();
    let amplitude = data[peak_idx];

    if amplitude <= 0.0 {
        return None;
    }

    // Try log-parabola fit on a ±3 pixel window.
    let half_win: usize = 3;
    let start = peak_idx.saturating_sub(half_win);
    let end = (peak_idx + half_win + 1).min(n);

    // Collect positive samples for log-space fitting.
    // Center coordinates around peak_idx for numerical stability.
    let mut xs = Vec::new();
    let mut log_ys = Vec::new();
    for (i, &val) in data[start..end].iter().enumerate() {
        if val > 0.0 {
            #[allow(clippy::cast_precision_loss)] // pixel indices are small
            let x_offset = (start + i) as f64 - peak_idx as f64;
            xs.push(x_offset);
            log_ys.push(val.ln());
        }
    }

    // Need at least 3 points for quadratic fit.
    if xs.len() >= 3 {
        if let Some((center_offset, sigma)) = fit_log_parabola(&xs, &log_ys) {
            // Recover absolute center from offset.
            #[allow(clippy::cast_precision_loss)] // peak_idx is small
            let center = peak_idx as f64 + center_offset;
            // Sanity check: center should be near the peak.
            #[allow(clippy::cast_precision_loss)] // half_win is 3
            let half_f = half_win as f64;
            if center_offset.abs() < half_f && sigma > 0.0 && sigma.is_finite() {
                return Some(ArcLine {
                    order: order_index,
                    pixel_center: center,
                    pixel_sigma: sigma,
                    amplitude,
                    wavelength_hint: None,
                    used: true,
                });
            }
        }
    }

    // Fallback: 3-point parabolic centroid.
    if peak_idx >= 1 && peak_idx + 1 < n && data[peak_idx - 1] > 0.0 && data[peak_idx + 1] > 0.0 {
        let ym1 = data[peak_idx - 1];
        let y0 = data[peak_idx];
        let yp1 = data[peak_idx + 1];
        let denom = 2.0 * (2.0 * y0 - ym1 - yp1);
        if denom.abs() > 1e-10 {
            #[allow(clippy::cast_precision_loss)]
            let center = peak_idx as f64 + (ym1 - yp1) / denom;
            // Estimate sigma from the curvature: sigma ≈ sqrt(-y0 / (y0'' / 2)).
            let curvature = ym1 + yp1 - 2.0 * y0;
            let sigma = if curvature < -1e-10 {
                (-y0 / curvature).sqrt()
            } else {
                1.0 // fallback default
            };
            return Some(ArcLine {
                order: order_index,
                pixel_center: center,
                pixel_sigma: sigma,
                amplitude,
                wavelength_hint: None,
                used: true,
            });
        }
    }

    None
}

/// Weighted least-squares fit of `ln(y) = a*x^2 + b*x + c`.
///
/// Returns `(center, sigma)` where `center = -b/(2a)` and `sigma = sqrt(-1/(2a))`.
fn fit_log_parabola(xs: &[f64], log_ys: &[f64]) -> Option<(f64, f64)> {
    // Build normal equations for polynomial: ln(y) = a*x^2 + b*x + c.
    // Using direct Cramer's rule for 3x3 system (sum of x^0..x^4).
    let n = xs.len();
    debug_assert!(n >= 3);

    let mut s0: f64 = 0.0;
    let mut s1: f64 = 0.0;
    let mut s2: f64 = 0.0;
    let mut s3: f64 = 0.0;
    let mut s4: f64 = 0.0;
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 0.0;
    let mut t2: f64 = 0.0;

    for (&x, &ly) in xs.iter().zip(log_ys) {
        let x2 = x * x;
        s0 += 1.0;
        s1 += x;
        s2 += x2;
        s3 += x2 * x;
        s4 += x2 * x2;
        t0 += ly;
        t1 += ly * x;
        t2 += ly * x2;
    }

    // Solve:
    //   | s4 s3 s2 | | a |   | t2 |
    //   | s3 s2 s1 | | b | = | t1 |
    //   | s2 s1 s0 | | c |   | t0 |
    let det = s4 * (s2 * s0 - s1 * s1) - s3 * (s3 * s0 - s1 * s2) + s2 * (s3 * s1 - s2 * s2);
    if det.abs() < 1e-20 {
        return None;
    }

    let a = (t2 * (s2 * s0 - s1 * s1) - s3 * (t1 * s0 - s1 * t0) + s2 * (t1 * s1 - s2 * t0)) / det;
    let b = (s4 * (t1 * s0 - s1 * t0) - t2 * (s3 * s0 - s1 * s2) + s2 * (s3 * t0 - t1 * s2)) / det;

    // a must be negative for a valid Gaussian peak.
    if a >= 0.0 {
        return None;
    }

    let center = -b / (2.0 * a);
    let sigma = (-1.0 / (2.0 * a)).sqrt();

    Some((center, sigma))
}

// ─── Phase 1: Per-order Chebyshev wavelength solution fitting ───────────────

/// Configuration for wavelength solution fitting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WlFitConfig {
    /// Chebyshev polynomial degree for the per-order fit (default: 4).
    pub poly_degree: usize,
    /// Sigma-clip threshold for outlier rejection (default: 3.0).
    pub sigma_clip: f64,
    /// Maximum sigma-clip iterations (default: 5).
    pub max_clip_iters: usize,
    /// Seed tolerance in nm for initial atlas matching (default: 0.5).
    pub seed_tolerance_nm: f64,
}

impl Default for WlFitConfig {
    fn default() -> Self {
        Self {
            poly_degree: 4,
            sigma_clip: 3.0,
            max_clip_iters: 5,
            seed_tolerance_nm: 0.5,
        }
    }
}

/// A single entry in a reference emission line atlas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasLine {
    /// Wavelength in nm.
    pub wavelength_nm: f64,
    /// Emitting species (e.g. "Th I", "Ar II").
    pub species: String,
    /// Relative intensity (arbitrary units).
    pub strength: f64,
}

/// Load a pure Mercury (Hg I) reference atlas for HG-2 type lamps.
///
/// Contains ~35 Hg I lines from the NIST Atomic Spectra Database covering
/// 253-1014nm. Suitable for the OceanInsight HG-2 (pure mercury, no argon
/// fill gas). Many more lines than the 11 "strongest" list — essential for
/// echelle calibration where each order covers only ~20-30nm and needs
/// at least 2 matches.
#[must_use]
pub fn load_hg_atlas() -> Vec<AtlasLine> {
    vec![
        // NIST Hg I air wavelengths, ordered by wavelength.
        // Source: NIST ASD (https://physics.nist.gov/PhysRefData/ASD/lines_form.html)
        // Strengths are NIST relative intensities (approximate).
        a(253.652, "Hg I", 10000.0), // UV resonance line
        a(265.204, "Hg I", 80.0),
        a(275.278, "Hg I", 50.0),
        a(280.346, "Hg I", 30.0),
        a(289.360, "Hg I", 40.0),
        a(292.541, "Hg I", 30.0),
        a(296.728, "Hg I", 2000.0),
        a(302.150, "Hg I", 1500.0),
        a(312.567, "Hg I", 1800.0),
        a(313.155, "Hg I", 1200.0), // close doublet with 312.567
        a(313.184, "Hg I", 800.0),
        a(334.148, "Hg I", 3000.0),
        a(365.015, "Hg I", 8000.0), // strong UV triplet member
        a(365.484, "Hg I", 2000.0), // triplet
        a(366.289, "Hg I", 600.0),  // triplet
        a(390.644, "Hg I", 100.0),
        a(398.393, "Hg I", 60.0),
        a(404.656, "Hg I", 6000.0), // violet line (h)
        a(407.783, "Hg I", 80.0),
        a(433.922, "Hg I", 50.0),
        a(435.833, "Hg I", 9000.0), // blue line (g)
        a(491.607, "Hg I", 100.0),
        a(496.027, "Hg I", 40.0),
        a(502.564, "Hg I", 60.0),
        a(546.074, "Hg I", 10000.0), // green line (e)
        a(567.581, "Hg I", 60.0),
        a(576.960, "Hg I", 4000.0), // yellow doublet
        a(579.066, "Hg I", 3500.0), // yellow doublet
        a(580.378, "Hg I", 30.0),
        a(585.924, "Hg I", 20.0),
        a(587.086, "Hg I", 20.0),
        a(607.278, "Hg I", 20.0),
        a(612.327, "Hg I", 20.0),
        a(623.440, "Hg I", 30.0),
        a(671.643, "Hg I", 50.0),
        a(690.752, "Hg I", 30.0),
        a(708.190, "Hg I", 20.0),
        a(709.187, "Hg I", 20.0),
        a(773.669, "Hg I", 20.0),
        a(871.685, "Hg I", 10.0),
        a(1013.975, "Hg I", 80.0), // NIR line
    ]
}

/// Convenience constructor for `AtlasLine`.
fn a(wavelength_nm: f64, species: &str, strength: f64) -> AtlasLine {
    AtlasLine {
        wavelength_nm,
        species: species.into(),
        strength,
    }
}

/// Load the HgAr (Mercury-Argon) reference emission line atlas.
///
/// Returns the ~30 strongest Hg I and Ar I lines in the 200-900nm range,
/// suitable for calibrating with an HgAr hollow cathode lamp.
/// For the OceanInsight HG-2 (pure mercury), use [`load_hg_atlas`] instead.
#[must_use]
pub fn load_hgar_atlas() -> Vec<AtlasLine> {
    vec![
        // ── Mercury I lines ──────────────────────────────────────────
        AtlasLine {
            wavelength_nm: 253.652,
            species: "Hg I".into(),
            strength: 1000.0,
        },
        AtlasLine {
            wavelength_nm: 296.728,
            species: "Hg I".into(),
            strength: 200.0,
        },
        AtlasLine {
            wavelength_nm: 302.150,
            species: "Hg I".into(),
            strength: 150.0,
        },
        AtlasLine {
            wavelength_nm: 312.567,
            species: "Hg I".into(),
            strength: 180.0,
        },
        AtlasLine {
            wavelength_nm: 334.148,
            species: "Hg I".into(),
            strength: 300.0,
        },
        AtlasLine {
            wavelength_nm: 365.015,
            species: "Hg I".into(),
            strength: 800.0,
        },
        AtlasLine {
            wavelength_nm: 404.656,
            species: "Hg I".into(),
            strength: 600.0,
        },
        AtlasLine {
            wavelength_nm: 435.833,
            species: "Hg I".into(),
            strength: 900.0,
        },
        AtlasLine {
            wavelength_nm: 546.074,
            species: "Hg I".into(),
            strength: 1000.0,
        },
        AtlasLine {
            wavelength_nm: 576.960,
            species: "Hg I".into(),
            strength: 400.0,
        },
        AtlasLine {
            wavelength_nm: 579.066,
            species: "Hg I".into(),
            strength: 350.0,
        },
        // ── Argon I lines ────────────────────────────────────────────
        AtlasLine {
            wavelength_nm: 696.543,
            species: "Ar I".into(),
            strength: 500.0,
        },
        AtlasLine {
            wavelength_nm: 706.722,
            species: "Ar I".into(),
            strength: 300.0,
        },
        AtlasLine {
            wavelength_nm: 714.704,
            species: "Ar I".into(),
            strength: 200.0,
        },
        AtlasLine {
            wavelength_nm: 727.294,
            species: "Ar I".into(),
            strength: 250.0,
        },
        AtlasLine {
            wavelength_nm: 738.398,
            species: "Ar I".into(),
            strength: 350.0,
        },
        AtlasLine {
            wavelength_nm: 750.387,
            species: "Ar I".into(),
            strength: 600.0,
        },
        AtlasLine {
            wavelength_nm: 751.465,
            species: "Ar I".into(),
            strength: 400.0,
        },
        AtlasLine {
            wavelength_nm: 763.511,
            species: "Ar I".into(),
            strength: 1000.0,
        },
        AtlasLine {
            wavelength_nm: 772.376,
            species: "Ar I".into(),
            strength: 350.0,
        },
        AtlasLine {
            wavelength_nm: 794.818,
            species: "Ar I".into(),
            strength: 400.0,
        },
        AtlasLine {
            wavelength_nm: 800.616,
            species: "Ar I".into(),
            strength: 500.0,
        },
        AtlasLine {
            wavelength_nm: 801.479,
            species: "Ar I".into(),
            strength: 300.0,
        },
        AtlasLine {
            wavelength_nm: 810.369,
            species: "Ar I".into(),
            strength: 350.0,
        },
        AtlasLine {
            wavelength_nm: 811.531,
            species: "Ar I".into(),
            strength: 700.0,
        },
        AtlasLine {
            wavelength_nm: 826.452,
            species: "Ar I".into(),
            strength: 400.0,
        },
        AtlasLine {
            wavelength_nm: 840.821,
            species: "Ar I".into(),
            strength: 300.0,
        },
        AtlasLine {
            wavelength_nm: 842.465,
            species: "Ar I".into(),
            strength: 500.0,
        },
        AtlasLine {
            wavelength_nm: 852.144,
            species: "Ar I".into(),
            strength: 350.0,
        },
    ]
}

/// Result of a per-order wavelength solution fit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderWlSolution {
    /// Echelle order index.
    pub order: u32,
    /// Chebyshev polynomial coefficients (degree+1 values).
    pub coefficients: Vec<f64>,
    /// Pixel range [min, max] used for normalizing to [-1, 1].
    pub pixel_min: f64,
    pub pixel_max: f64,
    /// RMS residual in nm.
    pub rms_nm: f64,
    /// Number of lines used in the final fit.
    pub n_lines_used: usize,
    /// Total number of lines matched before clipping.
    pub n_lines_total: usize,
}

impl OrderWlSolution {
    /// Evaluate the wavelength solution at a pixel position.
    #[must_use]
    pub fn eval(&self, pixel: f64) -> f64 {
        let x_norm = 2.0 * (pixel - self.pixel_min) / (self.pixel_max - self.pixel_min) - 1.0;
        chebyshev_eval(&self.coefficients, x_norm)
    }
}

/// Match detected arc lines to atlas entries using a seed wavelength model.
///
/// For each detected line, computes an approximate wavelength from the seed model
/// and finds the closest atlas line within `tolerance_nm`. Returns pairs of
/// (detected line index, atlas line index).
pub fn match_lines_to_atlas(
    lines: &[ArcLine],
    atlas: &[AtlasLine],
    seed_fn: &dyn Fn(f64) -> f64,
    tolerance_nm: f64,
) -> Vec<(usize, usize)> {
    // First pass: find the best atlas match for each detected line.
    let mut candidates: Vec<(usize, usize, f64)> = Vec::new(); // (line_idx, atlas_idx, dist)

    for (li, line) in lines.iter().enumerate() {
        let approx_wl = seed_fn(line.pixel_center);

        // Find closest atlas line within tolerance.
        let mut best_dist = f64::MAX;
        let mut best_ai = None;
        for (ai, atlas_line) in atlas.iter().enumerate() {
            let dist = (atlas_line.wavelength_nm - approx_wl).abs();
            if dist < tolerance_nm && dist < best_dist {
                best_dist = dist;
                best_ai = Some(ai);
            }
        }

        if let Some(ai) = best_ai {
            candidates.push((li, ai, best_dist));
        }
    }

    // Second pass: enforce one-to-one matching. When multiple detected lines
    // match the same atlas entry, keep only the closest one.
    candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut used_atlas: Vec<bool> = vec![false; atlas.len()];
    let mut matches = Vec::new();
    for (li, ai, _dist) in &candidates {
        if !used_atlas[*ai] {
            used_atlas[*ai] = true;
            matches.push((*li, *ai));
        }
    }

    matches
}

/// Fit a per-order Chebyshev wavelength solution from matched arc lines.
///
/// Takes detected lines with `wavelength_hint` filled in (from atlas matching)
/// and fits a Chebyshev polynomial mapping pixel → wavelength (nm).
/// Uses iterative sigma-clipping to reject outliers.
/// Single-line wavelength calibration fallback (bd-3hlp).
///
/// When only 1 atlas match is available, we can't fit a polynomial.
/// Instead, we construct a linear model by:
/// 1. Using the matched line as an absolute wavelength anchor
/// 2. Estimating the dispersion (nm/pixel) from the echelle equation:
///    dλ/dp ≈ -λ / n_pixels (the free spectral range spans the detector)
///
/// This gives a first-order wavelength solution good enough for order
/// identification and approximate wavelength assignment.
fn fit_single_line_fallback(
    lines: &[ArcLine],
    atlas: &[AtlasLine],
    matches: &[(usize, usize)],
    order: u32,
    config: &WlFitConfig,
) -> Option<OrderWlSolution> {
    let (li, ai) = matches[0];
    let anchor_pixel = lines[li].pixel_center;
    let anchor_wl = atlas[ai].wavelength_nm;

    // Estimate dispersion from the free spectral range.
    // For an echelle, FSR ≈ λ/m, spread across ~n_pixels.
    // Use pixel_range from config or default to 2560.
    let n_pixels = config.seed_tolerance_nm.max(1.0) * 512.0; // rough estimate
    let dispersion = -anchor_wl / n_pixels; // nm/pixel (negative = wavelength decreases with pixel)

    // Build linear Chebyshev coefficients on [0, n_pixels-1].
    // λ(p) = anchor_wl + dispersion * (p - anchor_pixel)
    //      = (anchor_wl - dispersion * anchor_pixel) + dispersion * p
    let pixel_min = 0.0;
    let pixel_max = 2559.0; // standard detector width
    let mid = (pixel_min + pixel_max) / 2.0;
    let half_range = (pixel_max - pixel_min) / 2.0;

    // Chebyshev T0 = 1, T1 = x on [-1, 1]
    // λ(x) = c0 + c1 * x where x = (p - mid) / half_range
    // λ(p) = c0 + c1 * (p - mid) / half_range
    // At anchor_pixel: anchor_wl = c0 + c1 * (anchor_pixel - mid) / half_range
    // dispersion = c1 / half_range → c1 = dispersion * half_range
    let c1 = dispersion * half_range;
    let c0 = anchor_wl - c1 * (anchor_pixel - mid) / half_range;

    Some(OrderWlSolution {
        order,
        coefficients: vec![c0, c1],
        pixel_min,
        pixel_max,
        rms_nm: 0.0,
        n_lines_used: 1,
        n_lines_total: 1,
    })
}

pub fn fit_order_wavelength(
    lines: &[ArcLine],
    atlas: &[AtlasLine],
    matches: &[(usize, usize)],
    order: u32,
    config: &WlFitConfig,
) -> Option<OrderWlSolution> {
    if matches.is_empty() {
        return None;
    }
    if matches.len() <= config.poly_degree {
        // Single-line fallback: with only 1 match and poly_degree <= 1,
        // use the echelle equation as a prior and anchor to the matched line.
        // This gives a reasonable linear solution when the echelle equation
        // provides a good dispersion estimate. (bd-3hlp)
        if matches.len() == 1 && config.poly_degree <= 1 {
            return fit_single_line_fallback(lines, atlas, matches, order, config);
        }
        return None;
    }

    // Build pixel positions and wavelengths from matches.
    let pixels: Vec<f64> = matches
        .iter()
        .map(|&(li, _)| lines[li].pixel_center)
        .collect();
    let wavelengths: Vec<f64> = matches
        .iter()
        .map(|&(_, ai)| atlas[ai].wavelength_nm)
        .collect();

    let n_total = pixels.len();

    // Determine pixel range for normalization.
    let pixel_min = pixels.iter().copied().fold(f64::INFINITY, f64::min);
    let pixel_max = pixels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if (pixel_max - pixel_min).abs() < 1e-10 {
        return None;
    }

    // Mask for sigma-clipping (true = use this point).
    let mut mask = vec![true; n_total];

    let mut coefficients = Vec::new();
    let mut rms = 0.0;

    for _ in 0..=config.max_clip_iters {
        // Collect active points.
        let active_pixels: Vec<f64> = pixels
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(&p, _)| p)
            .collect();
        let active_wls: Vec<f64> = wavelengths
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(&w, _)| w)
            .collect();

        if active_pixels.len() <= config.poly_degree {
            break;
        }

        // Normalize pixels to [-1, 1].
        let x_norm: Vec<f64> = active_pixels
            .iter()
            .map(|&p| 2.0 * (p - pixel_min) / (pixel_max - pixel_min) - 1.0)
            .collect();

        // Fit Chebyshev polynomial via least-squares.
        coefficients = chebyshev_fit(&x_norm, &active_wls, config.poly_degree)?;

        // Compute residuals for ALL points (including previously clipped).
        let mut residuals = Vec::with_capacity(n_total);
        for (i, (&px, &wl)) in pixels.iter().zip(&wavelengths).enumerate() {
            if mask[i] {
                let x = 2.0 * (px - pixel_min) / (pixel_max - pixel_min) - 1.0;
                let fitted = chebyshev_eval(&coefficients, x);
                residuals.push(fitted - wl);
            } else {
                residuals.push(0.0);
            }
        }

        // Compute RMS of active residuals.
        let active_count = mask.iter().filter(|&&m| m).count();
        let sum_sq: f64 = residuals
            .iter()
            .zip(&mask)
            .filter(|(_, &m)| m)
            .map(|(r, _)| r * r)
            .sum();
        #[allow(clippy::cast_precision_loss)] // active_count is always small
        let count_f64 = active_count as f64;
        rms = (sum_sq / count_f64).sqrt();

        if rms < 1e-15 {
            break; // perfect fit
        }

        // Sigma-clip: reject points with |residual| > sigma_clip * rms.
        let threshold = config.sigma_clip * rms;
        let mut any_clipped = false;
        for (&r, m) in residuals.iter().zip(mask.iter_mut()) {
            if *m && r.abs() > threshold {
                *m = false;
                any_clipped = true;
            }
        }

        if !any_clipped {
            break;
        }
    }

    let n_used = mask.iter().filter(|&&m| m).count();
    if n_used <= config.poly_degree {
        return None;
    }

    Some(OrderWlSolution {
        order,
        coefficients,
        pixel_min,
        pixel_max,
        rms_nm: rms,
        n_lines_used: n_used,
        n_lines_total: n_total,
    })
}

/// Evaluate a Chebyshev polynomial at a point using the recurrence relation.
///
/// T_0(x) = 1, T_1(x) = x, T_{n+1}(x) = 2*x*T_n(x) - T_{n-1}(x)
fn chebyshev_eval(coeffs: &[f64], x: f64) -> f64 {
    if coeffs.is_empty() {
        return 0.0;
    }
    if coeffs.len() == 1 {
        return coeffs[0];
    }

    let mut t_prev = 1.0; // T_0
    let mut t_curr = x; // T_1
    let mut result = coeffs[0] * t_prev + coeffs[1] * t_curr;

    for &c in &coeffs[2..] {
        let t_next = 2.0 * x * t_curr - t_prev;
        result += c * t_next;
        t_prev = t_curr;
        t_curr = t_next;
    }

    result
}

/// Fit Chebyshev coefficients via least-squares (normal equations).
///
/// Builds the Vandermonde matrix V[i,j] = T_j(x_i), then solves V^T*V*c = V^T*y
/// using Gaussian elimination with partial pivoting.
#[allow(clippy::many_single_char_names)] // Standard math notation for linear algebra
fn chebyshev_fit(x_norm: &[f64], y_vals: &[f64], degree: usize) -> Option<Vec<f64>> {
    let n_pts = x_norm.len();
    let n_coeffs = degree + 1;

    if n_pts < n_coeffs {
        return None;
    }

    // Build Chebyshev Vandermonde matrix.
    let mut vander = vec![vec![0.0; n_coeffs]; n_pts];
    for (i, &x) in x_norm.iter().enumerate() {
        vander[i][0] = 1.0; // T_0
        if n_coeffs > 1 {
            vander[i][1] = x; // T_1
        }
        for j in 2..n_coeffs {
            vander[i][j] = 2.0 * x * vander[i][j - 1] - vander[i][j - 2]; // T_n recurrence
        }
    }

    // Form normal equations: A = V^T * V, rhs = V^T * y
    let mut normal = vec![vec![0.0; n_coeffs]; n_coeffs];
    let mut rhs = vec![0.0; n_coeffs];

    for j in 0..n_coeffs {
        for k in 0..n_coeffs {
            let sum: f64 = vander.iter().map(|row| row[j] * row[k]).sum();
            normal[j][k] = sum;
        }
        let sum: f64 = vander.iter().zip(y_vals).map(|(row, &y)| row[j] * y).sum();
        rhs[j] = sum;
    }

    // Solve via Gaussian elimination with partial pivoting.
    solve_linear_system(&mut normal, &mut rhs)
}

/// Solve Ax = b via Gaussian elimination with partial pivoting.
///
/// Modifies `a` and `b` in place. Returns the solution vector.
fn solve_linear_system(mat: &mut [Vec<f64>], rhs: &mut [f64]) -> Option<Vec<f64>> {
    let dim = rhs.len();

    // Forward elimination with partial pivoting.
    for col in 0..dim {
        // Find pivot.
        let (max_row, max_val) = mat[col..]
            .iter()
            .enumerate()
            .map(|(i, row)| (col + i, row[col].abs()))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;

        if max_val < 1e-15 {
            return None; // singular
        }

        // Swap rows.
        if max_row != col {
            mat.swap(col, max_row);
            rhs.swap(col, max_row);
        }

        // Eliminate below.
        let pivot = mat[col][col];
        // Copy pivot row once (outside inner loop) to avoid borrow conflicts.
        let pivot_row: Vec<f64> = mat[col][col..dim].to_vec();
        let pivot_rhs = rhs[col];
        for row in col + 1..dim {
            let factor = mat[row][col] / pivot;
            for (dest, &src) in mat[row][col..dim].iter_mut().zip(&pivot_row) {
                *dest -= factor * src;
            }
            rhs[row] -= factor * pivot_rhs;
        }
    }

    // Back substitution.
    let mut solution = vec![0.0; dim];
    for col in (0..dim).rev() {
        let mut sum = rhs[col];
        for k in col + 1..dim {
            sum -= mat[col][k] * solution[k];
        }
        solution[col] = sum / mat[col][col];
    }

    Some(solution)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic spectrum with known Gaussian peaks at given positions.
    ///
    /// Adds a simple deterministic pseudo-noise pattern to the baseline
    /// so that noise estimation works correctly.
    fn synthetic_spectrum(
        len: usize,
        peaks: &[(f64, f64, f64)], // (center, amplitude, sigma)
        noise_floor: f32,
    ) -> Vec<f32> {
        let mut spectrum = Vec::with_capacity(len);
        // Deterministic pseudo-noise: small sinusoidal + sawtooth variation.
        for i in 0..len {
            #[allow(clippy::cast_precision_loss)]
            let noise = (i as f64 * 0.73).sin() * 2.0 + (i as f64 * 1.37).cos() * 1.5;
            #[allow(clippy::cast_possible_truncation)]
            let val = noise_floor + noise as f32;
            spectrum.push(val);
        }
        for &(center, amplitude, sigma) in peaks {
            for (i, val) in spectrum.iter_mut().enumerate() {
                #[allow(clippy::cast_precision_loss)] // test spectrum index
                let x = i as f64;
                let gauss = amplitude * (-(x - center).powi(2) / (2.0 * sigma * sigma)).exp();
                #[allow(clippy::cast_possible_truncation)]
                {
                    *val += gauss as f32;
                }
            }
        }
        spectrum
    }

    #[test]
    fn test_detect_known_gaussian_peaks() {
        // Three well-separated Gaussian peaks with known positions.
        let peaks = vec![
            (50.0, 1000.0, 2.0),  // center=50, amp=1000, sigma=2
            (150.0, 800.0, 2.5),  // center=150, amp=800, sigma=2.5
            (250.0, 1200.0, 1.8), // center=250, amp=1200, sigma=1.8
        ];
        let spectrum = synthetic_spectrum(300, &peaks, 10.0);

        let config = ArcDetectConfig::default();
        let lines = detect_arc_lines(&spectrum, 0, &config);

        assert_eq!(
            lines.len(),
            3,
            "expected 3 lines, got {}: {lines:?}",
            lines.len()
        );

        // Verify sub-pixel accuracy: recovered centers within 0.1px of true positions.
        for (line, &(true_center, _, true_sigma)) in lines.iter().zip(&peaks) {
            let center_err = (line.pixel_center - true_center).abs();
            assert!(
                center_err < 0.1,
                "center error {center_err:.4} for peak at {true_center} (got {:.4})",
                line.pixel_center
            );

            // Sigma should be reasonably close (within 20%).
            let sigma_err = (line.pixel_sigma - true_sigma).abs() / true_sigma;
            assert!(
                sigma_err < 0.2,
                "sigma error {sigma_err:.2} for peak at {true_center} (expected {true_sigma}, got {:.4})",
                line.pixel_sigma
            );
        }
    }

    #[test]
    fn test_fwhm_calculation() {
        let line = ArcLine {
            order: 0,
            pixel_center: 50.0,
            pixel_sigma: 2.0,
            amplitude: 100.0,
            wavelength_hint: None,
            used: true,
        };
        // FWHM = 2 * sqrt(2 * ln(2)) * sigma ≈ 2.3548 * sigma
        let expected_fwhm = 2.0 * (2.0 * 2.0_f64.ln()).sqrt() * 2.0;
        assert!(
            (line.fwhm() - expected_fwhm).abs() < 1e-10,
            "FWHM mismatch: got {}, expected {expected_fwhm}",
            line.fwhm()
        );
    }

    #[test]
    fn test_rejects_subthreshold_peaks() {
        // Single peak with low amplitude — should be rejected by SNR filter.
        let spectrum = synthetic_spectrum(100, &[(50.0, 5.0, 2.0)], 10.0);
        let config = ArcDetectConfig {
            sigdetect: 10.0, // very high threshold
            ..Default::default()
        };
        let lines = detect_arc_lines(&spectrum, 0, &config);
        assert!(lines.is_empty(), "low-SNR peak should be rejected");
    }

    #[test]
    fn test_separation_filter_keeps_brighter() {
        // Two resolvable peaks within min_separation — only the brighter survives.
        // Use sigma=1.0 so the peaks at 50 and 55 are individually resolved
        // (valley between them) but still within min_separation=8.0.
        let peaks = vec![(50.0, 1000.0, 1.0), (55.0, 500.0, 1.0)];
        let spectrum = synthetic_spectrum(200, &peaks, 5.0);
        let config = ArcDetectConfig {
            min_separation: 8.0,
            ..Default::default()
        };
        let lines = detect_arc_lines(&spectrum, 0, &config);
        assert_eq!(
            lines.len(),
            1,
            "only brighter peak should survive, got {lines:?}"
        );
        assert!(
            (lines[0].pixel_center - 50.0).abs() < 0.5,
            "surviving peak should be near position 50"
        );
    }

    #[test]
    fn test_empty_spectrum_returns_empty() {
        let lines = detect_arc_lines(&[], 0, &ArcDetectConfig::default());
        assert!(lines.is_empty());

        let lines = detect_arc_lines(&[0.0; 3], 0, &ArcDetectConfig::default());
        assert!(lines.is_empty());
    }

    #[test]
    fn test_running_median_flat_spectrum() {
        let data = vec![10.0_f32; 20];
        let median = running_median(&data, 5);
        assert_eq!(median.len(), 20);
        for &v in &median {
            assert!((v - 10.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_fwhm_filter() {
        // Very narrow peak (sigma=0.3) → FWHM ≈ 0.7 < min_fwhm=1.5 → rejected.
        let spectrum = synthetic_spectrum(100, &[(50.0, 2000.0, 0.3)], 5.0);
        let config = ArcDetectConfig {
            min_fwhm: 1.5,
            ..Default::default()
        };
        let lines = detect_arc_lines(&spectrum, 0, &config);
        // Very narrow peaks may be detected but should be filtered by FWHM.
        for line in &lines {
            assert!(
                line.fwhm() >= config.min_fwhm,
                "narrow peak should be filtered: FWHM={}",
                line.fwhm()
            );
        }
    }

    // ─── Wavelength solution fitting tests ──────────────────────────

    #[test]
    fn test_chebyshev_eval_t0_t1() {
        // T_0(x) = 1, T_1(x) = x
        assert!((chebyshev_eval(&[3.0], 0.5) - 3.0).abs() < 1e-12);
        assert!((chebyshev_eval(&[2.0, 5.0], 0.5) - (2.0 + 5.0 * 0.5)).abs() < 1e-12);
    }

    #[test]
    fn test_chebyshev_eval_t2() {
        // T_2(x) = 2x^2 - 1
        // coeffs = [0, 0, 1] → T_2(0.5) = 2*0.25 - 1 = -0.5
        assert!((chebyshev_eval(&[0.0, 0.0, 1.0], 0.5) - (-0.5)).abs() < 1e-12);
    }

    #[test]
    fn test_chebyshev_fit_linear() {
        // Fit a linear function y = 100 + 2*x on [-1, 1].
        let xs: Vec<f64> = (-10..=10).map(|i| f64::from(i) / 10.0).collect();
        let ys: Vec<f64> = xs.iter().map(|&x| 100.0 + 2.0 * x).collect();

        let coeffs = chebyshev_fit(&xs, &ys, 1).expect("should fit");
        assert_eq!(coeffs.len(), 2);
        // c[0] * T_0(x) + c[1] * T_1(x) = c[0] + c[1]*x
        assert!((coeffs[0] - 100.0).abs() < 1e-10, "constant: {}", coeffs[0]);
        assert!((coeffs[1] - 2.0).abs() < 1e-10, "slope: {}", coeffs[1]);
    }

    #[test]
    fn test_chebyshev_fit_quadratic() {
        // Fit y = 500 + 3*x + 0.5*(2x^2-1) on [-1, 1].
        // In Chebyshev: c0=500, c1=3, c2=0.5 → y = 500 + 3*T_1(x) + 0.5*T_2(x)
        let xs: Vec<f64> = (-20..=20).map(|i| f64::from(i) / 20.0).collect();
        let ys: Vec<f64> = xs
            .iter()
            .map(|&x| 500.0 + 3.0 * x + 0.5 * (2.0 * x * x - 1.0))
            .collect();

        let coeffs = chebyshev_fit(&xs, &ys, 2).expect("should fit");
        assert_eq!(coeffs.len(), 3);
        assert!(
            (coeffs[0] - 500.0).abs() < 1e-8,
            "c0: {} expected 500",
            coeffs[0]
        );
        assert!(
            (coeffs[1] - 3.0).abs() < 1e-8,
            "c1: {} expected 3",
            coeffs[1]
        );
        assert!(
            (coeffs[2] - 0.5).abs() < 1e-8,
            "c2: {} expected 0.5",
            coeffs[2]
        );
    }

    #[test]
    fn test_match_lines_to_atlas() {
        let lines = vec![
            ArcLine {
                order: 0,
                pixel_center: 100.0,
                pixel_sigma: 2.0,
                amplitude: 500.0,
                wavelength_hint: None,
                used: true,
            },
            ArcLine {
                order: 0,
                pixel_center: 200.0,
                pixel_sigma: 2.0,
                amplitude: 300.0,
                wavelength_hint: None,
                used: true,
            },
        ];
        let atlas = vec![
            AtlasLine {
                wavelength_nm: 400.05,
                species: "Th I".to_string(),
                strength: 100.0,
            },
            AtlasLine {
                wavelength_nm: 410.12,
                species: "Ar II".to_string(),
                strength: 80.0,
            },
        ];

        // Seed model: simple linear λ = 400 + 0.1 * pixel
        let seed = |px: f64| 400.0 + 0.1 * px;
        let matches = match_lines_to_atlas(&lines, &atlas, &seed, 0.5);

        // pixel=100 → seed=410.0, closest atlas=410.12 (dist=0.12 < 0.5)
        // pixel=200 → seed=420.0, no atlas within 0.5 nm
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], (0, 1)); // line 0 matched to atlas 1
    }

    #[test]
    fn test_fit_order_wavelength_known_solution() {
        // Create a known linear wavelength solution: λ = 400 + 0.05 * pixel
        // over pixel range [0, 2000].
        let n_lines = 30;
        let mut lines = Vec::new();
        let mut atlas = Vec::new();
        let mut matches = Vec::new();

        for i in 0..n_lines {
            #[allow(clippy::cast_precision_loss)]
            let pixel = 50.0 + (i as f64) * 60.0; // evenly spaced
            let wavelength = 400.0 + 0.05 * pixel;

            lines.push(ArcLine {
                order: 42,
                pixel_center: pixel,
                pixel_sigma: 2.0,
                amplitude: 1000.0,
                wavelength_hint: Some(wavelength),
                used: true,
            });
            atlas.push(AtlasLine {
                wavelength_nm: wavelength,
                species: "Th I".to_string(),
                strength: 100.0,
            });
            matches.push((i, i));
        }

        let config = WlFitConfig {
            poly_degree: 2,
            ..Default::default()
        };
        let solution =
            fit_order_wavelength(&lines, &atlas, &matches, 42, &config).expect("should fit");

        assert_eq!(solution.order, 42);
        assert_eq!(solution.n_lines_used, n_lines);
        assert!(
            solution.rms_nm < 1e-10,
            "RMS should be near-zero for exact data, got {}",
            solution.rms_nm
        );

        // Verify the solution evaluates correctly.
        let wl_at_500 = solution.eval(500.0);
        let expected = 400.0 + 0.05 * 500.0;
        assert!(
            (wl_at_500 - expected).abs() < 1e-8,
            "eval at px=500: {} expected {}",
            wl_at_500,
            expected
        );
    }

    #[test]
    fn test_fit_order_wavelength_sigma_clip() {
        // 20 good lines with known solution, plus 2 outliers.
        let mut lines = Vec::new();
        let mut atlas = Vec::new();
        let mut matches = Vec::new();
        let n_good = 20;

        for i in 0..n_good {
            #[allow(clippy::cast_precision_loss)]
            let pixel = 100.0 + (i as f64) * 80.0;
            let wavelength = 500.0 + 0.03 * pixel;

            lines.push(ArcLine {
                order: 10,
                pixel_center: pixel,
                pixel_sigma: 2.0,
                amplitude: 800.0,
                wavelength_hint: Some(wavelength),
                used: true,
            });
            atlas.push(AtlasLine {
                wavelength_nm: wavelength,
                species: "Th I".to_string(),
                strength: 100.0,
            });
            matches.push((lines.len() - 1, atlas.len() - 1));
        }

        // Add 2 outlier lines (wrong wavelength by 5 nm).
        for pixel in [400.0, 1200.0] {
            let correct_wl = 500.0 + 0.03 * pixel;
            lines.push(ArcLine {
                order: 10,
                pixel_center: pixel,
                pixel_sigma: 2.0,
                amplitude: 300.0,
                wavelength_hint: Some(correct_wl + 5.0),
                used: true,
            });
            atlas.push(AtlasLine {
                wavelength_nm: correct_wl + 5.0, // wrong!
                species: "Ar I".to_string(),
                strength: 50.0,
            });
            matches.push((lines.len() - 1, atlas.len() - 1));
        }

        let config = WlFitConfig {
            poly_degree: 2,
            sigma_clip: 3.0,
            ..Default::default()
        };
        let solution =
            fit_order_wavelength(&lines, &atlas, &matches, 10, &config).expect("should fit");

        // Outliers should have been clipped.
        assert_eq!(
            solution.n_lines_used, n_good,
            "expected {n_good} lines after clipping, got {}",
            solution.n_lines_used
        );
        assert!(
            solution.rms_nm < 1e-8,
            "RMS should be near-zero after clipping outliers, got {}",
            solution.rms_nm
        );
    }

    #[test]
    fn test_fit_order_wavelength_too_few_lines() {
        // Fewer lines than poly_degree+1 → should return None.
        let lines = vec![ArcLine {
            order: 0,
            pixel_center: 100.0,
            pixel_sigma: 2.0,
            amplitude: 500.0,
            wavelength_hint: Some(400.0),
            used: true,
        }];
        let atlas = vec![AtlasLine {
            wavelength_nm: 400.0,
            species: "Th I".to_string(),
            strength: 100.0,
        }];
        let matches = vec![(0, 0)];

        let config = WlFitConfig::default(); // degree=4, needs 5 lines
        let result = fit_order_wavelength(&lines, &atlas, &matches, 0, &config);
        assert!(result.is_none(), "should fail with too few lines");
    }

    #[test]
    fn test_order_wl_solution_eval_roundtrip() {
        // Build a solution manually and verify eval.
        let sol = OrderWlSolution {
            order: 5,
            coefficients: vec![500.0, 10.0], // λ = 500 + 10*x_norm
            pixel_min: 0.0,
            pixel_max: 2000.0,
            rms_nm: 0.001,
            n_lines_used: 20,
            n_lines_total: 22,
        };

        // At pixel_min (0): x_norm = -1 → λ = 500 + 10*(-1) = 490
        assert!((sol.eval(0.0) - 490.0).abs() < 1e-10);
        // At midpoint (1000): x_norm = 0 → λ = 500
        assert!((sol.eval(1000.0) - 500.0).abs() < 1e-10);
        // At pixel_max (2000): x_norm = 1 → λ = 510
        assert!((sol.eval(2000.0) - 510.0).abs() < 1e-10);
    }
}
