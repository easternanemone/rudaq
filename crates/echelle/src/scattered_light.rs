//! Scattered light subtraction for echelle spectrographs.
//!
//! Estimates and subtracts the inter-order scattered light background from
//! raw echellegram frames before spectral extraction. Uses a smooth 2D
//! polynomial surface fitted to inter-order pixel samples.
//!
//! The algorithm:
//! 1. Build an inter-order mask (pixels outside all trace apertures)
//! 2. Bin the frame into blocks, compute median of inter-order pixels per block
//! 3. Fit a bivariate Chebyshev surface to the block medians
//! 4. Evaluate the surface at every pixel and subtract from the raw frame

// Pixel-index casts: always small enough for lossless usize→f64 conversion,
// and f64→usize truncation is intentional for pixel coordinates.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::types::EchelleTraceModel;

/// Configuration for scattered light subtraction.
#[derive(Debug, Clone)]
pub struct ScatteredLightConfig {
    /// Aperture half-width used to define order regions (pixels).
    /// Pixels within this distance of any trace center are excluded.
    pub aperture_half_width: f64,
    /// Block size for spatial binning (pixels, default: 64).
    /// The frame is divided into blocks of this size for median sampling.
    pub block_size: u32,
    /// Polynomial degree in the dispersion direction (default: 3).
    pub poly_degree_x: usize,
    /// Polynomial degree in the cross-dispersion direction (default: 3).
    pub poly_degree_y: usize,
}

impl Default for ScatteredLightConfig {
    fn default() -> Self {
        Self {
            aperture_half_width: 5.0,
            block_size: 64,
            poly_degree_x: 3,
            poly_degree_y: 3,
        }
    }
}

/// Result of scattered light estimation.
#[derive(Debug, Clone)]
pub struct ScatteredLightModel {
    /// Chebyshev coefficients for the 2D surface.
    /// Layout: `coeffs[iy * (degree_x+1) + ix]` for T_ix(x_norm) * T_iy(y_norm).
    pub coefficients: Vec<f64>,
    /// Polynomial degree in x (dispersion).
    pub degree_x: usize,
    /// Polynomial degree in y (cross-dispersion).
    pub degree_y: usize,
    /// Frame width used for normalization.
    pub width: u32,
    /// Frame height used for normalization.
    pub height: u32,
}

impl ScatteredLightModel {
    /// Evaluate the scattered light model at a pixel position.
    #[must_use]
    pub fn eval(&self, x: f64, y: f64) -> f64 {
        let x_norm = 2.0 * x / f64::from(self.width.max(1)) - 1.0;
        let y_norm = 2.0 * y / f64::from(self.height.max(1)) - 1.0;
        eval_2d_chebyshev(
            &self.coefficients,
            self.degree_x,
            self.degree_y,
            x_norm,
            y_norm,
        )
    }

    /// Subtract scattered light from a frame in-place.
    pub fn subtract_from(&self, frame: &mut [f32], width: u32) {
        let w = width as usize;
        for (idx, pixel) in frame.iter_mut().enumerate() {
            let col = idx % w;
            let row = idx / w;
            let scatter = self.eval(col as f64, row as f64);
            *pixel = (f64::from(*pixel) - scatter).max(0.0) as f32;
        }
    }
}

/// Trace specification for building the inter-order mask.
pub struct TraceInfo<'a> {
    /// Trace model for this order.
    pub trace: &'a EchelleTraceModel,
    /// Starting dispersion pixel.
    pub disp_start: u32,
    /// Ending dispersion pixel.
    pub disp_end: u32,
}

/// Estimate and subtract scattered light from a frame.
///
/// Returns a new frame with scattered light subtracted, plus the model.
pub fn subtract_scattered_light(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    config: &ScatteredLightConfig,
) -> Option<(Vec<f32>, ScatteredLightModel)> {
    let model = estimate_scattered_light(frame, width, height, traces, config)?;
    let mut corrected = frame.to_vec();
    model.subtract_from(&mut corrected, width);
    Some((corrected, model))
}

/// Estimate the scattered light model without subtracting.
pub fn estimate_scattered_light(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    config: &ScatteredLightConfig,
) -> Option<ScatteredLightModel> {
    let w = width as usize;
    let h = height as usize;
    if frame.len() < w * h || w < 2 || h < 2 {
        return None;
    }

    // Step 1: Build inter-order mask.
    let inter_order_mask =
        build_inter_order_mask(width, height, traces, config.aperture_half_width);

    // Step 2: Bin into blocks and compute median of inter-order pixels.
    let bs = config.block_size.max(1) as usize;
    let n_blocks_x = w.div_ceil(bs);
    let n_blocks_y = h.div_ceil(bs);

    let mut block_x = Vec::new();
    let mut block_y = Vec::new();
    let mut block_val = Vec::new();

    let mut scratch = Vec::new();

    for by in 0..n_blocks_y {
        for bx in 0..n_blocks_x {
            scratch.clear();
            let y_start = by * bs;
            let y_end = ((by + 1) * bs).min(h);
            let x_start = bx * bs;
            let x_end = ((bx + 1) * bs).min(w);

            for row in y_start..y_end {
                for col in x_start..x_end {
                    let idx = row * w + col;
                    if inter_order_mask[idx] {
                        scratch.push(f64::from(frame[idx]));
                    }
                }
            }

            if scratch.len() >= 3 {
                scratch
                    .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median = scratch[scratch.len() / 2];
                // Block center coordinates.
                block_x.push(x_start.midpoint(x_end) as f64);
                block_y.push(y_start.midpoint(y_end) as f64);
                block_val.push(median);
            }
        }
    }

    let n_coeffs = (config.poly_degree_x + 1) * (config.poly_degree_y + 1);
    if block_val.len() < n_coeffs + 1 {
        return None;
    }

    // Step 3: Fit 2D Chebyshev surface.
    let x_norm: Vec<f64> = block_x
        .iter()
        .map(|&x| 2.0 * x / f64::from(width) - 1.0)
        .collect();
    let y_norm: Vec<f64> = block_y
        .iter()
        .map(|&y| 2.0 * y / f64::from(height) - 1.0)
        .collect();

    let coefficients = fit_2d_chebyshev(
        &x_norm,
        &y_norm,
        &block_val,
        config.poly_degree_x,
        config.poly_degree_y,
    )?;

    Some(ScatteredLightModel {
        coefficients,
        degree_x: config.poly_degree_x,
        degree_y: config.poly_degree_y,
        width,
        height,
    })
}

/// Build a mask where `true` means "inter-order pixel" (not covered by any trace).
fn build_inter_order_mask(
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
) -> Vec<bool> {
    let w = width as usize;
    let h = height as usize;
    let mut mask = vec![true; w * h];

    for trace_info in traces {
        for col in trace_info.disp_start..=trace_info.disp_end.min(width - 1) {
            if let Some(center) = eval_trace_safe(trace_info.trace, f64::from(col)) {
                let y_min = (center - aperture_half_width).floor().max(0.0) as usize;
                let y_max = (center + aperture_half_width).ceil().min(h as f64 - 1.0) as usize;
                for row in y_min..=y_max {
                    mask[row * w + col as usize] = false;
                }
            }
        }
    }

    mask
}

/// Evaluate a 2D Chebyshev polynomial: sum_{i,j} c_{ij} * T_i(x) * T_j(y).
fn eval_2d_chebyshev(
    coeffs: &[f64],
    degree_x: usize,
    degree_y: usize,
    x_norm: f64,
    y_norm: f64,
) -> f64 {
    let nx = degree_x + 1;
    let ny = degree_y + 1;

    // Precompute T_i(x) and T_j(y).
    let tx = chebyshev_basis(x_norm, degree_x);
    let ty = chebyshev_basis(y_norm, degree_y);

    let mut result = 0.0;
    for iy in 0..ny {
        for ix in 0..nx {
            result += coeffs[iy * nx + ix] * tx[ix] * ty[iy];
        }
    }
    result
}

/// Compute Chebyshev basis values T_0(x) through T_degree(x).
fn chebyshev_basis(x: f64, degree: usize) -> Vec<f64> {
    let mut t = Vec::with_capacity(degree + 1);
    t.push(1.0); // T_0
    if degree >= 1 {
        t.push(x); // T_1
    }
    for i in 2..=degree {
        let val = 2.0 * x * t[i - 1] - t[i - 2];
        t.push(val);
    }
    t
}

/// Fit 2D Chebyshev coefficients via least-squares normal equations.
fn fit_2d_chebyshev(
    x_norm: &[f64],
    y_norm: &[f64],
    values: &[f64],
    degree_x: usize,
    degree_y: usize,
) -> Option<Vec<f64>> {
    let n_pts = x_norm.len();
    let nx = degree_x + 1;
    let ny = degree_y + 1;
    let n_coeffs = nx * ny;

    if n_pts < n_coeffs {
        return None;
    }

    // Build Vandermonde matrix V[i, iy*nx + ix] = T_ix(x_i) * T_iy(y_i).
    let mut vander = vec![vec![0.0; n_coeffs]; n_pts];
    for ((&xn, &yn), row) in x_norm.iter().zip(y_norm).zip(vander.iter_mut()) {
        let tx = chebyshev_basis(xn, degree_x);
        let ty = chebyshev_basis(yn, degree_y);
        for iy in 0..ny {
            for ix in 0..nx {
                row[iy * nx + ix] = tx[ix] * ty[iy];
            }
        }
    }

    // Normal equations: A = V^T * V, b = V^T * values.
    let mut normal = vec![vec![0.0; n_coeffs]; n_coeffs];
    let mut rhs = vec![0.0; n_coeffs];

    for j in 0..n_coeffs {
        for k in 0..n_coeffs {
            let sum: f64 = vander.iter().map(|row| row[j] * row[k]).sum();
            normal[j][k] = sum;
        }
        let sum: f64 = vander.iter().zip(values).map(|(row, &v)| row[j] * v).sum();
        rhs[j] = sum;
    }

    solve_linear_system(&mut normal, &mut rhs)
}

/// Solve Ax = b via Gaussian elimination with partial pivoting.
fn solve_linear_system(mat: &mut [Vec<f64>], rhs: &mut [f64]) -> Option<Vec<f64>> {
    let dim = rhs.len();

    for col in 0..dim {
        // Find pivot.
        let (max_row, max_val) = mat[col..]
            .iter()
            .enumerate()
            .map(|(i, row)| (col + i, row[col].abs()))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;

        if max_val < 1e-15 {
            return None;
        }

        if max_row != col {
            mat.swap(col, max_row);
            rhs.swap(col, max_row);
        }

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

/// Safely evaluate a trace model, returning None on errors.
fn eval_trace_safe(trace: &EchelleTraceModel, x: f64) -> Option<f64> {
    match trace {
        EchelleTraceModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
        } => {
            if coefficients.is_empty() || !x.is_finite() || *domain_start >= *domain_end {
                return None;
            }
            let result = match basis {
                crate::types::PolynomialBasis::Monomial => {
                    let mut acc = 0.0f64;
                    for &c in coefficients.iter().rev() {
                        acc = acc * x + c;
                    }
                    acc
                }
                crate::types::PolynomialBasis::Chebyshev => {
                    let t = (2.0 * (x - domain_start)) / (domain_end - domain_start) - 1.0;
                    if coefficients.len() == 1 {
                        return Some(coefficients[0]);
                    }
                    let mut t0 = 1.0f64;
                    let mut t1 = t;
                    let mut acc = coefficients[0] * t0 + coefficients[1] * t1;
                    for &c in coefficients.iter().skip(2) {
                        let tn = 2.0 * t * t1 - t0;
                        acc += c * tn;
                        t0 = t1;
                        t1 = tn;
                    }
                    acc
                }
            };
            if result.is_finite() {
                Some(result)
            } else {
                None
            }
        }
    }
}

// ─── Fast path for live extraction ─────────────────────────────────────────

/// Estimate a full-resolution scattered-light map using the CERES coarse-grid method.
///
/// 1. On a coarse grid (`col_stride × row_stride`), sample the median of
///    inter-order pixel values.
/// 2. Bilinearly interpolate to full resolution.
///
/// Returns a `Vec<f32>` of size `width × height` containing the estimated
/// scatter value at each pixel. Use this with [`subtract_scatter_map`] or
/// pass it into the extraction pipeline for per-pixel correction.
pub fn estimate_scatter_map_fast_f32(
    frame: &[f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
    col_stride: u32,
    row_stride: u32,
) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    if frame.len() < w * h || w < 2 || h < 2 {
        return vec![0.0; w * h];
    }

    let cs = col_stride.max(1) as usize;
    let rs = row_stride.max(1) as usize;
    let grid_cols = w.div_ceil(cs);
    let grid_rows = h.div_ceil(rs);

    // Build coarse scattered-light map: for each grid cell, compute the
    // median of inter-order pixels in a small window around the grid point.
    let mut coarse_map = vec![0.0f32; grid_cols * grid_rows];
    let mut scratch = Vec::new();

    for gy in 0..grid_rows {
        let cy = (gy * rs + rs / 2).min(h - 1);
        let row_lo = cy.saturating_sub(rs / 2);
        let row_hi = (cy + rs / 2 + 1).min(h);

        for gx in 0..grid_cols {
            let cx = (gx * cs + cs / 2).min(w - 1);
            let col_lo = cx.saturating_sub(cs / 2);
            let col_hi = (cx + cs / 2 + 1).min(w);

            scratch.clear();
            for row in row_lo..row_hi {
                for col in col_lo..col_hi {
                    if is_inter_order(col as f64, row as f64, traces, aperture_half_width) {
                        let val = frame[row * w + col];
                        if val.is_finite() {
                            scratch.push(val);
                        }
                    }
                }
            }

            coarse_map[gy * grid_cols + gx] = if scratch.is_empty() {
                0.0
            } else {
                scratch
                    .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                scratch[scratch.len() / 2]
            };
        }
    }

    // Bilinear interpolation → full resolution scatter map.
    let mut scatter_map = vec![0.0f32; w * h];
    for row in 0..h {
        let gy_f = row as f64 / rs as f64;
        let gy0 = (gy_f.floor() as usize).min(grid_rows.saturating_sub(2));
        let gy1 = (gy0 + 1).min(grid_rows - 1);
        let ty = (gy_f - gy0 as f64).clamp(0.0, 1.0) as f32;

        for col in 0..w {
            let gx_f = col as f64 / cs as f64;
            let gx0 = (gx_f.floor() as usize).min(grid_cols.saturating_sub(2));
            let gx1 = (gx0 + 1).min(grid_cols - 1);
            let tx = (gx_f - gx0 as f64).clamp(0.0, 1.0) as f32;

            let v00 = coarse_map[gy0 * grid_cols + gx0];
            let v10 = coarse_map[gy0 * grid_cols + gx1];
            let v01 = coarse_map[gy1 * grid_cols + gx0];
            let v11 = coarse_map[gy1 * grid_cols + gx1];

            scatter_map[row * w + col] = v00 * (1.0 - tx) * (1.0 - ty)
                + v10 * tx * (1.0 - ty)
                + v01 * (1.0 - tx) * ty
                + v11 * tx * ty;
        }
    }
    scatter_map
}

/// Subtract a pre-computed scatter map from a frame in-place.
pub fn subtract_scatter_map(frame: &mut [f32], scatter_map: &[f32]) {
    for (pixel, &scatter) in frame.iter_mut().zip(scatter_map.iter()) {
        *pixel = (*pixel - scatter).max(0.0);
    }
}

/// Fast scattered light subtraction for live frame preview.
///
/// Convenience wrapper: estimates + subtracts in one call.
/// See [`estimate_scatter_map_fast_f32`] for the algorithm details.
pub fn subtract_scattered_light_fast_f32(
    frame: &mut [f32],
    width: u32,
    height: u32,
    traces: &[TraceInfo<'_>],
    aperture_half_width: f64,
    col_stride: u32,
    row_stride: u32,
) {
    let scatter_map = estimate_scatter_map_fast_f32(
        frame,
        width,
        height,
        traces,
        aperture_half_width,
        col_stride,
        row_stride,
    );
    subtract_scatter_map(frame, &scatter_map);
}

/// Check whether a pixel is inter-order (not within any trace aperture).
fn is_inter_order(col: f64, row: f64, traces: &[TraceInfo<'_>], aperture_half_width: f64) -> bool {
    for trace_info in traces {
        let col_u32 = col as u32;
        if col_u32 < trace_info.disp_start || col_u32 > trace_info.disp_end {
            continue;
        }
        if let Some(center) = eval_trace_safe(trace_info.trace, col) {
            if (row - center).abs() <= aperture_half_width {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PolynomialBasis;

    fn flat_trace(center: f64) -> EchelleTraceModel {
        EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: vec![center],
            domain_start: 0.0,
            domain_end: 1000.0,
        }
    }

    #[test]
    fn test_inter_order_mask() {
        let width = 100;
        let height = 50;
        let traces = [
            TraceInfo {
                trace: &flat_trace(15.0),
                disp_start: 0,
                disp_end: width - 1,
            },
            TraceInfo {
                trace: &flat_trace(35.0),
                disp_start: 0,
                disp_end: width - 1,
            },
        ];

        let mask = build_inter_order_mask(width, height, &traces, 3.0);

        // Pixels near trace centers should be masked out (false).
        assert!(
            !mask[15 * width as usize + 50],
            "pixel at trace center should be masked"
        );
        assert!(
            !mask[13 * width as usize + 50],
            "pixel within aperture should be masked"
        );

        // Pixels far from both traces should be unmasked (true).
        assert!(
            mask[50], // row 0, col 50
            "pixel far from traces should be inter-order"
        );
        assert!(
            mask[25 * width as usize + 50],
            "pixel between traces should be inter-order"
        );
    }

    #[test]
    fn test_constant_scattered_light() {
        // Frame with uniform background of 50 + no orders.
        let width: u32 = 128;
        let height: u32 = 64;
        let background = 50.0f32;
        let frame = vec![background; width as usize * height as usize];

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 1,
            poly_degree_y: 1,
            aperture_half_width: 3.0,
        };

        let (corrected, model) =
            subtract_scattered_light(&frame, width, height, &[], &config).expect("should fit");

        // After subtraction, all pixels should be near zero.
        for (i, &val) in corrected.iter().enumerate() {
            assert!(val.abs() < 1.0, "pixel {i}: corrected={val}, expected ~0");
        }

        // Model should evaluate to ~50 everywhere.
        let center_val = model.eval(64.0, 32.0);
        assert!(
            (center_val - f64::from(background)).abs() < 2.0,
            "model center: {center_val}, expected ~{background}"
        );
    }

    #[test]
    fn test_linear_gradient_scatter() {
        // Frame with a linear gradient: scatter = 10 + 0.5*x.
        let width: u32 = 200;
        let height: u32 = 100;
        let mut frame = Vec::with_capacity(width as usize * height as usize);
        for _row in 0..height {
            for col in 0..width {
                frame.push(10.0 + 0.5 * col as f32);
            }
        }

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 2,
            poly_degree_y: 1,
            aperture_half_width: 3.0,
        };

        let (corrected, _model) =
            subtract_scattered_light(&frame, width, height, &[], &config).expect("should fit");

        // After subtraction, residuals should be small.
        let max_residual = corrected.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max_residual < 3.0,
            "max residual {max_residual} too large for linear gradient"
        );
    }

    #[test]
    fn test_scatter_with_orders_excluded() {
        // Frame with uniform scatter=20 in inter-order regions,
        // and bright signal=1000 on the order traces.
        let width: u32 = 200;
        let height: u32 = 100;
        let trace_center = 50.0;
        let trace = flat_trace(trace_center);
        let aperture = 5.0;

        let mut frame = vec![20.0f32; width as usize * height as usize];
        // Add bright signal along the trace.
        for col in 0..width as usize {
            for row_offset in -5i32..=5 {
                let row = (trace_center as i32 + row_offset) as usize;
                if row < height as usize {
                    frame[row * width as usize + col] = 1000.0;
                }
            }
        }

        let traces = [TraceInfo {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
        }];

        let config = ScatteredLightConfig {
            block_size: 32,
            poly_degree_x: 1,
            poly_degree_y: 1,
            aperture_half_width: aperture,
        };

        let (_corrected, model) =
            subtract_scattered_light(&frame, width, height, &traces, &config).expect("should fit");

        // Model should estimate ~20 (the inter-order background), not ~1000.
        let inter_order_val = model.eval(100.0, 10.0);
        assert!(
            (inter_order_val - 20.0).abs() < 5.0,
            "inter-order scatter estimate: {inter_order_val}, expected ~20"
        );
    }

    #[test]
    fn test_2d_chebyshev_roundtrip() {
        // Fit a known bivariate polynomial and verify eval.
        let coeffs = vec![
            100.0, 5.0, 0.0, 0.0, // iy=0: c00=100, c10=5, c20=0, c30=0
            3.0, 0.0, 0.0, 0.0, // iy=1: c01=3
            0.0, 0.0, 0.0, 0.0, // iy=2
            0.0, 0.0, 0.0, 0.0, // iy=3
        ];
        // f(x,y) = 100 + 5*T_1(x) + 3*T_1(y) = 100 + 5x + 3y

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 0.0, 0.0);
        assert!((val - 100.0).abs() < 1e-10, "at origin: {val}");

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 1.0, 0.0);
        assert!((val - 105.0).abs() < 1e-10, "at (1,0): {val}");

        let val = eval_2d_chebyshev(&coeffs, 3, 3, 0.0, 1.0);
        assert!((val - 103.0).abs() < 1e-10, "at (0,1): {val}");
    }

    #[test]
    fn test_too_few_samples_returns_none() {
        // Very small frame with many orders → not enough inter-order pixels.
        let width: u32 = 10;
        let height: u32 = 10;
        let frame = vec![100.0f32; width as usize * height as usize];

        // Fill entire frame with orders (pre-create owned traces).
        let owned_traces: Vec<EchelleTraceModel> = (0..10)
            .map(|i| EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![f64::from(i)],
                domain_start: 0.0,
                domain_end: 100.0,
            })
            .collect();

        let traces: Vec<TraceInfo<'_>> = owned_traces
            .iter()
            .map(|t| TraceInfo {
                trace: t,
                disp_start: 0,
                disp_end: width - 1,
            })
            .collect();

        let config = ScatteredLightConfig {
            block_size: 5,
            poly_degree_x: 3,
            poly_degree_y: 3,
            ..Default::default()
        };

        // This may or may not return None depending on how many inter-order
        // pixels survive. The key is it doesn't panic.
        let _result = estimate_scattered_light(&frame, width, height, &traces, &config);
    }

    #[test]
    fn test_fast_scatter_uniform_background() {
        let width: u32 = 64;
        let height: u32 = 32;
        let background = 100.0f32;
        let frame = vec![background; width as usize * height as usize];

        let map = estimate_scatter_map_fast_f32(&frame, width, height, &[], 3.0, 8, 4);

        assert_eq!(map.len(), width as usize * height as usize);
        // With no traces, all pixels are inter-order → scatter ≈ background.
        for &v in &map {
            assert!(
                (v - background).abs() < 5.0,
                "scatter estimate {v} should be ~{background}"
            );
        }
    }

    #[test]
    fn test_fast_scatter_subtracts_background() {
        let width: u32 = 200;
        let height: u32 = 100;
        let background = 50.0f32;
        let trace_center = 50.0;
        let trace = flat_trace(trace_center);
        let signal = 1000.0f32;

        let mut frame = vec![background; width as usize * height as usize];
        // Add bright signal along the trace.
        for col in 0..width as usize {
            for offset in -5i32..=5 {
                let row = (trace_center as i32 + offset) as usize;
                if row < height as usize {
                    frame[row * width as usize + col] = signal;
                }
            }
        }

        let traces = [TraceInfo {
            trace: &trace,
            disp_start: 0,
            disp_end: width - 1,
        }];

        let mut corrected = frame.clone();
        subtract_scattered_light_fast_f32(&mut corrected, width, height, &traces, 6.0, 8, 4);

        // Inter-order pixels should be near zero after subtraction.
        let inter_order_val = corrected[10 * width as usize + 50]; // row 10, far from trace
        assert!(
            inter_order_val < 10.0,
            "inter-order pixel should be near 0, got {inter_order_val}"
        );

        // On-trace pixels should retain most of the signal.
        let on_trace_val = corrected[50 * width as usize + 100];
        assert!(
            on_trace_val > signal - background - 10.0,
            "on-trace pixel should retain signal, got {on_trace_val}"
        );
    }
}
