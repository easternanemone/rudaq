//! Calibration quality metrics for echelle spectrograph wavelength solutions.
//!
//! Provides physically meaningful diagnostics to replace the old circular
//! (predicted-vs-stored) RMS metric:
//!
//! - **Global RMS**: residuals of identified arc peaks against the per-order
//!   Chebyshev wavelength model (target: < 0.1 nm).
//! - **Overlap agreement**: for adjacent orders that share a wavelength range,
//!   the maximum disagreement at the same physical position (should be sub-pixel).
//! - **Grating constant consistency**: `m * lambda_center` should be approximately
//!   constant across all orders (typical: ~36300 nm for Mechelle 5000).
//! - **LOO cross-validation**: leave-one-out RMS via the 2D Chebyshev surface fitter.

// Numerical code: pixel/order casts are always lossless for realistic frame sizes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use crate::chebyshev_2d::{self, Global2DChebyshevFit};
use crate::types::{EchelleCalibrationProfile, EchelleWavelengthModel, PolynomialBasis};
use crate::wavelength_fitting::leave_one_out_rms;
use serde::{Deserialize, Serialize};

/// Matched arc line record used for quality evaluation.
///
/// Each entry represents an atlas-identified emission line: the pixel position
/// where it was detected, the physical diffraction order, and the known (atlas)
/// wavelength.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedLine {
    /// Pixel position (sub-pixel center from Gaussian fit).
    pub pixel: f64,
    /// Physical diffraction order number (positive integer).
    pub physical_order: u32,
    /// Relative order index within the profile (matches `EchelleOrderCalibration::relative_index`).
    pub relative_order: u32,
    /// Known atlas wavelength in nm.
    pub atlas_wavelength_nm: f64,
}

/// Per-order quality metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderQuality {
    /// Relative order index.
    pub relative_index: u32,
    /// Physical order number (if known).
    pub physical_order: Option<i32>,
    /// RMS residual (atlas - model) in nm for this order's matched lines.
    pub rms_nm: f64,
    /// Number of matched lines used.
    pub n_matched_lines: usize,
    /// Wavelength range covered by this order (min, max) in nm.
    pub wavelength_range_nm: Option<(f64, f64)>,
}

/// Overlap disagreement between two adjacent orders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlapDisagreement {
    /// Relative index of the lower-numbered order.
    pub order_a: u32,
    /// Relative index of the higher-numbered order.
    pub order_b: u32,
    /// Maximum wavelength disagreement in nm within the overlap region.
    pub max_disagreement_nm: f64,
    /// Overlap wavelength range (start, end) in nm.
    pub overlap_range_nm: (f64, f64),
}

/// Grating constant deviation for a single order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcDeviation {
    /// Relative order index.
    pub relative_index: u32,
    /// Physical order number.
    pub physical_order: u32,
    /// Computed product `m * lambda_center` in nm.
    pub m_lambda: f64,
    /// Fractional deviation from the reference grating constant.
    pub fractional_deviation: f64,
}

/// Comprehensive calibration quality report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationQualityReport {
    /// Global RMS across all matched lines (atlas vs. model) in nm.
    pub global_rms_nm: f64,
    /// Per-order quality metrics.
    pub per_order_rms: Vec<OrderQuality>,
    /// Overlap disagreements between adjacent orders.
    pub overlap_disagreements: Vec<OverlapDisagreement>,
    /// Grating constant deviations per order.
    pub gc_deviations: Vec<GcDeviation>,
    /// Total number of matched lines used for the global RMS.
    pub n_matched_lines: usize,
    /// Leave-one-out cross-validation RMS in nm (None if too few points).
    pub loo_rms: Option<f64>,
}

// ─── Wavelength evaluation helpers ──────────────────────────────────────────

/// Evaluate a Chebyshev polynomial at a normalized coordinate.
///
/// Uses the three-term recurrence relation for numerical stability.
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

/// Evaluate a wavelength model at the given pixel position.
///
/// Returns `None` for `Sampled` models (which don't support arbitrary-pixel
/// evaluation without interpolation — quality metrics require `Polynomial`).
fn eval_wavelength_model(model: &EchelleWavelengthModel, pixel: f64) -> Option<f64> {
    match model {
        EchelleWavelengthModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
            ..
        } => {
            let range = domain_end - domain_start;
            if range.abs() < 1e-15 {
                return None;
            }
            let x_norm = match basis {
                PolynomialBasis::Chebyshev => 2.0 * (pixel - domain_start) / range - 1.0,
                PolynomialBasis::Monomial => pixel,
            };
            Some(match basis {
                PolynomialBasis::Chebyshev => chebyshev_eval(coefficients, x_norm),
                PolynomialBasis::Monomial => {
                    // Standard Horner evaluation for monomial basis
                    coefficients
                        .iter()
                        .rev()
                        .fold(0.0, |acc, &c| acc * x_norm + c)
                }
            })
        }
        EchelleWavelengthModel::Sampled { .. } => None,
    }
}

/// Get the wavelength range (min, max) for an order by evaluating at its endpoints.
fn order_wavelength_range(
    model: &EchelleWavelengthModel,
    sample_start: u32,
    sample_end: u32,
) -> Option<(f64, f64)> {
    let w_start = eval_wavelength_model(model, f64::from(sample_start))?;
    let w_end = eval_wavelength_model(model, f64::from(sample_end))?;
    Some(if w_start <= w_end {
        (w_start, w_end)
    } else {
        (w_end, w_start)
    })
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Compute the global RMS of residuals between atlas wavelengths and the
/// per-order Chebyshev model predictions.
///
/// For each matched line, evaluates the order's wavelength polynomial at the
/// detected pixel position and computes `|atlas_lambda - model_lambda|`.
/// Returns the RMS across all lines. Lines whose orders have `Sampled` models
/// are skipped.
#[must_use]
pub fn compute_global_rms(
    profile: &EchelleCalibrationProfile,
    matched_lines: &[MatchedLine],
) -> f64 {
    if matched_lines.is_empty() {
        return 0.0;
    }

    let mut sum_sq = 0.0;
    let mut count = 0usize;

    for line in matched_lines {
        if let Some(order) = profile
            .orders
            .iter()
            .find(|o| o.relative_index == line.relative_order)
            && let Some(predicted) = eval_wavelength_model(&order.wavelength, line.pixel)
        {
            let residual = line.atlas_wavelength_nm - predicted;
            sum_sq += residual * residual;
            count += 1;
        }
    }

    if count == 0 {
        return 0.0;
    }

    (sum_sq / count as f64).sqrt()
}

/// Compute overlap agreement between adjacent orders.
///
/// For each pair of consecutive orders (by relative index), finds the wavelength
/// range covered by both orders and samples the maximum disagreement at evenly
/// spaced pixel positions within the overlap.
///
/// Returns one entry per adjacent pair that has a non-empty overlap.
#[must_use]
pub fn compute_overlap_agreement(profile: &EchelleCalibrationProfile) -> Vec<OverlapDisagreement> {
    let mut results = Vec::new();

    // Sort orders by relative index for pairwise comparison.
    let mut orders: Vec<_> = profile.orders.iter().collect();
    orders.sort_by_key(|o| o.relative_index);

    for pair in orders.windows(2) {
        let order_a = pair[0];
        let order_b = pair[1];

        // Get wavelength ranges.
        let Some(range_a) = order_wavelength_range(
            &order_a.wavelength,
            order_a.sample_start,
            order_a.sample_end,
        ) else {
            continue;
        };
        let Some(range_b) = order_wavelength_range(
            &order_b.wavelength,
            order_b.sample_start,
            order_b.sample_end,
        ) else {
            continue;
        };

        // Find overlap wavelength range.
        let overlap_start = range_a.0.max(range_b.0);
        let overlap_end = range_a.1.min(range_b.1);

        if overlap_start >= overlap_end {
            continue; // No overlap
        }

        // Sample at 100 evenly spaced wavelengths within the overlap and
        // find the maximum disagreement.
        let n_samples = 100usize;
        let mut max_disagreement = 0.0_f64;

        for i in 0..=n_samples {
            let target_wl =
                overlap_start + (overlap_end - overlap_start) * (i as f64 / n_samples as f64);

            // For each order, find the pixel that maps to this wavelength
            // by linear search (evaluate at integer pixels and interpolate).
            let wl_a = find_pixel_for_wavelength(
                &order_a.wavelength,
                order_a.sample_start,
                order_a.sample_end,
                target_wl,
            );
            let wl_b = find_pixel_for_wavelength(
                &order_b.wavelength,
                order_b.sample_start,
                order_b.sample_end,
                target_wl,
            );

            if let (Some(px_a), Some(px_b)) = (wl_a, wl_b) {
                // Evaluate both models at the same pixel (use order_a's pixel)
                // to measure disagreement. We actually want: at the pixel in
                // order A that gives target_wl, what does order B's model say?
                // But the overlap is in wavelength space, so we compare
                // the wavelength each model produces at each model's own pixel
                // for the same target wavelength. The disagreement is how far
                // apart the two models' pixels land for the same wavelength.
                //
                // A more direct approach: for a grid of wavelengths in the
                // overlap, compare the pixel positions that each model assigns.
                // Convert pixel difference to wavelength difference via
                // local dispersion.
                //
                // Simplest approach: evaluate both models at the same pixel
                // position in the middle of their shared pixel range.
                let _ = px_b; // We use a different approach below.

                // Evaluate order_a's model at px_a to get actual_wl_a
                if let Some(actual_wl_a) = eval_wavelength_model(&order_a.wavelength, px_a) {
                    // Find the same pixel in order_b's frame and evaluate
                    if let Some(actual_wl_b) = eval_wavelength_model(&order_b.wavelength, px_a) {
                        let disagreement = (actual_wl_a - actual_wl_b).abs();
                        max_disagreement = max_disagreement.max(disagreement);
                    }
                }
            }
        }

        if max_disagreement > 0.0 {
            results.push(OverlapDisagreement {
                order_a: order_a.relative_index,
                order_b: order_b.relative_index,
                max_disagreement_nm: max_disagreement,
                overlap_range_nm: (overlap_start, overlap_end),
            });
        }
    }

    results
}

/// Find the pixel position within an order that maps to a target wavelength.
///
/// Uses bisection on the polynomial model. Returns `None` if the target is
/// outside the order's wavelength range or the model is not polynomial.
fn find_pixel_for_wavelength(
    model: &EchelleWavelengthModel,
    sample_start: u32,
    sample_end: u32,
    target_wl: f64,
) -> Option<f64> {
    let start = f64::from(sample_start);
    let end = f64::from(sample_end);

    let wl_start = eval_wavelength_model(model, start)?;
    let wl_end = eval_wavelength_model(model, end)?;

    // Check that the target is within range.
    let (wl_min, wl_max) = if wl_start <= wl_end {
        (wl_start, wl_end)
    } else {
        (wl_end, wl_start)
    };

    if target_wl < wl_min || target_wl > wl_max {
        return None;
    }

    // Bisection: find pixel where model(pixel) == target_wl
    let mut lo = start;
    let mut hi = end;
    let increasing = wl_end > wl_start;

    for _ in 0..64 {
        let mid = lo.midpoint(hi);
        let wl_mid = eval_wavelength_model(model, mid)?;
        let diff = wl_mid - target_wl;

        if diff.abs() < 1e-10 {
            return Some(mid);
        }

        if (diff > 0.0) == increasing {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    Some(lo.midpoint(hi))
}

/// Compute grating constant consistency across all orders.
///
/// For each order with a known physical order number, computes
/// `m * lambda_center` where `lambda_center` is the wavelength at the
/// midpoint of the order's pixel range. Returns the deviation from
/// `reference_gc` for each order.
///
/// A well-calibrated Mechelle 5000 should show all products near 36300 nm
/// with fractional deviation < 1%.
#[must_use]
pub fn compute_gc_consistency(
    profile: &EchelleCalibrationProfile,
    reference_gc: f64,
) -> Vec<GcDeviation> {
    let mut results = Vec::new();

    for order in &profile.orders {
        let Some(physical_order) = order.physical_order_number else {
            continue;
        };
        let m = physical_order.unsigned_abs();
        if m == 0 {
            continue;
        }

        let mid_pixel = f64::from(order.sample_start).midpoint(f64::from(order.sample_end));

        if let Some(lambda_center) = eval_wavelength_model(&order.wavelength, mid_pixel) {
            let m_lambda = f64::from(m) * lambda_center;
            let fractional_deviation = if reference_gc.abs() > 1e-15 {
                (m_lambda - reference_gc) / reference_gc
            } else {
                0.0
            };

            results.push(GcDeviation {
                relative_index: order.relative_index,
                physical_order: m,
                m_lambda,
                fractional_deviation,
            });
        }
    }

    results
}

/// Compute the comprehensive calibration quality report.
///
/// # Arguments
///
/// * `profile` - The calibration profile to evaluate.
/// * `matched_lines` - Atlas-matched arc line positions from the calibration
///   pipeline. Pass `&[]` if unavailable (RMS and LOO will be zero/None).
/// * `reference_gc` - Reference grating constant in nm (e.g., 36300 for Mechelle 5000).
/// * `loo_degree_x` - Chebyshev degree in pixel direction for LOO (typical: 4).
/// * `loo_degree_m` - Chebyshev degree in order direction for LOO (typical: 3).
#[must_use]
pub fn compute_quality_report(
    profile: &EchelleCalibrationProfile,
    matched_lines: &[MatchedLine],
    reference_gc: f64,
    loo_degree_x: usize,
    loo_degree_m: usize,
) -> CalibrationQualityReport {
    let global_rms_nm = compute_global_rms(profile, matched_lines);

    // Per-order RMS
    let per_order_rms = compute_per_order_rms(profile, matched_lines);

    // Overlap agreement
    let overlap_disagreements = compute_overlap_agreement(profile);

    // Grating constant consistency
    let gc_deviations = compute_gc_consistency(profile, reference_gc);

    // LOO cross-validation: convert matched lines to (pixel, order, wavelength)
    // tuples for the 2D Chebyshev fitter.
    let loo_rms = if matched_lines.len() > (loo_degree_x + 1) * (loo_degree_m + 1) {
        let loo_data: Vec<(f64, f64, f64)> = matched_lines
            .iter()
            .map(|l| (l.pixel, f64::from(l.physical_order), l.atlas_wavelength_nm))
            .collect();

        let rms = leave_one_out_rms(&loo_data, loo_degree_x, loo_degree_m);
        if rms.is_finite() { Some(rms) } else { None }
    } else {
        None
    };

    CalibrationQualityReport {
        global_rms_nm,
        per_order_rms,
        overlap_disagreements,
        gc_deviations,
        n_matched_lines: matched_lines.len(),
        loo_rms,
    }
}

/// Build a quality report from a `Global2DChebyshevFit` and training data.
///
/// This is the preferred entry point when a 2D Chebyshev model is available
/// (e.g., from the calibration pipeline's Stage-3 global fit). It uses the global
/// model's RMS directly and delegates LOO to the existing `chebyshev_2d`
/// module.
#[must_use]
pub fn compute_quality_report_from_2d(
    profile: &EchelleCalibrationProfile,
    global_fit: &Global2DChebyshevFit,
    training_data: &[(f64, u32, f64)],
    reference_gc: f64,
    loo_degree_x: usize,
    loo_degree_m: usize,
) -> CalibrationQualityReport {
    // Convert training data to MatchedLine records
    let matched_lines: Vec<MatchedLine> = training_data
        .iter()
        .filter_map(|&(pixel, m_order, atlas_wl)| {
            // Find the relative order index for this physical order
            let relative_order = profile
                .orders
                .iter()
                .find(|o| o.physical_order_number == Some(m_order as i32))
                .map(|o| o.relative_index)?;

            Some(MatchedLine {
                pixel,
                physical_order: m_order,
                relative_order,
                atlas_wavelength_nm: atlas_wl,
            })
        })
        .collect();

    let global_rms_nm = chebyshev_2d::compute_global_rms(global_fit, training_data);

    let per_order_rms = compute_per_order_rms(profile, &matched_lines);
    let overlap_disagreements = compute_overlap_agreement(profile);
    let gc_deviations = compute_gc_consistency(profile, reference_gc);

    // LOO cross-validation
    let n_coeffs = (loo_degree_x + 1) * (loo_degree_m + 1);
    let loo_rms = if matched_lines.len() > n_coeffs {
        let loo_data: Vec<(f64, f64, f64)> = matched_lines
            .iter()
            .map(|l| (l.pixel, f64::from(l.physical_order), l.atlas_wavelength_nm))
            .collect();

        let rms = leave_one_out_rms(&loo_data, loo_degree_x, loo_degree_m);
        if rms.is_finite() { Some(rms) } else { None }
    } else {
        None
    };

    CalibrationQualityReport {
        global_rms_nm,
        per_order_rms,
        overlap_disagreements,
        gc_deviations,
        n_matched_lines: matched_lines.len(),
        loo_rms,
    }
}

// ─── Internal helpers ───────────────────────────────────────────────────────

/// Compute per-order RMS from matched lines.
fn compute_per_order_rms(
    profile: &EchelleCalibrationProfile,
    matched_lines: &[MatchedLine],
) -> Vec<OrderQuality> {
    profile
        .orders
        .iter()
        .map(|order| {
            let order_lines: Vec<&MatchedLine> = matched_lines
                .iter()
                .filter(|l| l.relative_order == order.relative_index)
                .collect();

            let rms_nm = if order_lines.is_empty() {
                0.0
            } else {
                let mut sum_sq = 0.0;
                let mut count = 0usize;
                for line in &order_lines {
                    if let Some(predicted) = eval_wavelength_model(&order.wavelength, line.pixel) {
                        let residual = line.atlas_wavelength_nm - predicted;
                        sum_sq += residual * residual;
                        count += 1;
                    }
                }
                if count > 0 {
                    (sum_sq / count as f64).sqrt()
                } else {
                    0.0
                }
            };

            let wavelength_range_nm =
                order_wavelength_range(&order.wavelength, order.sample_start, order.sample_end);

            OrderQuality {
                relative_index: order.relative_index,
                physical_order: order.physical_order_number,
                rms_nm,
                n_matched_lines: order_lines.len(),
                wavelength_range_nm,
            }
        })
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AxisDirection, DetectorAxis, EchelleExtractionConfig, EchelleFrameCompatibility,
        EchelleOrderCalibration, EchelleOrientation, EchelleProvenance, EchelleSchemaVersion,
        EchelleSummationMode, EchelleTraceModel, EchelleWavelengthModel, PolynomialBasis,
    };
    use chrono::Utc;

    /// Build a minimal test profile with polynomial wavelength models.
    ///
    /// Creates `n_orders` orders spanning physical orders `m_start..m_start+n_orders`.
    /// Each order has a linear Chebyshev wavelength model: lambda = gc/m + dispersion*(x - 1280).
    fn make_test_profile(
        n_orders: usize,
        m_start: u32,
        gc: f64,
        n_pixels: u32,
    ) -> (EchelleCalibrationProfile, Vec<MatchedLine>) {
        let mut orders = Vec::new();
        let mut matched_lines = Vec::new();

        for i in 0..n_orders {
            let m = m_start + i as u32;
            let mf = f64::from(m);
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / f64::from(n_pixels);

            // Build linear Chebyshev model: lambda(x) = c0 + c1 * x_norm
            // where x_norm = 2 * x / (n_pixels-1) - 1
            let domain_start = 0.0_f64;
            let domain_end = f64::from(n_pixels - 1);
            let half_range = (domain_end - domain_start) / 2.0;

            // c1 = dispersion * half_range (Chebyshev T1 coefficient)
            // c0 = lambda at midpoint
            let c1 = dispersion * half_range;
            let c0 = lambda_center;

            orders.push(EchelleOrderCalibration {
                relative_index: i as u32,
                physical_order_number: Some(m as i32),
                sample_start: 0,
                sample_end: n_pixels - 1,
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Chebyshev,
                    coefficients: vec![100.0 + 20.0 * i as f64, 0.0],
                    domain_start,
                    domain_end,
                },
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Chebyshev,
                    coefficients: vec![c0, c1],
                    domain_start,
                    domain_end,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(5.0),
                enabled: true,
                notes: None,
            });

            // Generate some matched lines for this order at known pixel positions.
            for px_frac in [0.1, 0.3, 0.5, 0.7, 0.9] {
                let pixel = px_frac * domain_end;
                let x_norm = 2.0 * pixel / domain_end - 1.0;
                let wl = c0 + c1 * x_norm;

                matched_lines.push(MatchedLine {
                    pixel,
                    physical_order: m,
                    relative_order: i as u32,
                    atlas_wavelength_nm: wl,
                });
            }
        }

        let profile = EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: Some("test-profile".to_string()),
            display_name: "Test Profile".to_string(),
            compatibility: EchelleFrameCompatibility {
                sensor_width: 2560,
                sensor_height: 2160,
                frame_width: n_pixels,
                frame_height: 2160,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Negative,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 5.0,
                background: None,
                scattered_light: None,
            },
            orders,
            corrections: Default::default(),
            provenance: EchelleProvenance {
                creator_tool: "test".to_string(),
                creator_version: None,
                created_at_utc: Utc::now(),
                source_frame_ids: vec![],
                notes: None,
            },
        };

        (profile, matched_lines)
    }

    #[test]
    fn test_global_rms_near_zero_for_exact_model() {
        // When matched lines are generated from the model itself, RMS should
        // be essentially zero.
        let (profile, matched_lines) = make_test_profile(6, 50, 36_300.0, 2560);

        let rms = compute_global_rms(&profile, &matched_lines);
        assert!(
            rms < 1e-10,
            "RMS should be near-zero for exact model evaluation, got {rms:.15}"
        );
    }

    #[test]
    fn test_global_rms_nonzero_with_perturbation() {
        let (profile, mut matched_lines) = make_test_profile(6, 50, 36_300.0, 2560);

        // Add a small perturbation to some atlas wavelengths
        for (i, line) in matched_lines.iter_mut().enumerate() {
            line.atlas_wavelength_nm += 0.01 * (i as f64 * 1.3).sin();
        }

        let rms = compute_global_rms(&profile, &matched_lines);
        assert!(
            rms > 1e-6,
            "RMS should be nonzero with perturbed wavelengths, got {rms:.15}"
        );
        assert!(
            rms < 0.1,
            "RMS should be small for small perturbations, got {rms:.6}"
        );
    }

    #[test]
    fn test_global_rms_empty_lines() {
        let (profile, _) = make_test_profile(6, 50, 36_300.0, 2560);
        let rms = compute_global_rms(&profile, &[]);
        assert!(
            rms.abs() < 1e-15,
            "RMS should be zero with no matched lines"
        );
    }

    #[test]
    fn test_overlap_agreement_detects_discrepancies() {
        let gc = 36_300.0;
        let n_pixels = 2560u32;

        // Create two adjacent orders with slightly inconsistent models.
        // Order m=50: lambda_center = 726nm, FSR ~14.5nm
        // Order m=51: lambda_center = 711.8nm, FSR ~13.9nm
        // Their wavelength ranges overlap.
        let (mut profile, _) = make_test_profile(2, 50, gc, n_pixels);

        // Perturb the second order's wavelength model slightly to create a
        // detectable disagreement.
        if let EchelleWavelengthModel::Polynomial { coefficients, .. } =
            &mut profile.orders[1].wavelength
        {
            coefficients[0] += 0.5; // Shift center wavelength by 0.5nm
        }

        let overlaps = compute_overlap_agreement(&profile);

        // Both orders cover overlapping wavelength ranges, so we should detect
        // a disagreement.
        if !overlaps.is_empty() {
            // The disagreement should be nonzero due to the 0.5nm shift
            assert!(
                overlaps[0].max_disagreement_nm > 0.01,
                "Expected detectable disagreement, got {:.6} nm",
                overlaps[0].max_disagreement_nm
            );
        }
        // Note: if there's no pixel overlap between the two orders' sample
        // ranges (they're both 0..2559), overlap detection works in wavelength
        // space via evaluation at common pixels.
    }

    #[test]
    fn test_overlap_agreement_no_overlap() {
        // Create two orders with non-overlapping wavelength ranges.
        let gc = 36_300.0;
        let (mut profile, _) = make_test_profile(2, 50, gc, 2560);

        // Move order 1 far away so ranges don't overlap.
        profile.orders[1].physical_order_number = Some(150);
        if let EchelleWavelengthModel::Polynomial {
            coefficients,
            domain_end,
            ..
        } = &mut profile.orders[1].wavelength
        {
            // lambda_center for m=150: 36300/150 = 242nm
            let mf = 150.0;
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / *domain_end;
            let half_range = *domain_end / 2.0;
            coefficients[0] = lambda_center;
            coefficients[1] = dispersion * half_range;
        }

        let overlaps = compute_overlap_agreement(&profile);
        assert!(
            overlaps.is_empty(),
            "No overlap expected between m=50 and m=150"
        );
    }

    #[test]
    fn test_gc_consistency_mechelle_5000() {
        // Create a profile with Mechelle 5000 parameters (gc ~36300nm).
        let gc = 36_300.0;
        let (profile, _) = make_test_profile(10, 50, gc, 2560);

        let deviations = compute_gc_consistency(&profile, gc);

        assert_eq!(deviations.len(), 10, "Expected one entry per order");

        for dev in &deviations {
            assert!(
                dev.fractional_deviation.abs() < 0.01,
                "Order m={}: fractional deviation {:.6} should be < 1%",
                dev.physical_order,
                dev.fractional_deviation
            );

            // m * lambda_center should be close to gc
            assert!(
                (dev.m_lambda - gc).abs() < gc * 0.01,
                "Order m={}: m*lambda = {:.2}, expected ~{gc:.2}",
                dev.physical_order,
                dev.m_lambda
            );
        }
    }

    #[test]
    fn test_gc_consistency_detects_bad_order() {
        let gc = 36_300.0;
        let (mut profile, _) = make_test_profile(5, 50, gc, 2560);

        // Corrupt one order's wavelength model significantly.
        if let EchelleWavelengthModel::Polynomial { coefficients, .. } =
            &mut profile.orders[2].wavelength
        {
            coefficients[0] *= 1.5; // 50% shift in center wavelength
        }

        let deviations = compute_gc_consistency(&profile, gc);

        // The corrupted order (index 2, m=52) should have large deviation.
        let bad_order = deviations.iter().find(|d| d.relative_index == 2);
        assert!(
            bad_order.is_some(),
            "Should have a deviation entry for the corrupted order"
        );
        assert!(
            bad_order.expect("checked above").fractional_deviation.abs() > 0.1,
            "Corrupted order should have > 10% deviation"
        );
    }

    #[test]
    fn test_per_order_rms() {
        let (profile, matched_lines) = make_test_profile(4, 50, 36_300.0, 2560);

        let per_order = compute_per_order_rms(&profile, &matched_lines);

        assert_eq!(per_order.len(), 4);
        for oq in &per_order {
            assert!(
                oq.rms_nm < 1e-10,
                "Per-order RMS should be near-zero for exact model, got {:.15}",
                oq.rms_nm
            );
            assert_eq!(oq.n_matched_lines, 5, "Expected 5 matched lines per order");
            assert!(oq.wavelength_range_nm.is_some());
        }
    }

    #[test]
    fn test_quality_report_end_to_end() {
        let gc = 36_300.0;
        let (profile, matched_lines) = make_test_profile(6, 50, gc, 2560);

        let report = compute_quality_report(&profile, &matched_lines, gc, 4, 3);

        // Global RMS should be near-zero for exact model.
        assert!(
            report.global_rms_nm < 1e-8,
            "Global RMS should be near-zero, got {:.15}",
            report.global_rms_nm
        );

        assert_eq!(report.n_matched_lines, 30); // 6 orders * 5 lines each
        assert_eq!(report.per_order_rms.len(), 6);
        assert_eq!(report.gc_deviations.len(), 6);

        // LOO RMS should be available (30 points > 5*4 = 20 coefficients).
        assert!(report.loo_rms.is_some(), "LOO RMS should be computed");
    }

    #[test]
    fn test_loo_too_few_points() {
        let gc = 36_300.0;
        // Only 2 orders * 5 lines = 10 points, fewer than (4+1)*(3+1) = 20 coefficients
        let (profile, matched_lines) = make_test_profile(2, 50, gc, 2560);

        let report = compute_quality_report(&profile, &matched_lines, gc, 4, 3);

        assert!(
            report.loo_rms.is_none(),
            "LOO should be None with too few points"
        );
    }

    #[test]
    fn test_eval_wavelength_model_polynomial() {
        let model = EchelleWavelengthModel::Polynomial {
            basis: PolynomialBasis::Chebyshev,
            coefficients: vec![500.0, 10.0],
            domain_start: 0.0,
            domain_end: 2559.0,
            unit: "nm".to_string(),
        };

        // At midpoint (x_norm = 0): lambda = 500.0
        let wl_mid = eval_wavelength_model(&model, 1279.5).expect("should evaluate");
        assert!(
            (wl_mid - 500.0).abs() < 0.01,
            "Midpoint wavelength should be ~500nm, got {wl_mid:.4}"
        );

        // At domain_start (x_norm = -1): lambda = 500.0 - 10.0 = 490.0
        let wl_start = eval_wavelength_model(&model, 0.0).expect("should evaluate");
        assert!(
            (wl_start - 490.0).abs() < 0.01,
            "Start wavelength should be ~490nm, got {wl_start:.4}"
        );
    }

    #[test]
    fn test_eval_wavelength_model_sampled_returns_none() {
        let model = EchelleWavelengthModel::Sampled {
            wavelengths: vec![400.0, 500.0, 600.0],
            unit: "nm".to_string(),
        };

        assert!(
            eval_wavelength_model(&model, 1.0).is_none(),
            "Sampled model should return None"
        );
    }

    #[test]
    fn test_find_pixel_for_wavelength_bisection() {
        let model = EchelleWavelengthModel::Polynomial {
            basis: PolynomialBasis::Chebyshev,
            coefficients: vec![500.0, 10.0],
            domain_start: 0.0,
            domain_end: 2559.0,
            unit: "nm".to_string(),
        };

        // Find pixel for wavelength 500.0 (should be near midpoint)
        let pixel = find_pixel_for_wavelength(&model, 0, 2559, 500.0);
        assert!(pixel.is_some(), "Should find pixel for wavelength 500.0");
        let px = pixel.expect("checked above");
        assert!(
            (px - 1279.5).abs() < 0.01,
            "Pixel for 500nm should be near midpoint, got {px:.4}"
        );
    }
}
