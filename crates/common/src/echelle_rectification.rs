//! Order rectification for echelle extraction.
//!
//! Extracts rectangular sub-images (bounding boxes) around each order trace
//! from a full echellegram frame. Each sub-image is contiguous in memory,
//! enabling cache-friendly access and SIMD auto-vectorization in downstream
//! extraction kernels (boxcar summation, Horne optimal extraction).
//!
//! For the Mechelle's gently curved orders (<10 px curvature across 4K),
//! bounding-box extraction is sufficient. Full bilinear resampling onto a
//! rotated grid is the upgrade path for spectrographs with >20px curvature.

// Module-level cast allowances: pixel indices are always small enough for
// lossless usize→f64, and f64→usize truncation is intentional for pixel coords.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::echelle::EchelleTraceModel;

/// A rectified order sub-image with aperture mask.
///
/// Contains a contiguous rectangular region of the frame centered on an order
/// trace, plus a weight mask for aperture-weighted extraction.
#[derive(Debug, Clone)]
pub struct RectifiedOrder {
    /// Contiguous sub-image data, row-major: `data[row * n_dispersion + col]`.
    /// Rows are cross-dispersion, columns are dispersion.
    pub data: Vec<f32>,
    /// Aperture weight mask, same layout as `data`.
    /// Values are 0.0 outside the aperture, Gaussian profile inside.
    /// For boxcar extraction, weights are 1.0 inside.
    pub mask: Vec<f32>,
    /// Number of dispersion (spectral) pixels (columns).
    pub n_dispersion: usize,
    /// Number of cross-dispersion (spatial) pixels (rows).
    pub n_cross: usize,
    /// Starting dispersion pixel in the full frame.
    pub disp_start: u32,
    /// Starting cross-dispersion pixel in the full frame.
    pub cross_start: u32,
    /// Echelle order index.
    pub order_index: u32,
    /// Sub-pixel trace center per dispersion column, relative to `cross_start`.
    /// Length equals `n_dispersion`.
    pub trace_centers: Vec<f64>,
}

impl RectifiedOrder {
    /// Get a pixel value at (dispersion, cross-dispersion) within the sub-image.
    #[must_use]
    pub fn get(&self, disp: usize, cross: usize) -> f32 {
        self.data[cross * self.n_dispersion + disp]
    }

    /// Get the aperture mask weight at (dispersion, cross-dispersion).
    #[must_use]
    pub fn weight(&self, disp: usize, cross: usize) -> f32 {
        self.mask[cross * self.n_dispersion + disp]
    }
}

/// Configuration for order rectification.
#[derive(Debug, Clone)]
pub struct RectifyConfig {
    /// Aperture half-width in pixels (default: 4.0).
    pub aperture_half_width: f64,
    /// If true, use Gaussian weights in the mask. If false, use uniform (boxcar).
    pub gaussian_weights: bool,
    /// Gaussian FWHM for mask weights (only used if `gaussian_weights` is true).
    /// Default: 3.0 pixels.
    pub fwhm: f64,
}

impl Default for RectifyConfig {
    fn default() -> Self {
        Self {
            aperture_half_width: 4.0,
            gaussian_weights: false,
            fwhm: 3.0,
        }
    }
}

/// Specification for a single order to rectify.
#[derive(Debug, Clone)]
pub struct OrderSpec<'a> {
    /// Trace model for this order.
    pub trace: &'a EchelleTraceModel,
    /// Starting dispersion pixel (inclusive).
    pub disp_start: u32,
    /// Ending dispersion pixel (inclusive).
    pub disp_end: u32,
    /// Order index.
    pub order_index: u32,
}

/// Rectify a single order from a frame.
///
/// Evaluates the trace model at each dispersion position, computes the
/// bounding box in cross-dispersion, and copies the sub-image into a
/// contiguous buffer. Also builds an aperture weight mask.
///
/// `frame` is row-major with `height` rows and `width` columns.
/// For echelle spectrographs, dispersion is typically along X (columns)
/// and cross-dispersion along Y (rows).
///
/// Returns `None` if the trace falls entirely outside the frame.
pub fn rectify_order(
    frame: &[f32],
    width: u32,
    height: u32,
    spec: &OrderSpec<'_>,
    config: &RectifyConfig,
) -> Option<RectifiedOrder> {
    let trace = spec.trace;
    let disp_start = spec.disp_start;
    let disp_end = spec.disp_end;
    let order_index = spec.order_index;
    let w = width as usize;
    let h = height as usize;

    if frame.len() < w * h {
        return None;
    }

    let n_disp = (disp_end.saturating_sub(disp_start) + 1) as usize;
    if n_disp == 0 {
        return None;
    }

    // Evaluate trace center at each dispersion position.
    let mut trace_centers_abs = Vec::with_capacity(n_disp);
    for col in disp_start..=disp_end {
        let center = eval_trace_safe(trace, f64::from(col))?;
        trace_centers_abs.push(center);
    }

    // Compute bounding box in cross-dispersion.
    let radius = config.aperture_half_width.ceil() as i32;
    let min_center = trace_centers_abs
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_center = trace_centers_abs
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);

    let cross_start_i = (min_center.floor() as i32 - radius).max(0);
    let cross_end_i = (max_center.ceil() as i32 + radius).min(h as i32 - 1);

    if cross_start_i > cross_end_i {
        return None;
    }

    let cross_start = cross_start_i as u32;
    let n_cross = (cross_end_i - cross_start_i + 1) as usize;

    // Compute trace centers relative to the sub-image origin.
    let trace_centers: Vec<f64> = trace_centers_abs
        .iter()
        .map(|&c| c - f64::from(cross_start))
        .collect();

    // Extract sub-image.
    let mut data = vec![0.0f32; n_cross * n_disp];
    let disp_start_usize = disp_start as usize;
    let cross_start_usize = cross_start as usize;

    for row in 0..n_cross {
        let frame_row = cross_start_usize + row;
        if frame_row >= h {
            break;
        }
        let frame_row_offset = frame_row * w;
        let sub_row_offset = row * n_disp;

        for col in 0..n_disp {
            let frame_col = disp_start_usize + col;
            if frame_col < w {
                data[sub_row_offset + col] = frame[frame_row_offset + frame_col];
            }
        }
    }

    // Build aperture mask.
    let sigma = if config.gaussian_weights {
        // FWHM = 2 * sqrt(2 * ln(2)) * sigma
        config.fwhm / (2.0 * (2.0 * 2.0_f64.ln()).sqrt())
    } else {
        0.0 // unused for boxcar
    };

    let mut mask = vec![0.0f32; n_cross * n_disp];
    let half_w = config.aperture_half_width;

    for col in 0..n_disp {
        let center = trace_centers[col];
        for row in 0..n_cross {
            let dist = (row as f64) - center;
            if dist.abs() <= half_w {
                let weight = if config.gaussian_weights && sigma > 0.0 {
                    (-0.5 * (dist / sigma).powi(2)).exp() as f32
                } else {
                    1.0
                };
                mask[row * n_disp + col] = weight;
            }
        }
    }

    Some(RectifiedOrder {
        data,
        mask,
        n_dispersion: n_disp,
        n_cross,
        disp_start,
        cross_start,
        order_index,
        trace_centers,
    })
}

/// Rectify all orders from a frame.
///
/// Processes each order sequentially. Could be parallelized with rayon in the future
/// if extraction latency becomes a bottleneck (see bd-12sg benchmark harness).
pub fn rectify_all_orders(
    frame: &[f32],
    width: u32,
    height: u32,
    orders: &[OrderSpec<'_>],
    config: &RectifyConfig,
) -> Vec<Option<RectifiedOrder>> {
    orders
        .iter()
        .map(|spec| rectify_order(frame, width, height, spec, config))
        .collect()
}

/// Safely evaluate a trace model, returning None instead of panicking.
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
                crate::echelle::PolynomialBasis::Monomial => {
                    let mut acc = 0.0f64;
                    for &c in coefficients.iter().rev() {
                        acc = acc * x + c;
                    }
                    acc
                }
                crate::echelle::PolynomialBasis::Chebyshev => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::echelle::PolynomialBasis;

    /// Create a flat trace (constant cross-dispersion position).
    fn flat_trace(center: f64) -> EchelleTraceModel {
        EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: vec![center],
            domain_start: 0.0,
            domain_end: 1000.0,
        }
    }

    /// Create a linear trace: y = intercept + slope * x.
    fn linear_trace(intercept: f64, slope: f64) -> EchelleTraceModel {
        EchelleTraceModel::Polynomial {
            basis: PolynomialBasis::Monomial,
            coefficients: vec![intercept, slope],
            domain_start: 0.0,
            domain_end: 1000.0,
        }
    }

    /// Create a test frame with a horizontal bright band at a given row.
    fn make_test_frame(width: u32, height: u32, bright_row: u32, brightness: f32) -> Vec<f32> {
        let w = width as usize;
        let h = height as usize;
        let mut frame = vec![10.0f32; w * h];
        if (bright_row as usize) < h {
            for col in 0..w {
                frame[bright_row as usize * w + col] = brightness;
            }
        }
        frame
    }

    fn spec(trace: &EchelleTraceModel, disp_end: u32, order_index: u32) -> OrderSpec<'_> {
        OrderSpec {
            trace,
            disp_start: 0,
            disp_end,
            order_index,
        }
    }

    #[test]
    fn test_rectify_flat_trace() {
        let width = 100;
        let height = 50;
        let trace_y = 25.0;
        let frame = make_test_frame(width, height, 25, 1000.0);
        let trace = flat_trace(trace_y);

        let config = RectifyConfig {
            aperture_half_width: 3.0,
            gaussian_weights: false,
            fwhm: 3.0,
        };

        let rect = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config)
            .expect("should rectify");

        assert_eq!(rect.n_dispersion, width as usize);
        // Cross dimension should be 2*ceil(3.0) + 1 = 7 rows (centered on row 25).
        assert!(rect.n_cross >= 7, "n_cross={}", rect.n_cross);
        assert_eq!(rect.order_index, 0);

        // The bright row (25) should be within the sub-image.
        let bright_row_local = 25 - rect.cross_start as usize;
        for col in 0..rect.n_dispersion {
            let val = rect.get(col, bright_row_local);
            assert!(
                (val - 1000.0).abs() < 0.1,
                "bright pixel at ({col}, {bright_row_local}) = {val}"
            );
        }

        // All trace centers should be close to the center of the sub-image.
        for &tc in &rect.trace_centers {
            assert!(
                (tc - (trace_y - f64::from(rect.cross_start))).abs() < 0.01,
                "trace center {tc}"
            );
        }
    }

    #[test]
    fn test_rectify_boxcar_mask() {
        let width = 50;
        let height = 30;
        let frame = vec![100.0f32; width as usize * height as usize];
        let trace = flat_trace(15.0);

        let config = RectifyConfig {
            aperture_half_width: 2.0,
            gaussian_weights: false,
            fwhm: 3.0,
        };

        let rect = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config)
            .expect("should rectify");

        // Count non-zero mask pixels per column.
        for col in 0..rect.n_dispersion {
            let center = rect.trace_centers[col];
            let mut n_active = 0;
            for row in 0..rect.n_cross {
                let w = rect.weight(col, row);
                let dist = (row as f64 - center).abs();
                if dist <= 2.0 {
                    assert!(
                        (w - 1.0).abs() < 1e-6,
                        "boxcar weight should be 1.0, got {w} at dist={dist:.2}"
                    );
                    n_active += 1;
                } else {
                    assert!(
                        w.abs() < 1e-6,
                        "weight outside aperture should be 0, got {w} at dist={dist:.2}"
                    );
                }
            }
            assert!(
                n_active >= 4,
                "should have at least 4 active pixels, got {n_active}"
            );
        }
    }

    #[test]
    fn test_rectify_gaussian_mask() {
        let width = 50;
        let height = 30;
        let frame = vec![100.0f32; width as usize * height as usize];
        let trace = flat_trace(15.0);

        let config = RectifyConfig {
            aperture_half_width: 4.0,
            gaussian_weights: true,
            fwhm: 3.0,
        };

        let rect = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config)
            .expect("should rectify");

        // Center pixel should have highest weight, decaying outward.
        let col = rect.n_dispersion / 2;
        let center_row = rect.trace_centers[col].round() as usize;
        let center_w = rect.weight(col, center_row);
        assert!(
            (center_w - 1.0).abs() < 1e-4,
            "center weight should be ~1.0, got {center_w}"
        );

        // 1 pixel off-center should be less than center.
        if center_row > 0 {
            let off_w = rect.weight(col, center_row - 1);
            assert!(
                off_w < center_w,
                "off-center weight {off_w} should be < center {center_w}"
            );
            assert!(off_w > 0.0, "should still be positive inside aperture");
        }
    }

    #[test]
    fn test_rectify_tilted_trace() {
        let width: u32 = 200;
        let height: u32 = 100;
        // Trace with slight tilt: y = 50 + 0.05*x (10px rise over 200px).
        let frame = vec![100.0f32; width as usize * height as usize];
        let trace = linear_trace(50.0, 0.05);

        let config = RectifyConfig {
            aperture_half_width: 3.0,
            gaussian_weights: false,
            fwhm: 3.0,
        };

        let rect = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config)
            .expect("should rectify");

        // Sub-image should be wider in cross-dispersion to accommodate tilt.
        // Trace goes from y=50 to y=60, so with ±3 margin: rows 47-63 = 17 rows.
        assert!(
            rect.n_cross >= 16,
            "tilted trace n_cross={} too small",
            rect.n_cross
        );

        // Verify trace centers follow the tilt.
        let first_center = rect.trace_centers[0];
        let last_center = rect.trace_centers[rect.n_dispersion - 1];
        let expected_rise = 0.05 * f64::from(width - 1);
        let actual_rise = last_center - first_center;
        assert!(
            (actual_rise - expected_rise).abs() < 0.1,
            "trace rise: expected {expected_rise:.2}, got {actual_rise:.2}"
        );
    }

    #[test]
    fn test_rectify_out_of_bounds() {
        let width: u32 = 100;
        let height: u32 = 50;
        let frame = vec![100.0f32; width as usize * height as usize];

        // Trace entirely below the frame.
        let trace = flat_trace(-10.0);
        let config = RectifyConfig::default();
        let result = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config);
        assert!(result.is_none(), "trace below frame should return None");
    }

    #[test]
    fn test_rectify_sub_image_contiguous() {
        // Verify that the data layout is truly row-major contiguous.
        let width: u32 = 100;
        let height: u32 = 50;
        let mut frame = vec![0.0f32; width as usize * height as usize];

        // Mark a specific pixel: row 25, col 50.
        frame[25 * width as usize + 50] = 42.0;

        let trace = flat_trace(25.0);
        let config = RectifyConfig {
            aperture_half_width: 2.0,
            ..Default::default()
        };

        let rect = rectify_order(&frame, width, height, &spec(&trace, width - 1, 0), &config)
            .expect("should rectify");

        // Find the marked pixel in the sub-image.
        let local_row = 25 - rect.cross_start as usize;
        let local_col = 50;
        assert!(
            (rect.data[local_row * rect.n_dispersion + local_col] - 42.0).abs() < 0.01,
            "marked pixel not found in sub-image"
        );
    }

    #[test]
    fn test_rectify_all_orders_sequential() {
        let width: u32 = 200;
        let height: u32 = 100;
        let frame = vec![100.0f32; width as usize * height as usize];

        let traces = [flat_trace(20.0), flat_trace(50.0), flat_trace(80.0)];
        let orders: Vec<OrderSpec<'_>> = traces
            .iter()
            .enumerate()
            .map(|(i, t)| OrderSpec {
                trace: t,
                disp_start: 0,
                disp_end: width - 1,
                order_index: i as u32,
            })
            .collect();

        let config = RectifyConfig::default();
        let results = rectify_all_orders(&frame, width, height, &orders, &config);

        assert_eq!(results.len(), 3);
        for (i, result) in results.iter().enumerate() {
            assert!(result.is_some(), "order {i} should be rectified");
            let rect = result.as_ref().unwrap();
            assert_eq!(rect.order_index, i as u32);
        }
    }
}
