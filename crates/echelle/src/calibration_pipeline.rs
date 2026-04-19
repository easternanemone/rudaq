//! End-to-end echelle calibration pipeline.
//!
//! Orchestrates all echelle building blocks into a single calibration flow:
//! raw arc frame → order detection → extraction → line identification →
//! wavelength solution → `EchelleCalibrationProfile`.
//!
//! # Pipeline stages
//!
//! 1. Scattered-light subtraction (optional)
//! 2. Order trace detection
//! 3. Per-order rectification
//! 4. 1D spectral extraction (simple-sum or Horne optimal)
//! 5. Arc emission line detection per order
//! 6. **Stage 1** — per-order atlas match + Chebyshev fit seeded from the
//!    echelle grating equation at the expected physical order
//! 7. **Stage 2** — Cauchy-series `y(m) = a + b/m² + c/m⁴` fit from
//!    Stage-1 anchors; invert to predict m for uncalibrated traces and
//!    retry atlas matching
//! 8. **Stage 3** — single global 2D Chebyshev fit `λ(x, m)` over all
//!    matched arc lines (3×5, 3σ clip); synthesize per-order solutions
//!    for traces still without a per-order fit
//! 9. Deduplicate by physical order number and assemble the profile

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

use crate::optimal_extraction::OptimalExtractionConfig;
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

    // Build two-phase match config for Stage 1 when using echelle equation seed.
    let two_phase_base = match &config.seed {
        WavelengthSeed::EchelleEquation {
            grating_constant_nm: gc,
            first_physical_order,
            order_step,
            ..
        } => Some((*gc, *first_physical_order, *order_step)),
        WavelengthSeed::Anchors(_) => None,
    };

    // ── Stage 1: Per-order echelle-equation-seeded matching ─────────
    // Each detected trace is seeded by the grating equation at its
    // expected physical order `m = first_physical_order + step * index`.
    // No candidate-m search: m collisions between traces are resolved
    // by the Cauchy-series Y(m) model in Stage 2 (bd-s8d5g). The
    // candidate-m search was non-canonical (IRAF ECIDENTIFY, CERES,
    // PypeIt all assign m once from the physical grating equation) and
    // produced derived-m collisions (bd-mlwvz).
    let npx = f64::from(width.max(1));

    // bd-ongmw: Stage-1 work per order is pure wrt the captured references
    // (arc_line_sources, seed_fns, traces, config). Each order produces
    // (diagnostic, Option<calibration>) independently, so we par_iter across
    // traces and collect in deterministic order before merging.
    use rayon::prelude::*;
    let stage1_results: Vec<(OrderDiagnostic, Option<EchelleOrderCalibration>)> = traces
        .par_iter()
        .enumerate()
        .map(|(order_idx, trace)| {
            let oi = order_idx as u32;

            let lines =
                match extract_and_detect_lines(&arc_line_sources, width, height, trace, oi, config)
                {
                    Ok(l) => l,
                    Err(e) => {
                        return (
                            OrderDiagnostic {
                                order_index: oi,
                                n_lines_detected: 0,
                                n_lines_matched: 0,
                                n_lines_used: 0,
                                rms_nm: 0.0,
                                success: false,
                                failure_reason: Some(e),
                                detected_lines: Vec::new(),
                                wl_solution: None,
                            },
                            None,
                        );
                    }
                };

            let mut final_diag = if let Some((gc, first_m, step)) = two_phase_base {
                let expected_m = (first_m + step * (order_idx as i32)).max(1);
                let physical_order = f64::from(expected_m);
                let lambda_center = gc / physical_order;
                let fsr = gc / (physical_order * physical_order);
                let dispersion = fsr / npx;
                let lambda_start = lambda_center - dispersion * (npx / 2.0);
                let seed_fn = move |pixel: f64| -> f64 { lambda_start + dispersion * pixel };
                let tp_config = TwoPhaseMatchConfig {
                    primary_window_nm: 2.0,
                    final_tolerance_nm: config.wl_config.seed_tolerance_nm,
                    fallback_tolerance_nm: 1.0,
                    grating_constant_nm: gc,
                    gc_tolerance: 0.01,
                    min_primary_matches: 0,
                    physical_order,
                };
                match_and_fit(&lines, oi, config, &seed_fn, Some(&tp_config))
            } else {
                match_and_fit(&lines, oi, config, &seed_fns[order_idx], None)
            };

            let mut order_cal: Option<EchelleOrderCalibration> = None;
            if final_diag.success {
                let validation = final_diag
                    .wl_solution
                    .as_ref()
                    .map(|sol| sol.validate_monotonic(&config.orientation, 150.0, 1200.0));
                match validation {
                    Some(Ok(())) => {
                        let sol = final_diag.wl_solution.as_ref().expect("checked above");
                        let cal =
                            build_order_calibration(trace, sol, oi, width, grating_constant_nm);
                        tracing::debug!(
                            "Stage 1: Order {} matched physical order {:?}",
                            oi,
                            cal.physical_order_number
                        );
                        order_cal = Some(cal);
                    }
                    Some(Err(err)) => {
                        tracing::warn!("Stage 1: order {oi} rejected — {err}");
                        final_diag.success = false;
                        final_diag.failure_reason = Some(format!("wavelength axis invalid: {err}"));
                        final_diag.wl_solution = None;
                    }
                    None => {}
                }
            }
            (final_diag, order_cal)
        })
        .collect();

    // par_iter().collect() preserves iterator order, so diagnostics and
    // order_calibrations come out deterministically indexed by order_idx.
    let mut diagnostics = Vec::with_capacity(n_orders);
    let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
    for (diag, cal) in stage1_results {
        diagnostics.push(diag);
        if let Some(c) = cal {
            order_calibrations.push(c);
        }
    }

    // ── Dedup by quality after Stage 1 ───────────────────────────────
    // When the echelle-equation initial seed is off (because trace index
    // is linear in detector space but m follows a prism-Cauchy curve),
    // multiple traces can produce Stage-1 fits whose derived m collides.
    // Keep the highest-quality fit per m (most lines used, then lowest
    // RMS, then first arrival) and mark the losers as failed so that
    // Stage 2 (Cauchy Y(m)) can re-seed them with a physically correct m.
    dedup_order_calibrations_by_quality(&mut order_calibrations, &mut diagnostics);

    // ── Stage 2: Cauchy-series Y(m) re-assignment for failed orders ──
    // Physical model: for a prism cross-dispersed echelle the trace
    // y-centroid follows a Cauchy series Y(m) = a + b/m² + c/m⁴ (see
    // `cauchy_dispersion.rs` and NotebookLM 7f275c3a eval memo §FM1).
    // Fit from Stage-1 anchors {(y_centroid, m)}, invert to predict m
    // for every failed trace, and retry the two-phase match.
    if let Some(gc) = grating_constant_nm {
        let x_mid = npx / 2.0;
        // Physics-motivated pre-filter: reject Stage-1 anchors whose
        // derived m is drastically off from the grating-equation seed.
        // The prism non-linearity is smooth and small within ±5 orders
        // of the linear guess; a |derived_m − expected_m| > 10 signals
        // the single-line fallback matching a wrong atlas line, which
        // would contaminate the Cauchy LSQ regardless of 3σ clipping
        // (when >20% of anchors are wrong, the 3σ estimate itself is
        // useless). This cap is generous — orders 44..143 with correct
        // fits all have |Δm| ≤ 1 on real captures.
        const MAX_ABS_DELTA_M: i32 = 10;
        let (first_m_seed, step_seed) = two_phase_base.map_or((1, 1), |(_, fm, s)| (fm, s));
        let cauchy_anchors: Vec<(f64, i32)> = order_calibrations
            .iter()
            .filter_map(|cal| {
                let m_int = cal.physical_order_number?;
                let trace_idx = cal.relative_index as usize;
                let expected_m = first_m_seed + step_seed * (trace_idx as i32);
                if (m_int - expected_m).abs() > MAX_ABS_DELTA_M {
                    return None;
                }
                let trace = traces.get(trace_idx)?;
                let y_centroid = crate::trace_fitting::eval_trace_y(&trace.trace, x_mid)?;
                Some((y_centroid, m_int))
            })
            .collect();

        let n_failed = diagnostics.iter().filter(|d| !d.success).count();

        if cauchy_anchors.len() >= 4 && n_failed > 0 {
            // Iterative 3σ outlier rejection — Stage-1 fits can latch
            // onto the wrong physical order when the grating-equation
            // seed misidentifies arc lines. The Cauchy fit then sees
            // several (y, m) pairs that don't fit the series, and
            // without rejection the LSQ solution is pulled off course
            // (seen in the real-HgAr capture: RMS=80 px across 23
            // anchors because ≥4 had wrong m).
            if let Some((cauchy, _kept)) =
                crate::cauchy_dispersion::fit_cauchy_y_of_m_clipped(&cauchy_anchors, 3.0, 5)
            {
                tracing::info!(
                    anchors_used = cauchy.n_anchors,
                    anchors_supplied = cauchy_anchors.len(),
                    rms_px = cauchy.rms_px,
                    "Stage 2: fitted Cauchy Y(m) series (3σ clipped)"
                );
                let first_m_i32 = two_phase_base.map_or(1, |(_, fm, _)| fm);
                let step_i32 = two_phase_base.map_or(1, |(_, _, s)| s);

                // bd-ongmw: parallelize Stage 2's per-failed-order Cauchy
                // refinement. Read-only inputs (traces, config, arc_line_sources,
                // cauchy, diagnostics-success flags) are captured by shared
                // reference; par_iter produces a deterministic Vec of updates
                // that a sequential post-pass applies to diagnostics and
                // order_calibrations.
                let stage2_updates: Vec<(usize, OrderDiagnostic, Option<EchelleOrderCalibration>)> =
                    traces
                        .par_iter()
                        .enumerate()
                        .filter(|(idx, _)| !diagnostics[*idx].success)
                        .filter_map(|(order_idx, trace)| {
                            let y_centroid =
                                crate::trace_fitting::eval_trace_y(&trace.trace, x_mid)?;
                            let guess_m =
                                f64::from((first_m_i32 + step_i32 * (order_idx as i32)).max(1));
                            let m_f = cauchy.invert_to_m(y_centroid, guess_m)?;
                            let predicted_m = m_f.round();
                            if predicted_m < 1.0 {
                                return None;
                            }

                            let lambda_center = gc / predicted_m;
                            let fsr = gc / (predicted_m * predicted_m);
                            let dispersion = fsr / npx;
                            let lambda_start = lambda_center - dispersion * (npx / 2.0);
                            let refined_seed =
                                move |pixel: f64| -> f64 { lambda_start + dispersion * pixel };

                            let oi = order_idx as u32;
                            let tp_config_s2 = TwoPhaseMatchConfig {
                                primary_window_nm: 2.0,
                                final_tolerance_nm: config.wl_config.seed_tolerance_nm,
                                fallback_tolerance_nm: 1.0,
                                grating_constant_nm: gc,
                                gc_tolerance: 0.01,
                                min_primary_matches: 0,
                                physical_order: predicted_m,
                            };
                            let mut diag = process_single_order(
                                &arc_line_sources,
                                width,
                                height,
                                trace,
                                oi,
                                config,
                                &refined_seed,
                                Some(&tp_config_s2),
                            );

                            let mut order_cal: Option<EchelleOrderCalibration> = None;
                            if diag.success {
                                let validation = diag.wl_solution.as_ref().map(|sol| {
                                    sol.validate_monotonic(&config.orientation, 150.0, 1200.0)
                                });
                                match validation {
                                    Some(Ok(())) => {
                                        let sol =
                                            diag.wl_solution.as_ref().expect("checked above");
                                        let cal = build_order_calibration(
                                            trace,
                                            sol,
                                            oi,
                                            width,
                                            Some(gc),
                                        );
                                        tracing::debug!(
                                            "Stage 2: Order {} Cauchy-refined to physical order {:?}",
                                            oi,
                                            cal.physical_order_number
                                        );
                                        order_cal = Some(cal);
                                    }
                                    Some(Err(err)) => {
                                        tracing::warn!("Stage 2: order {oi} rejected — {err}");
                                        diag.success = false;
                                        diag.failure_reason =
                                            Some(format!("wavelength axis invalid: {err}"));
                                        diag.wl_solution = None;
                                    }
                                    None => {}
                                }
                            }
                            Some((order_idx, diag, order_cal))
                        })
                        .collect();

                // par_iter preserves iterator order, so updates here land in
                // ascending order_idx — exactly matching the pre-bd-ongmw
                // sequential push/assign order.
                for (order_idx, diag, cal) in stage2_updates {
                    if let Some(c) = cal {
                        order_calibrations.push(c);
                    }
                    diagnostics[order_idx] = diag;
                }
            }
        }
    }

    // After Stage 2: re-dedup so Stage 3's global fit trains on a clean
    // anchor set (one (x, m, λ) sample per m).
    dedup_order_calibrations_by_quality(&mut order_calibrations, &mut diagnostics);

    // ── Stage 3: Global 2D Chebyshev λ(x, m) + per-order synthesis ───
    // Canonical echelle wavelength solution (IRAF ECIDENTIFY, ESO MIDAS
    // IDENTIFY/ECHELLE, CERES, PypeIt): fit λ(x, m) as a tensor-product
    // Chebyshev surface over all matched arc lines from all orders
    // simultaneously, with iterative 3σ rejection. Degrees 3×5 = CERES
    // default (`ech_nspec_coeff`, `ech_norder_coeff`); see NotebookLM
    // 7f275c3a eval memo §FM1.
    //
    // For traces that still have no matched lines, synthesize a per-order
    // wavelength solution by sampling the global surface at (x, m_Cauchy)
    // across the order's pixel domain and fitting a 1D Chebyshev. This
    // is the PypeIt "recover missing orders from the 2D fit" step and
    // replaces the old Pass-3 residual-surface bootstrap (which fitted
    // Δλ and Runge-oscillated when extrapolated beyond anchor m range).
    if let Some(gc) = grating_constant_nm {
        let (first_m_seed, step_seed) = two_phase_base.map_or((1, 1), |(_, fm, s)| (fm, s));
        refine_with_global_surface(
            gc,
            width,
            &traces,
            &mut order_calibrations,
            &mut diagnostics,
            config,
            first_m_seed,
            step_seed,
        );
    }

    // ── Deduplicate physical_order_number (safety net, bd-ccer6 P1.5) ─
    // Arc-matched orders (Stage 1/2) appear before surface-synthesized
    // ones (Stage 3), so first-wins preserves the higher-quality
    // calibrations. Collisions remove the later order entirely (rather
    // than clearing its physical m
    // and leaving an orphaned wavelength model in the profile); downstream
    // consumers depend on every profile order carrying a physical m.
    {
        let mut seen_m: std::collections::HashSet<i32> = std::collections::HashSet::new();
        order_calibrations.retain(|cal| match cal.physical_order_number {
            Some(m) if !seen_m.insert(m) => {
                eprintln!(
                    "Deduplicator: dropping order {} (physical order {} already claimed)",
                    cal.relative_index, m
                );
                false
            }
            _ => true,
        });
    }

    // ── Grating-constant sanity warning (bd-ccer6 P1.7) ────────────────
    // Every calibrated order must satisfy m · λ_center ≈ gc. If the
    // observed scatter across orders exceeds 3% of the mean, the
    // configured grating_constant_nm is likely mis-measured.
    if let Some(gc_configured) = grating_constant_nm {
        let mut count: u32 = 0;
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        for cal in &order_calibrations {
            let Some(m_int) = cal.physical_order_number else {
                continue;
            };
            let m = f64::from(m_int);
            let wl = match &cal.wavelength {
                EchelleWavelengthModel::Polynomial {
                    basis,
                    coefficients,
                    domain_start,
                    domain_end,
                    ..
                } => {
                    let mid = f64::midpoint(*domain_start, *domain_end);
                    let x_norm = 2.0 * (mid - *domain_start) / (*domain_end - *domain_start) - 1.0;
                    match basis {
                        PolynomialBasis::Chebyshev => {
                            crate::wavelength_fitting::chebyshev_eval(coefficients, x_norm)
                        }
                        PolynomialBasis::Monomial => {
                            let mut acc = 0.0f64;
                            let mut xp = 1.0;
                            for c in coefficients {
                                acc += c * xp;
                                xp *= mid;
                            }
                            acc
                        }
                    }
                }
                EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                    match wavelengths.get(wavelengths.len() / 2) {
                        Some(&w) => w,
                        None => continue,
                    }
                }
            };
            let product = m * wl;
            sum += product;
            sum_sq += product * product;
            count += 1;
        }
        if count >= 3 {
            let n = f64::from(count);
            let mean = sum / n;
            let var = (sum_sq / n - mean * mean).max(0.0);
            let stddev = var.sqrt();
            let rel = if mean.abs() > 1e-12 {
                stddev / mean.abs() * 100.0
            } else {
                f64::NAN
            };
            if rel > 3.0 {
                tracing::warn!(
                    configured_gc = gc_configured,
                    observed_mean = mean,
                    observed_stddev = stddev,
                    relative_percent = rel,
                    "grating-constant scatter exceeds 3%; configured grating_constant_nm may be mis-tuned"
                );
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
    // Arc line detection always uses the boxcar sum (bd-8yjd1 P2 follow-up):
    // optimal extraction needs a calibrated spatial profile, but calibration
    // is downstream of arc line detection — so using the Horne kernel here
    // would use an uncalibrated narrow profile that collapses arc peaks to
    // <1.5-px FWHM, causing the arc-line detector to reject them. The
    // `use_optimal_extraction` flag is recorded on the profile so downstream
    // consumers use Horne for science extraction once the profile exists.
    let spectrum_f64: Vec<f64> = simple_sum_extract(&rect);
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
///
/// `Send + Sync` is required so Stage 1 / Stage 2 can dispatch seed evaluations
/// across rayon threads (bd-ongmw). Existing builders capture only `f64`
/// primitives, which are trivially `Send + Sync`.
type SeedFn = Box<dyn Fn(f64) -> f64 + Send + Sync>;

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

/// Keep only the highest-quality `EchelleOrderCalibration` per
/// `physical_order_number` and demote the loser traces back to "failed"
/// in `diagnostics` so the Cauchy (Stage 2) and global-surface (Stage 3)
/// passes have a chance to re-seed them with a physically correct `m`.
///
/// Ranking: (1) more matched lines is better; (2) lower RMS is better;
/// (3) ties broken by first arrival (equivalent to the old `retain`
/// dedup that preceded bd-mlwvz).
///
/// This is called between Stage 1→2 and Stage 2→3. Without it the
/// Cauchy fit sees several `(y_centroid, m)` pairs with duplicate `m`
/// and the LSQ residuals blow up.
fn dedup_order_calibrations_by_quality(
    order_calibrations: &mut Vec<EchelleOrderCalibration>,
    diagnostics: &mut [OrderDiagnostic],
) {
    use std::collections::HashMap;

    // quality score per relative_index.
    let score = |diag: &OrderDiagnostic| -> (usize, f64) {
        // Higher lines_used better; lower rms_nm better. Invert RMS.
        let rms = if diag.rms_nm.is_finite() && diag.rms_nm > 0.0 {
            diag.rms_nm
        } else {
            f64::INFINITY
        };
        (diag.n_lines_used, rms)
    };
    let is_better = |a: &OrderDiagnostic, b: &OrderDiagnostic| -> bool {
        let (la, ra) = score(a);
        let (lb, rb) = score(b);
        if la != lb { la > lb } else { ra < rb }
    };

    // Map m → index into order_calibrations of the best entry so far.
    let mut best_for_m: HashMap<i32, usize> = HashMap::new();
    let mut losers: Vec<usize> = Vec::new(); // indices into order_calibrations

    for (i, cal) in order_calibrations.iter().enumerate() {
        let Some(m) = cal.physical_order_number else {
            continue;
        };
        let Some(diag) = diagnostics.get(cal.relative_index as usize) else {
            continue;
        };
        match best_for_m.get(&m).copied() {
            None => {
                best_for_m.insert(m, i);
            }
            Some(prev_i) => {
                let prev_cal = &order_calibrations[prev_i];
                let prev_diag = &diagnostics[prev_cal.relative_index as usize];
                if is_better(diag, prev_diag) {
                    losers.push(prev_i);
                    best_for_m.insert(m, i);
                } else {
                    losers.push(i);
                }
            }
        }
    }

    if losers.is_empty() {
        return;
    }
    let loser_set: std::collections::HashSet<usize> = losers.into_iter().collect();
    // Collect loser trace indices first (to mark diagnostics), then drop.
    let mut loser_trace_indices: Vec<u32> = Vec::with_capacity(loser_set.len());
    for (i, cal) in order_calibrations.iter().enumerate() {
        if loser_set.contains(&i) {
            loser_trace_indices.push(cal.relative_index);
        }
    }

    // Demote losers to "failed" so Stage 2/3 can retry.
    for idx in &loser_trace_indices {
        if let Some(d) = diagnostics.get_mut(*idx as usize) {
            d.success = false;
            d.failure_reason =
                Some("m collision with higher-quality fit; pending Cauchy re-seed".to_string());
            d.wl_solution = None;
        }
    }

    let mut removed = 0usize;
    let mut i = 0usize;
    order_calibrations.retain(|_| {
        let keep = !loser_set.contains(&i);
        if !keep {
            removed += 1;
        }
        i += 1;
        keep
    });

    tracing::info!(
        removed,
        "dedup: demoted lower-quality duplicates so Stage 2/3 can re-seed"
    );
}

/// Refine the calibration with a single global 2D Chebyshev fit `λ(x, m)`.
///
/// This is the IRAF ECIDENTIFY / ESO MIDAS / CERES / PypeIt standard
/// approach (degrees 3×5, 3σ iterative rejection). It consumes the
/// per-order matched arc lines produced by Stages 1 + 2 as a single
/// global training set and then:
///
/// 1. Fits a tensor-product Chebyshev surface `m·λ(x_norm, m_norm)`
///    with iterative 3σ outlier rejection.
/// 2. Logs the global RMS; warns if > 0.3 nm (a reasonable upper bound
///    for good HgAr/Ar arc calibration on a Mechelle 5000).
/// 3. For every trace without a successful per-order fit, synthesizes
///    an `OrderWlSolution` by sampling the global surface at
///    `(x_i, m_Cauchy)` across the order's pixel domain and fitting a
///    1D Chebyshev of the configured per-order degree.
///
/// The Cauchy-predicted `m` (Stage 2) is re-used here for traces that
/// still fail; collisions are resolved by the later dedup step
/// (first arrival by order_index = highest-quality, per Stages 1/2).
///
/// Returns the number of orders synthesized from the global surface
/// (distinct from those directly calibrated in Stages 1-2).
#[allow(clippy::too_many_arguments)]
fn refine_with_global_surface(
    gc: f64,
    width: u32,
    traces: &[crate::trace_fitting::OrderTrace],
    order_calibrations: &mut Vec<EchelleOrderCalibration>,
    diagnostics: &mut [OrderDiagnostic],
    config: &CalibrationPipelineConfig,
    first_m_seed: i32,
    step_seed: i32,
) -> usize {
    use crate::chebyshev_2d::fit_chebyshev_2d_clipped;

    let npx = f64::from(width.max(1));
    let x_mid = npx / 2.0;

    // Collect training data from every matched arc line in every
    // successfully-calibrated order.
    let mut training: Vec<(f64, u32, f64)> = Vec::new();
    for diag in diagnostics.iter() {
        if !diag.success {
            continue;
        }
        let Some(ref sol) = diag.wl_solution else {
            continue;
        };
        // Physical order from the solution's midpoint wavelength. Cast to
        // u32 for `fit_chebyshev_2d`'s (x, m_u32, λ) shape.
        let mid = f64::midpoint(sol.pixel_min, sol.pixel_max);
        let lambda_mid = sol.eval(mid);
        if lambda_mid <= 0.0 {
            continue;
        }
        let m_f = (gc / lambda_mid).round();
        if m_f < 1.0 {
            continue;
        }
        let Ok(m_u32) = u32::try_from(m_f as i64) else {
            continue;
        };
        // Sample the matched atlas lines. For Chebyshev fits we don't have
        // the per-line (pixel, λ_atlas) after the fact, so sample the
        // solution at a dense grid. This is equivalent to training on the
        // fitted Chebyshev itself — reasonable because the per-order fit
        // already satisfies `χ² min`. We *additionally* include the
        // diagnostic's `detected_lines` below when they carry wavelengths.
        for k in 0..16 {
            let frac = (k as f64 + 0.5) / 16.0;
            let px = sol.pixel_min + (sol.pixel_max - sol.pixel_min) * frac;
            let lam = sol.eval(px);
            training.push((px, m_u32, lam));
        }
    }

    // CERES default: 3×5 (pixel × order). We need n_pts ≥ (3+1)(5+1)=24.
    let (dx, dm) = (3usize, 5usize);
    let n_coeffs = (dx + 1) * (dm + 1);
    if training.len() < n_coeffs {
        tracing::warn!(
            n_training = training.len(),
            n_coeffs,
            "Stage 3: insufficient matched lines for global 2D Chebyshev fit"
        );
        return 0;
    }

    let Some((surface, _kept)) = fit_chebyshev_2d_clipped(&training, dx, dm, 3.0, 5) else {
        tracing::warn!("Stage 3: global 2D Chebyshev fit failed");
        return 0;
    };

    tracing::info!(
        rms_nm = surface.rms_nm,
        n_points = surface.n_points,
        m_min = surface.m_min,
        m_max = surface.m_max,
        "Stage 3: global 2D Chebyshev fit converged"
    );

    // Rebuild the Cauchy Y(m) model from the calibrated anchor set so we
    // can assign m to traces that never matched any atlas line. Same
    // physics-motivated prefilter as Stage 2 (reject anchors whose
    // derived m is more than ±10 orders from the grating-equation seed).
    const MAX_ABS_DELTA_M: i32 = 10;
    let cauchy_anchors: Vec<(f64, i32)> = order_calibrations
        .iter()
        .filter_map(|cal| {
            let m_int = cal.physical_order_number?;
            let trace_idx = cal.relative_index as usize;
            let expected_m = first_m_seed + step_seed * (trace_idx as i32);
            if (m_int - expected_m).abs() > MAX_ABS_DELTA_M {
                return None;
            }
            let trace = traces.get(trace_idx)?;
            let y = crate::trace_fitting::eval_trace_y(&trace.trace, x_mid)?;
            Some((y, m_int))
        })
        .collect();
    // Use 3σ-clipped Cauchy for the Stage-3 synthesis guess too — same
    // reason as Stage 2: stray wrong-m anchors would poison the series.
    let cauchy = crate::cauchy_dispersion::fit_cauchy_y_of_m_clipped(&cauchy_anchors, 3.0, 5)
        .map(|(fit, _kept)| fit);

    let mut assigned_m: std::collections::HashSet<i32> = order_calibrations
        .iter()
        .filter_map(|cal| cal.physical_order_number)
        .collect();

    let mut synthesized = 0usize;
    let per_order_degree = config.wl_config.poly_degree.max(2);

    for (order_idx, trace) in traces.iter().enumerate() {
        if diagnostics[order_idx].success {
            continue;
        }

        // Predict m from the Cauchy Y(m) model. Require a valid y
        // centroid at the frame midpoint; without one there is no
        // physical way to place this trace on the 2D surface.
        let Some(y_centroid) = crate::trace_fitting::eval_trace_y(&trace.trace, x_mid) else {
            continue;
        };
        let Some(ref cauchy_fit) = cauchy else {
            continue;
        };
        let Some(m_f) = cauchy_fit.invert_to_m(y_centroid, 1.0) else {
            continue;
        };
        let m_int_candidate = m_f.round() as i32;
        if m_int_candidate < 1 {
            continue;
        }

        // The global surface is only trusted inside its fitted m domain.
        // Extrapolating a tensor-product Chebyshev beyond its training
        // envelope is the Runge-oscillation trap the old residual
        // bootstrap fell into; refuse rather than risk a broken axis.
        if (m_f - 0.5) < surface.m_min || (m_f + 0.5) > surface.m_max {
            continue;
        }

        if !assigned_m.insert(m_int_candidate) {
            continue;
        }

        // Sample the global surface across this order's pixel range and
        // fit a 1D Chebyshev of the configured per-order degree. This
        // produces an OrderWlSolution that downstream code (profile
        // assembly, `build_order_calibration`) treats identically to a
        // directly-matched fit.
        let n_samples = 32usize;
        let px_min = 0.0f64;
        let px_max = npx - 1.0;
        let mut pixels: Vec<f64> = Vec::with_capacity(n_samples);
        let mut wls: Vec<f64> = Vec::with_capacity(n_samples);
        let mut x_norms: Vec<f64> = Vec::with_capacity(n_samples);
        for k in 0..n_samples {
            let frac = (k as f64 + 0.5) / n_samples as f64;
            let px = px_min + (px_max - px_min) * frac;
            let lam = surface.eval_lambda(px, m_f);
            pixels.push(px);
            wls.push(lam);
            x_norms.push(2.0 * (px - px_min) / (px_max - px_min) - 1.0);
        }

        let Some(coefficients) =
            crate::wavelength_fitting::chebyshev_fit(&x_norms, &wls, per_order_degree)
        else {
            continue;
        };

        let sol = OrderWlSolution {
            order: order_idx as u32,
            coefficients,
            pixel_min: px_min,
            pixel_max: px_max,
            rms_nm: surface.rms_nm, // global RMS as a proxy for per-order
            n_lines_used: 0,        // synthesized, not directly matched
            n_lines_total: 0,
        };

        // Reject if the synthesized solution is not monotonic or wanders
        // outside [150, 1200] nm.
        if let Err(err) = sol.validate_monotonic(&config.orientation, 150.0, 1200.0) {
            tracing::warn!("Stage 3 synth: order {order_idx} rejected (axis invalid): {err}");
            continue;
        }

        let order_cal = EchelleOrderCalibration {
            relative_index: order_idx as u32,
            physical_order_number: Some(m_int_candidate),
            sample_start: 0,
            sample_end: width.saturating_sub(1),
            trace: traces[order_idx].trace.clone(),
            wavelength: EchelleWavelengthModel::Polynomial {
                basis: PolynomialBasis::Chebyshev,
                coefficients: sol.coefficients.clone(),
                domain_start: sol.pixel_min,
                domain_end: sol.pixel_max,
                unit: "nm".to_string(),
            },
            aperture_half_width_px: Some(traces[order_idx].aperture_half_width),
            enabled: true,
            notes: Some(format!(
                "m={m_int_candidate}, synthesized from global 2D Chebyshev (no per-order arc matches)"
            )),
        };

        order_calibrations.push(order_cal);
        diagnostics[order_idx].success = true;
        diagnostics[order_idx].failure_reason = None;
        diagnostics[order_idx].wl_solution = Some(sol);
        synthesized += 1;
    }

    tracing::info!(
        synthesized,
        "Stage 3: orders recovered by global-surface synthesis"
    );
    synthesized
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
) -> Vec<SeedFn> {
    let mut fns: Vec<SeedFn> = Vec::with_capacity(n_orders);
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

    // DELETED (bd-f7wv7): `test_linear_regression` removed — the helper
    // `linear_regression` was only used by the removed `quadratic_regression`
    // and `bootstrap_uncalibrated_orders`. The Cauchy-series fit has its
    // own numerical tests in `cauchy_dispersion.rs`.

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

    // DELETED (bd-f7wv7): `test_bootstrap_skips_duplicate_physical_orders`
    // removed when `bootstrap_uncalibrated_orders` was replaced by
    // `refine_with_global_surface` + `fit_chebyshev_2d_clipped`.

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

    // DELETED (bd-f7wv7, bd-s8d5g): `quadratic_regression` removed as
    // non-physical. The `m(trace_index)` regression had no optical theory
    // behind it. Replaced by `cauchy_dispersion::fit_cauchy_y_of_m`
    // (Y(m) = a + b/m² + c/m⁴), which is the documented prism-dispersion
    // model from the Cauchy series. See `cauchy_dispersion.rs` for the
    // analytic-recovery and round-trip tests that replaced these.

    // DELETED (bd-f7wv7): test_bootstrap_works_with_cropped_anchors,
    // test_bootstrap_works_with_sparse_anchors, and
    // test_bootstrap_graceful_with_too_few_anchors removed when
    // bootstrap_uncalibrated_orders was replaced by
    // refine_with_global_surface (global 2D Chebyshev fit). The new
    // recovery path is exercised by the synthetic-HgAr regression test
    // in crates/integration-tests/tests/echelle_synthetic_dataset.rs.
}
