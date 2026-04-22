//! Baseline HgAr DoE matrix metrics (bd-g22gu.2.2.2 step 1).
//!
//! For each of the 9 MCP gain / exposure fixtures committed under
//! `debug/phase5/hgar_matrix/`, run the current Stage 1/2/3 calibration
//! pipeline — echelle-equation seed plus two-phase atlas match — and
//! record per-order and per-frame metrics. The committed DH3P flat at
//! `debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff` supplies trace
//! geometry; HgAr alone does not illuminate enough orders for
//! `detect_orders` to find them (see
//! `calibration_pipeline::run_calibration_pipeline_with_flat` docstring).
//!
//! Output: `debug/phase5/hgar_matrix/baseline_matches.json`.
//!
//! This file is the bar that the optional DTW seed (step 2 of
//! bd-g22gu.2.2.2, gated behind the future `dtw_line_matching` feature)
//! must clear on the `g350_t10000ms` / `g500_t3000ms` frames — the memo's
//! "DTW sweet spot" regime — to justify implementation.

// Rust guideline compliant 2026-02-21

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use echelle::calibration_pipeline::{
    CalibrationPipelineConfig, CalibrationResult, WavelengthSeed,
    run_calibration_pipeline_with_flat,
};
use echelle::rectification::{OrderSpec, RectifyConfig, rectify_order};
use echelle::trace_fitting::{OrderTrace, detect_orders};
use echelle::wavelength_fitting::{AtlasLine, load_hgar_atlas};
use echelle::{
    EchelleCalibrationProfile, EchelleFrameCompatibility, EchelleOrientation,
    EchelleWavelengthModel, PolynomialBasis,
};
use serde::{Deserialize, Serialize};

// ─── Fixture paths (workspace-root relative) ─────────────────────────────────

const FIXTURE_FLAT_REL: &str = "debug/phase5/flats/dh3p_flat_5s_g2000_acc10.tiff";
const FIXTURE_CONFIG_REL: &str = "config/calibration/mechelle_5000.toml";
const MATRIX_DIR_REL: &str = "debug/phase5/hgar_matrix";
const MATRIX_JSON_REL: &str = "debug/phase5/hgar_matrix/matrix.json";
const BASELINE_OUT_REL: &str = "debug/phase5/hgar_matrix/baseline_matches.json";
/// Intermediate-data dump consumed by `plot_baseline.py` (bd-g22gu.2.2.2
/// diagnostic visualisation). Compact JSON, ~2 MB. Not committed if the
/// Git attribute ever changes; for now it's committed alongside the
/// summary file so reviewers can regenerate plots without re-running
/// the integration test.
const BASELINE_DETAILS_REL: &str = "debug/phase5/hgar_matrix/baseline_details.json";

/// Mechelle 5000 / iStar AOI width — matches committed TIFF fixtures.
/// Mismatch means the fixture set was regenerated at a different AOI; fail
/// fast so nobody silently compares against stale numbers.
const EXPECTED_FRAME_WIDTH: u32 = 2560;
/// Mechelle 5000 / iStar AOI height — matches committed TIFF fixtures.
const EXPECTED_FRAME_HEIGHT: u32 = 2160;

/// Frames that the step-3 DTW comparison will be measured against.
/// These are the "DTW sweet spot" per `debug/phase5/memo-dtw-line-id.md`
/// §5: high enough S/N that atlas lines are detectable, MCP gain in the
/// responsive region (≥200). If the baseline itself cannot calibrate
/// these, the whole comparison is meaningless and the test should fail.
const BRIGHT_FRAMES: &[&str] = &["hgar_g350_t10000ms", "hgar_g500_t3000ms"];

// ─── Config-file mirror ──────────────────────────────────────────────────────

/// Subset of `bin/calibrate.rs::CalibrateFileConfig` parsed here so the
/// integration test does not depend on the binary crate. If the TOML
/// schema changes, this struct must track it; the parallel mirror in
/// `echelle_merged_spectrum_regression.rs` must also track it.
#[derive(Debug, Deserialize)]
struct FixtureConfig {
    instrument: InstrumentSection,
    detector: DetectorSection,
    orientation: EchelleOrientation,
    #[serde(default)]
    tuning: TuningSection,
}

#[derive(Debug, Deserialize)]
struct InstrumentSection {
    grating_constant_nm: f64,
    first_physical_order: i32,
    #[serde(default = "default_order_step")]
    order_step: i32,
}

#[derive(Debug, Deserialize)]
struct DetectorSection {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TuningSection {
    trace_min_snr: f64,
    trace_fwhm: f64,
    trace_min_peak_distance: usize,
    arc_sigdetect: f64,
    wl_poly_degree: usize,
    wl_seed_tolerance_nm: f64,
    min_lines_per_order: usize,
    #[serde(default)]
    allow_single_line_fallback: bool,
    max_fit_rms_nm: f64,
}

fn default_order_step() -> i32 {
    1
}

// ─── matrix.json mirror ──────────────────────────────────────────────────────

/// One entry of `debug/phase5/hgar_matrix/matrix.json`.
/// The capture script (`scripts/hardware/capture_hgar_matrix.sh`) writes
/// additional fields (`elapsed_s`, `size_kb`, `captured_at`) that are
/// ignored by `#[serde(deny_unknown_fields = false)]` default behavior.
#[derive(Debug, Deserialize)]
struct MatrixEntry {
    label: String,
    mcp_gain: u32,
    exposure_ms: u32,
    tiff: String,
}

// ─── Output schema ───────────────────────────────────────────────────────────

/// Per-order baseline metrics written to `baseline_matches.json`.
#[derive(Debug, Serialize)]
struct OrderBaseline {
    order_index: u32,
    lines_detected: usize,
    lines_matched: usize,
    lines_used: usize,
    rms_nm: f64,
    success: bool,
    failure_reason: Option<String>,
}

/// Per-frame baseline metrics written to `baseline_matches.json`.
#[derive(Debug, Serialize)]
struct FrameBaseline {
    label: String,
    mcp_gain: u32,
    exposure_ms: u32,
    pipeline_ok: bool,
    pipeline_error: Option<String>,
    n_orders_detected: usize,
    n_orders_calibrated: usize,
    overall_rms_nm: f64,
    total_lines_detected: usize,
    total_lines_matched: usize,
    median_rms_nm: f64,
    per_order: Vec<OrderBaseline>,
}

/// One detected peak in an extracted order, with the fit's best guess at
/// its wavelength and the nearest atlas line within tolerance.
#[derive(Debug, Serialize)]
struct DetectedPeak {
    pixel_center: f64,
    amplitude: f64,
    saturated: bool,
    /// λ evaluated at `pixel_center` through the fitted polynomial.
    fit_wavelength_nm: Option<f64>,
    /// Nearest atlas line within `atlas_match_tolerance_nm`, if any.
    nearest_atlas_nm: Option<f64>,
    nearest_atlas_species: Option<String>,
    nearest_atlas_delta_nm: Option<f64>,
}

/// An atlas line projected back to pixel-space through the fit, for
/// overlaying "where the pipeline thinks this line should be" on the
/// extracted 1D flux plot.
#[derive(Debug, Serialize)]
struct AtlasProjection {
    wavelength_nm: f64,
    species: String,
    strength: f64,
    pixel: Option<f64>,
}

/// Per-order diagnostic panel data consumed by `plot_baseline.py`.
#[derive(Debug, Serialize)]
struct OrderDetails {
    order_index: u32,
    /// Whether the pipeline successfully calibrated this order
    /// (polynomial fit through matched atlas lines satisfied the
    /// `max_fit_rms_nm` and `min_lines_per_order` gates).
    success: bool,
    /// Reason the order failed, if `success == false`. Pulled from the
    /// `OrderDiagnostic.failure_reason` field for uncalibrated traces.
    failure_reason: Option<String>,
    /// For successful orders: `"fit"` (wavelength model came from the
    /// fitted polynomial). For failed orders: `"seed"` (wavelength model
    /// came from the echelle-equation seed `λ(x,m)` used by Stage 1
    /// before the fit failed; this is what DTW would need to beat).
    wavelength_source: &'static str,
    /// Estimated physical order number `m = first_physical_order +
    /// order_step · relative_index` (same convention as Stage 1 seed).
    physical_order_m: i32,
    /// Integer dispersion range extracted from the detector.
    disp_start: u32,
    disp_end: u32,
    rms_nm: f64,
    lines_used: usize,
    lines_detected: usize,
    lines_matched: usize,
    /// Boxcar-summed 1D flux along the rectified order (length
    /// `disp_end - disp_start + 1`).
    flux: Vec<f32>,
    /// Per-pixel wavelength: fit polynomial for success, echelle-equation
    /// seed for failed. Same length as `flux`.
    wavelengths_nm: Vec<f64>,
    detected_peaks: Vec<DetectedPeak>,
    /// All atlas lines whose λ fell inside the order's wavelength range,
    /// projected back to pixel-space through either the fit (success)
    /// or the seed (failed).
    atlas_projections: Vec<AtlasProjection>,
}

/// Per-frame details written to `baseline_details.json`.
#[derive(Debug, Serialize)]
struct FrameDetails {
    label: String,
    mcp_gain: u32,
    exposure_ms: u32,
    /// Tolerance used for `nearest_atlas_*` fields in `DetectedPeak`.
    /// Sourced from `WlFitConfig.seed_tolerance_nm` / the TOML tuning.
    atlas_match_tolerance_nm: f64,
    orders: Vec<OrderDetails>,
}

/// Pair of outputs produced in lock-step by `summarize_frame`.
#[derive(Debug)]
struct FrameOutput {
    baseline: FrameBaseline,
    details: FrameDetails,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Workspace-root-relative path. Integration tests run with CWD set to
/// `crates/integration-tests/`, so the workspace root is two levels up.
fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Load a 16-bit TIFF as row-major `Vec<f32>` with no scaling (u16→f32).
fn load_tiff_u16_as_f32(path: &Path) -> Result<(Vec<f32>, u32, u32)> {
    let img = image::open(path).with_context(|| format!("open {}", path.display()))?;
    let gray = img.into_luma16();
    let (w, h) = gray.dimensions();
    let buf = gray.pixels().map(|p| f32::from(p[0])).collect();
    Ok((buf, w, h))
}

/// Build a pipeline config from the fixture TOML — mirrors the
/// `bin/calibrate.rs::into_pipeline_config` semantics relevant to the
/// arc-calibration stage. Scatter policy: arc OFF (HgAr emission
/// over-subtracts under morph opening), flat ON (MCP halo).
fn build_pipeline_config(cfg: &FixtureConfig, atlas: Vec<AtlasLine>) -> CalibrationPipelineConfig {
    let trace_config = echelle::trace_fitting::TraceFitConfig {
        min_snr: cfg.tuning.trace_min_snr,
        fwhm_gaussian: cfg.tuning.trace_fwhm,
        min_peak_distance: cfg.tuning.trace_min_peak_distance,
        ..Default::default()
    };
    let arc_config = echelle::wavelength_fitting::ArcDetectConfig {
        sigdetect: cfg.tuning.arc_sigdetect,
        ..Default::default()
    };
    let wl_config = echelle::wavelength_fitting::WlFitConfig {
        poly_degree: cfg.tuning.wl_poly_degree,
        seed_tolerance_nm: cfg.tuning.wl_seed_tolerance_nm,
        max_fit_rms_nm: cfg.tuning.max_fit_rms_nm,
        ..Default::default()
    };

    CalibrationPipelineConfig {
        trace_config,
        trace_validation: echelle::trace_validation::TraceValidationConfig::mechelle_5000_istar(),
        arc_config,
        wl_config,
        arc_scatter: None,
        flat_scatter: Some(
            echelle::scattered_light::ScatteredLightConfig::mechelle_5000_istar_flat(),
        ),
        rectify_config: RectifyConfig::default(),
        use_optimal_extraction: false,
        optimal_config: echelle::optimal_extraction::OptimalExtractionConfig::istar_iccd(),
        atlas,
        seed: WavelengthSeed::EchelleEquation {
            grating_constant_nm: cfg.instrument.grating_constant_nm,
            first_physical_order: cfg.instrument.first_physical_order,
            order_step: cfg.instrument.order_step,
            n_pixels: cfg.detector.width,
        },
        frame_compat: EchelleFrameCompatibility {
            sensor_width: cfg.detector.width,
            sensor_height: cfg.detector.height,
            frame_width: cfg.detector.width,
            frame_height: cfg.detector.height,
            roi_x: 0,
            roi_y: 0,
            binning_x: 1,
            binning_y: 1,
            bit_depth: Some(16),
        },
        orientation: cfg.orientation.clone(),
        profile_name: "bd-g22gu.2.2.2 baseline".to_string(),
        min_lines_per_order: cfg.tuning.min_lines_per_order,
        allow_single_line_fallback: cfg.tuning.allow_single_line_fallback,
        hdr_extra_arc_frames: Vec::new(),
        hdr_merge_tol_px: 1.0,
        hdr_prefer_unsaturated: true,
    }
}

// ─── Polynomial & extraction helpers (adapted from
//     echelle_merged_spectrum_regression.rs lines 250–347 / 474–515) ──────────

fn eval_polynomial(
    basis: PolynomialBasis,
    coeffs: &[f64],
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> f64 {
    if coeffs.is_empty() {
        return f64::NAN;
    }
    match basis {
        PolynomialBasis::Monomial => {
            let mut acc = 0.0f64;
            for &c in coeffs.iter().rev() {
                acc = acc * x + c;
            }
            acc
        }
        PolynomialBasis::Chebyshev => {
            let range = domain_end - domain_start;
            if range <= 0.0 {
                return f64::NAN;
            }
            let t = 2.0 * (x - domain_start) / range - 1.0;
            if coeffs.len() == 1 {
                return coeffs[0];
            }
            let mut t0 = 1.0f64;
            let mut t1 = t;
            let mut acc = coeffs[0] * t0 + coeffs[1] * t1;
            for &c in coeffs.iter().skip(2) {
                let tn = 2.0 * t * t1 - t0;
                acc += c * tn;
                t0 = t1;
                t1 = tn;
            }
            acc
        }
    }
}

/// Per-pixel wavelengths along an order by evaluating its stored model at
/// integer pixel positions `[sample_start ..= sample_end]`.
fn eval_order_wavelengths(order: &echelle::EchelleOrderCalibration) -> Vec<f64> {
    let n = (order.sample_end - order.sample_start + 1) as usize;
    let mut wls = Vec::with_capacity(n);
    for px in order.sample_start..=order.sample_end {
        let x = f64::from(px);
        let wl = match &order.wavelength {
            EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
                ..
            } => eval_polynomial(*basis, coefficients, *domain_start, *domain_end, x),
            EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                let idx = (px - order.sample_start) as usize;
                wavelengths.get(idx).copied().unwrap_or(f64::NAN)
            }
        };
        wls.push(wl);
    }
    wls
}

/// Invert a monotone Chebyshev polynomial λ(x) to find the pixel at
/// which λ equals `target_nm`, by bisection. Returns `None` if `target_nm`
/// falls outside the order's λ range (i.e. atlas line not visible on the
/// detector for this order).
fn project_wavelength_to_pixel(
    basis: PolynomialBasis,
    coeffs: &[f64],
    domain_start: f64,
    domain_end: f64,
    pixel_min: f64,
    pixel_max: f64,
    target_nm: f64,
) -> Option<f64> {
    if pixel_min >= pixel_max {
        return None;
    }
    let lambda_lo = eval_polynomial(basis, coeffs, domain_start, domain_end, pixel_min);
    let lambda_hi = eval_polynomial(basis, coeffs, domain_start, domain_end, pixel_max);
    if !lambda_lo.is_finite() || !lambda_hi.is_finite() {
        return None;
    }
    let (lam_min, lam_max) = if lambda_lo <= lambda_hi {
        (lambda_lo, lambda_hi)
    } else {
        (lambda_hi, lambda_lo)
    };
    if target_nm < lam_min || target_nm > lam_max {
        return None;
    }
    let ascending = lambda_hi > lambda_lo;
    let (mut lo, mut hi) = (pixel_min, pixel_max);
    // 64 iterations gives ~sub-picometer precision over 2560 px.
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        let lam_mid = eval_polynomial(basis, coeffs, domain_start, domain_end, mid);
        if (hi - lo).abs() < 1e-6 {
            return Some(mid);
        }
        if (ascending && lam_mid < target_nm) || (!ascending && lam_mid > target_nm) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Simple-sum extraction matching `calibration_pipeline::simple_sum_extract`.
fn simple_sum_extract(rect: &echelle::rectification::RectifiedOrder) -> Vec<f32> {
    let mut flux = vec![0.0f32; rect.n_dispersion];
    for (col, f) in flux.iter_mut().enumerate() {
        for row in 0..rect.n_cross {
            let idx = row * rect.n_dispersion + col;
            *f += rect.data[idx] * rect.mask[idx];
        }
    }
    flux
}

/// Polynomial-model parameters pulled from `EchelleOrderCalibration` so
/// that px → λ and λ → px projections can share a single evaluator.
struct FitParams {
    basis: PolynomialBasis,
    coefficients: Vec<f64>,
    domain_start: f64,
    domain_end: f64,
}

/// Per-pixel wavelength prediction from the Stage-1 echelle-equation seed.
///
/// `λ(x) = λ_center + (x - n/2) · dispersion` where `λ_center = gc/m`
/// and `dispersion = gc/(m²·n)`. This is what the pipeline uses to bootstrap
/// the atlas match before any polynomial fitting; for failed orders it is
/// the best wavelength estimate available and is what DTW would have to
/// improve upon to be justified.
fn seed_wavelength_at(pixel: f64, physical_order_m: i32, gc_nm: f64, n_pixels: u32) -> f64 {
    let m = f64::from(physical_order_m);
    let n = f64::from(n_pixels);
    let lambda_center = gc_nm / m;
    let dispersion = gc_nm / (m * m * n);
    lambda_center + (pixel - 0.5 * n) * dispersion
}

fn seed_project_atlas(
    atlas_nm: f64,
    physical_order_m: i32,
    gc_nm: f64,
    n_pixels: u32,
    px_lo: f64,
    px_hi: f64,
) -> Option<f64> {
    let m = f64::from(physical_order_m);
    let n = f64::from(n_pixels);
    let lambda_center = gc_nm / m;
    let dispersion = gc_nm / (m * m * n);
    if dispersion == 0.0 {
        return None;
    }
    let px = 0.5 * n + (atlas_nm - lambda_center) / dispersion;
    if px >= px_lo && px <= px_hi {
        Some(px)
    } else {
        None
    }
}

/// Evaluate per-pixel wavelengths along an uncalibrated trace using the
/// echelle-equation seed, for the integer dispersion range `[disp_start,
/// disp_end]`.
fn seed_wavelengths_for_range(
    disp_start: u32,
    disp_end: u32,
    physical_order_m: i32,
    gc_nm: f64,
    n_pixels: u32,
) -> Vec<f64> {
    (disp_start..=disp_end)
        .map(|x| seed_wavelength_at(f64::from(x), physical_order_m, gc_nm, n_pixels))
        .collect()
}

/// Extract per-order diagnostic panels covering *every* detected trace
/// (successful and failed) so the plot harness can compare calibrated
/// orders against the ones the pipeline dropped. Failed orders use the
/// echelle-equation seed for their wavelength estimate and atlas
/// projections — that is the baseline DTW would need to beat.
#[allow(clippy::too_many_arguments)]
fn order_details_for_frame(
    arc_frame: &[f32],
    flat_frame: &[f32],
    profile: &EchelleCalibrationProfile,
    rectify_cfg: &RectifyConfig,
    trace_config: &echelle::trace_fitting::TraceFitConfig,
    per_order_diag: &[echelle::calibration_pipeline::OrderDiagnostic],
    atlas: &[AtlasLine],
    atlas_match_tolerance_nm: f64,
    seed_gc_nm: f64,
    seed_first_physical_order: i32,
    seed_order_step: i32,
    seed_n_pixels: u32,
) -> Vec<OrderDetails> {
    // Re-detect traces on the raw flat so failed orders have a trace we
    // can rectify against. The pipeline's internal detection runs on a
    // scatter-subtracted flat; re-running on the raw flat yields nearly-
    // identical traces for high-SNR DH3P continuum. Any count mismatch
    // is absorbed by the `order_index` lookup below.
    let all_traces: Vec<OrderTrace> = detect_orders(
        flat_frame,
        EXPECTED_FRAME_WIDTH,
        EXPECTED_FRAME_HEIGHT,
        trace_config,
    );

    let mut out = Vec::with_capacity(all_traces.len());
    for (idx, trace) in all_traces.iter().enumerate() {
        // Trace count is bounded by detector rows (~2160); u32 fits.
        let relative_index = u32::try_from(idx).unwrap_or(u32::MAX);
        // `relative_index` is a small non-negative integer (<256 typical)
        // so the u32 → i32 conversion is exact. Use `i32::try_from` to
        // stay within the workspace MSRV (1.85) — `u32::cast_signed` is
        // 1.87+.
        let physical_order_m = seed_first_physical_order
            + seed_order_step * i32::try_from(relative_index).unwrap_or(i32::MAX);

        let Some(diag) = per_order_diag
            .iter()
            .find(|d| d.order_index == relative_index)
        else {
            continue;
        };

        // Find the matching profile order (if calibrated) to reuse its
        // polynomial. Failed orders won't have one.
        let profile_order = profile
            .orders
            .iter()
            .find(|o| o.enabled && o.relative_index == relative_index);

        // Dispersion range: use the profile's sample range if we have it
        // (Stage 3 may have trimmed), else the full detector width.
        let (disp_start, disp_end) = profile_order
            .map(|o| (o.sample_start, o.sample_end))
            .unwrap_or((0, EXPECTED_FRAME_WIDTH - 1));

        let spec = OrderSpec {
            trace: &trace.trace,
            disp_start,
            disp_end,
            order_index: relative_index,
        };
        let Some(rect) = rectify_order(
            arc_frame,
            EXPECTED_FRAME_WIDTH,
            EXPECTED_FRAME_HEIGHT,
            &spec,
            rectify_cfg,
        ) else {
            continue;
        };
        let flux = simple_sum_extract(&rect);

        // Build wavelength axis + atlas projection machinery.
        let px_min = f64::from(disp_start);
        let px_max = f64::from(disp_end);

        struct OrderWlContext {
            wavelengths: Vec<f64>,
            atlas_projections: Vec<AtlasProjection>,
            wavelength_source: &'static str,
            fit_params: Option<FitParams>,
        }
        let ctx: OrderWlContext = if let Some(order) = profile_order.filter(|_| diag.success) {
            let wl = eval_order_wavelengths(order);
            let fp = match &order.wavelength {
                EchelleWavelengthModel::Polynomial {
                    basis,
                    coefficients,
                    domain_start,
                    domain_end,
                    ..
                } => Some(FitParams {
                    basis: *basis,
                    coefficients: coefficients.clone(),
                    domain_start: *domain_start,
                    domain_end: *domain_end,
                }),
                EchelleWavelengthModel::Sampled { .. } => None,
            };
            let projections: Vec<AtlasProjection> = fp
                .as_ref()
                .map(|f| {
                    atlas
                        .iter()
                        .map(|a| AtlasProjection {
                            wavelength_nm: a.wavelength_nm,
                            species: a.species.clone(),
                            strength: a.strength,
                            pixel: project_wavelength_to_pixel(
                                f.basis,
                                &f.coefficients,
                                f.domain_start,
                                f.domain_end,
                                px_min,
                                px_max,
                                a.wavelength_nm,
                            ),
                        })
                        .filter(|p| p.pixel.is_some())
                        .collect()
                })
                .unwrap_or_default();
            OrderWlContext {
                wavelengths: wl,
                atlas_projections: projections,
                wavelength_source: "fit",
                fit_params: fp,
            }
        } else {
            // Failed-order path: seed-based wavelength axis + atlas projection.
            let wl = seed_wavelengths_for_range(
                disp_start,
                disp_end,
                physical_order_m,
                seed_gc_nm,
                seed_n_pixels,
            );
            let projections: Vec<AtlasProjection> = atlas
                .iter()
                .map(|a| AtlasProjection {
                    wavelength_nm: a.wavelength_nm,
                    species: a.species.clone(),
                    strength: a.strength,
                    pixel: seed_project_atlas(
                        a.wavelength_nm,
                        physical_order_m,
                        seed_gc_nm,
                        seed_n_pixels,
                        px_min,
                        px_max,
                    ),
                })
                .filter(|p| p.pixel.is_some())
                .collect();
            OrderWlContext {
                wavelengths: wl,
                atlas_projections: projections,
                wavelength_source: "seed",
                fit_params: None,
            }
        };

        if ctx.wavelengths.len() != flux.len() {
            continue;
        }

        // Per-peak annotation: evaluate the chosen wavelength model at
        // each detected line's pixel and pair with nearest atlas within
        // tolerance. This mirrors what the pipeline's match stage would
        // see, so a "matched" annotation for a failed order means the
        // seed *should* have anchored the line but something else
        // (tolerance, RMS gate, min_lines) vetoed it.
        let detected_peaks: Vec<DetectedPeak> = diag
            .detected_lines
            .iter()
            .map(|line| {
                let px = line.pixel_center;
                let fit_lambda = if let Some(f) = ctx.fit_params.as_ref() {
                    if f.coefficients.is_empty() {
                        None
                    } else {
                        let v = eval_polynomial(
                            f.basis,
                            &f.coefficients,
                            f.domain_start,
                            f.domain_end,
                            px,
                        );
                        v.is_finite().then_some(v)
                    }
                } else {
                    let v = seed_wavelength_at(px, physical_order_m, seed_gc_nm, seed_n_pixels);
                    v.is_finite().then_some(v)
                };
                let (nearest_atlas_nm, nearest_atlas_species, nearest_atlas_delta_nm) =
                    if let Some(lambda) = fit_lambda {
                        let best = atlas
                            .iter()
                            .map(|a| (a, (a.wavelength_nm - lambda).abs()))
                            .min_by(|x, y| {
                                x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal)
                            });
                        match best {
                            Some((a, d)) if d <= atlas_match_tolerance_nm => (
                                Some(a.wavelength_nm),
                                Some(a.species.clone()),
                                Some(a.wavelength_nm - lambda),
                            ),
                            _ => (None, None, None),
                        }
                    } else {
                        (None, None, None)
                    };
                DetectedPeak {
                    pixel_center: px,
                    amplitude: line.amplitude,
                    saturated: line.saturated,
                    fit_wavelength_nm: fit_lambda,
                    nearest_atlas_nm,
                    nearest_atlas_species,
                    nearest_atlas_delta_nm,
                }
            })
            .collect();

        out.push(OrderDetails {
            order_index: diag.order_index,
            success: diag.success,
            failure_reason: diag.failure_reason.clone(),
            wavelength_source: ctx.wavelength_source,
            physical_order_m,
            disp_start,
            disp_end,
            rms_nm: diag.rms_nm,
            lines_used: diag.n_lines_used,
            lines_detected: diag.n_lines_detected,
            lines_matched: diag.n_lines_matched,
            flux,
            wavelengths_nm: ctx.wavelengths,
            detected_peaks,
            atlas_projections: ctx.atlas_projections,
        });
    }
    out
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    // `% 2 == 0` rather than `.is_multiple_of(2)` to stay within the
    // workspace MSRV (1.85); `is_multiple_of` stabilised in 1.87.
    #[allow(clippy::manual_is_multiple_of, reason = "MSRV 1.85")]
    if values.len() % 2 == 0 {
        0.5 * (values[mid - 1] + values[mid])
    } else {
        values[mid]
    }
}

fn frame_from_result(entry: &MatrixEntry, result: &CalibrationResult) -> FrameBaseline {
    let per_order: Vec<OrderBaseline> = result
        .per_order_diagnostics
        .iter()
        .map(|d| OrderBaseline {
            order_index: d.order_index,
            lines_detected: d.n_lines_detected,
            lines_matched: d.n_lines_matched,
            lines_used: d.n_lines_used,
            rms_nm: d.rms_nm,
            success: d.success,
            failure_reason: d.failure_reason.clone(),
        })
        .collect();

    let total_lines_detected = per_order.iter().map(|o| o.lines_detected).sum();
    let total_lines_matched = per_order.iter().map(|o| o.lines_matched).sum();
    let mut rms_values: Vec<f64> = per_order
        .iter()
        .filter(|o| o.success && o.rms_nm.is_finite())
        .map(|o| o.rms_nm)
        .collect();
    let median_rms_nm = median(&mut rms_values);

    FrameBaseline {
        label: entry.label.clone(),
        mcp_gain: entry.mcp_gain,
        exposure_ms: entry.exposure_ms,
        pipeline_ok: true,
        pipeline_error: None,
        n_orders_detected: result.n_orders_detected,
        n_orders_calibrated: result.n_orders_calibrated,
        overall_rms_nm: result.overall_rms_nm,
        total_lines_detected,
        total_lines_matched,
        median_rms_nm,
        per_order,
    }
}

fn frame_from_error(entry: &MatrixEntry, error: String) -> FrameBaseline {
    FrameBaseline {
        label: entry.label.clone(),
        mcp_gain: entry.mcp_gain,
        exposure_ms: entry.exposure_ms,
        pipeline_ok: false,
        pipeline_error: Some(error),
        n_orders_detected: 0,
        n_orders_calibrated: 0,
        overall_rms_nm: f64::NAN,
        total_lines_detected: 0,
        total_lines_matched: 0,
        median_rms_nm: f64::NAN,
        per_order: Vec::new(),
    }
}

fn empty_details(entry: &MatrixEntry, tol: f64) -> FrameDetails {
    FrameDetails {
        label: entry.label.clone(),
        mcp_gain: entry.mcp_gain,
        exposure_ms: entry.exposure_ms,
        atlas_match_tolerance_nm: tol,
        orders: Vec::new(),
    }
}

fn summarize_frame(entry: &MatrixEntry, flat: &[f32], cfg: &FixtureConfig) -> FrameOutput {
    let tol = cfg.tuning.wl_seed_tolerance_nm.max(f64::EPSILON);
    let tiff_path = repo_path(&format!("{MATRIX_DIR_REL}/{}", entry.tiff));
    let (arc, w, h) = match load_tiff_u16_as_f32(&tiff_path) {
        Ok(v) => v,
        Err(e) => {
            return FrameOutput {
                baseline: frame_from_error(entry, format!("tiff load: {e:#}")),
                details: empty_details(entry, tol),
            };
        }
    };
    if w != EXPECTED_FRAME_WIDTH || h != EXPECTED_FRAME_HEIGHT {
        return FrameOutput {
            baseline: frame_from_error(
                entry,
                format!(
                    "arc dim mismatch {w}x{h}, expected {EXPECTED_FRAME_WIDTH}x{EXPECTED_FRAME_HEIGHT}"
                ),
            ),
            details: empty_details(entry, tol),
        };
    }

    let atlas = load_hgar_atlas();
    let pipeline_cfg = build_pipeline_config(cfg, atlas.clone());
    match run_calibration_pipeline_with_flat(
        &arc,
        flat,
        EXPECTED_FRAME_WIDTH,
        EXPECTED_FRAME_HEIGHT,
        &pipeline_cfg,
    ) {
        Ok(result) => {
            let orders = order_details_for_frame(
                &arc,
                flat,
                &result.profile,
                &pipeline_cfg.rectify_config,
                &pipeline_cfg.trace_config,
                &result.per_order_diagnostics,
                &atlas,
                tol,
                cfg.instrument.grating_constant_nm,
                cfg.instrument.first_physical_order,
                cfg.instrument.order_step,
                cfg.detector.width,
            );
            FrameOutput {
                baseline: frame_from_result(entry, &result),
                details: FrameDetails {
                    label: entry.label.clone(),
                    mcp_gain: entry.mcp_gain,
                    exposure_ms: entry.exposure_ms,
                    atlas_match_tolerance_nm: tol,
                    orders,
                },
            }
        }
        Err(e) => FrameOutput {
            baseline: frame_from_error(entry, e),
            details: empty_details(entry, tol),
        },
    }
}

// ─── Test ────────────────────────────────────────────────────────────────────

#[test]
fn baseline_hgar_doe_matrix() -> Result<()> {
    // The raw TIFF fixtures under `debug/phase5/` are gitignored
    // (`/debug/**/*.tiff`, Apr 21 cleanup). Only machines that captured or
    // retain them locally (hardware runners, the maintainer's workstation)
    // can exercise this baseline. Skip cleanly when missing so CI stays
    // green on ephemeral runners; the flag-driven per-frame failure path
    // inside `summarize_frame` handles individual arc-tiff absences.
    let flat_path = repo_path(FIXTURE_FLAT_REL);
    if !flat_path.exists() {
        eprintln!("baseline_hgar_doe_matrix: skipping — missing fixture {FIXTURE_FLAT_REL}");
        return Ok(());
    }

    let matrix_raw = fs::read_to_string(repo_path(MATRIX_JSON_REL))
        .with_context(|| format!("read {MATRIX_JSON_REL}"))?;
    let matrix: Vec<MatrixEntry> =
        serde_json::from_str(&matrix_raw).context("parse matrix.json")?;
    if matrix.is_empty() {
        return Err(anyhow!("matrix.json is empty"));
    }

    let (flat, fw, fh) = load_tiff_u16_as_f32(&flat_path)?;
    if fw != EXPECTED_FRAME_WIDTH || fh != EXPECTED_FRAME_HEIGHT {
        return Err(anyhow!(
            "flat dim mismatch {fw}x{fh}, expected {EXPECTED_FRAME_WIDTH}x{EXPECTED_FRAME_HEIGHT}"
        ));
    }

    let cfg_raw = fs::read_to_string(repo_path(FIXTURE_CONFIG_REL))
        .with_context(|| format!("read {FIXTURE_CONFIG_REL}"))?;
    let cfg: FixtureConfig = toml::from_str(&cfg_raw).context("parse mechelle_5000.toml")?;

    let outputs: Vec<FrameOutput> = matrix
        .iter()
        .map(|entry| summarize_frame(entry, &flat, &cfg))
        .collect();
    let baselines: Vec<&FrameBaseline> = outputs.iter().map(|o| &o.baseline).collect();
    let details: Vec<&FrameDetails> = outputs.iter().map(|o| &o.details).collect();

    let out_path = repo_path(BASELINE_OUT_REL);
    let serialized = serde_json::to_string_pretty(&baselines).context("serialize baseline")?;
    fs::write(&out_path, serialized).with_context(|| format!("write {}", out_path.display()))?;

    // Details JSON is machine-consumed (plot_baseline.py); compact form
    // keeps per-order flux arrays from ballooning the file.
    let details_path = repo_path(BASELINE_DETAILS_REL);
    let details_serialized = serde_json::to_string(&details).context("serialize details")?;
    fs::write(&details_path, details_serialized)
        .with_context(|| format!("write {}", details_path.display()))?;

    let n_ok = baselines.iter().filter(|b| b.pipeline_ok).count();
    println!(
        "bd-g22gu.2.2.2 baseline: wrote {} ({} / {} frames pipeline_ok)",
        out_path.display(),
        n_ok,
        baselines.len(),
    );
    for b in &baselines {
        println!(
            "  {: <22} gain={: >3} exp={: >5}ms  pipeline_ok={}  orders={}/{}  lines={}→{}  overall_rms={:.4}  median_rms={:.4}",
            b.label,
            b.mcp_gain,
            b.exposure_ms,
            b.pipeline_ok,
            b.n_orders_calibrated,
            b.n_orders_detected,
            b.total_lines_detected,
            b.total_lines_matched,
            b.overall_rms_nm,
            b.median_rms_nm,
        );
    }

    // Bright-frame sanity gate: the two fixtures step 3 will measure DTW
    // against must successfully calibrate under the current pipeline.
    for label in BRIGHT_FRAMES {
        let frame = baselines
            .iter()
            .find(|b| b.label == *label)
            .ok_or_else(|| anyhow!("bright fixture '{label}' not found in matrix.json"))?;
        if !frame.pipeline_ok {
            return Err(anyhow!(
                "bright fixture '{label}' failed baseline pipeline: {:?}",
                frame.pipeline_error,
            ));
        }
    }

    Ok(())
}
