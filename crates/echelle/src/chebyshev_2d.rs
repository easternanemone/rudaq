//! Global 2D Chebyshev wavelength model for echelle spectrographs.
//!
//! Fits the echelle product `m * lambda` as a tensor-product Chebyshev surface
//! over `(x_pixel, m_order)`, so that `lambda(x, m) = surface(x, m) / m`.
//!
//! This replaces per-order polynomial fits with a single global model that
//! enforces smoothness across order boundaries and produces a physically
//! meaningful RMS (residual against NIST atlas wavelengths, not circular
//! predicted-vs-stored).

// Numerical code: pixel-index casts are always lossless for realistic frame sizes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

/// Evaluate Chebyshev polynomial T_n(x) for a single degree.
#[must_use]
pub fn chebyshev_t(n: usize, x: f64) -> f64 {
    match n {
        0 => 1.0,
        1 => x,
        _ => {
            let mut t_prev = 1.0;
            let mut t_curr = x;
            for _ in 2..=n {
                let t_next = 2.0 * x * t_curr - t_prev;
                t_prev = t_curr;
                t_curr = t_next;
            }
            t_curr
        }
    }
}

/// Result of fitting a global 2D Chebyshev surface for `m * lambda`.
#[derive(Debug, Clone)]
pub struct Global2DChebyshevFit {
    /// Flattened coefficient matrix: `coeffs[i * (dm+1) + j]` for `T_i(x_norm) * T_j(m_norm)`.
    pub coefficients: Vec<f64>,
    /// Maximum degree in x (pixel) direction.
    pub degree_x: usize,
    /// Maximum degree in m (order) direction.
    pub degree_m: usize,
    /// Pixel range for normalization: `x_norm = 2 * (x - 0) / (x_max - 0) - 1 = 2*x/x_max - 1`.
    /// Stored as `[0, x_max]` where `x_max` is typically 2559.
    pub x_max: f64,
    /// Order range for normalization: `m_norm = 2 * (m - m_min) / (m_max - m_min) - 1`.
    pub m_min: f64,
    pub m_max: f64,
    /// Global RMS of residuals `|lambda_atlas - lambda_predicted|` in nm.
    pub rms_nm: f64,
    /// Number of training points used.
    pub n_points: usize,
}

impl Global2DChebyshevFit {
    /// Evaluate `lambda(x, m)` at a given pixel and physical order number.
    ///
    /// Returns `lambda = surface(x, m) / m` where `surface` fits `m * lambda`.
    #[must_use]
    pub fn eval_lambda(&self, x: f64, m: f64) -> f64 {
        if m.abs() < 1e-15 {
            return 0.0;
        }
        let ml = self.eval_ml(x, m);
        ml / m
    }

    /// Evaluate the raw `m * lambda` surface at `(x, m)`.
    #[must_use]
    fn eval_ml(&self, x: f64, m: f64) -> f64 {
        let x_norm = self.normalize_x(x);
        let m_norm = self.normalize_m(m);
        eval_chebyshev_2d_surface(
            &self.coefficients,
            self.degree_x,
            self.degree_m,
            x_norm,
            m_norm,
        )
    }

    fn normalize_x(&self, x: f64) -> f64 {
        if self.x_max.abs() < 1e-15 {
            return 0.0;
        }
        2.0 * x / self.x_max - 1.0
    }

    fn normalize_m(&self, m: f64) -> f64 {
        let range = self.m_max - self.m_min;
        if range.abs() < 1e-15 {
            return 0.0;
        }
        2.0 * (m - self.m_min) / range - 1.0
    }
}

/// Evaluate a 2D Chebyshev tensor-product surface at normalized coordinates.
fn eval_chebyshev_2d_surface(
    coeffs: &[f64],
    dx: usize,
    dm: usize,
    x_norm: f64,
    m_norm: f64,
) -> f64 {
    let nx = dx + 1;
    let nm = dm + 1;

    // Precompute Chebyshev basis values via recurrence.
    let tx = chebyshev_basis_vec(x_norm, nx);
    let tm = chebyshev_basis_vec(m_norm, nm);

    let mut result = 0.0;
    for i in 0..nx {
        for j in 0..nm {
            result += coeffs[i * nm + j] * tx[i] * tm[j];
        }
    }
    result
}

/// Compute Chebyshev basis values `[T_0(x), T_1(x), ..., T_{n-1}(x)]`.
fn chebyshev_basis_vec(x: f64, n: usize) -> Vec<f64> {
    let mut t = vec![0.0; n];
    if n == 0 {
        return t;
    }
    t[0] = 1.0;
    if n == 1 {
        return t;
    }
    t[1] = x;
    for k in 2..n {
        t[k] = 2.0 * x * t[k - 1] - t[k - 2];
    }
    t
}

/// Fit a global 2D Chebyshev surface to matched arc line data.
///
/// # Arguments
///
/// * `training_data` - Slice of `(x_pixel, physical_order_m, lambda_nm)` tuples
///   from atlas-matched arc lines across all orders.
/// * `dx` - Maximum Chebyshev degree in the pixel (x) direction. Typical: 4.
/// * `dm` - Maximum Chebyshev degree in the order (m) direction. Typical: 3.
///
/// # Mathematical formulation
///
/// Fits the echelle product rather than lambda directly:
///
/// ```text
/// m * lambda = sum_ij( c_ij * T_i(x_norm) * T_j(m_norm) )
/// ```
///
/// where `x_norm` maps `[0, x_max]` to `[-1, 1]` and `m_norm` maps
/// `[m_min, m_max]` to `[-1, 1]`.
///
/// Solves via normal equations with Gaussian elimination (partial pivoting).
///
/// # Returns
///
/// `None` if there are fewer data points than coefficients, or if the
/// normal equations are singular.
#[allow(clippy::many_single_char_names)]
pub fn fit_chebyshev_2d(
    training_data: &[(f64, u32, f64)], // (x_pixel, m_order, lambda_nm)
    dx: usize,
    dm: usize,
) -> Option<Global2DChebyshevFit> {
    let nx = dx + 1;
    let nm = dm + 1;
    let n_coeffs = nx * nm;
    let n_pts = training_data.len();

    if n_pts < n_coeffs {
        return None;
    }

    // Determine normalization ranges from the data.
    let x_max = training_data
        .iter()
        .map(|&(x, _, _)| x)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(1.0);
    let m_values: Vec<f64> = training_data
        .iter()
        .map(|&(_, m, _)| f64::from(m))
        .collect();
    let m_min = m_values.iter().copied().fold(f64::INFINITY, f64::min);
    let m_max = m_values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    if (m_max - m_min).abs() < 1e-15 {
        return None; // Need at least 2 distinct orders
    }

    // Build the Vandermonde matrix and target vector (y = m * lambda).
    let mut vandermonde = vec![0.0; n_pts * n_coeffs];
    let mut y_vals = vec![0.0; n_pts];

    for (row, &(x, m_u32, lambda)) in training_data.iter().enumerate() {
        let m = f64::from(m_u32);
        let x_norm = 2.0 * x / x_max - 1.0;
        let m_range = m_max - m_min;
        let m_norm = 2.0 * (m - m_min) / m_range - 1.0;

        let tx = chebyshev_basis_vec(x_norm, nx);
        let tm = chebyshev_basis_vec(m_norm, nm);

        for i in 0..nx {
            for j in 0..nm {
                vandermonde[row * n_coeffs + i * nm + j] = tx[i] * tm[j];
            }
        }
        y_vals[row] = m * lambda; // Fit m*lambda, not lambda
    }

    // Form normal equations: A = V^T * V, rhs = V^T * y
    let mut normal = vec![vec![0.0; n_coeffs]; n_coeffs];
    let mut rhs = vec![0.0; n_coeffs];

    for c1 in 0..n_coeffs {
        for c2 in c1..n_coeffs {
            let mut sum = 0.0;
            for row in 0..n_pts {
                sum += vandermonde[row * n_coeffs + c1] * vandermonde[row * n_coeffs + c2];
            }
            normal[c1][c2] = sum;
            normal[c2][c1] = sum; // Symmetric
        }
        let mut sum = 0.0;
        for row in 0..n_pts {
            sum += vandermonde[row * n_coeffs + c1] * y_vals[row];
        }
        rhs[c1] = sum;
    }

    // Solve via Gaussian elimination with partial pivoting.
    let coefficients = solve_linear_system(&mut normal, &mut rhs)?;

    // Compute global RMS in lambda (nm).
    let fit = Global2DChebyshevFit {
        coefficients,
        degree_x: dx,
        degree_m: dm,
        x_max,
        m_min,
        m_max,
        rms_nm: 0.0,
        n_points: n_pts,
    };

    let rms_nm = compute_global_rms(&fit, training_data);

    Some(Global2DChebyshevFit { rms_nm, ..fit })
}

/// Evaluate the global 2D Chebyshev model at `(x, m)` returning lambda in nm.
///
/// Convenience wrapper: `lambda = eval_chebyshev_2d(coeffs, x, m, ...) / m`.
#[must_use]
pub fn eval_chebyshev_2d(fit: &Global2DChebyshevFit, x: f64, m: u32) -> f64 {
    fit.eval_lambda(x, f64::from(m))
}

/// Fit a global 2D Chebyshev surface with iterative 3σ outlier rejection.
///
/// This is the literature-standard procedure used by IRAF ECIDENTIFY,
/// ESO MIDAS IDENTIFY/ECHELLE, CERES, and PypeIt. On each iteration, the
/// wavelength residual `|λ_atlas - λ_predicted|` is computed for every
/// training point, the sample standard deviation is taken, and points
/// whose residual exceeds `sigma_threshold` times that deviation are
/// dropped from the fit. Convergence is declared when the kept set is
/// unchanged from the previous iteration or `max_iters` is reached.
///
/// Returns `(fit, kept_mask)` where `kept_mask[i]` is `true` if the i-th
/// training point survived all rejection passes. Returns `None` if the
/// initial fit fails or fewer than `n_coeffs` points remain.
#[must_use]
pub fn fit_chebyshev_2d_clipped(
    training_data: &[(f64, u32, f64)],
    dx: usize,
    dm: usize,
    sigma_threshold: f64,
    max_iters: usize,
) -> Option<(Global2DChebyshevFit, Vec<bool>)> {
    let nx = dx + 1;
    let nm = dm + 1;
    let n_coeffs = nx * nm;
    let n_pts = training_data.len();

    if n_pts < n_coeffs {
        return None;
    }

    let mut kept: Vec<bool> = vec![true; n_pts];
    let mut prev_kept_count = n_pts + 1; // force at least one iteration
    let mut current_fit: Option<Global2DChebyshevFit> = None;

    for _iter in 0..max_iters.max(1) {
        let filtered: Vec<(f64, u32, f64)> = training_data
            .iter()
            .zip(kept.iter())
            .filter_map(|(&pt, &k)| if k { Some(pt) } else { None })
            .collect();

        if filtered.len() < n_coeffs {
            return None;
        }

        let fit = fit_chebyshev_2d(&filtered, dx, dm)?;

        // Residuals only on currently-kept points.
        let residuals: Vec<f64> = training_data
            .iter()
            .zip(kept.iter())
            .map(|(&(x, m, lam), &k)| {
                if !k {
                    0.0
                } else {
                    lam - fit.eval_lambda(x, f64::from(m))
                }
            })
            .collect();

        let (sum, cnt) = residuals
            .iter()
            .zip(kept.iter())
            .filter(|&(_, &k)| k)
            .fold((0.0, 0usize), |(s, c), (&r, _)| (s + r, c + 1));
        if cnt == 0 {
            return None;
        }
        let mean = sum / cnt as f64;
        let variance: f64 = residuals
            .iter()
            .zip(kept.iter())
            .filter(|&(_, &k)| k)
            .map(|(&r, _)| {
                let d = r - mean;
                d * d
            })
            .sum::<f64>()
            / cnt as f64;
        let sigma = variance.sqrt();
        let threshold = sigma_threshold * sigma;

        // Reject any currently-kept point whose residual magnitude exceeds threshold.
        let mut new_kept = kept.clone();
        let mut new_count = 0usize;
        for (i, (&r, &k)) in residuals.iter().zip(kept.iter()).enumerate() {
            if k {
                if (r - mean).abs() > threshold && sigma > 0.0 {
                    new_kept[i] = false;
                } else {
                    new_count += 1;
                }
            }
        }

        current_fit = Some(fit);
        kept = new_kept;

        if new_count == prev_kept_count {
            break;
        }
        prev_kept_count = new_count;

        if new_count < n_coeffs {
            // Next iteration would fail; stop with last good fit.
            break;
        }
    }

    current_fit.map(|f| (f, kept))
}

/// Compute the global RMS of residuals between atlas wavelengths and the
/// Chebyshev model predictions.
///
/// ```text
/// RMS = sqrt( (1/N) * sum( (lambda_atlas - lambda_2D(x, m))^2 ) )
/// ```
#[must_use]
pub fn compute_global_rms(
    fit: &Global2DChebyshevFit,
    data: &[(f64, u32, f64)], // (x_pixel, m_order, lambda_nm)
) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let sum_sq: f64 = data
        .iter()
        .map(|&(x, m, lambda_atlas)| {
            let lambda_pred = fit.eval_lambda(x, f64::from(m));
            let residual = lambda_atlas - lambda_pred;
            residual * residual
        })
        .sum();

    (sum_sq / data.len() as f64).sqrt()
}

/// Solve `A * x = b` via Gaussian elimination with partial pivoting.
///
/// Modifies `mat` and `rhs` in place. Returns the solution vector,
/// or `None` if the system is singular.
fn solve_linear_system(mat: &mut [Vec<f64>], rhs: &mut [f64]) -> Option<Vec<f64>> {
    let dim = rhs.len();

    // Forward elimination with partial pivoting.
    for col in 0..dim {
        // Find pivot row.
        let (max_row, max_val) = mat[col..]
            .iter()
            .enumerate()
            .map(|(i, row)| (col + i, row[col].abs()))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;

        if max_val < 1e-15 {
            return None; // Singular
        }

        // Swap rows.
        if max_row != col {
            mat.swap(col, max_row);
            rhs.swap(col, max_row);
        }

        // Eliminate below pivot.
        let pivot = mat[col][col];
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fitting known points on a simple echelle model should recover them.
    #[test]
    fn test_fit_recovers_known_points() {
        // Synthetic echelle: m * lambda = 50_000 (constant grating equation)
        // with a small quadratic pixel dependence.
        let gc = 50_000.0;
        let mut training = Vec::new();

        for m in [50u32, 60, 70, 80, 90, 100] {
            let mf = f64::from(m);
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / 2560.0;

            // Sample at several pixel positions
            for px_frac in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let x = px_frac * 2559.0;
                let lambda = lambda_center + dispersion * (x - 1280.0);
                training.push((x, m, lambda));
            }
        }

        let fit = fit_chebyshev_2d(&training, 4, 3).expect("fit should succeed");

        // Check that each training point is recovered within tolerance.
        // The 2D Chebyshev surface approximates the 1/m echelle relation;
        // with degree 4x3 and 30 points across 6 orders, residuals are ~0.02 nm.
        for &(x, m, lambda_true) in &training {
            let lambda_pred = eval_chebyshev_2d(&fit, x, m);
            let err = (lambda_true - lambda_pred).abs();
            assert!(
                err < 0.05,
                "point (x={x}, m={m}): true={lambda_true:.4}, pred={lambda_pred:.4}, err={err:.6}"
            );
        }

        // RMS should be very small on training data.
        assert!(
            fit.rms_nm < 0.05,
            "global RMS {:.6} should be < 0.05 nm",
            fit.rms_nm
        );
    }

    /// Wavelengths should be monotonic within each order (either all increasing
    /// or all decreasing, depending on the spectrograph geometry).
    #[test]
    fn test_monotonicity_per_order() {
        let gc = 50_000.0;
        let mut training = Vec::new();

        for m in [50u32, 60, 70, 80, 90, 100] {
            let mf = f64::from(m);
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / 2560.0;

            for px_i in 0..20 {
                let x = f64::from(px_i) * 128.0;
                let lambda = lambda_center + dispersion * (x - 1280.0);
                training.push((x, m, lambda));
            }
        }

        let fit = fit_chebyshev_2d(&training, 4, 3).expect("fit should succeed");

        // For each order, evaluate at regular pixel intervals and verify monotonicity.
        for m in [50u32, 60, 70, 80, 90, 100] {
            let lambdas: Vec<f64> = (0..=255)
                .map(|i| eval_chebyshev_2d(&fit, f64::from(i) * 10.0, m))
                .collect();

            // Determine direction from first two points.
            let increasing = lambdas[1] > lambdas[0];
            for w in lambdas.windows(2) {
                if increasing {
                    assert!(
                        w[1] >= w[0],
                        "order {m}: non-monotonic at lambda={:.4} -> {:.4}",
                        w[0],
                        w[1]
                    );
                } else {
                    assert!(
                        w[1] <= w[0],
                        "order {m}: non-monotonic at lambda={:.4} -> {:.4}",
                        w[0],
                        w[1]
                    );
                }
            }
        }
    }

    /// Global RMS should be non-zero and reasonable on synthetic data with
    /// a small amount of noise.
    #[test]
    fn test_global_rms_nonzero_and_reasonable() {
        let gc = 50_000.0;
        let mut training = Vec::new();

        for m in [50u32, 60, 70, 80, 90, 100] {
            let mf = f64::from(m);
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / 2560.0;

            for px_i in 0..20 {
                let x = f64::from(px_i) * 128.0;
                let lambda = lambda_center + dispersion * (x - 1280.0);
                // Add deterministic pseudo-noise
                let noise = 0.005
                    * ((f64::from(px_i) * 7.3 + f64::from(m) * 1.7).sin()
                        + (f64::from(px_i) * 3.1 - f64::from(m) * 5.9).cos() * 0.5);
                training.push((x, m, lambda + noise));
            }
        }

        let fit = fit_chebyshev_2d(&training, 4, 3).expect("fit should succeed");

        // RMS should be non-zero (we added noise).
        assert!(
            fit.rms_nm > 1e-6,
            "RMS should be > 0 (noise was added), got {:.10}",
            fit.rms_nm
        );

        // RMS should be small (< 0.1 nm on this synthetic data).
        assert!(
            fit.rms_nm < 0.1,
            "RMS {:.6} should be < 0.1 nm on synthetic data",
            fit.rms_nm
        );
    }

    /// `chebyshev_t` should match known values.
    #[test]
    fn test_chebyshev_t_known_values() {
        // T_0(x) = 1 for all x
        assert!((chebyshev_t(0, 0.5) - 1.0).abs() < 1e-15);
        assert!((chebyshev_t(0, -0.3) - 1.0).abs() < 1e-15);

        // T_1(x) = x
        assert!((chebyshev_t(1, 0.7) - 0.7).abs() < 1e-15);

        // T_2(x) = 2x^2 - 1
        let x = 0.6;
        assert!((chebyshev_t(2, x) - (2.0 * x * x - 1.0)).abs() < 1e-14);

        // T_3(x) = 4x^3 - 3x
        assert!((chebyshev_t(3, x) - (4.0 * x * x * x - 3.0 * x)).abs() < 1e-14);

        // T_n(1) = 1 for all n
        for n in 0..10 {
            assert!(
                (chebyshev_t(n, 1.0) - 1.0).abs() < 1e-12,
                "T_{n}(1) should be 1"
            );
        }

        // T_n(-1) = (-1)^n for all n
        for n in 0..10 {
            let expected = if n % 2 == 0 { 1.0 } else { -1.0 };
            assert!(
                (chebyshev_t(n, -1.0) - expected).abs() < 1e-12,
                "T_{n}(-1) should be {expected}"
            );
        }
    }

    /// Insufficient data should return None.
    #[test]
    fn test_fit_insufficient_data() {
        // dx=4, dm=3 requires (4+1)*(3+1) = 20 coefficients, need >= 20 points.
        let training: Vec<(f64, u32, f64)> = (0..5).map(|i| (f64::from(i), 50, 500.0)).collect();
        assert!(
            fit_chebyshev_2d(&training, 4, 3).is_none(),
            "should return None with too few points"
        );
    }

    /// Single order (all same m) should return None since normalization
    /// requires at least 2 distinct m values.
    #[test]
    fn test_fit_single_order_returns_none() {
        let training: Vec<(f64, u32, f64)> = (0..30)
            .map(|i| (f64::from(i) * 100.0, 60, 500.0 + f64::from(i) * 0.1))
            .collect();
        assert!(
            fit_chebyshev_2d(&training, 2, 2).is_none(),
            "should return None with only one order"
        );
    }
}
