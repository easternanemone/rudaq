//! End-to-end echelle calibration pipeline.
//!
//! Orchestrates the building blocks (trace detection, extraction, atlas match,
//! wavelength fit) into a single flow that turns a raw arc frame into an
//! [`EchelleCalibrationProfile`].
//!
//! # Pipeline stages
//!
//! 1. Optional scattered-light subtraction on the arc (and flat, if supplied).
//! 2. Order trace detection, rectification, and 1D extraction.
//! 3. Per-order arc line detection (with optional HDR merge across exposures).
//! 4. **Stage 1** — per-order atlas match + Chebyshev fit, seeded from the
//!    echelle grating equation at the expected physical order.
//! 5. **Stage 2** — fit the Cauchy series `y(m) = a + b/m² + c/m⁴` from
//!    Stage-1 anchors, invert it to predict `m` for uncalibrated traces, and
//!    retry the atlas match with a physics-refined seed.
//! 6. **Stage 3** — single global 2D Chebyshev `λ(x, m)` over all matched
//!    lines (3×5, 3σ clip); synthesize per-order solutions for traces with
//!    no successful per-order fit.
//! 7. Deduplicate by physical order number and assemble the profile.

// Pixel indices and order numbers always fit in the target types at realistic
// detector sizes; the numerical code relies on f64 precision elsewhere.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use std::sync::Arc;

use chrono::Utc;

// Rayon on native, sequential shim on wasm32 (where rayon is cfg'd out at the
// workspace level). The shim keeps the per-order map closures single-sourced.
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

#[cfg(target_arch = "wasm32")]
trait ParIterShim<T> {
    fn par_iter(&self) -> core::slice::Iter<'_, T>;
}

#[cfg(target_arch = "wasm32")]
impl<T> ParIterShim<T> for Vec<T> {
    fn par_iter(&self) -> core::slice::Iter<'_, T> {
        self.iter()
    }
}

#[cfg(target_arch = "wasm32")]
impl<T> ParIterShim<T> for [T] {
    fn par_iter(&self) -> core::slice::Iter<'_, T> {
        self.iter()
    }
}

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

/// Initial pixel→wavelength guess used to bootstrap atlas matching.
#[derive(Debug, Clone)]
pub enum WavelengthSeed {
    /// User-supplied `(order_index, pixel, wavelength_nm)` anchors.
    ///
    /// With ≥2 anchors per order a linear seed is fitted; orders without
    /// anchors are interpolated from their nearest neighbours.
    Anchors(Vec<SeedAnchor>),

    /// Use the grating equation `m · λ_center ≈ constant` to seed every order.
    EchelleEquation {
        /// Grating constant in nm (`m · λ_center`). Mechelle 5000 ≈ 1_050_000 / grating_density.
        grating_constant_nm: f64,
        /// Physical diffraction order assigned to detected order index 0.
        first_physical_order: i32,
        /// Increment per detected order index (typically `-1` for echelles).
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
    /// Scatter policy applied to the arc frame before line detection.
    ///
    /// Default `None` is correct for pure-emission lamps (HgAr, ThAr) where
    /// morphological opening would over-subtract line cores.
    pub arc_scatter: Option<ScatteredLightConfig>,
    /// Scatter policy applied to the flat frame before trace detection.
    ///
    /// For MCP-intensified detectors (iStar ICCD on DH3P continuum), set to
    /// `Some(ScatteredLightConfig::mechelle_5000_istar_flat())` to remove the
    /// ~4500-count MCP halo baseline before blaze computation.
    pub flat_scatter: Option<ScatteredLightConfig>,
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
    ///
    /// Orders whose matched-line count falls below this are rejected as
    /// under-constrained — unless `allow_single_line_fallback` is set and
    /// exactly one match is available, in which case the single-line
    /// [`SingleLineFallbackSeed`] path (bd-ccer6) is used and the order
    /// is flagged with [`FitKind::AnchorOnly`] in its diagnostic.
    pub min_lines_per_order: usize,
    /// Opt in to the single-line fallback for sparsely-populated orders.
    ///
    /// When `true`, orders that matched exactly one atlas line are still
    /// calibrated (anchored at that wavelength with a physics-seeded
    /// dispersion) and flagged [`FitKind::AnchorOnly`] for downstream
    /// consumers. When `false` (default), such orders are rejected with
    /// the same "too few atlas matches" error as zero-match orders.
    ///
    /// Pairs with raising `min_lines_per_order` from 1 → 2 (bd-3yb8.30.2.3):
    /// the default posture is "two matches required", but Mechelle-class
    /// UV orders with only one visible Hg/Ar line can opt into the
    /// fallback explicitly.
    pub allow_single_line_fallback: bool,
    /// Extra arc-lamp frames merged HDR-style via [`merge_arc_lines_hdr`].
    ///
    /// Same dimensions as the primary arc. Shared via [`Arc`] so cloning the
    /// config does not duplicate multi-megapixel buffers.
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
            arc_scatter: None,
            flat_scatter: None,
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
            allow_single_line_fallback: false,
            hdr_extra_arc_frames: Vec::new(),
            hdr_merge_tol_px: 1.0,
            hdr_prefer_unsaturated: true,
        }
    }
}

// ─── Result types ────────────────────────────────────────────────────────────

/// How an order's wavelength solution was obtained.
///
/// When only a single atlas match is available for an order, the fitter
/// uses the [`SingleLineFallbackSeed`] path (bd-ccer6, supersedes
/// bd-3hlp) — the polynomial is anchored at that one wavelength and the
/// dispersion is derived from the echelle equation, not fit from data.
/// Those orders carry no per-order dispersion degrees of freedom, so
/// downstream consumers (e.g. Stage 3's global 2D Chebyshev surface,
/// extraction-quality gates, diagnostic plots) should treat them as
/// "constrained in position only" and avoid using them as independent
/// data points in cross-order fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitKind {
    /// Ordinary per-order polynomial fit with ≥`min_lines_per_order`
    /// matched atlas lines.
    Normal,
    /// Single-line fallback: the order's dispersion was seeded from the
    /// echelle equation rather than fit from data (bd-3hlp / bd-ccer6).
    AnchorOnly,
}

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
    /// How the wavelength solution was obtained — [`FitKind::Normal`] or
    /// [`FitKind::AnchorOnly`] (single-line fallback).
    pub fit_kind: FitKind,
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

    // Arc-frame scatter subtraction (optional). When enabled, the same
    // preliminary trace geometry is reused across the primary arc and every
    // HDR extra so all line-detection sources see consistently corrected data.
    let preliminary_arc_traces: Option<Vec<OrderTrace>> = if config.arc_scatter.is_some() {
        Some(detect_orders(
            arc_frame,
            width,
            height,
            &config.trace_config,
        ))
    } else {
        None
    };

    let trace_infos_for_arc_scatter: Option<Vec<TraceInfo<'_>>> =
        preliminary_arc_traces.as_ref().map(|preliminary_traces| {
            preliminary_traces
                .iter()
                .map(|t| TraceInfo {
                    trace: &t.trace,
                    disp_start: 0,
                    disp_end: width.saturating_sub(1),
                })
                .collect()
        });

    let (working_frame, arc_scatter_active): (Vec<f32>, bool) =
        match (&config.arc_scatter, trace_infos_for_arc_scatter.as_ref()) {
            (Some(scatter_cfg), Some(trace_infos)) => {
                match subtract_scattered_light(arc_frame, width, height, trace_infos, scatter_cfg) {
                    // Scatter subtraction can fail when there aren't enough
                    // inter-order pixels; fall back to the raw frame.
                    Some((corrected, _model)) => (corrected, true),
                    None => (arc_frame.to_vec(), false),
                }
            }
            _ => (arc_frame.to_vec(), false),
        };
    let frame_ref: &[f32] = working_frame.as_slice();

    // HDR extras: build owned corrected frames in one pass so downstream
    // `arc_line_sources` can hold slices without lifetime conflicts.
    let hdr_after_scatter: Vec<Option<Vec<f32>>> = match (
        arc_scatter_active,
        trace_infos_for_arc_scatter.as_ref(),
        config.arc_scatter.as_ref(),
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

    // Flat-frame scatter subtraction (optional). Applied before trace
    // detection so the detector sees a clean flat; the preliminary traces
    // are only consumed by the `InterOrderMedian` path but are harmless to
    // `MorphologicalOpening`.
    let flat_scatter_corrected: Option<Vec<f32>> = match (flat_frame, &config.flat_scatter) {
        (Some(flat), Some(scatter_cfg)) => {
            let prelim_flat_traces = detect_orders(flat, width, height, &config.trace_config);
            let flat_trace_infos: Vec<TraceInfo<'_>> = prelim_flat_traces
                .iter()
                .map(|t| TraceInfo {
                    trace: &t.trace,
                    disp_start: 0,
                    disp_end: width.saturating_sub(1),
                })
                .collect();
            subtract_scattered_light(flat, width, height, &flat_trace_infos, scatter_cfg)
                .map(|(corrected, _model)| corrected)
        }
        _ => None,
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

    // Prefer flat for trace detection (broadband illuminates all orders);
    // use the scatter-corrected flat when available, else raw flat, else arc.
    let trace_source: &[f32] = match (flat_frame, flat_scatter_corrected.as_deref()) {
        (_, Some(corrected_flat)) => corrected_flat,
        (Some(raw_flat), None) => raw_flat,
        (None, _) => frame_ref,
    };
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

    // Stage 1: per-order atlas match seeded from the grating equation at the
    // expected physical order `m = first_physical_order + step · index`.
    // Any `m` collisions between traces are resolved later by the Cauchy
    // `Y(m)` re-seed in Stage 2. Per-order work is pure wrt captured state
    // (arc_line_sources, seed_fns, traces, config) so it runs in parallel.
    let npx = f64::from(width.max(1));

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
                                fit_kind: FitKind::Normal,
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

    // `par_iter().collect()` preserves iterator order, so everything below
    // lands deterministically indexed by `order_idx`.
    let mut diagnostics = Vec::with_capacity(n_orders);
    let mut order_calibrations: Vec<EchelleOrderCalibration> = Vec::new();
    for (diag, cal) in stage1_results {
        diagnostics.push(diag);
        if let Some(c) = cal {
            order_calibrations.push(c);
        }
    }

    // Because trace index is linear in Y while `m` follows a prism-Cauchy
    // curve, Stage 1 can produce multiple fits with the same derived `m`.
    // Keep the best per `m` and demote the losers so Stage 2 can re-seed.
    dedup_order_calibrations_by_quality(&mut order_calibrations, &mut diagnostics);

    // Stage 2: fit `Y(m) = a + b/m² + c/m⁴` from Stage-1 anchors, invert to
    // predict `m` for every still-failed trace, and retry the match. This is
    // the prism-dispersion Cauchy series (see `cauchy_dispersion.rs`).
    if let Some(gc) = grating_constant_nm {
        let x_mid = npx / 2.0;
        // Physics-motivated prefilter: reject anchors whose derived `m` is
        // far from the grating-equation seed. Prism non-linearity is smooth
        // and small (|Δm| ≤ 1 on real captures); |Δm| > 10 is a sign that
        // the single-line fallback matched a wrong atlas line, which would
        // poison the Cauchy LSQ beyond what 3σ clipping can repair.
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
            // Iterative 3σ rejection — without it, a handful of wrong-m
            // Stage-1 fits pull the LSQ solution off course (seen on real
            // HgAr captures: 23 anchors → 80 px RMS when ≥4 had wrong m).
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

                // Per-failed-order refinement is pure in its captures, so we
                // run it in parallel and apply the updates sequentially below.
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
                            // Tight 2 nm `final_tolerance_nm` = `primary_window_nm`:
                            // the Cauchy-refined seed is physics-verified, so the
                            // loose 5 nm Stage-1 fallback would only admit
                            // wrong-line matches from the expanded NIST atlas.
                            let tp_config_s2 = TwoPhaseMatchConfig {
                                primary_window_nm: 2.0,
                                final_tolerance_nm: 2.0,
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

                // Ascending `order_idx` order is preserved by `par_iter().collect()`.
                for (order_idx, diag, cal) in stage2_updates {
                    if let Some(c) = cal {
                        order_calibrations.push(c);
                    }
                    diagnostics[order_idx] = diag;
                }
            }
        }
    }

    // Re-dedup so Stage 3 trains on one `(x, m, λ)` sample per `m`.
    dedup_order_calibrations_by_quality(&mut order_calibrations, &mut diagnostics);

    // Stage 3: canonical echelle global fit — tensor-product Chebyshev
    // `λ(x, m)` over every matched arc line across every order (3×5 degrees,
    // 3σ iterative rejection). For traces still without a per-order fit,
    // synthesize one by sampling the global surface at `(x, m_Cauchy)` and
    // refitting a 1D Chebyshev over the order's pixel domain.
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

    // Safety-net dedup by physical `m`. Arc-matched (Stage 1/2) calibrations
    // precede surface-synthesized ones, so first-wins preserves quality.
    // Downstream consumers require every profile order to carry an `m`, so
    // collisions drop the later order rather than clearing its `m`.
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

    // Grating-constant sanity: every calibrated order must satisfy
    // `m · λ_center ≈ gc`. Scatter > 3 % suggests the configured value is
    // mis-tuned.
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

    // Assemble the EchelleCalibrationProfile.
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
    // Arc line detection always uses boxcar summation: Horne optimal
    // extraction needs a calibrated spatial profile, and calibration is
    // downstream of arc line detection. `use_optimal_extraction` is still
    // recorded on the profile so science extraction uses Horne later.
    let spectrum_f64: Vec<f64> = simple_sum_extract(&rect);
    Some(spectrum_f64.iter().map(|&v| v as f32).collect())
}

/// Extract the 1D spectrum for an order and detect arc lines; merges across
/// frames in HDR mode, preferring unsaturated peaks.
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

/// Match detected arc lines to the atlas and fit the wavelength solution.
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
        fit_kind: FitKind::Normal,
        failure_reason: None,
        detected_lines: lines.to_vec(),
        wl_solution: None,
    };

    // Minimum detected-lines threshold: when the single-line fallback is
    // enabled, one detected peak is enough to attempt a match. When it's
    // off, the full `min_lines_per_order` is required up front
    // (bd-3yb8.30.2.3).
    let effective_min_detected = if config.allow_single_line_fallback {
        1
    } else {
        config.min_lines_per_order
    };
    if lines.len() < effective_min_detected {
        diag.failure_reason = Some(format!(
            "too few arc lines detected ({}, need {})",
            lines.len(),
            effective_min_detected
        ));
        return diag;
    }

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

    // Single-line fallback opt-in (bd-3yb8.30.2.3 / bd-3hlp): when
    // `allow_single_line_fallback` is true and exactly one match is
    // available, proceed through the fitter — the single-line path
    // (bd-ccer6) will anchor the polynomial at that wavelength and
    // derive dispersion from the echelle equation. Flag the resulting
    // solution as [`FitKind::AnchorOnly`] so downstream consumers know
    // it carries no per-order dispersion degrees of freedom.
    let anchor_only_path =
        config.allow_single_line_fallback && matches.len() == 1 && config.min_lines_per_order > 1;
    let effective_min_matches = if anchor_only_path {
        1
    } else {
        config.min_lines_per_order
    };
    if matches.len() < effective_min_matches {
        diag.failure_reason = Some(format!(
            "too few atlas matches ({}, need {})",
            matches.len(),
            effective_min_matches
        ));
        return diag;
    }
    if anchor_only_path {
        diag.fit_kind = FitKind::AnchorOnly;
    }

    // Thread the physical order + grating constant through to the fitter so
    // its single-line fallback can derive a physically-correct dispersion.
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
            // Self-consistency gate: when the two-phase seed supplies a
            // candidate `m` + grating constant, the fitted midpoint wavelength
            // must agree with `λ_center = gc / m` to within ±FSR. Without
            // this, a degree-reduced 2-point fit on cross-order matches can
            // report RMS=0 while being physically wrong.
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

/// Rectify, extract, detect, match, and fit a single order (HDR-aware).
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
                    fit_kind: FitKind::Normal,
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

/// Boxed seed closure mapping pixel → approximate wavelength in nm.
///
/// `Send + Sync` lets Stage 1 / Stage 2 dispatch seed evaluations across
/// rayon threads; existing builders only capture `f64` primitives.
type SeedFn = Box<dyn Fn(f64) -> f64 + Send + Sync>;

/// Build per-order seed wavelength closures from the seed configuration.
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

/// Keep only the best calibration per `physical_order_number` and demote the
/// losers back to "failed" so Stages 2/3 can re-seed them.
///
/// Ranking: most matched lines, then lowest RMS, then first arrival. Called
/// between Stage 1→2 and Stage 2→3; without it the Cauchy LSQ sees duplicate
/// `(y_centroid, m)` anchors and blows up.
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

/// Fit a global 2D Chebyshev `λ(x, m)` and synthesize missing per-order fits.
///
/// This is the IRAF ECIDENTIFY / ESO MIDAS / CERES / PypeIt standard: fit the
/// tensor-product surface over every matched arc line from Stages 1 + 2 with
/// 3σ iterative rejection (degrees 3×5), then for every trace still without a
/// per-order fit sample the surface at `(x, m_Cauchy)` and refit a 1D
/// Chebyshev of the configured per-order degree. Returns the number of orders
/// recovered by synthesis.
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
    // We don't retain the per-line `(pixel, λ_atlas)` pairs after fitting, so
    // we sample each successful per-order solution densely. This is equivalent
    // to training on the fitted Chebyshev itself — fine because each per-order
    // fit already minimises χ² on its matched atlas lines.
    let mut training: Vec<(f64, u32, f64)> = Vec::new();
    for diag in diagnostics.iter() {
        if !diag.success {
            continue;
        }
        let Some(ref sol) = diag.wl_solution else {
            continue;
        };
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
        for k in 0..16 {
            let frac = (k as f64 + 0.5) / 16.0;
            let px = sol.pixel_min + (sol.pixel_max - sol.pixel_min) * frac;
            let lam = sol.eval(px);
            training.push((px, m_u32, lam));
        }
    }

    // CERES default: degrees 3 (pixel) × 5 (order) → need ≥ 4·6 = 24 points.
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

    // Rebuild the Cauchy `Y(m)` from the calibrated anchor set so we can
    // assign `m` to traces that never matched any atlas line. Same physics
    // prefilter as Stage 2 (see `MAX_ABS_DELTA_M` above).
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

        // Predict `m` from the Cauchy `Y(m)`. Requires a valid midpoint
        // centroid to place the trace on the 2D surface.
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

        // Trust the surface only inside its fitted `m` envelope —
        // tensor-product Chebyshevs Runge-oscillate when extrapolated.
        if (m_f - 0.5) < surface.m_min || (m_f + 0.5) > surface.m_max {
            continue;
        }

        if !assigned_m.insert(m_int_candidate) {
            continue;
        }

        // Sample the surface and refit a 1D Chebyshev so downstream code
        // treats this solution identically to a directly-matched fit.
        let n_samples = 32usize;
        let px_min = 0.0f64;
        let px_max = npx - 1.0;
        let mut wls: Vec<f64> = Vec::with_capacity(n_samples);
        let mut x_norms: Vec<f64> = Vec::with_capacity(n_samples);
        for k in 0..n_samples {
            let frac = (k as f64 + 0.5) / n_samples as f64;
            let px = px_min + (px_max - px_min) * frac;
            wls.push(surface.eval_lambda(px, m_f));
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

        // Reject non-monotonic axes or solutions wandering outside [150, 1200] nm.
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

/// Build seed closures from the echelle grating equation.
///
/// Per order: `λ_center = gc / m`, `FSR = gc / m²` (one FSR spans the
/// detector), linear dispersion `FSR / n_pixels`.
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
/// Sample range is clamped to the solution's fitted domain because the
/// Chebyshev is only reliable inside its training data. When a grating
/// constant is supplied, the physical order is `round(gc / λ_mid)`.
fn build_order_calibration(
    trace: &OrderTrace,
    sol: &OrderWlSolution,
    order_index: u32,
    width: u32,
    grating_constant_nm: Option<f64>,
) -> EchelleOrderCalibration {
    // The Chebyshev was fit with normalisation over [pixel_min, pixel_max];
    // changing the domain would rescale the x → [-1, 1] mapping and produce
    // wrong wavelengths, so we clamp the sample range to the fitted window.
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

/// Coefficient of variation of `m · λ_center` across all calibrated orders.
///
/// Uses the profile's `physical_order_number` when set, else the seed-based
/// assignment. Values below 0.01 (1 %) indicate good grating-equation
/// consistency.
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
        .filter_map(|d| {
            let sol = d.wl_solution.as_ref()?;

            let m = result
                .profile
                .orders
                .iter()
                .find(|o| o.relative_index == d.order_index)
                .and_then(|o| o.physical_order_number)
                .map(|m| m.unsigned_abs() as f64)
                .unwrap_or_else(|| {
                    (first_physical_order + order_step * d.order_index as i32).unsigned_abs() as f64
                })
                .max(1.0);

            let mid_pixel = sol.pixel_min.midpoint(sol.pixel_max);
            Some(m * sol.eval(mid_pixel))
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

    /// Build the canonical 3-order HgAr synthetic used by several tests:
    /// 200×300 frame, three horizontal bands at y = 60 / 150 / 240 covering
    /// 400–420, 500–525, 700–740 nm, with up to 5 strongest HgAr lines per
    /// order injected on top of a continuum. Returns (frame, config).
    fn build_synthetic_hgar_pipeline(
        profile_name: &str,
    ) -> (Vec<f32>, usize, usize, CalibrationPipelineConfig) {
        let width = 200;
        let height = 300;
        let orders = vec![
            (60.0, 400.0, 420.0),
            (150.0, 500.0, 525.0),
            (240.0, 700.0, 740.0),
        ];

        // The full NIST-backed HgAr atlas has ~360 entries; injecting every
        // line into a 200-px order produces unresolved blends. Cap at the
        // 5 strongest per order, close to what a real iStar detects.
        let atlas = load_hgar_atlas();
        let mut injected: Vec<f64> = Vec::new();
        for &(_, lo, hi) in &orders {
            let mut in_range: Vec<&AtlasLine> = atlas
                .iter()
                .filter(|a| a.wavelength_nm >= lo && a.wavelength_nm <= hi)
                .collect();
            in_range.sort_by(|a, b| {
                b.strength
                    .partial_cmp(&a.strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            injected.extend(in_range.iter().take(5).map(|a| a.wavelength_nm));
        }

        let frame = synthetic_arc_frame(width, height, &orders, &injected, 2.5, 2000.0, 2.5);

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
            profile_name: profile_name.to_string(),
            min_lines_per_order: 2,
            ..Default::default()
        };

        (frame, width, height, config)
    }

    #[test]
    fn test_pipeline_with_synthetic_arc() {
        // Height is chosen so the order peaks are a small fraction of the
        // spatial profile, otherwise the inter-percentile noise estimator
        // gets inflated by the peaks themselves.
        let (frame, width, height, config) = build_synthetic_hgar_pipeline("Test HgAr Calibration");

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
        // Identical primary + extra exposure must merge to the same line
        // census as a single exposure (duplicate detections coalesce within
        // merge tolerance).
        let (frame, width, height, base) = build_synthetic_hgar_pipeline("HDR dup test");

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
        // Hypothetical echelle with gc=2800: detector width covers 1 FSR at
        // m=10 (λ_c=280 nm). Orders m=10, 9, 8 at y = 60, 150, 240.
        let width = 300;
        let height = 300;

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

        // Hg lines that fall inside those ranges.
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
            fit_kind: FitKind::Normal,
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
            fit_kind: FitKind::Normal,
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
    fn test_non_consecutive_order_refinement() {
        // Three orders at non-consecutive `m` (10, 9, 6) with the seed
        // assuming consecutive orders starting at m=10. Stage 1 picks up
        // m=10 and m=9; Stage 2's Cauchy regression must recover m=6.
        let width = 300;
        let height = 400;
        let grating_const = 2800.0;

        let m10_center = grating_const / 10.0;
        let m9_center = grating_const / 9.0;
        let m6_center = grating_const / 6.0;

        let m_ref = 10.0f64;
        let fsr_ref = grating_const / (m_ref * m_ref);
        let disp_ref = fsr_ref / width as f64;

        let m10_disp = disp_ref;
        let m9_disp = disp_ref * (m_ref / 9.0);
        let m6_disp = disp_ref * (m_ref / 6.0);

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

        // Atlas lines placed to land in each order's range.
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

    /// Verify `build_echelle_seeds` on a cropped detector (half the expected
    /// orders): each seed's midpoint must satisfy `m · λ = gc`.
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

    // ── B3 (bd-3yb8.30.2.3): single-line fallback opt-in ────────────────
    //
    // These tests exercise `match_and_fit` directly — the private entry
    // point where `min_lines_per_order` and `allow_single_line_fallback`
    // interact. Synthetic arc lines are positioned near a known atlas
    // line so the two-phase matcher finds a match at a predictable
    // (pixel, wavelength) anchor. A constant seed_fn is enough because
    // we only verify the pass/fail + `fit_kind` branch, not dispersion
    // accuracy.

    fn b3_test_config(
        min_lines_per_order: usize,
        allow_single_line_fallback: bool,
    ) -> CalibrationPipelineConfig {
        CalibrationPipelineConfig {
            atlas: vec![AtlasLine {
                wavelength_nm: 546.074, // Hg I green
                species: "Hg I".into(),
                strength: 1.0,
            }],
            wl_config: WlFitConfig {
                poly_degree: 1,
                seed_tolerance_nm: 2.0,
                max_fit_rms_nm: 0.0, // disable RMS gate for this unit test
                ..Default::default()
            },
            frame_compat: EchelleFrameCompatibility {
                sensor_width: 200,
                sensor_height: 200,
                frame_width: 200,
                frame_height: 200,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: Some(16),
            },
            min_lines_per_order,
            allow_single_line_fallback,
            ..Default::default()
        }
    }

    fn b3_single_arc_line() -> ArcLine {
        ArcLine {
            order: 0,
            pixel_center: 100.0,
            pixel_sigma: 1.5,
            amplitude: 1000.0,
            wavelength_hint: None,
            used: true,
            saturated: false,
        }
    }

    fn b3_two_phase() -> TwoPhaseMatchConfig {
        // grating constant chosen so gc/m ≈ 546 nm at m=20 — matches the
        // single-line atlas entry so the two-phase matcher seeds cleanly.
        TwoPhaseMatchConfig {
            physical_order: 20.0,
            grating_constant_nm: 546.074 * 20.0,
            primary_window_nm: 2.0,
            fallback_tolerance_nm: 1.0,
            final_tolerance_nm: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn b3_single_match_rejected_when_fallback_disabled() {
        let config = b3_test_config(2, false);
        let seed_fn = |_px: f64| 546.074_f64;
        let lines = vec![b3_single_arc_line()];
        let tp = b3_two_phase();
        let diag = match_and_fit(&lines, 0, &config, &seed_fn, Some(&tp));
        assert!(!diag.success, "expected rejection, got success: {diag:?}");
        assert_eq!(diag.fit_kind, FitKind::Normal);
        // Either gate is a valid rejection: the detected-lines gate fires
        // first when min_lines_per_order=2 and only one line was detected.
        let reason = diag.failure_reason.as_deref().unwrap_or("");
        assert!(
            reason.contains("too few arc lines detected")
                || reason.contains("too few atlas matches"),
            "unexpected failure reason: {:?}",
            diag.failure_reason
        );
    }

    #[test]
    fn b3_single_match_accepted_as_anchor_only_when_fallback_enabled() {
        let config = b3_test_config(2, true);
        let seed_fn = |_px: f64| 546.074_f64;
        let lines = vec![b3_single_arc_line()];
        let tp = b3_two_phase();
        let diag = match_and_fit(&lines, 0, &config, &seed_fn, Some(&tp));
        assert!(
            diag.success,
            "expected single-line fallback to succeed, got: {:?}",
            diag.failure_reason
        );
        assert_eq!(diag.fit_kind, FitKind::AnchorOnly);
        assert_eq!(diag.n_lines_matched, 1);
    }

    #[test]
    fn b3_default_config_rejects_single_match() {
        // `CalibrationPipelineConfig::default()` keeps `allow_single_line_fallback`
        // off — regression guard so library consumers that take the
        // default don't silently admit under-constrained orders.
        let default = CalibrationPipelineConfig::default();
        assert!(!default.allow_single_line_fallback);
        assert_eq!(default.min_lines_per_order, 3);
    }
}
