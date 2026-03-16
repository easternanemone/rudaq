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

use chrono::Utc;

use crate::echelle::{
    AxisDirection, DetectorAxis, EchelleCalibrationProfile, EchelleCorrections,
    EchelleExtractionConfig, EchelleFrameCompatibility, EchelleOrderCalibration,
    EchelleOrientation, EchelleProvenance, EchelleSchemaVersion, EchelleSummationMode,
    EchelleWavelengthModel, PolynomialBasis,
};
use crate::echelle_optimal_extraction::{optimal_extract, OptimalExtractionConfig};
use crate::echelle_rectification::{rectify_order, OrderSpec, RectifyConfig};
use crate::echelle_scattered_light::{subtract_scattered_light, ScatteredLightConfig, TraceInfo};
use crate::echelle_trace_fitting::{detect_orders, OrderTrace, TraceFitConfig};
use crate::echelle_wavelength_fitting::{
    detect_arc_lines, fit_order_wavelength, match_lines_to_atlas, ArcDetectConfig, ArcLine,
    AtlasLine, OrderWlSolution, WlFitConfig,
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
}

impl Default for CalibrationPipelineConfig {
    fn default() -> Self {
        Self {
            trace_config: TraceFitConfig::default(),
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
    if let Some(flat) = flat_frame {
        if flat.len() < w * h {
            return Err(format!(
                "flat frame too small: {} pixels for {}x{} = {}",
                flat.len(),
                width,
                height,
                w * h
            ));
        }
    }

    // ── Stage 1: Scattered light subtraction (optional) ──────────────
    let working_frame: Vec<f32>;
    let frame_ref = if let Some(scatter_cfg) = &config.scatter_config {
        // For scattered light, we need trace info. Detect orders first on the raw frame
        // to build the inter-order mask, then subtract, then re-detect on the clean frame.
        let preliminary_traces = detect_orders(arc_frame, width, height, &config.trace_config);
        let trace_infos: Vec<TraceInfo<'_>> = preliminary_traces
            .iter()
            .map(|t| TraceInfo {
                trace: &t.trace,
                disp_start: 0,
                disp_end: width.saturating_sub(1),
            })
            .collect();

        if let Some((corrected, _model)) =
            subtract_scattered_light(arc_frame, width, height, &trace_infos, scatter_cfg)
        {
            working_frame = corrected;
            working_frame.as_slice()
        } else {
            // Scattered light subtraction failed (not enough inter-order pixels);
            // proceed with the raw frame.
            working_frame = arc_frame.to_vec();
            working_frame.as_slice()
        }
    } else {
        working_frame = arc_frame.to_vec();
        working_frame.as_slice()
    };

    // ── Stage 2: Order trace detection ───────────────────────────────
    // Use flat frame for trace detection if provided (broadband source
    // illuminates all orders); otherwise detect from the arc frame.
    let trace_source = flat_frame.unwrap_or(frame_ref);
    let traces = detect_orders(trace_source, width, height, &config.trace_config);
    if traces.is_empty() {
        return Err(if flat_frame.is_some() {
            "no echelle orders detected in flat frame".to_string()
        } else {
            "no echelle orders detected in frame".to_string()
        });
    }
    let n_orders = traces.len();

    // Build seed wavelength functions per order.
    let seed_fns = build_seed_functions(&config.seed, n_orders, width)?;

    // ── Stages 3-6: Per-order processing ─────────────────────────────
    // Always extract arc lines from the arc frame (frame_ref), using
    // trace positions found from the flat/arc frame above.
    let mut diagnostics = Vec::with_capacity(n_orders);
    let mut order_calibrations = Vec::new();

    for (order_idx, trace) in traces.iter().enumerate() {
        let oi = order_idx as u32;
        let diag = process_single_order(
            frame_ref,
            width,
            height,
            trace,
            oi,
            config,
            &seed_fns[order_idx],
        );

        if diag.success {
            if let Some(ref sol) = diag.wl_solution {
                let order_cal = build_order_calibration(trace, sol, oi, width);
                order_calibrations.push(order_cal);
            }
        }
        diagnostics.push(diag);
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

/// Process a single order: rectify → extract → detect lines → match → fit.
fn process_single_order(
    frame: &[f32],
    width: u32,
    height: u32,
    trace: &OrderTrace,
    order_index: u32,
    config: &CalibrationPipelineConfig,
    seed_fn: &dyn Fn(f64) -> f64,
) -> OrderDiagnostic {
    let mut diag = OrderDiagnostic {
        order_index,
        n_lines_detected: 0,
        n_lines_matched: 0,
        n_lines_used: 0,
        rms_nm: 0.0,
        success: false,
        failure_reason: None,
        detected_lines: Vec::new(),
        wl_solution: None,
    };

    // ── Stage 3: Rectify ─────────────────────────────────────────────
    let spec = OrderSpec {
        trace: &trace.trace,
        disp_start: 0,
        disp_end: width.saturating_sub(1),
        order_index,
    };
    let Some(rect) = rectify_order(frame, width, height, &spec, &config.rectify_config) else {
        diag.failure_reason = Some("rectification failed (trace out of bounds)".into());
        return diag;
    };

    // ── Stage 4: Extract 1D spectrum ─────────────────────────────────
    let spectrum_f64: Vec<f64> = if config.use_optimal_extraction {
        match optimal_extract(&rect, None, &config.optimal_config) {
            Some(result) => result.flux,
            None => {
                // Fall back to simple summation.
                simple_sum_extract(&rect)
            }
        }
    } else {
        simple_sum_extract(&rect)
    };

    // Convert to f32 for arc line detection (API requirement).
    let spectrum_f32: Vec<f32> = spectrum_f64.iter().map(|&v| v as f32).collect();

    // ── Stage 5: Detect arc lines ────────────────────────────────────
    let lines = detect_arc_lines(&spectrum_f32, order_index, &config.arc_config);
    diag.n_lines_detected = lines.len();
    diag.detected_lines.clone_from(&lines);

    if lines.len() < config.min_lines_per_order {
        diag.failure_reason = Some(format!(
            "too few arc lines detected ({}, need {})",
            lines.len(),
            config.min_lines_per_order
        ));
        return diag;
    }

    // ── Stage 6: Match to atlas and fit wavelength solution ──────────
    let matches = match_lines_to_atlas(
        &lines,
        &config.atlas,
        seed_fn,
        config.wl_config.seed_tolerance_nm,
    );
    diag.n_lines_matched = matches.len();

    if matches.len() < config.min_lines_per_order {
        diag.failure_reason = Some(format!(
            "too few atlas matches ({}, need {})",
            matches.len(),
            config.min_lines_per_order
        ));
        return diag;
    }

    match fit_order_wavelength(
        &lines,
        &config.atlas,
        &matches,
        order_index,
        &config.wl_config,
    ) {
        Some(sol) => {
            diag.n_lines_used = sol.n_lines_used;
            diag.rms_nm = sol.rms_nm;
            diag.success = true;
            diag.wl_solution = Some(sol);
        }
        None => {
            diag.failure_reason =
                Some("wavelength fit failed (singular or too few points after clipping)".into());
        }
    }

    diag
}

/// Simple aperture-weighted summation extraction.
fn simple_sum_extract(rect: &crate::echelle_rectification::RectifiedOrder) -> Vec<f64> {
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

    // For an echelle grating, angular dispersion dbeta/dlambda is m / (d * cos(beta)).
    // Linear dispersion dx/dlambda across the detector is proportional to m.
    // Therefore, the wavelength dispersion dl/dx (nm/pixel) is proportional to 1/m.
    // We assume the detector width `n_pixels` covers exactly 1 FSR at the FIRST order
    // to anchor the constant of proportionality.
    let m_ref = (first_physical_order).abs().max(1) as f64;
    let fsr_ref = grating_constant_nm / (m_ref * m_ref);
    let disp_ref = fsr_ref / npx;

    for i in 0..n_orders {
        let m = (first_physical_order + order_step * i as i32).abs().max(1) as f64;
        let lambda_center = grating_constant_nm / m;

        // Dispersion scales as 1/m, so disp = disp_ref * (m_ref / m).
        let dispersion = disp_ref * (m_ref / m);
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
fn build_order_calibration(
    trace: &OrderTrace,
    sol: &OrderWlSolution,
    order_index: u32,
    width: u32,
) -> EchelleOrderCalibration {
    // The Chebyshev coefficients were fitted using normalization over
    // [pixel_min, pixel_max]. The domain MUST match the fitted range exactly —
    // extending it would change the x → [-1,1] mapping and produce wrong wavelengths.
    // The sample range is constrained to the fitted domain.
    let sample_start = sol.pixel_min.ceil().max(0.0) as u32;
    let sample_end = (sol.pixel_max.floor() as u32).min(width.saturating_sub(1));

    EchelleOrderCalibration {
        relative_index: order_index,
        physical_order_number: None, // Unknown until echelle equation is applied.
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
        notes: Some(format!(
            "RMS={:.4}nm, {}/{} lines",
            sol.rms_nm, sol.n_lines_used, sol.n_lines_total
        )),
    }
}

// ─── Cross-order consistency check ───────────────────────────────────────────

/// Check echelle equation consistency: m × λ_center should be approximately
/// constant across all calibrated orders.
///
/// `order_step` is the increment per detected order index (typically -1 for
/// echelle spectrographs where higher Y → lower order number).
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
        .filter_map(|d| {
            let sol = d.wl_solution.as_ref()?;
            let m =
                (first_physical_order + order_step * d.order_index as i32).unsigned_abs() as f64;
            let m = m.max(1.0); // guard against m=0
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
    use crate::echelle_wavelength_fitting::load_hgar_atlas;

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
}
