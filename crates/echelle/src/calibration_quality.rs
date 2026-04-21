//! Calibration quality metrics for echelle spectrograph wavelength solutions.
//!
//! Diagnostics:
//! - **Global RMS**: arc-peak residuals against the per-order Chebyshev model
//!   (target < 0.1 nm).
//! - **Overlap agreement**: max wavelength disagreement between adjacent orders
//!   in their shared range (should be sub-pixel).
//! - **Grating constant consistency**: `m * λ_center` stays approximately
//!   constant across orders (~36300 nm for Mechelle 5000).
//! - **LOO cross-validation**: leave-one-out RMS via the 2D Chebyshev fitter.

// Pixel/order casts are always lossless for realistic frame sizes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use crate::chebyshev_2d::{self, Global2DChebyshevFit};
use crate::chebyshev_common::chebyshev_eval;
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

/// Evaluate a wavelength model at a pixel position; `None` for `Sampled`.
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
            match basis {
                PolynomialBasis::Chebyshev => {
                    let x_norm = 2.0 * (pixel - domain_start) / range - 1.0;
                    Some(chebyshev_eval(coefficients, x_norm))
                }
                PolynomialBasis::Monomial => Some(
                    coefficients
                        .iter()
                        .rev()
                        .fold(0.0, |acc, &c| acc * pixel + c),
                ),
            }
        }
        EchelleWavelengthModel::Sampled { .. } => None,
    }
}

/// Wavelength range `(min, max)` of an order, derived from model endpoints.
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

/// Global RMS of atlas-vs-model residuals across all matched lines (nm).
///
/// Lines whose orders use `Sampled` models are skipped.
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

/// Max wavelength disagreement between adjacent orders within their overlap.
///
/// For each consecutive pair by relative index, samples 101 wavelengths in the
/// shared range and compares each order's model evaluated at the pixel where
/// order A lands the target wavelength. Returns one entry per non-empty overlap.
#[must_use]
pub fn compute_overlap_agreement(profile: &EchelleCalibrationProfile) -> Vec<OverlapDisagreement> {
    let mut results = Vec::new();

    let mut orders: Vec<_> = profile.orders.iter().collect();
    orders.sort_by_key(|o| o.relative_index);

    for pair in orders.windows(2) {
        let order_a = pair[0];
        let order_b = pair[1];

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

        let overlap_start = range_a.0.max(range_b.0);
        let overlap_end = range_a.1.min(range_b.1);
        if overlap_start >= overlap_end {
            continue;
        }

        let n_samples = 100usize;
        let mut max_disagreement = 0.0_f64;

        for i in 0..=n_samples {
            let target_wl =
                overlap_start + (overlap_end - overlap_start) * (i as f64 / n_samples as f64);

            let Some(px_a) = find_pixel_for_wavelength(
                &order_a.wavelength,
                order_a.sample_start,
                order_a.sample_end,
                target_wl,
            ) else {
                continue;
            };
            // Skip order_b's find-pixel (we measure at px_a for a direct
            // per-pixel wavelength comparison between the two models).
            if let (Some(wl_a), Some(wl_b)) = (
                eval_wavelength_model(&order_a.wavelength, px_a),
                eval_wavelength_model(&order_b.wavelength, px_a),
            ) {
                max_disagreement = max_disagreement.max((wl_a - wl_b).abs());
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

/// Bisect to find the pixel within an order that maps to `target_wl`.
///
/// Returns `None` if the target is outside the order's wavelength range or
/// the model is not polynomial.
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

    let (wl_min, wl_max) = if wl_start <= wl_end {
        (wl_start, wl_end)
    } else {
        (wl_end, wl_start)
    };
    if target_wl < wl_min || target_wl > wl_max {
        return None;
    }

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

/// Grating-constant deviation `m·λ_center` per order vs `reference_gc`.
///
/// `λ_center` is the wavelength at the midpoint of the order's pixel range.
/// A well-calibrated Mechelle 5000 yields fractional deviations below 1 %
/// around a reference of ~36300 nm.
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

/// Build the full calibration quality report from matched arc lines.
///
/// # Arguments
///
/// * `profile` - Calibration profile to evaluate.
/// * `matched_lines` - Atlas-matched arc line positions. Pass `&[]` if
///   unavailable (RMS and LOO degrade to zero / `None`).
/// * `reference_gc` - Reference grating constant in nm (36300 for Mechelle 5000).
/// * `loo_degree_x` - Chebyshev degree along pixel axis for LOO (typical 4).
/// * `loo_degree_m` - Chebyshev degree along order axis for LOO (typical 3).
#[must_use]
pub fn compute_quality_report(
    profile: &EchelleCalibrationProfile,
    matched_lines: &[MatchedLine],
    reference_gc: f64,
    loo_degree_x: usize,
    loo_degree_m: usize,
) -> CalibrationQualityReport {
    CalibrationQualityReport {
        global_rms_nm: compute_global_rms(profile, matched_lines),
        per_order_rms: compute_per_order_rms(profile, matched_lines),
        overlap_disagreements: compute_overlap_agreement(profile),
        gc_deviations: compute_gc_consistency(profile, reference_gc),
        n_matched_lines: matched_lines.len(),
        loo_rms: loo_rms_from_matched(matched_lines, loo_degree_x, loo_degree_m),
    }
}

/// LOO cross-validation RMS from matched lines; `None` when under-determined.
fn loo_rms_from_matched(
    matched_lines: &[MatchedLine],
    loo_degree_x: usize,
    loo_degree_m: usize,
) -> Option<f64> {
    let n_coeffs = (loo_degree_x + 1) * (loo_degree_m + 1);
    if matched_lines.len() <= n_coeffs {
        return None;
    }
    let loo_data: Vec<(f64, f64, f64)> = matched_lines
        .iter()
        .map(|l| (l.pixel, f64::from(l.physical_order), l.atlas_wavelength_nm))
        .collect();
    let rms = leave_one_out_rms(&loo_data, loo_degree_x, loo_degree_m);
    rms.is_finite().then_some(rms)
}

/// Quality report built from a 2D Chebyshev global fit plus training data.
///
/// Preferred over [`compute_quality_report`] when the pipeline's Stage-3 global
/// fit is available: the report's `global_rms_nm` is taken directly from that
/// model, and LOO reuses the shared `chebyshev_2d` path.
#[must_use]
pub fn compute_quality_report_from_2d(
    profile: &EchelleCalibrationProfile,
    global_fit: &Global2DChebyshevFit,
    training_data: &[(f64, u32, f64)],
    reference_gc: f64,
    loo_degree_x: usize,
    loo_degree_m: usize,
) -> CalibrationQualityReport {
    let matched_lines: Vec<MatchedLine> = training_data
        .iter()
        .filter_map(|&(pixel, m_order, atlas_wl)| {
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

    CalibrationQualityReport {
        global_rms_nm: chebyshev_2d::compute_global_rms(global_fit, training_data),
        per_order_rms: compute_per_order_rms(profile, &matched_lines),
        overlap_disagreements: compute_overlap_agreement(profile),
        gc_deviations: compute_gc_consistency(profile, reference_gc),
        n_matched_lines: matched_lines.len(),
        loo_rms: loo_rms_from_matched(&matched_lines, loo_degree_x, loo_degree_m),
    }
}

/// Per-order RMS of atlas-vs-model residuals, with wavelength coverage.
fn compute_per_order_rms(
    profile: &EchelleCalibrationProfile,
    matched_lines: &[MatchedLine],
) -> Vec<OrderQuality> {
    profile
        .orders
        .iter()
        .map(|order| {
            let mut sum_sq = 0.0;
            let mut count = 0usize;
            let mut n_for_order = 0usize;
            for line in matched_lines {
                if line.relative_order != order.relative_index {
                    continue;
                }
                n_for_order += 1;
                if let Some(predicted) = eval_wavelength_model(&order.wavelength, line.pixel) {
                    let residual = line.atlas_wavelength_nm - predicted;
                    sum_sq += residual * residual;
                    count += 1;
                }
            }
            let rms_nm = if count > 0 {
                (sum_sq / count as f64).sqrt()
            } else {
                0.0
            };
            OrderQuality {
                relative_index: order.relative_index,
                physical_order: order.physical_order_number,
                rms_nm,
                n_matched_lines: n_for_order,
                wavelength_range_nm: order_wavelength_range(
                    &order.wavelength,
                    order.sample_start,
                    order.sample_end,
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AxisDirection, DetectorAxis, EchelleExtractionConfig, EchelleFrameCompatibility,
        EchelleOrderCalibration, EchelleOrientation, EchelleProvenance, EchelleSchemaVersion,
        EchelleSummationMode, EchelleTraceModel, EchelleWavelengthModel, PolynomialBasis,
    };
    use chrono::Utc;

    /// Minimal polynomial profile: `n_orders` linear Chebyshev orders starting at `m_start`.
    ///
    /// Each order uses `λ(x) = gc/m + dispersion·(x − midpoint)` with five matched
    /// lines per order at fractional positions `{0.1, 0.3, 0.5, 0.7, 0.9}`.
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

            let domain_start = 0.0_f64;
            let domain_end = f64::from(n_pixels - 1);
            let half_range = (domain_end - domain_start) / 2.0;
            // Chebyshev linear form: c0 = λ at midpoint, c1 = dispersion · half_range.
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
        for (i, line) in matched_lines.iter_mut().enumerate() {
            line.atlas_wavelength_nm += 0.01 * (i as f64 * 1.3).sin();
        }
        let rms = compute_global_rms(&profile, &matched_lines);
        assert!(rms > 1e-6, "expected nonzero RMS, got {rms:.15}");
        assert!(rms < 0.1, "expected small RMS, got {rms:.6}");
    }

    #[test]
    fn test_global_rms_empty_lines() {
        let (profile, _) = make_test_profile(6, 50, 36_300.0, 2560);
        let rms = compute_global_rms(&profile, &[]);
        assert!(rms.abs() < 1e-15);
    }

    #[test]
    fn test_overlap_agreement_detects_discrepancies() {
        let gc = 36_300.0;
        let (mut profile, _) = make_test_profile(2, 50, gc, 2560);
        if let EchelleWavelengthModel::Polynomial { coefficients, .. } =
            &mut profile.orders[1].wavelength
        {
            coefficients[0] += 0.5; // shift λ-center by 0.5 nm
        }
        let overlaps = compute_overlap_agreement(&profile);
        if !overlaps.is_empty() {
            assert!(
                overlaps[0].max_disagreement_nm > 0.01,
                "Expected detectable disagreement, got {:.6} nm",
                overlaps[0].max_disagreement_nm
            );
        }
    }

    #[test]
    fn test_overlap_agreement_no_overlap() {
        let gc = 36_300.0;
        let (mut profile, _) = make_test_profile(2, 50, gc, 2560);

        // Push order 1 to m=150 (λ_center ≈ 242 nm) so ranges no longer overlap m=50.
        profile.orders[1].physical_order_number = Some(150);
        if let EchelleWavelengthModel::Polynomial {
            coefficients,
            domain_end,
            ..
        } = &mut profile.orders[1].wavelength
        {
            let mf = 150.0;
            let lambda_center = gc / mf;
            let fsr = gc / (mf * mf);
            let dispersion = fsr / *domain_end;
            let half_range = *domain_end / 2.0;
            coefficients[0] = lambda_center;
            coefficients[1] = dispersion * half_range;
        }

        assert!(compute_overlap_agreement(&profile).is_empty());
    }

    #[test]
    fn test_gc_consistency_mechelle_5000() {
        let gc = 36_300.0;
        let (profile, _) = make_test_profile(10, 50, gc, 2560);
        let deviations = compute_gc_consistency(&profile, gc);

        assert_eq!(deviations.len(), 10);
        for dev in &deviations {
            assert!(
                dev.fractional_deviation.abs() < 0.01,
                "Order m={}: fractional deviation {:.6} should be < 1%",
                dev.physical_order,
                dev.fractional_deviation
            );
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
        if let EchelleWavelengthModel::Polynomial { coefficients, .. } =
            &mut profile.orders[2].wavelength
        {
            coefficients[0] *= 1.5; // 50 % λ-center shift
        }

        let deviations = compute_gc_consistency(&profile, gc);
        let bad = deviations
            .iter()
            .find(|d| d.relative_index == 2)
            .expect("deviation entry for corrupted order");
        assert!(
            bad.fractional_deviation.abs() > 0.1,
            "Corrupted order should have > 10% deviation"
        );
    }

    #[test]
    fn test_per_order_rms() {
        let (profile, matched_lines) = make_test_profile(4, 50, 36_300.0, 2560);
        let per_order = compute_per_order_rms(&profile, &matched_lines);

        assert_eq!(per_order.len(), 4);
        for oq in &per_order {
            assert!(oq.rms_nm < 1e-10, "got {:.15}", oq.rms_nm);
            assert_eq!(oq.n_matched_lines, 5);
            assert!(oq.wavelength_range_nm.is_some());
        }
    }

    #[test]
    fn test_quality_report_end_to_end() {
        let gc = 36_300.0;
        let (profile, matched_lines) = make_test_profile(6, 50, gc, 2560);
        let report = compute_quality_report(&profile, &matched_lines, gc, 4, 3);

        assert!(
            report.global_rms_nm < 1e-8,
            "got {:.15}",
            report.global_rms_nm
        );
        assert_eq!(report.n_matched_lines, 30); // 6 orders × 5 lines
        assert_eq!(report.per_order_rms.len(), 6);
        assert_eq!(report.gc_deviations.len(), 6);
        // 30 points > (4+1)·(3+1) = 20 coefficients → LOO available.
        assert!(report.loo_rms.is_some());
    }

    #[test]
    fn test_loo_too_few_points() {
        let gc = 36_300.0;
        // 2 × 5 = 10 points, under the 20-coefficient threshold.
        let (profile, matched_lines) = make_test_profile(2, 50, gc, 2560);
        let report = compute_quality_report(&profile, &matched_lines, gc, 4, 3);
        assert!(report.loo_rms.is_none());
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

        let wl_mid = eval_wavelength_model(&model, 1279.5).expect("midpoint");
        assert!((wl_mid - 500.0).abs() < 0.01, "got {wl_mid:.4}");

        let wl_start = eval_wavelength_model(&model, 0.0).expect("start");
        assert!((wl_start - 490.0).abs() < 0.01, "got {wl_start:.4}");
    }

    #[test]
    fn test_eval_wavelength_model_sampled_returns_none() {
        let model = EchelleWavelengthModel::Sampled {
            wavelengths: vec![400.0, 500.0, 600.0],
            unit: "nm".to_string(),
        };
        assert!(eval_wavelength_model(&model, 1.0).is_none());
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
        let px = find_pixel_for_wavelength(&model, 0, 2559, 500.0).expect("bisected");
        assert!((px - 1279.5).abs() < 0.01, "got {px:.4}");
    }
}
