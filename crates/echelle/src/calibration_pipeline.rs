//! End-to-end echelle calibration pipeline.
//!
//! Orchestrates all echelle building blocks into a single calibration flow:
//! raw arc frame → order detection → extraction → line identification →
//! wavelength solution → `EchelleCalibrationProfile`.
//!
//! # Pipeline stages
//!
//! 1. Scattered light subtraction (optional)
//! 2. Order trace detection
//! 3. Per-order rectification
//! 4. 1D spectral extraction (simple-sum or Horne optimal)
//! 5. Arc emission line detection per order
//! 6. Atlas matching + Chebyshev wavelength fit per order
//! 7. Assembly into an `EchelleCalibrationProfile`

// Numerical code: pixel-index casts are always lossless for realistic frame sizes.
// Order indices are always small enough that u32→i32 won't wrap.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use std::sync::Arc;

use chrono::Utc;

use crate::optimal_extraction::{OptimalExtractionConfig, optimal_extract};
use crate::rectification::{OrderSpec, RectifyConfig, rectify_order};
use crate::scattered_light::{ScatteredLightConfig, TraceInfo, subtract_scattered_light};
use crate::trace_fitting::{OrderTrace, TraceFitConfig, detect_orders};
use crate::types::{
    AxisDirection, DetectorAxis, EchelleCalibrationProfile, EchelleCorrections,
    EchelleExtractionConfig, EchelleFrameCompatibility, EchelleOrderCalibration,
    EchelleOrientation, EchelleProvenance, EchelleSchemaVersion, EchelleSummationMode,
    EchelleWavelengthModel, PolynomialBasis,
};
use crate::wavelength_fitting::{
    ArcDetectConfig, ArcLine, AtlasLine, OrderWlSolution, SingleLineFallbackSeed,
    TwoPhaseMatchConfig, WlFitConfig, detect_arc_lines, fit_order_wavelength, match_lines_to_atlas,
    match_lines_two_phase, merge_arc_lines_hdr,
};

// ─── Configuration ───────────────────────────────────────────────────────────

/// Wavelength seed model for bootstrapping the pixel → wavelength mapping.
///
/// Echelle calibration requires an approximate initial guess of which
/// wavelength range each order covers, so that detected arc lines can be
/// matched to known atlas wavelengths. This enum provides different ways
/// to supply that initial guess.
#[derive(Debug, Clone)]
pub enum WavelengthSeed {
    /// User-identified anchor points: (order_index, pixel, wavelength_nm).
    ///
    /// At least 2 anchors per order are needed for a linear seed model.
    /// Orders without anchors are bootstrapped via the echelle equation
    /// if `echelle_constant_nm` is provided in the pipeline config.
    Anchors(Vec<SeedAnchor>),

    /// Use the echelle grating equation: m × λ_center ≈ constant.
    ///
    /// Given the grating constant and the physical order number of the
    /// first detected order, computes approximate wavelength ranges for
    /// all orders.
    EchelleEquation {
        /// The echelle grating constant in nm (m × λ_center).
        /// For the Mechelle 5000, this is approximately 1_050_000 / grating_density.
        grating_constant_nm: f64,
        /// Physical diffraction order number assigned to detected order index 0
        /// (the order at the smallest cross-dispersion position).
        first_physical_order: i32,
        /// Increment per detected order index. Typically -1 for echelle
        /// spectrographs where higher Y → lower order number → longer wavelength.
        order_step: i32,
        /// Number of dispersion pixels across each order.
        n_pixels: u32,
    },
}

/// A single anchor point mapping a pixel position to a known wavelength.
#[derive(Debug, Clone)]
pub struct SeedAnchor {
    /// Detected order index (0-based, sorted by cross-dispersion position).
    pub order_index: u32,
    /// Pixel position along the dispersion axis.
    pub pixel: f64,
    /// Known wavelength in nm at that pixel.
    pub wavelength_nm: f64,
}

/// Full configuration for the calibration pipeline.
#[derive(Debug, Clone)]
pub struct CalibrationPipelineConfig {
    /// Trace detection parameters.
    pub trace_config: TraceFitConfig,
    /// Post-detection trace validation (all filters disabled by default).
    pub trace_validation: crate::trace_validation::TraceValidationConfig,
    /// Arc line detection parameters.
    pub arc_config: ArcDetectConfig,
    /// Wavelength fitting parameters.
    pub wl_config: WlFitConfig,
    /// Scattered light subtraction (None = skip).
    pub scatter_config: Option<ScatteredLightConfig>,
    /// Order rectification parameters.
    pub rectify_config: RectifyConfig,
    /// Whether to use Horne optimal extraction (true) or simple summation (false).
    pub use_optimal_extraction: bool,
    /// Optimal extraction parameters (only used if `use_optimal_extraction` is true).
    pub optimal_config: OptimalExtractionConfig,
    /// Reference emission line atlas.
    pub atlas: Vec<AtlasLine>,
    /// Seed wavelength model for bootstrapping.
    pub seed: WavelengthSeed,
    /// Frame dimensions and detector metadata.
    pub frame_compat: EchelleFrameCompatibility,
    /// Echelle orientation.
    pub orientation: EchelleOrientation,
    /// Human-readable name for the generated profile.
    pub profile_name: String,
    /// Minimum number of matched lines required per order (default: 3).
    pub min_lines_per_order: usize,
    /// Extra arc lamp frames (same width×height as the primary arc) for HDR-style
    /// line merging: detect per exposure, then merge with [`merge_arc_lines_hdr`].
    ///
    /// Stored as [`Arc`] so cloning [`CalibrationPipelineConfig`] does not duplicate
    /// multi-megapixel buffers.
    pub hdr_extra_arc_frames: Vec<Arc<Vec<f32>>>,
    /// Pixel chaining tolerance passed to [`merge_arc_lines_hdr`].
    pub hdr_merge_tol_px: f64,
    /// Prefer unsaturated centroids when HDR-merge picks a cluster representative.
    pub hdr_prefer_unsaturated: bool,
}

impl Default for CalibrationPipelineConfig {
    fn default() -> Self {
        Self {
            trace_config: TraceFitConfig::default(),
            trace_validation: crate::trace_validation::TraceValidationConfig::default(),
            arc_config: ArcDetectConfig::default(),
            wl_config: WlFitConfig::default(),
            scatter_config: None,
            rectify_config: RectifyConfig::default(),
            use_optimal_extraction: false,
            optimal_config: OptimalExtractionConfig::default(),
            atlas: Vec::new(),
            seed: WavelengthSeed::Anchors(Vec::new()),
            frame_compat: EchelleFrameCompatibility {
                sensor_width: 2048,
                sensor_height: 2048,
                frame_width: 2048,
                frame_height: 2048,
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
            profile_name: "Calibration".to_string(),
            min_lines_per_order: 3,
            hdr_extra_arc_frames: Vec::new(),
            hdr_merge_tol_px: 1.0,
            hdr_prefer_unsaturated: true,
        }
    }
}

// ─── Result types ────────────────────────────────────────────────────────────

/// Diagnostic information for a single order's calibration.
#[derive(Debug, Clone)]
pub struct OrderDiagnostic {
    /// Order index (0-based).
    pub order_index: u32,
    /// Number of arc lines detected in this order.
    pub n_lines_detected: usize,
    /// Number of lines matched to the atlas.
    pub n_lines_matched: usize,
    /// Number of lines used in the final fit (after sigma-clipping).
    pub n_lines_used: usize,
    /// RMS residual of the wavelength fit in nm.
    pub rms_nm: f64,
    /// Whether this order was successfully calibrated.
    pub success: bool,
    /// Reason for failure, if any.
    pub failure_reason: Option<String>,
    /// The detected arc lines (for debugging).
    pub detected_lines: Vec<ArcLine>,
    /// The wavelength solution (if successful).
    pub wl_solution: Option<OrderWlSolution>,
}

/// Result of the full calibration pipeline.
#[derive(Debug, Clone)]
pub struct CalibrationResult {
    /// The assembled calibration profile, ready to save.
    pub profile: EchelleCalibrationProfile,
    /// Per-order diagnostic information.
    pub per_order_diagnostics: Vec<OrderDiagnostic>,
    /// Overall RMS across all successfully calibrated orders (nm).
    pub overall_rms_nm: f64,
    /// Number of orders successfully calibrated.
    pub n_orders_calibrated: usize,
    /// Total number of orders detected.
    pub n_orders_detected: usize,
}

// ─── Pipeline implementation ─────────────────────────────────────────────────

/// Run the full calibration pipeline on an arc lamp frame.
///
/// Takes a raw arc frame (row-major f32 pixels) and produces a calibration
/// profile with per-order wavelength solutions.
///
/// # Errors
///
/// Returns `Err` if no orders are detected or if the seed model cannot
/// generate wavelength estimates.
pub fn run_calibration_pipeline(
    arc_frame: &[f32],
    width: u32,
    height: u32,
    config: &CalibrationPipelineConfig,
) -> Result<CalibrationResult, String> {
    run_calibration_pipeline_impl(arc_frame, None, width, height, config)
}

/// Run the calibration pipeline with a separate flat-field frame for trace detection.
///
/// Uses `flat_frame` to detect order traces (all orders visible with broadband
/// continuum), then extracts and calibrates arc lines from `arc_frame`.
/// This two-frame approach is essential when the arc lamp only illuminates a
/// subset of orders (e.g., HgAr with ~29 lines across ~74 orders).
///
/// # Errors
///
/// Returns `Err` if no orders are detected in the flat frame or if the seed
/// model cannot generate wavelength estimates.
pub fn run_calibration_pipeline_with_flat(
    arc_frame: &[f32],
    flat_frame: &[f32],
    width: u32,
    height: u32,
    config: &CalibrationPipelineConfig,
) -> Result<CalibrationResult, String> {
    run_calibration_pipeline_impl(arc_frame, Some(flat_frame), width, height, config)
}

#[allow(clippy::many_single_char_names)] // Mathematical variable names (a, b, c, w, h, i)
fn run_calibration_pipeline_impl(
    arc_frame: &[f32],
    flat_frame: Option<&[f32]>,
    width: u32,
    height: u32,
    config: &CalibrationPipelineConfig,
) -> Result<CalibrationResult, String> {
    let w = width as usize;
    let h = height as usize;
    if arc_frame.len() < w * h {
        return Err(format!(
            "frame too small: {} pixels for {}x{} = {}",
            arc_frame.len(),
            width,
            height,
            w * h
        ));
    }

    // Validate frame_compat consistency with actual frame dimensions.
    if config.frame_compat.frame_width != width || config.frame_compat.frame_height != height {
        return Err(format!(
            "frame_compat dimensions ({}x{}) do not match actual frame ({}x{})",
            config.frame_compat.frame_width, config.frame_compat.frame_height, width, height
        ));
    }

    // Validate flat frame dimensions if provided.
    if let Some(flat) = flat_frame
        && flat.len() < w * h
    {
        return Err(format!(
            "flat frame too small: {} pixels for {}x{} = {}",
            flat.len(),
            width,
            height,
            w * h
        ));
    }

    for (i, ex) in config.hdr_extra_arc_frames.iter().enumerate() {
        if ex.len() < w * h {
            return Err(format!(
                "HDR extra arc frame {i} too small: {} pixels for {}x{} = {}",
                ex.len(),
                width,
                height,
                w * h
            ));
        }
    }

    // ── Stage 1: Scattered light subtraction (optional) ──────────────
    // When scatter subtraction is enabled, build trace geometry once on the primary
    // arc and reuse it for the primary and every HDR extra so all line-detection
    // sources see consistently corrected (or consistently raw) data.
    let preliminary_traces_for_scatter: Option<Vec<OrderTrace>> = if config.scatter_config.is_some()
    {
        Some(detect_orders(
            arc_frame,
            width,
            height,
            &config.trace_config,
        ))
    } else {
        None
    };

    let trace_infos_for_scatter: Option<Vec<TraceInfo<'_>>> = preliminary_traces_for_scatter
        .as_ref()
        .map(|preliminary_traces| {
            preliminary_traces
                .iter()
                .map(|t| TraceInfo {
                    trace: &t.trace,
                    disp_start: 0,
                    disp_end: width.saturating_sub(1),
                })
                .collect()
        });

    let (working_frame, scatter_correction_active): (Vec<f32>, bool) =
        match (&config.scatter_config, trace_infos_for_scatter.as_ref()) {
            (Some(scatter_cfg), Some(trace_infos)) => {
                if let Some((corrected, _model)) =
                    subtract_scattered_light(arc_frame, width, height, trace_infos, scatter_cfg)
                {
                    (corrected, true)
                } else {
                    // Scattered light subtraction failed (not enough inter-order pixels);
                    // proceed with the raw frame.
                    (arc_frame.to_vec(), false)
                }
            }
            _ => (arc_frame.to_vec(), false),
        };
    let frame_ref: &[f32] = working_frame.as_slice();

    // HDR extras after scatter correction: build owned corrected frames in one pass, then
    // take slices (avoids overlapping &mut vs & across `arc_line_sources` growth).
    let hdr_after_scatter: Vec<Option<Vec<f32>>> = match (
        scatter_correction_active,
        trace_infos_for_scatter.as_ref(),
        config.scatter_config.as_ref(),
    ) {
        (true, Some(trace_infos), Some(scatter_cfg)) => config
            .hdr_extra_arc_frames
            .iter()
            .map(|ex| {
                let ex_data: &[f32] = ex.as_ref();
                subtract_scattered_light(ex_data, width, height, trace_infos, scatter_cfg)
                    .map(|(corrected, _model)| corrected)
            })
            .collect(),
        _ => vec![None; config.hdr_extra_arc_frames.len()],
    };

    let mut arc_line_sources: Vec<&[f32]> =
        Vec::with_capacity(1 + config.hdr_extra_arc_frames.len());
    arc_line_sources.push(frame_ref);

    for (ex, corrected_opt) in config
        .hdr_extra_arc_frames
        .iter()
        .zip(hdr_after_scatter.iter())
    {
        let slice: &[f32] = match corrected_opt {
            Some(v) => v.as_slice(),
            None => ex.as_ref().as_slice(),
        };
        arc_line_sources.push(slice);
    }

    // ── Stage 2: Order trace detection ───────────────────────────────
    // Use flat frame for trace detection if provided (broadband source
    // illuminates all orders); otherwise detect from the arc frame.
    let trace_source = flat_frame.unwrap_or(frame_ref);
    let raw_traces = detect_orders(trace_source, width, height, &config.trace_config);
    if raw_traces.is_empty() {
        return Err(if flat_frame.is_some() {
            "no echelle orders detected in flat frame".to_string()
        } else {
            "no echelle orders detected in frame".to_string()
        });
    }

    // Optionally filter spurious traces.
    let traces = if config.trace_validation.is_empty() {
        raw_traces
    } else {
        let validated = crate::trace_validation::validate_traces(
            &raw_traces,
            &config.trace_validation,
            trace_source,
            width,
            height,
            config.rectify_config.aperture_half_width,
        );
        tracing::info!(
            "trace validation: {} → {} traces",
            raw_traces.len(),
            validated.len()
        );
        validated
    };

    if traces.is_empty() {
        return Err("all traces rejected by validation filters".to_string());
    }
    let n_orders = traces.len();

    // Build seed wavelength functions per order.
    let seed_fns = build_seed_functions(&config.seed, n_orders, width)?;

    // Extract grating constant from seed config (used for physical order assignment).
    let grating_constant_nm = match &config.seed {
        WavelengthSeed::EchelleEquation {
            grating_constant_nm,
            ..
        } => Some(*grating_constant_nm),
        WavelengthSeed::Anchors(_) => None,
    };

    // Build two-phase match config for Pass 1 when using echelle equation seed.
    let two_phase_base = match &config.seed {
        WavelengthSeed::EchelleEquation {
            grating_constant_nm: gc,
            first_physical_order,
            order_step,
            ..
        } => Some((*gc, *first_physical_order, *order_step)),
        WavelengthSeed::Anchors(_) => None,
    };

    // ── Stages 3-6: Per-order processing (Pass 1) ────────────────────
    // Always extract arc lines from the arc frame (frame_ref), using
    // trace positions found from the flat/arc frame above.
    let mut diagnostics = Vec::with_capacity(n_orders);
    let mut order_calibrations = Vec::new();

    for (order_idx, trace) in traces.iter().enumerate() {
        let oi = order_idx as u32;

        let lines =
            match extract_and_detect_lines(&arc_line_sources, width, height, trace, oi, config) {
                Ok(l) => l,
                Err(e) => {
                    diagnostics.push(OrderDiagnostic {
                        order_index: oi,
                        n_lines_detected: 0,
                        n_lines_matched: 0,
                        n_lines_used: 0,
                        rms_nm: 0.0,
                        success: false,
                        failure_reason: Some(e),
                        detected_lines: Vec::new(),
                        wl_solution: None,
                    });
                    continue;
                }
            };

        let mut best_diag = None;

        if let Some((gc, first_m, step)) = two_phase_base {
            // Search over candidate physical orders to robustly handle sparse traces.
            // We enforce uniqueness: an 'm' value can only be claimed by one trace.
            // Constrain each order's candidate search to a narrow window around
            // the seed-predicted m for this trace index (bd-0poyt). A wide
            // global window lets sparse-source orders match wrong-m candidates
            // whose spurious atlas hits outnumber the true-m's real hits —
            // classic degeneracy for HgAr lamps where most orders have ≤1
            // atlas line in their true FSR.
            let expected_m = first_m + step * (order_idx as i32);
            const CANDIDATE_HALF_WINDOW: i32 = 3;
            let search_start = (expected_m - CANDIDATE_HALF_WINDOW).max(1);
            let search_end = expected_m + CANDIDATE_HALF_WINDOW;

            let mut max_matched = 0;
            let mut min_rms = f64::MAX;

            let npx = f64::from(width.max(1));

            for candidate_m in search_start..=search_end {
                // Skip if this physical order was already successfully claimed by a previous trace
                if order_calibrations
                    .iter()
                    .any(|c: &EchelleOrderCalibration| c.physical_order_number == Some(candidate_m))
                {
                    continue;
                }

                let physical_order = candidate_m as f64;
                let tp_config = TwoPhaseMatchConfig {
                    primary_window_nm: 2.0,
                    final_tolerance_nm: config.wl_config.seed_tolerance_nm,
                    fallback_tolerance_nm: 1.0,
                    grating_constant_nm: gc,
                    gc_tolerance: 0.01,
                    min_primary_matches: 0,
                    physical_order,
                };

                let lambda_center = gc / physical_order;
                let fsr = gc / (physical_order * physical_order);
                let dispersion = fsr / npx;
                let lambda_start = lambda_center - dispersion * (npx / 2.0);

                let seed_fn = move |pixel: f64| -> f64 { lambda_start + dispersion * pixel };

                let cand_diag = match_and_fit(&lines, oi, config, &seed_fn, Some(&tp_config));

                if cand_diag.success
                    && (cand_diag.n_lines_matched > max_matched
                        || (cand_diag.n_lines_matched == max_matched && cand_diag.rms_nm < min_rms))
                {
                    max_matched = cand_diag.n_lines_matched;
                    min_rms = cand_diag.rms_nm;
                    best_diag = Some(cand_diag);
                }
            }
        } else {
            let tp_config = None;
            let cand_diag = match_and_fit(&lines, oi, config, &seed_fns[order_idx], tp_config);
            // Keep diagnostic even if match_and_fit failed
            best_diag = Some(cand_diag);
        }

        let mut final_diag = best_diag.unwrap_or_else(|| OrderDiagnostic {
            order_index: oi,
            n_lines_detected: lines.len(),
            n_lines_matched: 0,
            n_lines_used: 0,
            rms_nm: 0.0,
            success: false,
            failure_reason: Some(
                "No physical order candidate produced a successful match".to_string(),
            ),
            detected_lines: lines.clone(),
            wl_solution: None,
        });

        if final_diag.success {
            let validation = final_diag
                .wl_solution
                .as_ref()
                .map(|sol| sol.validate_monotonic(&config.orientation, 150.0, 1200.0));
            match validation {
                Some(Ok(())) => {
                    let sol = final_diag.wl_solution.as_ref().expect("checked above");
                    let order_cal =
                        build_order_calibration(trace, sol, oi, width, grating_constant_nm);
                    eprintln!(
                        "Pass 1: Order {} matched physical order {:?}",
                        oi, order_cal.physical_order_number
                    );
                    order_calibrations.push(order_cal);
                }
                Some(Err(err)) => {
                    tracing::warn!("Pass 1: order {oi} rejected — {err}");
                    eprintln!("Pass 1: Order {oi} REJECTED (wavelength axis): {err}");
                    final_diag.success = false;
                    final_diag.failure_reason = Some(format!("wavelength axis invalid: {err}"));
                    final_diag.wl_solution = None;
                }
                None => {}
            }
        }

        diagnostics.push(final_diag);
    }

    // ── Pass 2: Refine seeds for failed orders ───────────────────────
    // When using the echelle equation seed, the initial seed assumes
    // detected traces correspond to consecutive physical orders. But with
    // sparse emission sources (e.g., HgAr lamp), detected orders are
    // often non-consecutive. Use successfully calibrated orders to build
    // a trace_index → physical_order model, then re-seed failed orders.
    if let Some(gc) = grating_constant_nm {
        let n_failed = diagnostics.iter().filter(|d| !d.success).count();
        let anchors: Vec<(f64, f64)> = order_calibrations
            .iter()
            .filter_map(|cal| {
                let m = cal.physical_order_number? as f64;
                Some((f64::from(cal.relative_index), m))
            })
            .collect();

        if anchors.len() >= 2 && n_failed > 0 {
            // Fit quadratic model: m(i) = a + b*i + c*i²
            // The quadratic term captures the prism's Cauchy dispersion (Y ∝ m²)
            // which causes the linear model to fail in the middle orders.
            let (a, b, c) = quadratic_regression(&anchors);
            let npx = f64::from(width.max(1));

            for (order_idx, trace) in traces.iter().enumerate() {
                if diagnostics[order_idx].success {
                    continue;
                }
                let i = order_idx as f64;
                let predicted_m = (a + b * i + c * i * i).round();
                if predicted_m < 1.0 {
                    continue;
                }

                let lambda_center = gc / predicted_m;
                let fsr = gc / (predicted_m * predicted_m);
                let dispersion = fsr / npx;
                let lambda_start = lambda_center - dispersion * (npx / 2.0);
                let refined_seed = move |pixel: f64| -> f64 { lambda_start + dispersion * pixel };

                let oi = order_idx as u32;
                // Pass 2 uses two-phase matching with the refined (predicted) physical order.
                let tp_config_p2 = TwoPhaseMatchConfig {
                    primary_window_nm: 2.0,
                    final_tolerance_nm: config.wl_config.seed_tolerance_nm,
                    fallback_tolerance_nm: 1.0,
                    grating_constant_nm: gc,
                    gc_tolerance: 0.01,
                    min_primary_matches: 0,
                    physical_order: predicted_m,
                };
                let diag = process_single_order(
                    &arc_line_sources,
                    width,
                    height,
                    trace,
                    oi,
                    config,
                    &refined_seed,
                    Some(&tp_config_p2),
                );

                let mut diag = diag;
                if diag.success {
                    let validation = diag
                        .wl_solution
                        .as_ref()
                        .map(|sol| sol.validate_monotonic(&config.orientation, 150.0, 1200.0));
                    match validation {
                        Some(Ok(())) => {
                            let sol = diag.wl_solution.as_ref().expect("checked above");
                            let order_cal =
                                build_order_calibration(trace, sol, oi, width, Some(gc));
                            eprintln!(
                                "Pass 2: Order {} matched physical order {:?}",
                                oi, order_cal.physical_order_number
                            );
                            order_calibrations.push(order_cal);
                        }
                        Some(Err(err)) => {
                            tracing::warn!("Pass 2: order {oi} rejected — {err}");
                            eprintln!("Pass 2: Order {oi} REJECTED (wavelength axis): {err}");
                            diag.success = false;
                            diag.failure_reason = Some(format!("wavelength axis invalid: {err}"));
                            diag.wl_solution = None;
                        }
                        None => {}
                    }
                }
                diagnostics[order_idx] = diag;
            }
        }
    }

    // ── Pass 3: Physics baseline + 2D residual bootstrap (bd-hpzi) ───
    // For orders that still failed (no HgAr lines at all), use the
    // grating equation as a rigid physical baseline and a 2D Chebyshev
    // residual surface fit from calibrated orders to predict wavelengths.
    if let Some(gc) = grating_constant_nm {
        let n_still_failed = diagnostics.iter().filter(|d| !d.success).count();
        if order_calibrations.len() >= 6 && n_still_failed > 0 {
            let bootstrapped = bootstrap_uncalibrated_orders(
                gc,
                width,
                &traces,
                &mut order_calibrations,
                &mut diagnostics,
            );
            if bootstrapped > 0 {
                tracing::info!(
                    bootstrapped,
                    "Pass 3: bootstrapped orders via physics + 2D residual"
                );
            }
        }
    }

    // ── Deduplicate physical_order_number (safety net) ────────────────
    // Arc-matched orders (Pass 1/2) appear before bootstrapped ones, so
    // first-wins preserves the higher-quality calibrations.
    {
        let mut seen_m: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for cal in &mut order_calibrations {
            if let Some(m) = cal.physical_order_number
                && !seen_m.insert(m)
            {
                eprintln!(
                    "Deduplicator: Clearing order {} because physical order {} is duplicate",
                    cal.relative_index, m
                );
                cal.physical_order_number = None;
                if let Some(notes) = cal.notes.as_mut() {
                    notes.push_str(" [physical_order_number cleared: duplicate]");
                }
            }
        }
    }

    // ── Stage 7: Assemble EchelleCalibrationProfile ──────────────────
    let n_calibrated = order_calibrations.len();
    if n_calibrated == 0 {
        let reasons: Vec<String> = diagnostics
            .iter()
            .filter_map(|d| d.failure_reason.clone())
            .collect();
        return Err(format!(
            "no orders were successfully calibrated ({n_orders} detected). Reasons: {}",
            reasons.join("; ")
        ));
    }
    // Pooled RMS: weight each order's contribution by the number of lines used,
    // so an order with 30 matched lines contributes more than one with 3.
    let overall_rms = if n_calibrated > 0 {
        let total_lines: usize = diagnostics
            .iter()
            .filter(|d| d.success)
            .map(|d| d.n_lines_used)
            .sum();
        if total_lines > 0 {
            let weighted_sum_sq: f64 = diagnostics
                .iter()
                .filter(|d| d.success)
                .map(|d| d.rms_nm * d.rms_nm * d.n_lines_used as f64)
                .sum();
            (weighted_sum_sq / total_lines as f64).sqrt()
        } else {
            0.0
        }
    } else {
        0.0
    };

    let summation_mode = if config.use_optimal_extraction {
        EchelleSummationMode::Optimal
    } else {
        EchelleSummationMode::SimpleSum
    };

    let profile = EchelleCalibrationProfile {
        schema_version: EchelleSchemaVersion::v1(),
        profile_id: Some(format!(
            "cal-{}-{}",
            Utc::now().format("%Y%m%d-%H%M%S"),
            n_calibrated
        )),
        display_name: config.profile_name.clone(),
        compatibility: config.frame_compat.clone(),
        orientation: config.orientation.clone(),
        extraction: EchelleExtractionConfig {
            summation_mode,
            default_aperture_half_width_px: config.rectify_config.aperture_half_width,
            background: None,
            scattered_light: None,
        },
        orders: order_calibrations,
        corrections: EchelleCorrections::default(),
        provenance: EchelleProvenance {
            creator_tool: "rust-daq echelle_calibration_pipeline".to_string(),
            creator_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            created_at_utc: Utc::now(),
            source_frame_ids: Vec::new(),
            notes: Some(format!(
                "Calibrated {n_calibrated}/{n_orders} orders, overall RMS = {overall_rms:.4} nm"
            )),
        },
    };

    Ok(CalibrationResult {
        profile,
        per_order_diagnostics: diagnostics,
        overall_rms_nm: overall_rms,
        n_orders_calibrated: n_calibrated,
        n_orders_detected: n_orders,
    })
}

// ─── Per-order processing ────────────────────────────────────────────────────

/// Extract per-order 1D spectrum as `f32` for arc line detection.
fn extract_order_spectrum_f32(
    frame: &[f32],
    width: u32,
    height: u32,
    trace: &OrderTrace,
    order_index: u32,
    config: &CalibrationPipelineConfig,
) -> Option<Vec<f32>> {
    let spec = OrderSpec {
        trace: &trace.trace,
        disp_start: 0,
        disp_end: width.saturating_sub(1),
        order_index,
    };
    let rect = rectify_order(frame, width, height, &spec, &config.rectify_config)?;
    let spectrum_f64: Vec<f64> = if config.use_optimal_extraction {
        match optimal_extract(&rect, None, &config.optimal_config) {
            Some(result) => result.flux,
            None => simple_sum_extract(&rect),
        }
    } else {
        simple_sum_extract(&rect)
    };
    Some(spectrum_f64.iter().map(|&v| v as f32).collect())
}

/// Extracts the 1D spectrum for a single order and detects arc lines within it.
/// If multiple frames are provided (HDR mode), it processes each frame and merges
/// the detected lines, preferring unsaturated peaks.
#[allow(clippy::too_many_arguments)]
fn extract_and_detect_lines(
    line_frames: &[&[f32]],
    width: u32,
    height: u32,
    trace: &OrderTrace,
    order_index: u32,
    config: &CalibrationPipelineConfig,
) -> Result<Vec<ArcLine>, String> {
    if line_frames.is_empty() {
        return Err("internal error: no arc frames for line detection".into());
    }

    // ── Stages 3–5: rectify → extract → detect (optional HDR merge across exposures)
    let lines: Vec<ArcLine> = if line_frames.len() == 1 {
        let Some(spectrum_f32) =
            extract_order_spectrum_f32(line_frames[0], width, height, trace, order_index, config)
        else {
            return Err("rectification failed (trace out of bounds)".into());
        };
        detect_arc_lines(&spectrum_f32, order_index, &config.arc_config)
    } else {
        let mut runs: Vec<Vec<ArcLine>> = Vec::with_capacity(line_frames.len());
        for frame in line_frames {
            let Some(spectrum_f32) =
                extract_order_spectrum_f32(frame, width, height, trace, order_index, config)
            else {
                return Err(
                    "rectification failed on an HDR arc frame (trace out of bounds)".into(),
                );
            };
            runs.push(detect_arc_lines(
                &spectrum_f32,
                order_index,
                &config.arc_config,
            ));
        }
        merge_arc_lines_hdr(
            &runs,
            config.hdr_merge_tol_px,
            config.hdr_prefer_unsaturated,
        )
    };

    Ok(lines)
}

/// Takes detected arc lines and performs cross-correlation against the reference atlas
/// using the provided seed function and Phase 2 config. Returns a full wavelength solution.
fn match_and_fit(
    lines: &[ArcLine],
    order_index: u32,
    config: &CalibrationPipelineConfig,
    seed_fn: &dyn Fn(f64) -> f64,
    two_phase: Option<&TwoPhaseMatchConfig>,
) -> OrderDiagnostic {
    let mut diag = OrderDiagnostic {
        order_index,
        n_lines_detected: lines.len(),
        n_lines_matched: 0,
        n_lines_used: 0,
        rms_nm: 0.0,
        success: false,
        failure_reason: None,
        detected_lines: lines.to_vec(),
        wl_solution: None,
    };

    if lines.len() < config.min_lines_per_order {
        diag.failure_reason = Some(format!(
            "too few arc lines detected ({}, need {})",
            lines.len(),
            config.min_lines_per_order
        ));
        return diag;
    }

    // ── Stage 6: Match to atlas and fit wavelength solution ──────────
    let matches = if let Some(tp_config) = two_phase {
        match_lines_two_phase(lines, &config.atlas, seed_fn, tp_config)
    } else {
        match_lines_to_atlas(
            lines,
            &config.atlas,
            seed_fn,
            config.wl_config.seed_tolerance_nm,
        )
    };
    diag.n_lines_matched = matches.len();

    if matches.len() < config.min_lines_per_order {
        diag.failure_reason = Some(format!(
            "too few atlas matches ({}, need {})",
            matches.len(),
            config.min_lines_per_order
        ));
        return diag;
    }

    // When we know the physical order and grating constant for this trace
    // (via the two-phase seed), thread them to the wavelength fitter so the
    // single-line fallback can derive a physically-correct dispersion rather
    // than the legacy hardcoded heuristic. Orientation sign and detector
    // width come from the pipeline-level instrument config.
    let mut wl_config_local = config.wl_config.clone();
    if let Some(tp) = two_phase {
        let physical_order = tp.physical_order.round() as i32;
        let n_pixels = f64::from(config.frame_compat.frame_width.max(1));
        wl_config_local.fallback_seed = Some(SingleLineFallbackSeed {
            grating_constant_nm: tp.grating_constant_nm,
            physical_order,
            n_pixels,
            wavelength_ascending: config
                .orientation
                .wavelength_increase_with_dispersion_positive,
        });
    }

    match fit_order_wavelength(
        lines,
        &config.atlas,
        &matches,
        order_index,
        &wl_config_local,
    ) {
        Some(sol) => {
            // Self-consistency gate (bd-0poyt): when the two-phase config
            // supplies a candidate physical_order + grating constant, the
            // fitted polynomial's midpoint wavelength must agree with the
            // echelle equation `λ_center = gc / m` to within ±FSR. Otherwise
            // the fit is drawn from spurious matches for a different order —
            // typical when a degree-reduced 2-point linear fit overfits a
            // pair of cross-order atlas matches, giving RMS=0 despite being
            // physically wrong. Reject such fits so the candidate-m search
            // doesn't accept them as winners.
            let self_consistent = match two_phase {
                Some(tp) if tp.physical_order > 0.0 && tp.grating_constant_nm > 0.0 => {
                    let midpoint = sol.pixel_min.midpoint(sol.pixel_max);
                    let midpoint_wl = sol.eval(midpoint);
                    crate::wavelength_fitting::is_within_order_fsr(
                        tp.physical_order,
                        midpoint_wl,
                        tp.grating_constant_nm,
                        1.0,
                    )
                }
                _ => true,
            };
            if self_consistent {
                diag.n_lines_used = sol.n_lines_used;
                diag.rms_nm = sol.rms_nm;
                diag.success = true;
                diag.wl_solution = Some(sol);
            } else {
                diag.failure_reason =
                    Some("fitted polynomial midpoint not consistent with candidate m".into());
            }
        }
        None => {
            diag.failure_reason = Some("wavelength fitting failed".into());
        }
    }

    diag
}

/// Process a single order: rectify → extract → detect lines (HDR merge if multiple frames) → match → fit.
/// Now incorporates dynamic `candidate_m` evaluation to robustly map traces to physical order numbers.
#[allow(clippy::too_many_arguments)]
fn process_single_order(
    line_frames: &[&[f32]],
    width: u32,
    height: u32,
    trace: &OrderTrace,
    order_index: u32,
    config: &CalibrationPipelineConfig,
    seed_fn: &dyn Fn(f64) -> f64,
    two_phase: Option<&TwoPhaseMatchConfig>,
) -> OrderDiagnostic {
    let lines =
        match extract_and_detect_lines(line_frames, width, height, trace, order_index, config) {
            Ok(l) => l,
            Err(e) => {
                return OrderDiagnostic {
                    order_index,
                    n_lines_detected: 0,
                    n_lines_matched: 0,
                    n_lines_used: 0,
                    rms_nm: 0.0,
                    success: false,
                    failure_reason: Some(e),
                    detected_lines: Vec::new(),
                    wl_solution: None,
                };
            }
        };
    match_and_fit(&lines, order_index, config, seed_fn, two_phase)
}

/// Simple aperture-weighted summation extraction.
fn simple_sum_extract(rect: &crate::rectification::RectifiedOrder) -> Vec<f64> {
    let mut flux = vec![0.0f64; rect.n_dispersion];
    for (col, f) in flux.iter_mut().enumerate() {
        for row in 0..rect.n_cross {
            let idx = row * rect.n_dispersion + col;
            *f += f64::from(rect.data[idx]) * f64::from(rect.mask[idx]);
        }
    }
    flux
}

// ─── Seed wavelength functions ───────────────────────────────────────────────

/// A boxed seed function mapping pixel position → approximate wavelength in nm.
type SeedFn = Box<dyn Fn(f64) -> f64>;

/// Build per-order seed wavelength functions from the seed configuration.
///
/// Returns a Vec of closures, one per detected order, each mapping
/// pixel position → approximate wavelength in nm.
fn build_seed_functions(
    seed: &WavelengthSeed,
    n_orders: usize,
    width: u32,
) -> Result<Vec<SeedFn>, String> {
    match seed {
        WavelengthSeed::Anchors(anchors) => build_anchor_seeds(anchors, n_orders, width),
        WavelengthSeed::EchelleEquation {
            grating_constant_nm,
            first_physical_order,
            order_step,
            n_pixels,
        } => Ok(build_echelle_seeds(
            *grating_constant_nm,
            *first_physical_order,
            *order_step,
            n_orders,
            *n_pixels,
        )),
    }
}

/// Build seed functions from user-provided anchor points.
///
/// Groups anchors by order_index and fits a linear model per order.
/// Orders without anchors get an interpolated seed based on neighboring orders.
fn build_anchor_seeds(
    anchors: &[SeedAnchor],
    n_orders: usize,
    _width: u32,
) -> Result<Vec<SeedFn>, String> {
    if anchors.is_empty() {
        return Err("no seed anchors provided".to_string());
    }

    // Group anchors by order.
    let mut order_anchors: Vec<Vec<&SeedAnchor>> = vec![Vec::new(); n_orders];
    for anchor in anchors {
        let idx = anchor.order_index as usize;
        if idx < n_orders {
            order_anchors[idx].push(anchor);
        }
    }

    // For orders with >=2 anchors: fit a linear seed (slope + intercept).
    // For orders with 1 anchor: use a flat seed centered on that wavelength.
    // For orders with 0 anchors: interpolate from nearest calibrated orders.
    let mut seeds: Vec<Option<(f64, f64)>> = vec![None; n_orders]; // (intercept, slope)

    for (oi, oa) in order_anchors.iter().enumerate() {
        if oa.len() >= 2 {
            // Linear fit: λ = a + b * pixel
            let n = oa.len() as f64;
            let sx: f64 = oa.iter().map(|a| a.pixel).sum();
            let sy: f64 = oa.iter().map(|a| a.wavelength_nm).sum();
            let sxy: f64 = oa.iter().map(|a| a.pixel * a.wavelength_nm).sum();
            let sxx: f64 = oa.iter().map(|a| a.pixel * a.pixel).sum();
            let denom = n * sxx - sx * sx;
            if denom.abs() > 1e-10 {
                let slope = (n * sxy - sx * sy) / denom;
                let intercept = (sy - slope * sx) / n;
                seeds[oi] = Some((intercept, slope));
            }
        } else if oa.len() == 1 {
            // Single anchor: assume a rough dispersion of 0.1 nm/pixel
            // centered on the known wavelength.
            let a = oa[0];
            let assumed_slope = 0.1; // nm/pixel, rough default
            let intercept = a.wavelength_nm - assumed_slope * a.pixel;
            seeds[oi] = Some((intercept, assumed_slope));
        }
    }

    // Interpolate missing orders from nearest neighbors.
    for oi in 0..n_orders {
        if seeds[oi].is_some() {
            continue;
        }
        let left = seeds[..oi]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(j, s)| s.map(|v| (j, v)));
        let right = seeds[oi + 1..]
            .iter()
            .enumerate()
            .find_map(|(j, s)| s.map(|v| (oi + 1 + j, v)));

        seeds[oi] = match (left, right) {
            (Some((_, ls)), Some((_, rs))) => Some((ls.0.midpoint(rs.0), ls.1.midpoint(rs.1))),
            (Some((_, s)), None) | (None, Some((_, s))) => Some(s),
            (None, None) => None,
        };
    }

    // Verify all orders have valid seeds before building closures.
    for (oi, seed) in seeds.iter().enumerate() {
        if seed.is_none() {
            return Err(format!(
                "order {oi} has no seed wavelength model (no anchors and no neighbors to interpolate from)"
            ));
        }
    }

    // Build closures.
    let fns: Vec<SeedFn> = seeds
        .iter()
        .map(|seed| -> SeedFn {
            let (intercept, slope) = seed.expect("verified above");
            Box::new(move |pixel: f64| intercept + slope * pixel)
        })
        .collect();

    Ok(fns)
}

/// Ordinary least-squares linear regression on `(x, y)` pairs.
///
/// Returns `(slope, intercept)` such that `y ≈ slope * x + intercept`.
/// Requires at least 2 points.
fn linear_regression(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    let sum_x: f64 = points.iter().map(|(x, _)| x).sum();
    let sum_y: f64 = points.iter().map(|(_, y)| y).sum();
    let sum_xx: f64 = points.iter().map(|(x, _)| x * x).sum();
    let sum_xy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < 1e-15 {
        return (0.0, sum_y / n);
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}

/// Least-squares quadratic regression on `(x, y)` pairs.
///
/// Returns `(a, b, c)` such that `y ≈ a + b*x + c*x²`.
/// Falls back to linear regression if fewer than 3 points.
///
#[allow(clippy::many_single_char_names)] // Standard math notation for regression coefficients
/// Solves the 3×3 normal equations for the Vandermonde system:
/// ```text
/// [n    Σx   Σx²] [a]   [Σy  ]
/// [Σx   Σx²  Σx³] [b] = [Σxy ]
/// [Σx²  Σx³  Σx⁴] [c]   [Σx²y]
/// ```
fn quadratic_regression(points: &[(f64, f64)]) -> (f64, f64, f64) {
    if points.len() < 3 {
        let (slope, intercept) = linear_regression(points);
        return (intercept, slope, 0.0);
    }

    let n = points.len() as f64;
    let mut s = [0.0f64; 5]; // s[k] = Σ x^k
    let mut t = [0.0f64; 3]; // t[k] = Σ x^k * y
    for &(x, y) in points {
        let mut xp = 1.0;
        for sk in &mut s {
            *sk += xp;
            xp *= x;
        }
        // s has 5 elements but loop only fills 5 via overflow — fix below
        let mut xp = 1.0;
        for tk in &mut t {
            *tk += xp * y;
            xp *= x;
        }
    }
    // Recompute properly
    s = [0.0; 5];
    t = [0.0; 3];
    for &(x, y) in points {
        s[0] += 1.0;
        s[1] += x;
        s[2] += x * x;
        s[3] += x * x * x;
        s[4] += x * x * x * x;
        t[0] += y;
        t[1] += x * y;
        t[2] += x * x * y;
    }

    // Solve 3x3 system via Cramer's rule
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };

    let mat = [[n, s[1], s[2]], [s[1], s[2], s[3]], [s[2], s[3], s[4]]];
    let d = det(mat);
    if d.abs() < 1e-30 {
        let (slope, intercept) = linear_regression(points);
        return (intercept, slope, 0.0);
    }

    let a = det([[t[0], s[1], s[2]], [t[1], s[2], s[3]], [t[2], s[3], s[4]]]) / d;
    let b = det([[n, t[0], s[2]], [s[1], t[1], s[3]], [s[2], t[2], s[4]]]) / d;
    let c = det([[n, s[1], t[0]], [s[1], s[2], t[1]], [s[2], s[3], t[2]]]) / d;

    (a, b, c)
}

/// Bootstrap uncalibrated orders using physics baseline + 2D residual correction.
///
/// For each uncalibrated order:
/// 1. Assign physical order m via quadratic interpolation from calibrated anchors
/// 2. Compute physics baseline: λ_base(x, m) = gc/m + disp(m) × (x - w/2)
/// 3. Correct with 2D Chebyshev residual surface fit from calibrated orders
/// 4. Create an EchelleOrderCalibration with the predicted wavelength solution
///
/// Returns the number of newly bootstrapped orders.
fn bootstrap_uncalibrated_orders(
    gc: f64,
    width: u32,
    traces: &[crate::trace_fitting::OrderTrace],
    order_calibrations: &mut Vec<EchelleOrderCalibration>,
    diagnostics: &mut [OrderDiagnostic],
) -> usize {
    use crate::wavelength_fitting::chebyshev_fit_2d;

    let npx = f64::from(width.max(1));
    let half_w = npx / 2.0;

    // Collect calibrated anchor data: (trace_index, physical_order_m)
    let anchors: Vec<(f64, f64)> = order_calibrations
        .iter()
        .filter_map(|cal| {
            let m = cal.physical_order_number? as f64;
            Some((f64::from(cal.relative_index), m))
        })
        .collect();

    if anchors.len() < 3 {
        return 0;
    }

    // Step 1: Assign m to all traces via quadratic interpolation
    let (a, b, c) = quadratic_regression(&anchors);

    // Collect already-assigned physical order numbers so we can skip duplicates
    let mut assigned_m: std::collections::HashSet<i32> = order_calibrations
        .iter()
        .filter_map(|cal| cal.physical_order_number)
        .collect();

    // Step 2: Compute physics baseline dispersion model.
    // Fit the actual dispersion (nm/pixel) vs m from calibrated orders.
    // Dispersion theoretically scales as gc/m²/npx. We fit a scale factor.
    let mut disp_products: Vec<f64> = Vec::new();
    for diag in diagnostics.iter() {
        if !diag.success {
            continue;
        }
        if let Some(ref sol) = diag.wl_solution {
            let mid = sol.pixel_min.midpoint(sol.pixel_max);
            let lambda_center = sol.eval(mid);
            let m = (gc / lambda_center).round();
            if m < 1.0 {
                continue;
            }
            // Compute actual dispersion from the Chebyshev solution
            let p_lo = sol.pixel_min + 10.0;
            let p_hi = sol.pixel_max - 10.0;
            if p_hi <= p_lo {
                continue;
            }
            let actual_disp = (sol.eval(p_hi) - sol.eval(p_lo)) / (p_hi - p_lo);
            let theoretical_disp = gc / (m * m * npx);
            if theoretical_disp.abs() > 1e-15 {
                disp_products.push(actual_disp / theoretical_disp);
            }
        }
    }
    let disp_scale = if disp_products.is_empty() {
        1.0
    } else {
        disp_products.iter().sum::<f64>() / disp_products.len() as f64
    };

    // Step 3: Collect (pixel, m, δλ) residuals from calibrated orders
    let m_values: Vec<f64> = anchors.iter().map(|(_, m)| *m).collect();
    let m_min = m_values.iter().copied().fold(f64::INFINITY, f64::min);
    let m_max = m_values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let m_center = f64::midpoint(m_min, m_max);
    let m_scale = ((m_max - m_min) / 2.0).max(1.0);
    let x_center = half_w;
    let x_scale = half_w.max(1.0);

    let mut residual_data: Vec<(f64, f64, f64)> = Vec::new();
    for diag in diagnostics.iter() {
        if !diag.success {
            continue;
        }
        if let Some(ref sol) = diag.wl_solution {
            let mid = sol.pixel_min.midpoint(sol.pixel_max);
            let lambda_center_actual = sol.eval(mid);
            let mf = (gc / lambda_center_actual).round();
            if mf < 1.0 {
                continue;
            }
            let theoretical_disp = disp_scale * gc / (mf * mf * npx);
            let lambda_center = gc / mf;

            // Sample the calibrated wavelength solution at several pixel positions
            // and compute residuals vs the physics baseline
            for px_frac in [0.1, 0.25, 0.5, 0.75, 0.9] {
                let px = sol.pixel_min + (sol.pixel_max - sol.pixel_min) * px_frac;
                let wl_actual = sol.eval(px);
                let wl_base = lambda_center + theoretical_disp * (px - half_w);
                let residual = wl_actual - wl_base;
                residual_data.push((px, mf, residual));
            }
        }
    }

    if residual_data.len() < 20 {
        // Not enough data for a 4×3 fit (20 coefficients)
        return 0;
    }

    // Step 4: Fit 2D Chebyshev residual surface (degree 4 in pixel, 3 in order)
    let Some(surface) = chebyshev_fit_2d(
        &residual_data,
        4, // degree_x (pixel)
        3, // degree_m (order)
        x_center,
        x_scale,
        m_center,
        m_scale,
    ) else {
        return 0;
    };

    // Step 5: Bootstrap uncalibrated orders
    let mut bootstrapped = 0;
    for (order_idx, _trace) in traces.iter().enumerate() {
        if diagnostics[order_idx].success {
            continue;
        }

        let i = order_idx as f64;
        let predicted_m = (a + b * i + c * i * i).round();
        if predicted_m < 1.0 {
            continue;
        }
        let mf = predicted_m;
        let m_int = mf as i32;

        // Skip if this physical order number is already assigned to another order
        if !assigned_m.insert(m_int) {
            continue;
        }

        let lambda_center = gc / mf;
        let theoretical_disp = disp_scale * gc / (mf * mf * npx);

        // Build the predicted wavelength as: baseline + 2D residual correction
        // We store it as a monomial polynomial for the EchelleOrderCalibration
        // Evaluate at 5 points and fit a low-order polynomial
        let mut px_wl: Vec<(f64, f64)> = Vec::new();
        for px_i in 0..10 {
            let px = npx * (px_i as f64 + 0.5) / 10.0;
            let wl_base = lambda_center + theoretical_disp * (px - half_w);
            let wl_correction = surface.eval(px, mf);
            px_wl.push((px, wl_base + wl_correction));
        }

        // Fit a linear monomial: λ(px) = intercept + slope * px
        if px_wl.len() < 2 {
            continue;
        }
        let wl_start = px_wl.first().map(|(_, w)| *w).unwrap_or(0.0);
        let wl_end = px_wl.last().map(|(_, w)| *w).unwrap_or(0.0);
        let px_start = px_wl.first().map(|(p, _)| *p).unwrap_or(0.0);
        let px_end = px_wl.last().map(|(p, _)| *p).unwrap_or(npx);
        let slope = if (px_end - px_start).abs() > 1e-10 {
            (wl_end - wl_start) / (px_end - px_start)
        } else {
            0.0
        };
        let intercept = wl_start - slope * px_start;

        let order_cal = EchelleOrderCalibration {
            relative_index: order_idx as u32,
            physical_order_number: Some(m_int),
            sample_start: 0,
            sample_end: width.saturating_sub(1),
            trace: traces[order_idx].trace.clone(),
            wavelength: EchelleWavelengthModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![intercept, slope],
                domain_start: 0.0,
                domain_end: npx,
                unit: "nm".to_string(),
            },
            aperture_half_width_px: Some(traces[order_idx].aperture_half_width),
            enabled: true,
            notes: Some(format!(
                "m={m_int}, bootstrapped (physics+2D residual, no arc lines)"
            )),
        };

        order_calibrations.push(order_cal);
        diagnostics[order_idx].success = true;
        diagnostics[order_idx].failure_reason = None;
        bootstrapped += 1;
    }

    bootstrapped
}

/// Build seed functions from the echelle grating equation.
///
/// For order number m: λ_center = grating_constant / m
/// Free spectral range: Δλ = λ_center / m = grating_constant / m²
/// Linear approximation: λ(pixel) ≈ λ_start + (Δλ / n_pixels) * pixel
fn build_echelle_seeds(
    grating_constant_nm: f64,
    first_physical_order: i32,
    order_step: i32,
    n_orders: usize,
    n_pixels: u32,
) -> Vec<Box<dyn Fn(f64) -> f64>> {
    let mut fns: Vec<Box<dyn Fn(f64) -> f64>> = Vec::with_capacity(n_orders);
    let npx = f64::from(n_pixels.max(1));

    for i in 0..n_orders {
        let m = (first_physical_order + order_step * i as i32).abs().max(1) as f64;
        let lambda_center = grating_constant_nm / m;
        // One free spectral range spans the detector: Δλ = gc/m² (nm), linear in pixel.
        // Using disp_ref*(m_ref/m) was wrong — it yields gc/(m_ref·m·npx) instead of gc/(m²·npx),
        // which mis-scales atlas matching for every order where m ≠ first_physical_order
        // (bd-kt8k synthetic / NIR cluster RMS regression).
        let fsr = grating_constant_nm / (m * m);
        let dispersion = fsr / npx;
        let lambda_start = lambda_center - dispersion * (npx / 2.0);

        fns.push(Box::new(move |pixel: f64| {
            lambda_start + dispersion * pixel
        }));
    }

    fns
}

// ─── Profile assembly ────────────────────────────────────────────────────────

/// Convert a trace + wavelength solution into an `EchelleOrderCalibration`.
///
/// The sample range is constrained to the wavelength solution's fitted domain
/// (pixel_min..pixel_max) because the Chebyshev polynomial is only reliable
/// within the range of its training data. Extrapolation beyond the matched
/// line positions would produce unreliable wavelengths.
///
/// If `grating_constant_nm` is provided, the physical order number is computed
/// from the wavelength at the midpoint of the fitted pixel range:
/// `m = round(grating_constant / λ_center)`.
fn build_order_calibration(
    trace: &OrderTrace,
    sol: &OrderWlSolution,
    order_index: u32,
    width: u32,
    grating_constant_nm: Option<f64>,
) -> EchelleOrderCalibration {
    // The Chebyshev coefficients were fitted using normalization over
    // [pixel_min, pixel_max]. The domain MUST match the fitted range exactly —
    // extending it would change the x → [-1,1] mapping and produce wrong wavelengths.
    // The sample range is constrained to the fitted domain.
    let sample_start = sol.pixel_min.ceil().max(0.0) as u32;
    let sample_end = (sol.pixel_max.floor() as u32).min(width.saturating_sub(1));

    let physical_order_number = grating_constant_nm.and_then(|gc| {
        let mid_pixel = sol.pixel_min.midpoint(sol.pixel_max);
        let lambda_center = sol.eval(mid_pixel);
        if lambda_center > 0.0 {
            Some((gc / lambda_center).round() as i32)
        } else {
            None
        }
    });

    let notes = match physical_order_number {
        Some(m) => format!(
            "m={m}, RMS={:.4}nm, {}/{} lines",
            sol.rms_nm, sol.n_lines_used, sol.n_lines_total
        ),
        None => format!(
            "RMS={:.4}nm, {}/{} lines",
            sol.rms_nm, sol.n_lines_used, sol.n_lines_total
        ),
    };

    EchelleOrderCalibration {
        relative_index: order_index,
        physical_order_number,
        sample_start,
        sample_end,
        trace: trace.trace.clone(),
        wavelength: EchelleWavelengthModel::Polynomial {
            basis: PolynomialBasis::Chebyshev,
            coefficients: sol.coefficients.clone(),
            domain_start: sol.pixel_min,
            domain_end: sol.pixel_max,
            unit: "nm".to_string(),
        },
        aperture_half_width_px: Some(trace.aperture_half_width),
        enabled: true,
        notes: Some(notes),
    }
}

// ─── Cross-order consistency check ───────────────────────────────────────────

/// Check echelle equation consistency: m × λ_center should be approximately
/// constant across all calibrated orders.
///
/// Prefers using `physical_order_number` from the profile (computed from
/// wavelength solutions) when available. Falls back to the seed-based
/// assignment (`first_physical_order + order_step * index`) if not set.
///
/// Returns the coefficient of variation (std_dev / mean) of the products.
/// Values < 0.01 (1%) indicate good consistency.
#[must_use]
pub fn check_echelle_consistency(
    result: &CalibrationResult,
    first_physical_order: i32,
    order_step: i32,
) -> Option<f64> {
    let products: Vec<f64> = result
        .per_order_diagnostics
        .iter()
        .filter(|d| d.success)
        .enumerate()
        .filter_map(|(cal_idx, d)| {
            let sol = d.wl_solution.as_ref()?;

            // Prefer the wavelength-derived physical_order_number from the profile
            // (set during build_order_calibration). Fall back to seed-based assignment.
            let m = result
                .profile
                .orders
                .iter()
                .find(|o| o.relative_index == d.order_index)
                .and_then(|o| o.physical_order_number)
                .map(|m| m.unsigned_abs() as f64)
                .unwrap_or_else(|| {
                    let _ = cal_idx; // suppress unused warning
                    let m_seed = (first_physical_order + order_step * d.order_index as i32)
                        .unsigned_abs() as f64;
                    m_seed.max(1.0)
                });
            let m = m.max(1.0);

            let mid_pixel = sol.pixel_min.midpoint(sol.pixel_max);
            let lambda_center = sol.eval(mid_pixel);
            Some(m * lambda_center)
        })
        .collect();

    if products.len() < 2 {
        return None;
    }

    let mean = products.iter().sum::<f64>() / products.len() as f64;
    let var = products.iter().map(|&p| (p - mean).powi(2)).sum::<f64>() / products.len() as f64;
    Some(var.sqrt() / mean)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EchelleTraceModel;
    use crate::wavelength_fitting::load_hgar_atlas;
    use std::sync::Arc;

    /// Create a synthetic echelle frame with horizontal order traces
    /// containing known emission lines at specified wavelengths.
    ///
    /// Each order is a horizontal band at a given Y position with:
    /// - A continuum component (Gaussian spatial profile × uniform spectral)
    /// - Emission lines as Gaussian peaks at pixel positions determined by
    ///   the given linear wavelength model
    ///
    /// The continuum is essential because `detect_orders` uses sigma-clipped
    /// mean per row — without it, bright emission lines get clipped as outliers
    /// and orders become invisible in the spatial profile.
    fn synthetic_arc_frame(
        width: usize,
        height: usize,
        orders: &[(f64, f64, f64)], // (y_center, lambda_start, lambda_end)
        lines: &[f64],              // wavelengths to inject
        sigma_px: f64,              // Gaussian sigma of each line (pixels)
        peak_flux: f64,
        spatial_sigma: f64, // cross-dispersion Gaussian sigma
    ) -> Vec<f32> {
        let mut frame = vec![5.0_f32; width * height]; // baseline noise floor
        let continuum_flux = 100.0; // continuous baseline along each order

        for &(y_center, lambda_start, lambda_end) in orders {
            let dlambda = lambda_end - lambda_start;
            if dlambda.abs() < 1e-10 {
                continue;
            }
            let dispersion = dlambda / width as f64; // nm/pixel

            // Paint continuum along the order (Gaussian spatial × uniform spectral).
            for row in 0..height {
                let dy = row as f64 - y_center;
                let spatial_weight = (-0.5 * (dy / spatial_sigma).powi(2)).exp();
                if spatial_weight < 1e-4 {
                    continue;
                }
                for col in 0..width {
                    frame[row * width + col] += (continuum_flux * spatial_weight) as f32;
                }
            }

            // Paint emission lines on top of the continuum.
            for &wl in lines {
                if wl < lambda_start || wl > lambda_end {
                    continue;
                }
                let px = (wl - lambda_start) / dispersion;

                for row in 0..height {
                    let dy = row as f64 - y_center;
                    let spatial_weight = (-0.5 * (dy / spatial_sigma).powi(2)).exp();
                    if spatial_weight < 1e-4 {
                        continue;
                    }
                    for col in 0..width {
                        let dx = col as f64 - px;
                        let spectral_weight = (-0.5 * (dx / sigma_px).powi(2)).exp();
                        frame[row * width + col] +=
                            (peak_flux * spatial_weight * spectral_weight) as f32;
                    }
                }
            }
        }

        frame
    }

    #[test]
    fn test_pipeline_with_synthetic_arc() {
        // Create a synthetic 200×300 frame with 3 orders.
        // Height must be large enough that order peaks are a small fraction
        // of the spatial profile, so the inter-percentile noise estimator
        // isn't inflated by the peaks themselves.
        let width = 200;
        let height = 300;

        // Order layout: 3 horizontal bands at y=60, 150, 240.
        // Order 0: 400-420 nm, Order 1: 500-525 nm, Order 2: 700-740 nm.
        let orders = vec![
            (60.0, 400.0, 420.0),
            (150.0, 500.0, 525.0),
            (240.0, 700.0, 740.0),
        ];

        // Inject lines from the HgAr atlas that fall within these ranges.
        let atlas = load_hgar_atlas();
        let all_wavelengths: Vec<f64> = atlas.iter().map(|a| a.wavelength_nm).collect();

        let frame = synthetic_arc_frame(
            width,
            height,
            &orders,
            &all_wavelengths,
            2.5,    // sigma_px
            2000.0, // peak flux
            2.5,    // spatial sigma
        );

        // Build seed anchors from known wavelength solutions.
        let mut anchors = Vec::new();
        for (oi, &(_, lambda_start, lambda_end)) in orders.iter().enumerate() {
            // Provide 2 anchor points per order (start and end).
            anchors.push(SeedAnchor {
                order_index: oi as u32,
                pixel: 0.0,
                wavelength_nm: lambda_start,
            });
            anchors.push(SeedAnchor {
                order_index: oi as u32,
                pixel: (width - 1) as f64,
                wavelength_nm: lambda_end,
            });
        }

        let config = CalibrationPipelineConfig {
            trace_config: TraceFitConfig {
                min_snr: 3.0,
                step_pixels: 5,
                poly_degree: 2,
                ..Default::default()
            },
            arc_config: ArcDetectConfig {
                sigdetect: 3.0,
                min_fwhm: 1.5,
                max_fwhm: 10.0,
                min_separation: 3.0,
                continuum_window: 51,
            },
            wl_config: WlFitConfig {
                poly_degree: 2,
                seed_tolerance_nm: 2.0, // generous for synthetic data
                ..Default::default()
            },
            rectify_config: RectifyConfig {
                aperture_half_width: 5.0,
                gaussian_weights: false,
                fwhm: 3.0,
            },
            atlas,
            seed: WavelengthSeed::Anchors(anchors),
            frame_compat: EchelleFrameCompatibility {
                sensor_width: width as u32,
                sensor_height: height as u32,
                frame_width: width as u32,
                frame_height: height as u32,
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
            profile_name: "Test HgAr Calibration".to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        let result = run_calibration_pipeline(&frame, width as u32, height as u32, &config)
            .expect("pipeline should succeed");

        // Should detect 3 orders.
        assert_eq!(
            result.n_orders_detected, 3,
            "expected 3 orders, got {}",
            result.n_orders_detected
        );

        // At least some orders should be calibrated.
        // (Not all may succeed depending on how many atlas lines fall in each range.)
        assert!(
            result.n_orders_calibrated > 0,
            "at least one order should be calibrated, diagnostics: {:?}",
            result
                .per_order_diagnostics
                .iter()
                .map(|d| (&d.failure_reason, d.n_lines_detected, d.n_lines_matched))
                .collect::<Vec<_>>()
        );

        // Profile should pass validation.
        result
            .profile
            .validate()
            .expect("generated profile should be valid");
    }

    #[test]
    fn test_hdr_duplicate_extra_frame_matches_single_path() {
        // Identical primary + extra exposure should merge to the same line census
        // as a single exposure (duplicate detections coalesce within merge tolerance).
        let width = 200;
        let height = 300;
        let orders = vec![
            (60.0, 400.0, 420.0),
            (150.0, 500.0, 525.0),
            (240.0, 700.0, 740.0),
        ];
        let atlas = load_hgar_atlas();
        let all_wavelengths: Vec<f64> = atlas.iter().map(|a| a.wavelength_nm).collect();
        let frame = synthetic_arc_frame(width, height, &orders, &all_wavelengths, 2.5, 2000.0, 2.5);
        let mut anchors = Vec::new();
        for (oi, &(_, lambda_start, lambda_end)) in orders.iter().enumerate() {
            anchors.push(SeedAnchor {
                order_index: oi as u32,
                pixel: 0.0,
                wavelength_nm: lambda_start,
            });
            anchors.push(SeedAnchor {
                order_index: oi as u32,
                pixel: (width - 1) as f64,
                wavelength_nm: lambda_end,
            });
        }
        let base = CalibrationPipelineConfig {
            trace_config: TraceFitConfig {
                min_snr: 3.0,
                step_pixels: 5,
                poly_degree: 2,
                ..Default::default()
            },
            arc_config: ArcDetectConfig {
                sigdetect: 3.0,
                min_fwhm: 1.5,
                max_fwhm: 10.0,
                min_separation: 3.0,
                continuum_window: 51,
            },
            wl_config: WlFitConfig {
                poly_degree: 2,
                seed_tolerance_nm: 2.0,
                ..Default::default()
            },
            rectify_config: RectifyConfig {
                aperture_half_width: 5.0,
                gaussian_weights: false,
                fwhm: 3.0,
            },
            atlas,
            seed: WavelengthSeed::Anchors(anchors),
            frame_compat: EchelleFrameCompatibility {
                sensor_width: width as u32,
                sensor_height: height as u32,
                frame_width: width as u32,
                frame_height: height as u32,
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
            profile_name: "HDR dup test".to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        let single = run_calibration_pipeline(&frame, width as u32, height as u32, &base)
            .expect("single-path pipeline");

        let mut hdr_cfg = base.clone();
        hdr_cfg.hdr_extra_arc_frames = vec![Arc::new(frame.clone())];
        let merged = run_calibration_pipeline(&frame, width as u32, height as u32, &hdr_cfg)
            .expect("HDR pipeline");

        assert_eq!(single.n_orders_detected, merged.n_orders_detected);
        assert_eq!(
            single.per_order_diagnostics.len(),
            merged.per_order_diagnostics.len()
        );
        for (a, b) in single
            .per_order_diagnostics
            .iter()
            .zip(merged.per_order_diagnostics.iter())
        {
            assert_eq!(a.order_index, b.order_index);
            assert_eq!(
                a.n_lines_detected, b.n_lines_detected,
                "HDR duplicate merge should not change detected line counts (order {})",
                a.order_index
            );
        }
    }

    #[test]
    fn test_pipeline_with_echelle_equation_seed() {
        // Test the echelle equation seed mode.
        // Frame height must be large enough for inter-percentile noise estimation.
        let width = 300;
        let height = 300;

        // Simulate 3 orders of a hypothetical echelle.
        // Assuming the detector width (300px) covers 1 FSR at m=10:
        // m=10: disp = 28 / 300 = 0.0933 nm/px. λ_center = 280nm → 266.0 - 294.0nm
        // m=9:  disp = 0.0933 * (10/9) = 0.1037 nm/px. λ_center = 311.11nm → 295.55 - 326.67nm
        // m=8:  disp = 0.0933 * (10/8) = 0.1166 nm/px. λ_center = 350.0nm → 332.5 - 367.5nm
        let grating_const = 2800.0;
        let disp_ref = (grating_const / 100.0) / (width as f64);

        let m10_disp = disp_ref * (10.0 / 10.0);
        let m10_start = 2800.0 / 10.0 - m10_disp * (width as f64 / 2.0);
        let m10_end = m10_start + m10_disp * (width as f64);

        let m9_disp = disp_ref * (10.0 / 9.0);
        let m9_start = 2800.0 / 9.0 - m9_disp * (width as f64 / 2.0);
        let m9_end = m9_start + m9_disp * (width as f64);

        let m8_disp = disp_ref * (10.0 / 8.0);
        let m8_start = 2800.0 / 8.0 - m8_disp * (width as f64 / 2.0);
        let m8_end = m8_start + m8_disp * (width as f64);

        let orders = vec![
            (60.0, m10_start, m10_end),
            (150.0, m9_start, m9_end),
            (240.0, m8_start, m8_end),
        ];

        // Use some of the HgAr Hg lines that fall in these ranges.
        let hg_lines = vec![296.728, 302.150, 312.567, 334.148, 365.015];

        let frame = synthetic_arc_frame(width, height, &orders, &hg_lines, 2.5, 3000.0, 2.5);

        let atlas = vec![
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
        ];

        let config = CalibrationPipelineConfig {
            trace_config: TraceFitConfig {
                min_snr: 3.0,
                step_pixels: 5,
                poly_degree: 2,
                ..Default::default()
            },
            arc_config: ArcDetectConfig {
                sigdetect: 3.0,
                min_fwhm: 1.5,
                max_fwhm: 10.0,
                min_separation: 3.0,
                continuum_window: 51,
            },
            wl_config: WlFitConfig {
                poly_degree: 2,
                seed_tolerance_nm: 3.0,
                ..Default::default()
            },
            rectify_config: RectifyConfig {
                aperture_half_width: 5.0,
                gaussian_weights: false,
                fwhm: 3.0,
            },
            atlas,
            seed: WavelengthSeed::EchelleEquation {
                grating_constant_nm: grating_const,
                first_physical_order: 10,
                order_step: -1, // order numbers decrease with increasing Y
                n_pixels: width as u32,
            },
            frame_compat: EchelleFrameCompatibility {
                sensor_width: width as u32,
                sensor_height: height as u32,
                frame_width: width as u32,
                frame_height: height as u32,
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
            profile_name: "Test Echelle Equation Seed".to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        let result = run_calibration_pipeline(&frame, width as u32, height as u32, &config)
            .expect("pipeline should succeed");

        assert!(
            result.n_orders_detected >= 2,
            "expected at least 2 orders, got {}",
            result.n_orders_detected,
        );
        // At least some orders should calibrate (orders with >=2 Hg lines in range).
        assert!(
            result.n_orders_calibrated > 0,
            "at least one order should calibrate, diagnostics: {:?}",
            result
                .per_order_diagnostics
                .iter()
                .map(|d| (&d.failure_reason, d.n_lines_detected, d.n_lines_matched))
                .collect::<Vec<_>>()
        );

        // Profile validation should pass.
        result.profile.validate().expect("profile should be valid");
    }

    #[test]
    fn test_hgar_atlas_is_sorted_and_nonempty() {
        let atlas = load_hgar_atlas();
        assert!(
            atlas.len() >= 29,
            "atlas should have ~30 lines, got {}",
            atlas.len()
        );

        // Verify wavelengths are sorted.
        for pair in atlas.windows(2) {
            assert!(
                pair[0].wavelength_nm <= pair[1].wavelength_nm,
                "atlas not sorted: {} > {}",
                pair[0].wavelength_nm,
                pair[1].wavelength_nm
            );
        }

        // Verify key Hg and Ar lines are present.
        let has_hg_green = atlas
            .iter()
            .any(|l| (l.wavelength_nm - 546.074).abs() < 0.01);
        let has_ar_763 = atlas
            .iter()
            .any(|l| (l.wavelength_nm - 763.511).abs() < 0.01);
        assert!(has_hg_green, "missing Hg 546.074 nm green line");
        assert!(has_ar_763, "missing Ar 763.511 nm line");
    }

    #[test]
    fn test_pipeline_empty_frame_errors() {
        let mut config = CalibrationPipelineConfig::default();
        config.frame_compat.frame_width = 100;
        config.frame_compat.frame_height = 100;
        config.frame_compat.sensor_width = 100;
        config.frame_compat.sensor_height = 100;
        let result = run_calibration_pipeline(&[], 100, 100, &config);
        assert!(result.is_err(), "empty frame should error");
    }

    #[test]
    fn test_pipeline_no_orders_detected() {
        // Uniform frame → no order traces.
        let frame = vec![10.0f32; 200 * 100];
        let config = CalibrationPipelineConfig {
            seed: WavelengthSeed::Anchors(vec![SeedAnchor {
                order_index: 0,
                pixel: 0.0,
                wavelength_nm: 400.0,
            }]),
            frame_compat: EchelleFrameCompatibility {
                sensor_width: 200,
                sensor_height: 100,
                frame_width: 200,
                frame_height: 100,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            ..Default::default()
        };
        let result = run_calibration_pipeline(&frame, 200, 100, &config);
        assert!(result.is_err(), "uniform frame should yield no orders");
    }

    #[test]
    fn test_echelle_consistency_check() {
        // Build a mock result with known consistent products.
        let sol1 = OrderWlSolution {
            order: 0,
            coefficients: vec![500.0, 10.0], // mid-pixel → 500nm
            pixel_min: 0.0,
            pixel_max: 200.0,
            rms_nm: 0.01,
            n_lines_used: 10,
            n_lines_total: 12,
        };
        let sol2 = OrderWlSolution {
            order: 1,
            coefficients: vec![454.545, 9.09], // mid-pixel → ~454.5nm
            pixel_min: 0.0,
            pixel_max: 200.0,
            rms_nm: 0.02,
            n_lines_used: 8,
            n_lines_total: 10,
        };

        let diag1 = OrderDiagnostic {
            order_index: 0,
            n_lines_detected: 12,
            n_lines_matched: 12,
            n_lines_used: 10,
            rms_nm: 0.01,
            success: true,
            failure_reason: None,
            detected_lines: Vec::new(),
            wl_solution: Some(sol1),
        };
        let diag2 = OrderDiagnostic {
            order_index: 1,
            n_lines_detected: 10,
            n_lines_matched: 10,
            n_lines_used: 8,
            rms_nm: 0.02,
            success: true,
            failure_reason: None,
            detected_lines: Vec::new(),
            wl_solution: Some(sol2),
        };

        let result = CalibrationResult {
            profile: EchelleCalibrationProfile {
                schema_version: EchelleSchemaVersion::v1(),
                profile_id: None,
                display_name: "test".into(),
                compatibility: EchelleFrameCompatibility {
                    sensor_width: 200,
                    sensor_height: 100,
                    frame_width: 200,
                    frame_height: 100,
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
                    default_aperture_half_width_px: 4.0,
                    background: None,
                    scattered_light: None,
                },
                orders: Vec::new(),
                corrections: EchelleCorrections::default(),
                provenance: EchelleProvenance {
                    creator_tool: "test".into(),
                    creator_version: None,
                    created_at_utc: Utc::now(),
                    source_frame_ids: Vec::new(),
                    notes: None,
                },
            },
            per_order_diagnostics: vec![diag1, diag2],
            overall_rms_nm: 0.015,
            n_orders_calibrated: 2,
            n_orders_detected: 2,
        };

        // With first_physical_order = 10, order_step = 1:
        // Order 0 (m=10): λ_center = 500nm → product = 5000
        // Order 1 (m=11): λ_center = 454.545nm → product = 5000
        // CV should be very small.
        let cv = check_echelle_consistency(&result, 10, 1);
        assert!(cv.is_some());
        assert!(
            cv.unwrap() < 0.01,
            "echelle consistency CV should be < 1%, got {:.4}",
            cv.unwrap()
        );
    }

    #[test]
    fn test_physical_order_from_wavelength() {
        use crate::types::EchelleTraceModel;

        // Verify that build_order_calibration computes m = round(gc / λ_center).
        let trace = OrderTrace {
            trace: EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![100.0],
                domain_start: 0.0,
                domain_end: 200.0,
            },
            aperture_half_width: 5.0,
            fit_rms: 0.1,
            n_samples: 50,
            order_number: None,
        };

        // Wavelength solution centered at 577nm → m = round(36300 / 577) = 63
        let sol = OrderWlSolution {
            order: 0,
            coefficients: vec![577.0, 0.1], // nearly flat at ~577nm
            pixel_min: 0.0,
            pixel_max: 200.0,
            rms_nm: 0.05,
            n_lines_used: 5,
            n_lines_total: 5,
        };

        let cal = build_order_calibration(&trace, &sol, 0, 200, Some(36300.0));
        assert_eq!(
            cal.physical_order_number,
            Some(63),
            "expected m=63 for λ≈577nm, gc=36300"
        );
        assert!(
            cal.notes.as_ref().unwrap().contains("m=63"),
            "notes should contain physical order: {:?}",
            cal.notes
        );

        // Without grating constant, physical_order_number should be None.
        let cal_no_gc = build_order_calibration(&trace, &sol, 0, 200, None);
        assert_eq!(cal_no_gc.physical_order_number, None);
    }

    #[test]
    fn test_linear_regression() {
        // Perfect linear data: y = 2x + 10
        let points = vec![(0.0, 10.0), (1.0, 12.0), (2.0, 14.0), (3.0, 16.0)];
        let (slope, intercept) = linear_regression(&points);
        assert!(
            (slope - 2.0).abs() < 1e-10,
            "expected slope=2.0, got {slope}"
        );
        assert!(
            (intercept - 10.0).abs() < 1e-10,
            "expected intercept=10.0, got {intercept}"
        );

        // Non-consecutive x values (simulating sparse order detection).
        let sparse = vec![(0.0, 43.0), (5.0, 63.0), (10.0, 83.0)];
        let (slope, intercept) = linear_regression(&sparse);
        assert!(
            (slope - 4.0).abs() < 1e-10,
            "expected slope=4.0, got {slope}"
        );
        assert!(
            (intercept - 43.0).abs() < 1e-10,
            "expected intercept=43.0, got {intercept}"
        );

        // Predict: trace_index=3 → m = 4*3 + 43 = 55
        let predicted = slope * 3.0 + intercept;
        assert!(
            (predicted - 55.0).abs() < 1e-10,
            "expected prediction=55.0, got {predicted}"
        );
    }

    #[test]
    fn test_non_consecutive_order_refinement() {
        // Create a synthetic frame with 3 orders at NON-consecutive physical
        // order numbers, using echelle equation seed that assumes consecutive.
        // The two-pass refinement should recover the middle order.
        let width = 300;
        let height = 400;
        let grating_const = 2800.0;

        // Three orders: m=10 (y=60), m=8 (y=200), m=6 (y=340).
        // With first_physical_order=10 and order_step=-1:
        //   Pass 1: trace 0 → seed m=10 ✓, trace 1 → seed m=9 ✗ (actual=8),
        //           trace 2 → seed m=8 ✗ (actual=6).
        // Pass 2: linear model from trace 0 (m=10) alone won't trigger (needs ≥2).
        // So instead, let's use orders m=10, m=9, m=6 where m=9 matches on pass 1
        // and m=6 is recovered by the regression from m=10 and m=9.
        let m10_center = grating_const / 10.0; // 280nm
        let m9_center = grating_const / 9.0; // 311nm
        let m6_center = grating_const / 6.0; // 466nm

        // Dispersion at m_ref=10: FSR/npx = (gc/m²)/npx
        let m_ref = 10.0f64;
        let fsr_ref = grating_const / (m_ref * m_ref);
        let disp_ref = fsr_ref / width as f64;

        let m10_disp = disp_ref; // m=10
        let m9_disp = disp_ref * (m_ref / 9.0); // m=9
        let m6_disp = disp_ref * (m_ref / 6.0); // m=6

        let m10_start = m10_center - m10_disp * (width as f64 / 2.0);
        let m10_end = m10_start + m10_disp * width as f64;
        let m9_start = m9_center - m9_disp * (width as f64 / 2.0);
        let m9_end = m9_start + m9_disp * width as f64;
        let m6_start = m6_center - m6_disp * (width as f64 / 2.0);
        let m6_end = m6_start + m6_disp * width as f64;

        let orders = vec![
            (80.0, m10_start, m10_end),
            (200.0, m9_start, m9_end),
            (340.0, m6_start, m6_end),
        ];

        // Atlas lines carefully placed to land in each order's range.
        let atlas = vec![
            AtlasLine {
                wavelength_nm: 275.0,
                species: "test".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 285.0,
                species: "test".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 296.728,
                species: "Hg I".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 302.150,
                species: "Hg I".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 312.567,
                species: "Hg I".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 320.0,
                species: "test".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 455.0,
                species: "test".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 465.0,
                species: "test".into(),
                strength: 500.0,
            },
            AtlasLine {
                wavelength_nm: 475.0,
                species: "test".into(),
                strength: 500.0,
            },
        ];

        let all_wls: Vec<f64> = atlas.iter().map(|a| a.wavelength_nm).collect();
        let frame = synthetic_arc_frame(width, height, &orders, &all_wls, 2.5, 3000.0, 2.5);

        let config = CalibrationPipelineConfig {
            trace_config: TraceFitConfig {
                min_snr: 3.0,
                step_pixels: 5,
                poly_degree: 2,
                ..Default::default()
            },
            arc_config: ArcDetectConfig {
                sigdetect: 3.0,
                min_fwhm: 1.5,
                max_fwhm: 10.0,
                min_separation: 3.0,
                continuum_window: 51,
            },
            wl_config: WlFitConfig {
                poly_degree: 2,
                seed_tolerance_nm: 3.0,
                ..Default::default()
            },
            rectify_config: RectifyConfig {
                aperture_half_width: 5.0,
                gaussian_weights: false,
                fwhm: 3.0,
            },
            atlas,
            seed: WavelengthSeed::EchelleEquation {
                grating_constant_nm: grating_const,
                first_physical_order: 10,
                order_step: -1,
                n_pixels: width as u32,
            },
            frame_compat: EchelleFrameCompatibility {
                sensor_width: width as u32,
                sensor_height: height as u32,
                frame_width: width as u32,
                frame_height: height as u32,
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
            profile_name: "Test Non-Consecutive Refinement".to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        let result = run_calibration_pipeline(&frame, width as u32, height as u32, &config)
            .expect("pipeline should succeed");

        assert_eq!(result.n_orders_detected, 3, "should detect all 3 orders");

        // All calibrated orders should have physical_order_number set.
        for order in &result.profile.orders {
            assert!(
                order.physical_order_number.is_some(),
                "order {} should have physical_order_number",
                order.relative_index
            );
        }

        result.profile.validate().expect("profile should be valid");
    }

    /// Verify that `bootstrap_uncalibrated_orders` skips orders whose predicted
    /// physical order number m is already assigned to another (arc-matched) order.
    ///
    /// Setup: 5 calibrated anchors at indices 0,3,6,9,12 with m=100..104 (slope ≈ 1/3
    /// per index). The 11 uncalibrated orders in between all round to already-taken m
    /// values — except index 14 which reaches m=105.
    #[test]
    fn test_bootstrap_skips_duplicate_physical_orders() {
        use crate::trace_fitting::OrderTrace;
        use crate::wavelength_fitting::OrderWlSolution;

        let gc = 5000.0_f64;
        let width = 1024_u32;
        let npx = f64::from(width);
        let n_traces: usize = 16;

        let traces: Vec<OrderTrace> = (0..n_traces)
            .map(|i| OrderTrace {
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![20.0 + 15.0 * i as f64],
                    domain_start: 0.0,
                    domain_end: npx,
                },
                aperture_half_width: 5.0,
                fit_rms: 0.1,
                n_samples: 50,
                order_number: None,
            })
            .collect();

        // 5 calibrated anchors: enough for 2D Chebyshev fit (5 × 5 = 25 > 20 points)
        let anchors: [(usize, i32); 5] = [(0, 100), (3, 101), (6, 102), (9, 103), (12, 104)];

        let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
        for &(idx, m) in &anchors {
            let mf = m as f64;
            let lambda_center = gc / mf;
            // Chebyshev coeff[1] such that eval dispersion ≈ gc/(m²·npx)
            let b = gc / (mf * mf * 2.0);

            order_calibrations.push(EchelleOrderCalibration {
                relative_index: idx as u32,
                physical_order_number: Some(m),
                sample_start: 0,
                sample_end: width - 1,
                trace: traces[idx].trace.clone(),
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![lambda_center, b],
                    domain_start: 0.0,
                    domain_end: npx,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(5.0),
                enabled: true,
                notes: Some(format!("m={m}, anchor")),
            });
        }

        let mut diagnostics: Vec<OrderDiagnostic> = (0..n_traces)
            .map(|i| {
                if let Some(&(_, m)) = anchors.iter().find(|&&(idx, _)| idx == i) {
                    let mf = m as f64;
                    let lambda_center = gc / mf;
                    let b = gc / (mf * mf * 2.0);
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 10,
                        n_lines_matched: 8,
                        n_lines_used: 6,
                        rms_nm: 0.01,
                        success: true,
                        failure_reason: None,
                        detected_lines: vec![],
                        wl_solution: Some(OrderWlSolution {
                            order: i as u32,
                            coefficients: vec![lambda_center, b],
                            pixel_min: 0.0,
                            pixel_max: npx,
                            rms_nm: 0.01,
                            n_lines_used: 6,
                            n_lines_total: 8,
                        }),
                    }
                } else {
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 0,
                        n_lines_matched: 0,
                        n_lines_used: 0,
                        rms_nm: 0.0,
                        success: false,
                        failure_reason: Some("uncalibrated".to_string()),
                        detected_lines: vec![],
                        wl_solution: None,
                    }
                }
            })
            .collect();

        let bootstrapped = bootstrap_uncalibrated_orders(
            gc,
            width,
            &traces,
            &mut order_calibrations,
            &mut diagnostics,
        );

        // No duplicate physical_order_number values
        let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for cal in &order_calibrations {
            if let Some(m) = cal.physical_order_number {
                assert!(
                    seen.insert(m),
                    "duplicate physical_order_number {m} after bootstrap"
                );
            }
        }

        // With slope ≈ 1/3, most uncalibrated indices round to already-taken m values.
        // Without the fix all 11 would be bootstrapped, producing many duplicates.
        assert!(
            bootstrapped < 11,
            "expected duplicate-m orders to be skipped, but bootstrapped={bootstrapped}"
        );
    }

    /// Verify that the post-assembly safety-net deduplication correctly clears
    /// duplicate `physical_order_number` values (keeping the first occurrence).
    #[test]
    fn test_post_assembly_dedup_clears_duplicates() {
        let make_cal = |idx: u32, m: Option<i32>, notes: &str| EchelleOrderCalibration {
            relative_index: idx,
            physical_order_number: m,
            sample_start: 0,
            sample_end: 1023,
            trace: EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![100.0],
                domain_start: 0.0,
                domain_end: 1024.0,
            },
            wavelength: EchelleWavelengthModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![500.0, -0.01],
                domain_start: 0.0,
                domain_end: 1024.0,
                unit: "nm".to_string(),
            },
            aperture_half_width_px: Some(5.0),
            enabled: true,
            notes: Some(notes.to_string()),
        };

        let mut cals = vec![
            make_cal(0, Some(50), "arc-matched"),  // first m=50 — keep
            make_cal(1, Some(51), "arc-matched"),  // unique — keep
            make_cal(2, Some(50), "bootstrapped"), // dup m=50 — clear
            make_cal(3, Some(52), "bootstrapped"), // unique — keep
            make_cal(4, Some(51), "bootstrapped"), // dup m=51 — clear
            make_cal(5, None, "no m assigned"),    // already None — skip
            make_cal(6, Some(53), "bootstrapped"), // unique — keep
            make_cal(7, Some(53), "bootstrapped"), // dup m=53 — clear
        ];

        // Apply the same dedup logic as the pipeline's safety net
        {
            let mut seen_m: std::collections::HashSet<i32> = std::collections::HashSet::new();
            for cal in &mut cals {
                if let Some(m) = cal.physical_order_number
                    && !seen_m.insert(m)
                {
                    cal.physical_order_number = None;
                    if let Some(ref mut notes) = cal.notes {
                        notes.push_str(" [physical_order_number cleared: duplicate]");
                    }
                }
            }
        }

        // First occurrences kept
        assert_eq!(cals[0].physical_order_number, Some(50));
        assert_eq!(cals[1].physical_order_number, Some(51));
        assert_eq!(cals[3].physical_order_number, Some(52));
        assert_eq!(cals[6].physical_order_number, Some(53));

        // Duplicates cleared
        assert_eq!(cals[2].physical_order_number, None);
        assert_eq!(cals[4].physical_order_number, None);
        assert_eq!(cals[7].physical_order_number, None);

        // Already-None unchanged
        assert_eq!(cals[5].physical_order_number, None);

        // Notes annotated on cleared entries
        for &idx in &[2, 4, 7] {
            assert!(
                cals[idx]
                    .notes
                    .as_ref()
                    .unwrap()
                    .contains("[physical_order_number cleared: duplicate]"),
                "order {idx} should have dedup annotation in notes"
            );
        }
    }

    // ── Task 2 (bd-qe8p.1.7): Seed-model validation for cropped/sparse data ──

    /// Verify that `build_echelle_seeds` produces valid wavelength models when
    /// only half the expected orders are present (simulating a cropped detector).
    ///
    /// The seed function for each order should still map pixel → wavelength
    /// correctly: λ_center should satisfy the echelle equation m × λ = gc.
    #[test]
    fn test_echelle_seed_cropped_half_orders() {
        let gc = 2800.0;
        let n_pixels = 300_u32;
        let first_physical_order = 10;
        let order_step = -1;

        // Full detector would have ~10 orders (m=10 down to m=1).
        // Simulate a cropped detector with only the first 5.
        let n_orders = 5;
        let seeds = build_echelle_seeds(gc, first_physical_order, order_step, n_orders, n_pixels);
        assert_eq!(seeds.len(), n_orders);

        for (i, seed_fn) in seeds.iter().enumerate() {
            let m = (first_physical_order + order_step * i as i32).abs().max(1) as f64;
            let expected_center = gc / m;

            // Evaluate at the midpoint pixel
            let mid_px = f64::from(n_pixels) / 2.0;
            let wl_mid = seed_fn(mid_px);

            // The wavelength at the center pixel should be close to gc/m.
            let err = (wl_mid - expected_center).abs();
            assert!(
                err < 1.0,
                "order {i} (m={m}): seed center wavelength {wl_mid:.2}nm \
                 should be near {expected_center:.2}nm (err={err:.4})"
            );

            // Wavelength should be monotonic across the order
            let wl_left = seed_fn(0.0);
            let wl_right = seed_fn(f64::from(n_pixels));
            assert!(
                (wl_right - wl_left).abs() > 1e-3,
                "order {i}: seed should have nonzero dispersion"
            );
        }
    }

    /// Verify that the quadratic regression in Pass 2 works correctly when
    /// calibrated orders are non-consecutive (sparse data).
    ///
    /// Sets up anchors at non-consecutive order indices (0, 4, 9, 14, 19)
    /// and checks that the quadratic model interpolates reasonable m values
    /// for the gaps.
    #[test]
    fn test_quadratic_regression_sparse_orders() {
        // Simulate a real echelle: m(i) has a mild quadratic trend from
        // the prism's Cauchy dispersion.
        // True model: m(i) = 100 - 2*i + 0.01*i²
        let true_m = |i: f64| -> f64 { 100.0 - 2.0 * i + 0.01 * i * i };

        // Only 5 anchors at non-consecutive indices
        let anchors: Vec<(f64, f64)> = vec![0.0, 4.0, 9.0, 14.0, 19.0]
            .into_iter()
            .map(|i| (i, true_m(i)))
            .collect();

        let (a, b, c) = quadratic_regression(&anchors);

        // Check that the fitted model recovers m at the anchor points
        for &(i, expected_m) in &anchors {
            let predicted = a + b * i + c * i * i;
            let err = (predicted - expected_m).abs();
            assert!(
                err < 0.01,
                "anchor i={i}: predicted m={predicted:.4}, expected {expected_m:.4} (err={err:.6})"
            );
        }

        // Check interpolation at non-anchor points
        for gap_i in [2.0, 7.0, 12.0, 17.0] {
            let predicted = a + b * gap_i + c * gap_i * gap_i;
            let expected = true_m(gap_i);
            let err = (predicted - expected).abs();
            assert!(
                err < 0.1,
                "gap i={gap_i}: predicted m={predicted:.4}, expected {expected:.4} (err={err:.6})"
            );
        }
    }

    /// Verify that the quadratic regression degrades gracefully to linear
    /// when only 2 anchor points are available (too few for quadratic).
    #[test]
    fn test_quadratic_regression_too_few_points_falls_back_to_linear() {
        // With only 2 points, quadratic_regression should fall back to
        // linear regression (c=0).
        let anchors = vec![(0.0, 100.0), (10.0, 80.0)];

        let (a, b, c) = quadratic_regression(&anchors);

        // The quadratic coefficient should be zero (linear fallback)
        assert!(
            c.abs() < 1e-10,
            "with 2 points, quadratic coefficient should be ~0, got {c}"
        );

        // Linear model should still interpolate correctly
        let predicted_mid = a + b * 5.0;
        let expected_mid = 90.0; // linear interp between 100 and 80
        assert!(
            (predicted_mid - expected_mid).abs() < 0.1,
            "linear fallback: predicted {predicted_mid:.4}, expected {expected_mid:.4}"
        );
    }

    /// Verify that `bootstrap_uncalibrated_orders` works when calibrated
    /// orders cover only the top half of the detector (cropped scenario).
    ///
    /// All anchors are in indices 0..5 out of 10 total traces; the function
    /// should still bootstrap the remaining indices 5..9.
    #[test]
    fn test_bootstrap_works_with_cropped_anchors() {
        use crate::trace_fitting::OrderTrace;
        use crate::wavelength_fitting::OrderWlSolution;

        let gc = 5000.0_f64;
        let width = 1024_u32;
        let npx = f64::from(width);
        let n_traces: usize = 10;

        let traces: Vec<OrderTrace> = (0..n_traces)
            .map(|i| OrderTrace {
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![20.0 + 15.0 * i as f64],
                    domain_start: 0.0,
                    domain_end: npx,
                },
                aperture_half_width: 5.0,
                fit_rms: 0.1,
                n_samples: 50,
                order_number: None,
            })
            .collect();

        // Calibrated anchors only in the first half: indices 0..5
        let anchors: [(usize, i32); 5] = [(0, 50), (1, 51), (2, 52), (3, 53), (4, 54)];

        let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
        for &(idx, m) in &anchors {
            let mf = m as f64;
            let lambda_center = gc / mf;
            let disp = gc / (mf * mf * npx);

            order_calibrations.push(EchelleOrderCalibration {
                relative_index: idx as u32,
                physical_order_number: Some(m),
                sample_start: 0,
                sample_end: width - 1,
                trace: traces[idx].trace.clone(),
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![lambda_center - disp * npx / 2.0, disp],
                    domain_start: 0.0,
                    domain_end: npx,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(5.0),
                enabled: true,
                notes: Some(format!("m={m}, anchor")),
            });
        }

        let mut diagnostics: Vec<OrderDiagnostic> = (0..n_traces)
            .map(|i| {
                if let Some(&(_, m)) = anchors.iter().find(|&&(idx, _)| idx == i) {
                    let mf = m as f64;
                    let lambda_center = gc / mf;
                    let disp = gc / (mf * mf * npx);
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 10,
                        n_lines_matched: 8,
                        n_lines_used: 6,
                        rms_nm: 0.01,
                        success: true,
                        failure_reason: None,
                        detected_lines: vec![],
                        wl_solution: Some(OrderWlSolution {
                            order: i as u32,
                            coefficients: vec![lambda_center, disp * npx / 2.0],
                            pixel_min: 0.0,
                            pixel_max: npx,
                            rms_nm: 0.01,
                            n_lines_used: 6,
                            n_lines_total: 8,
                        }),
                    }
                } else {
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 0,
                        n_lines_matched: 0,
                        n_lines_used: 0,
                        rms_nm: 0.0,
                        success: false,
                        failure_reason: Some("uncalibrated".to_string()),
                        detected_lines: vec![],
                        wl_solution: None,
                    }
                }
            })
            .collect();

        let bootstrapped = bootstrap_uncalibrated_orders(
            gc,
            width,
            &traces,
            &mut order_calibrations,
            &mut diagnostics,
        );

        // Should bootstrap at least some of the 5 uncalibrated orders
        assert!(
            bootstrapped > 0,
            "should bootstrap at least one order from cropped anchors, got 0"
        );

        // All bootstrapped orders should have a physical_order_number
        for cal in &order_calibrations {
            assert!(
                cal.physical_order_number.is_some(),
                "order {} should have a physical_order_number",
                cal.relative_index
            );
        }

        // Bootstrapped orders should have m values that continue the sequence
        for cal in &order_calibrations {
            if let Some(m) = cal.physical_order_number {
                assert!(
                    (50..=60).contains(&m),
                    "bootstrapped m={m} for order {} is out of expected range 50..60",
                    cal.relative_index
                );
            }
        }
    }

    /// Verify that `bootstrap_uncalibrated_orders` works with sparse
    /// (non-consecutive) calibrated anchors scattered across the detector.
    #[test]
    fn test_bootstrap_works_with_sparse_anchors() {
        use crate::trace_fitting::OrderTrace;
        use crate::wavelength_fitting::OrderWlSolution;

        let gc = 5000.0_f64;
        let width = 1024_u32;
        let npx = f64::from(width);
        let n_traces: usize = 15;

        let traces: Vec<OrderTrace> = (0..n_traces)
            .map(|i| OrderTrace {
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![20.0 + 15.0 * i as f64],
                    domain_start: 0.0,
                    domain_end: npx,
                },
                aperture_half_width: 5.0,
                fit_rms: 0.1,
                n_samples: 50,
                order_number: None,
            })
            .collect();

        // Sparse anchors: only every 3rd order is calibrated
        let sparse_anchors: [(usize, i32); 5] = [(0, 80), (3, 83), (6, 86), (9, 89), (12, 92)];

        let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
        for &(idx, m) in &sparse_anchors {
            let mf = m as f64;
            let lambda_center = gc / mf;
            let disp = gc / (mf * mf * npx);

            order_calibrations.push(EchelleOrderCalibration {
                relative_index: idx as u32,
                physical_order_number: Some(m),
                sample_start: 0,
                sample_end: width - 1,
                trace: traces[idx].trace.clone(),
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![lambda_center - disp * npx / 2.0, disp],
                    domain_start: 0.0,
                    domain_end: npx,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(5.0),
                enabled: true,
                notes: Some(format!("m={m}, anchor")),
            });
        }

        let mut diagnostics: Vec<OrderDiagnostic> = (0..n_traces)
            .map(|i| {
                if let Some(&(_, m)) = sparse_anchors.iter().find(|&&(idx, _)| idx == i) {
                    let mf = m as f64;
                    let lambda_center = gc / mf;
                    let disp = gc / (mf * mf * npx);
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 10,
                        n_lines_matched: 8,
                        n_lines_used: 6,
                        rms_nm: 0.01,
                        success: true,
                        failure_reason: None,
                        detected_lines: vec![],
                        wl_solution: Some(OrderWlSolution {
                            order: i as u32,
                            coefficients: vec![lambda_center, disp * npx / 2.0],
                            pixel_min: 0.0,
                            pixel_max: npx,
                            rms_nm: 0.01,
                            n_lines_used: 6,
                            n_lines_total: 8,
                        }),
                    }
                } else {
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 0,
                        n_lines_matched: 0,
                        n_lines_used: 0,
                        rms_nm: 0.0,
                        success: false,
                        failure_reason: Some("uncalibrated".to_string()),
                        detected_lines: vec![],
                        wl_solution: None,
                    }
                }
            })
            .collect();

        let bootstrapped = bootstrap_uncalibrated_orders(
            gc,
            width,
            &traces,
            &mut order_calibrations,
            &mut diagnostics,
        );

        // Should bootstrap at least some of the 10 uncalibrated orders
        assert!(
            bootstrapped > 0,
            "should bootstrap orders from sparse anchors, got 0"
        );

        // Verify the interpolated m values are reasonable: they should fill
        // the gaps (m=81,82,84,85,87,88,90,91,93,94)
        let mut all_m: Vec<i32> = order_calibrations
            .iter()
            .filter_map(|cal| cal.physical_order_number)
            .collect();
        all_m.sort_unstable();
        all_m.dedup();

        // With stride-1 m assignment, the gaps should be filled
        assert!(
            all_m.len() > sparse_anchors.len(),
            "bootstrapping should add more m values than the {0} anchors, got {1}",
            sparse_anchors.len(),
            all_m.len()
        );
    }

    /// Verify that `bootstrap_uncalibrated_orders` returns 0 when there are
    /// too few calibrated anchors for the quadratic regression (< 3 points).
    #[test]
    fn test_bootstrap_graceful_with_too_few_anchors() {
        use crate::trace_fitting::OrderTrace;
        use crate::wavelength_fitting::OrderWlSolution;

        let gc = 5000.0_f64;
        let width = 1024_u32;
        let npx = f64::from(width);
        let n_traces: usize = 5;

        let traces: Vec<OrderTrace> = (0..n_traces)
            .map(|i| OrderTrace {
                trace: EchelleTraceModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![20.0 + 15.0 * i as f64],
                    domain_start: 0.0,
                    domain_end: npx,
                },
                aperture_half_width: 5.0,
                fit_rms: 0.1,
                n_samples: 50,
                order_number: None,
            })
            .collect();

        // Only 2 calibrated anchors — not enough for quadratic regression
        let anchors: [(usize, i32); 2] = [(0, 50), (1, 51)];

        let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
        for &(idx, m) in &anchors {
            let mf = m as f64;
            let lambda_center = gc / mf;
            let disp = gc / (mf * mf * npx);

            order_calibrations.push(EchelleOrderCalibration {
                relative_index: idx as u32,
                physical_order_number: Some(m),
                sample_start: 0,
                sample_end: width - 1,
                trace: traces[idx].trace.clone(),
                wavelength: EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![lambda_center - disp * npx / 2.0, disp],
                    domain_start: 0.0,
                    domain_end: npx,
                    unit: "nm".to_string(),
                },
                aperture_half_width_px: Some(5.0),
                enabled: true,
                notes: Some(format!("m={m}, anchor")),
            });
        }

        let mut diagnostics: Vec<OrderDiagnostic> = (0..n_traces)
            .map(|i| {
                if let Some(&(_, m)) = anchors.iter().find(|&&(idx, _)| idx == i) {
                    let mf = m as f64;
                    let lambda_center = gc / mf;
                    let disp = gc / (mf * mf * npx);
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 10,
                        n_lines_matched: 8,
                        n_lines_used: 6,
                        rms_nm: 0.01,
                        success: true,
                        failure_reason: None,
                        detected_lines: vec![],
                        wl_solution: Some(OrderWlSolution {
                            order: i as u32,
                            coefficients: vec![lambda_center, disp * npx / 2.0],
                            pixel_min: 0.0,
                            pixel_max: npx,
                            rms_nm: 0.01,
                            n_lines_used: 6,
                            n_lines_total: 8,
                        }),
                    }
                } else {
                    OrderDiagnostic {
                        order_index: i as u32,
                        n_lines_detected: 0,
                        n_lines_matched: 0,
                        n_lines_used: 0,
                        rms_nm: 0.0,
                        success: false,
                        failure_reason: Some("uncalibrated".to_string()),
                        detected_lines: vec![],
                        wl_solution: None,
                    }
                }
            })
            .collect();

        let bootstrapped = bootstrap_uncalibrated_orders(
            gc,
            width,
            &traces,
            &mut order_calibrations,
            &mut diagnostics,
        );

        // With only 2 anchors, bootstrap_uncalibrated_orders should bail
        // early (needs >= 3 for quadratic regression).
        assert_eq!(
            bootstrapped, 0,
            "should not bootstrap any orders with only 2 anchors"
        );

        // The 3 uncalibrated orders should remain failed
        for (i, diag) in diagnostics.iter().enumerate().skip(2) {
            assert!(!diag.success, "order {i} should remain uncalibrated");
        }
    }
}
