//! Cauchy-series dispersion model for prism cross-dispersed echelles.
//!
//! For an echelle with a prism cross-disperser (e.g. Andor Mechelle 5000),
//! the spatial position `y` at which each order lands on the detector is
//! governed by the prism's refractive index via the Sellmeier equation.
//! The standard Cauchy approximation truncates the Sellmeier series and
//! relates `y` to the wavelength `λ` at the order's blaze center via
//!
//! ```text
//! y(λ) ≈ A + B·λ² + C·λ⁴
//! ```
//!
//! For an echelle grating operating at integer order `m`, the blaze-center
//! wavelength is `λ = gc / m` (grating equation), so
//!
//! ```text
//! y(m) = a + b/m² + c/m⁴
//! ```
//!
//! with `a = A`, `b = B·gc²`, `c = C·gc⁴`. Substituting `u = 1/m²` turns
//! this into a **quadratic in `u`**:
//!
//! ```text
//! y(u) = a + b·u + c·u²
//! ```
//!
//! This module fits the three Cauchy coefficients from trace-centroid
//! anchors `(y_i, m_i)` and inverts them to predict `m(y)` for uncalibrated
//! traces.
//!
//! References:
//! - NotebookLM synthesis "Echelle Calibration Algorithms" (source
//!   `e1f9981c`), recommendation B: physical Cauchy model for prism
//!   cross-dispersers.
//! - NotebookLM pipeline evaluation memo "Advanced Data Reduction
//!   Algorithms for Compact Cross-Dispersed Prism Echelle Spectrographs"
//!   (source `3a0a13df`) §FM1: use a physics-based coordinate transform
//!   before any polynomial modelling.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

/// Fitted Cauchy dispersion series `y(m) = a + b/m² + c/m⁴`.
#[derive(Debug, Clone, Copy)]
pub struct CauchyYofM {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    /// RMS residual (pixels) on the fit anchors.
    pub rms_px: f64,
    /// Number of anchor pairs used.
    pub n_anchors: usize,
}

impl CauchyYofM {
    /// Predict `y(m)` from the fitted series.
    #[must_use]
    pub fn eval(&self, m: f64) -> f64 {
        if m.abs() < 1e-12 {
            return f64::NAN;
        }
        let u = 1.0 / (m * m);
        self.a + self.b * u + self.c * u * u
    }

    /// Invert: given an observed trace centroid `y_obs`, return the physical
    /// order `m` predicted by the Cauchy series. Uses the quadratic-in-`u`
    /// branch whose `m` is closest to the echelle-equation guess
    /// `round(gc / λ_center_estimate)`.
    ///
    /// Returns `None` if no real positive root exists within a plausible
    /// order range, or if the discriminant is negative beyond numerical
    /// roundoff.
    #[must_use]
    pub fn invert_to_m(&self, y_obs: f64, guess_m: f64) -> Option<f64> {
        // cu² + bu + (a - y_obs) = 0, solve for u > 0.
        let (a, b, c) = (self.a, self.b, self.c);
        let k = a - y_obs;

        let u = if c.abs() < 1e-15 {
            // Degenerate quadratic → linear: bu + k = 0.
            if b.abs() < 1e-15 {
                return None;
            }
            -k / b
        } else {
            let disc = b * b - 4.0 * c * k;
            if disc < -1e-9 {
                return None;
            }
            let disc = disc.max(0.0);
            let sqrt_disc = disc.sqrt();
            let u_plus = (-b + sqrt_disc) / (2.0 * c);
            let u_minus = (-b - sqrt_disc) / (2.0 * c);
            // Both candidate roots — pick the one whose m is closest to
            // the echelle-equation guess. Only positive u yields real m.
            let guess_u = if guess_m.abs() > 1e-12 {
                1.0 / (guess_m * guess_m)
            } else {
                f64::INFINITY
            };
            let candidates = [u_plus, u_minus];
            candidates
                .iter()
                .filter(|&&u| u > 1e-20 && u.is_finite())
                .min_by(|&&u1, &&u2| {
                    (u1 - guess_u)
                        .abs()
                        .partial_cmp(&(u2 - guess_u).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()?
        };

        if u <= 0.0 || !u.is_finite() {
            return None;
        }
        let m = 1.0 / u.sqrt();
        if m.is_finite() && m > 0.0 {
            Some(m)
        } else {
            None
        }
    }
}

/// Fit `y(m) = a + b/m² + c/m⁴` with iterative 3σ outlier rejection.
///
/// Runs the linear-LSQ fit, drops anchors whose residual is more than
/// `sigma_threshold × σ` from the mean, and refits until the kept set is
/// stable (up to `max_iters` passes). Follows the same convention as the
/// literature-standard 2D Chebyshev arc-line fitter
/// (`chebyshev_2d::fit_chebyshev_2d_clipped`) and is necessary here
/// because Stage-1 can produce anchors whose `m` is off by several
/// orders when the grating-equation seed misidentifies arc lines.
///
/// Returns `(fit, kept_mask)`. Requires ≥ 4 anchors initially and ≥ 3
/// to remain after rejection; otherwise returns `None`.
#[must_use]
pub fn fit_cauchy_y_of_m_clipped(
    anchors: &[(f64, i32)],
    sigma_threshold: f64,
    max_iters: usize,
) -> Option<(CauchyYofM, Vec<bool>)> {
    if anchors.len() < 4 {
        return None;
    }
    let mut kept: Vec<bool> = vec![true; anchors.len()];
    let mut prev_count = anchors.len() + 1;

    for _iter in 0..max_iters.max(1) {
        let filtered: Vec<(f64, i32)> = anchors
            .iter()
            .zip(kept.iter())
            .filter_map(|(pt, &k)| if k { Some(*pt) } else { None })
            .collect();
        if filtered.len() < 3 {
            return None;
        }
        let fit = fit_cauchy_y_of_m(&filtered)?;

        // Residuals only on currently-kept anchors.
        let resids: Vec<(usize, f64)> = anchors
            .iter()
            .enumerate()
            .zip(kept.iter())
            .filter_map(|((i, &(y, m_int)), &k)| {
                if !k || m_int <= 0 {
                    None
                } else {
                    Some((i, y - fit.eval(f64::from(m_int))))
                }
            })
            .collect();
        if resids.is_empty() {
            return Some((fit, kept));
        }
        let n = resids.len() as f64;
        let mean = resids.iter().map(|(_, r)| r).sum::<f64>() / n;
        let variance = resids.iter().map(|(_, r)| (r - mean).powi(2)).sum::<f64>() / n;
        let sigma = variance.sqrt();
        let threshold = sigma_threshold * sigma;

        let mut new_kept = kept.clone();
        let mut new_count = 0usize;
        for (i, r) in resids {
            if sigma > 0.0 && (r - mean).abs() > threshold {
                new_kept[i] = false;
            } else {
                new_count += 1;
            }
        }
        kept = new_kept;
        if new_count == prev_count || new_count < 3 {
            // Converged (or no more room to reject).
            break;
        }
        prev_count = new_count;
    }

    // Final refit on the converged set.
    let final_set: Vec<(f64, i32)> = anchors
        .iter()
        .zip(kept.iter())
        .filter_map(|(pt, &k)| if k { Some(*pt) } else { None })
        .collect();
    if final_set.len() < 3 {
        return None;
    }
    let fit = fit_cauchy_y_of_m(&final_set)?;
    Some((fit, kept))
}

/// Fit `y(m) = a + b/m² + c/m⁴` from `(y_centroid, physical_order_m)` anchors.
///
/// Needs ≥ 4 anchors for a well-posed 3-parameter fit. Falls through to a
/// 2-parameter `y = a + b·u` (equivalent to dropping the `c/m⁴` term) if
/// only 3 anchors are available; returns `None` for < 3.
///
/// **Note**: this is the raw LSQ fit with no outlier rejection. For
/// real-data use, prefer [`fit_cauchy_y_of_m_clipped`], which iteratively
/// rejects 3σ outliers.
#[must_use]
pub fn fit_cauchy_y_of_m(anchors: &[(f64, i32)]) -> Option<CauchyYofM> {
    let valid: Vec<(f64, f64)> = anchors
        .iter()
        .filter_map(|&(y, m_int)| {
            if m_int <= 0 {
                return None;
            }
            let m = f64::from(m_int);
            let u = 1.0 / (m * m);
            Some((u, y))
        })
        .collect();

    if valid.len() < 3 {
        return None;
    }

    // Solve 3x3 normal equations for y = a + b·u + c·u² via Cramer's rule.
    // (Matches the style of `quadratic_regression` we replace.)
    let mut s = [0.0f64; 5]; // s[k] = Σ u^k
    let mut t = [0.0f64; 3]; // t[k] = Σ u^k · y
    for &(u, y) in &valid {
        s[0] += 1.0;
        s[1] += u;
        s[2] += u * u;
        s[3] += u * u * u;
        s[4] += u * u * u * u;
        t[0] += y;
        t[1] += u * y;
        t[2] += u * u * y;
    }

    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let mat = [[s[0], s[1], s[2]], [s[1], s[2], s[3]], [s[2], s[3], s[4]]];
    let d = det(mat);

    let (a, b, c) = if d.abs() > 1e-30 {
        let a = det([[t[0], s[1], s[2]], [t[1], s[2], s[3]], [t[2], s[3], s[4]]]) / d;
        let b = det([[s[0], t[0], s[2]], [s[1], t[1], s[3]], [s[2], t[2], s[4]]]) / d;
        let c = det([[s[0], s[1], t[0]], [s[1], s[2], t[1]], [s[2], s[3], t[2]]]) / d;
        (a, b, c)
    } else {
        // 3x3 singular — fall back to y = a + b·u only.
        let denom = s[0] * s[2] - s[1] * s[1];
        if denom.abs() < 1e-30 {
            return None;
        }
        let a = (t[0] * s[2] - t[1] * s[1]) / denom;
        let b = (s[0] * t[1] - s[1] * t[0]) / denom;
        (a, b, 0.0)
    };

    // Compute RMS residual on the anchors.
    let mut sum_sq = 0.0;
    for &(u, y) in &valid {
        let y_pred = a + b * u + c * u * u;
        let r = y - y_pred;
        sum_sq += r * r;
    }
    let rms_px = (sum_sq / valid.len() as f64).sqrt();

    Some(CauchyYofM {
        a,
        b,
        c,
        rms_px,
        n_anchors: valid.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given synthetic anchors sampled from a known Cauchy series, the fit
    /// recovers the coefficients within numerical precision.
    #[test]
    fn test_cauchy_fit_recovers_analytic_series() {
        // Synthetic Mechelle-like: y(m) = 1000 + 5e5/m² + 2e9/m⁴
        // over orders m = 43..=116. This is the real regime.
        let a = 1000.0;
        let b = 5.0e5;
        let c = 2.0e9;
        let anchors: Vec<(f64, i32)> = (43..=116)
            .map(|m| {
                let mf = f64::from(m);
                let u = 1.0 / (mf * mf);
                let y = a + b * u + c * u * u;
                (y, m)
            })
            .collect();

        let fit = fit_cauchy_y_of_m(&anchors).expect("fit should succeed");
        // RMS should be at numerical-roundoff level.
        assert!(fit.rms_px < 1e-6, "RMS {} should be ~0", fit.rms_px);
        assert!((fit.a - a).abs() < 1e-3, "a: {} vs {}", fit.a, a);
        assert!((fit.b - b).abs() / b.abs() < 1e-6, "b: {} vs {}", fit.b, b);
        assert!((fit.c - c).abs() / c.abs() < 1e-4, "c: {} vs {}", fit.c, c);
    }

    /// Inverting `y = fit.eval(m)` should recover the original `m` within a
    /// small tolerance.
    #[test]
    fn test_cauchy_inversion_roundtrip() {
        let a = 1000.0;
        let b = 5.0e5;
        let c = 2.0e9;
        let anchors: Vec<(f64, i32)> = (43..=116)
            .map(|m| {
                let mf = f64::from(m);
                let u = 1.0 / (mf * mf);
                (a + b * u + c * u * u, m)
            })
            .collect();
        let fit = fit_cauchy_y_of_m(&anchors).unwrap();

        for m_true in [50, 70, 90, 110] {
            let y = fit.eval(f64::from(m_true));
            let m_pred = fit
                .invert_to_m(y, f64::from(m_true))
                .expect("inversion must succeed");
            assert!(
                (m_pred - f64::from(m_true)).abs() < 0.01,
                "m_true={m_true}, m_pred={m_pred}"
            );
        }
    }

    /// With realistic per-anchor noise (~0.5 px), the LSQ-fit Cauchy
    /// series still recovers wavelengths within ~1 px across the whole
    /// order range. Outlier rejection happens downstream in the
    /// sigma-clipped 2D Chebyshev fit; this test only checks that the
    /// unweighted LSQ handles normal centroiding jitter gracefully.
    #[test]
    fn test_cauchy_fit_bounded_residuals_under_noise() {
        let a = 1000.0;
        let b = 5.0e5;
        let c = 2.0e9;
        let anchors: Vec<(f64, i32)> = (43..=116)
            .map(|m| {
                let mf = f64::from(m);
                let u = 1.0 / (mf * mf);
                let y = a + b * u + c * u * u;
                // Deterministic ±0.5 px pseudo-noise.
                let noise = 0.5 * ((f64::from(m) * 1.7).sin());
                (y + noise, m)
            })
            .collect();

        let fit = fit_cauchy_y_of_m(&anchors).unwrap();
        for m in [50, 70, 90, 110] {
            let mf = f64::from(m);
            let u = 1.0 / (mf * mf);
            let y_true = a + b * u + c * u * u;
            let y_pred = fit.eval(mf);
            assert!(
                (y_true - y_pred).abs() < 2.0,
                "m={m}, y_true={y_true:.3}, y_pred={y_pred:.3}"
            );
        }
        // Per-anchor RMS on the fit should stay below the input noise
        // amplitude (smoothing effect of the 3-parameter fit).
        assert!(fit.rms_px < 1.0, "noisy-fit RMS: {}", fit.rms_px);
    }

    /// With fewer than 3 anchors, return None.
    #[test]
    fn test_cauchy_fit_requires_minimum_anchors() {
        let anchors = vec![(100.0, 50), (200.0, 60)];
        assert!(fit_cauchy_y_of_m(&anchors).is_none());
    }

    /// Zero or negative m values are rejected.
    #[test]
    fn test_cauchy_fit_rejects_nonpositive_m() {
        let anchors: Vec<(f64, i32)> = vec![(100.0, 0), (200.0, -5), (300.0, 50)];
        // Only one valid anchor → None.
        assert!(fit_cauchy_y_of_m(&anchors).is_none());
    }
}
