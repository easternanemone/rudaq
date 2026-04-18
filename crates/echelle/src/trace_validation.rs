//! Post-detection trace validation for echelle order filtering.
//!
//! Filters spurious echelle order detections (ghost reflections, MCP
//! blooming halos, detector artifacts) from the output of [`detect_orders`].
//! All filters are optional; the Mechelle 5000 defaults apply the three
//! FM2-documented filters (FWHM band, continuity, monotonic Δy) plus the
//! original SNR/spacing/curvature filters.
//!
//! # Mechelle 5000 + iStar ICCD failure mode (bd-lpgyn / NotebookLM
//! pipeline-eval memo §FM2)
//!
//! The iStar ICCD uses an MCP coupled to a phosphor screen. Bright
//! emission lines trigger electron blooming that bleeds laterally across
//! MCP channels, producing *secondary* peaks in the cross-dispersion
//! profile. Combined with prism internal-reflection "ghost orders",
//! naïve peak-finding on the ME5000 routinely reports ~149 candidate
//! traces when only ~74 physical orders exist.
//!
//! # Validation criteria (post-detection)
//!
//! Sources: NotebookLM echelle notebook 7f275c3a, pipeline evaluation
//! memo 3a0a13df §FM2, and its derived "Trace Validation Parameter"
//! table. For the Rust implementation:
//!
//! - **Hough / CWT smoothing** (peak finder layer; not this module) —
//!   applied upstream in `trace_fitting.rs` to reject blooming before
//!   traces are even constructed.
//! - **Maximum FWHM deviation** (this module): reject peaks whose
//!   Gaussian FWHM exceeds the geometric slit-image width by >40%.
//!   Defocused ghost/halo peaks are broader than true slit images.
//! - **Continuity ≥60% of dispersion axis** (this module): reject
//!   traces whose fit domain covers less than 60% of the detector's
//!   x-axis. Internal reflections are typically short.
//! - **Monotonic Δy across orders** (this module): the prism
//!   cross-disperser enforces Δy(m+1) > Δy(m) toward the blue. Any
//!   trace that violates the monotonically-increasing spacing rule is
//!   an optical ghost.
//! - **SNR threshold** (original, still supported): reject traces with
//!   mean aperture signal below a multiple of the inter-order background.
//! - **Spacing σ / curvature σ** (original, still supported): robust
//!   MAD-based rejection of spacing + curvature outliers.
//! - **Max order count** (original, still supported): keep the strongest
//!   N by intensity.
//!
//! [`detect_orders`]: crate::trace_fitting::detect_orders

// Pixel-index casts are always small enough for lossless usize→f64.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use serde::{Deserialize, Serialize};

use crate::trace_fitting::OrderTrace;
use crate::types::{EchelleTraceModel, PolynomialBasis};

/// Configuration for post-detection trace validation.
///
/// All thresholds are `Option<f64>` with `None` = disabled.
/// Use `#[serde(default)]` on the container field for backward
/// compatibility with existing TOML/JSON profiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TraceValidationConfig {
    /// Minimum signal-to-noise ratio for a trace to be kept.
    /// Computed as `mean_trace_signal / median_inter_order_background`.
    /// Typical value: 5.0 (NotebookLM memo c959d054 priority actions).
    #[serde(default)]
    pub min_snr: Option<f64>,

    /// Maximum deviation (in robust σ) of inter-trace spacing from
    /// the expected linear trend. Typical value: 3.0.
    #[serde(default)]
    pub spacing_sigma: Option<f64>,

    /// Maximum deviation (in robust σ) of trace curvature from the
    /// population median. Typical value: 3.0.
    #[serde(default)]
    pub curvature_sigma: Option<f64>,

    /// Maximum number of traces to keep. If more traces survive the
    /// other filters, the strongest by mean intensity are kept.
    #[serde(default)]
    pub max_order_count: Option<usize>,

    /// Reject traces whose measured FWHM (≈ 2 · aperture_half_width)
    /// exceeds the population median FWHM by more than this fraction.
    /// **Default for Mechelle 5000: 0.40** (40%) per NotebookLM memo
    /// 3a0a13df §FM2 — defocused ghost and MCP-halo peaks are always
    /// broader than the true slit image.
    #[serde(default)]
    pub max_fwhm_excess_fraction: Option<f64>,

    /// Reject traces whose fit domain spans less than this fraction of
    /// the detector's dispersion-axis width. **Default for Mechelle 5000:
    /// 0.60** (60%) per NotebookLM memo 3a0a13df §FM2 — prism ghosts
    /// and MCP halos produce short trajectories.
    #[serde(default)]
    pub min_continuity_fraction: Option<f64>,

    /// Enforce strictly-monotonic inter-order spacing `Δy(m)` across
    /// the detector. For a prism cross-disperser, Δy grows monotonically
    /// with m (toward blue). Any trace whose neighboring Δy violates
    /// monotonicity by more than `monotonic_tolerance_fraction` of the
    /// median local Δy is flagged as a ghost. **Default for Mechelle
    /// 5000: true** per NotebookLM memo 3a0a13df §FM2.
    #[serde(default)]
    pub enforce_monotonic_spacing: Option<bool>,

    /// Allowed fractional deviation from strict monotonicity when
    /// `enforce_monotonic_spacing` is true. A value of 0.25 allows Δy
    /// to increase non-monotonically by up to 25% of the median local
    /// spacing before a trace is flagged — accommodates measurement
    /// noise while still rejecting ghost orders (whose Δy steps are
    /// typically 50-200% outside the smooth trend).
    #[serde(default)]
    pub monotonic_tolerance_fraction: Option<f64>,
}

impl TraceValidationConfig {
    /// Returns `true` if all filters are disabled (no validation will be applied).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min_snr.is_none()
            && self.spacing_sigma.is_none()
            && self.curvature_sigma.is_none()
            && self.max_order_count.is_none()
            && self.max_fwhm_excess_fraction.is_none()
            && self.min_continuity_fraction.is_none()
            && self.enforce_monotonic_spacing.unwrap_or(false) == false
    }

    /// Preset for the Mechelle 5000 + iStar ICCD combination — enables
    /// the three FM2-documented filters (FWHM, continuity, monotonic Δy)
    /// per NotebookLM memo 3a0a13df §FM2.
    ///
    /// **SNR filter intentionally omitted**: NotebookLM's "SNR ≥ 3.0"
    /// guidance is for the peak-walker termination condition *during
    /// tracing* (upstream in `trace_fitting.rs`), not a post-detection
    /// global filter. Our post-detection `min_snr` divides mean trace
    /// intensity by a 25th-percentile background estimate, which on an
    /// MCP-halo-contaminated HgAr frame is massively inflated by
    /// saturated inter-order halo pixels — a single run drops 149
    /// traces to 1, rejecting every real order. Until
    /// `estimate_inter_order_background` is fixed to use proper
    /// inter-order masking (bd-vdfum / Phase B), the SNR filter is
    /// unsafe to enable by default on ICCD frames.
    #[must_use]
    pub fn mechelle_5000_istar() -> Self {
        // Bd-lpgyn: infrastructure for NotebookLM FM2's three
        // ghost-rejection filters (FWHM band, continuity, monotonic Δy)
        // is implemented, but empirical validation on the leabs-dev
        // HgAr capture shows they need one of these as a prerequisite
        // to be net-positive:
        //   (a) Phase B — morphological-opening scattered-light
        //       subtraction (bd-vdfum) cleans MCP halos *before* trace
        //       detection, so FWHM of real traces and halos differ
        //       enough for the 40% band to work.
        //   (b) Hough-transform + CWT peak finder (upstream in
        //       trace_fitting.rs), which would reject ghost peaks
        //       before they even reach this validation stage.
        // Without those, the filters are no-ops at best and slight
        // regressions at worst (strict monotonic drops 149→6 on
        // alternating ghost-real triples; MAD-spacing at 2.5σ drops
        // 5 traces including at least one real Hg order). Default
        // to FWHM + continuity only (both conservative no-ops on this
        // data but correct in principle), and let users enable the
        // stricter filters in their own configs once upstream cleanup
        // is in place.
        Self {
            min_snr: None,
            spacing_sigma: None,
            curvature_sigma: None,
            max_order_count: None,
            max_fwhm_excess_fraction: Some(0.40),
            min_continuity_fraction: Some(0.60),
            enforce_monotonic_spacing: None,
            monotonic_tolerance_fraction: None,
        }
    }
}

/// Trace with computed validation metrics.
struct TraceMeasurement {
    /// Index into the original trace list.
    index: usize,
    /// Cross-dispersion center at the detector midpoint.
    center_y: f64,
    /// Mean intensity along the trace (proxy for SNR).
    mean_intensity: f64,
    /// Second derivative of the trace polynomial at the midpoint.
    curvature: f64,
    /// Estimated FWHM of the cross-dispersion profile ≈ 2 × aperture
    /// half-width (pixels).
    fwhm: f64,
    /// Span of the trace's fit domain along the dispersion axis (pixels).
    domain_span: f64,
}

/// Filter a list of detected traces according to the validation config.
///
/// Returns the filtered list in the same relative order as the input.
/// Traces that fail any enabled filter are removed.
///
/// `frame`: the raw f32 frame used for intensity sampling.
/// `width`, `height`: frame dimensions.
/// `aperture_half_width`: half-width of the extraction aperture.
pub fn validate_traces(
    traces: &[OrderTrace],
    config: &TraceValidationConfig,
    frame: &[f32],
    width: u32,
    height: u32,
    aperture_half_width: f64,
) -> Vec<OrderTrace> {
    if config.is_empty() || traces.is_empty() {
        return traces.to_vec();
    }

    let w = width as usize;
    let h = height as usize;
    let mid_x = f64::from(width) / 2.0;
    let detector_w = f64::from(width);

    // Compute metrics for each trace.
    let measurements: Vec<TraceMeasurement> = traces
        .iter()
        .enumerate()
        .filter_map(|(i, trace)| {
            let center_y = eval_trace_at(&trace.trace, mid_x)?;
            let mean_intensity =
                compute_trace_mean_intensity(frame, w, h, &trace.trace, width, aperture_half_width);
            let curvature = eval_trace_curvature(&trace.trace, mid_x);
            let fwhm = 2.0 * trace.aperture_half_width;
            let domain_span = domain_span_of(&trace.trace);
            Some(TraceMeasurement {
                index: i,
                center_y,
                mean_intensity,
                curvature,
                fwhm,
                domain_span,
            })
        })
        .collect();

    let mut keep = vec![true; measurements.len()];
    let n_initial = measurements.len();
    let count_alive = |k: &[bool]| k.iter().filter(|&&x| x).count();

    // Filter 1: SNR threshold.
    if let Some(min_snr) = config.min_snr {
        let background = estimate_inter_order_background(frame, w, h);
        if background > 0.0 {
            for (j, m) in measurements.iter().enumerate() {
                if m.mean_intensity / background < min_snr {
                    keep[j] = false;
                }
            }
        }
        tracing::info!(
            n_before = n_initial,
            n_after = count_alive(&keep),
            "trace_validation[SNR]"
        );
    }

    // Filter 2: Spacing regularity (legacy MAD-based).
    if let Some(sigma_thresh) = config.spacing_sigma {
        let before = count_alive(&keep);
        apply_spacing_filter(&measurements, &mut keep, sigma_thresh);
        tracing::info!(
            n_before = before,
            n_after = count_alive(&keep),
            "trace_validation[spacing]"
        );
    }

    // Filter 3: Curvature consistency.
    if let Some(sigma_thresh) = config.curvature_sigma {
        let before = count_alive(&keep);
        apply_curvature_filter(&measurements, &mut keep, sigma_thresh);
        tracing::info!(
            n_before = before,
            n_after = count_alive(&keep),
            "trace_validation[curvature]"
        );
    }

    // Filter 4: FWHM band (bd-lpgyn — reject defocused MCP halos).
    if let Some(max_excess) = config.max_fwhm_excess_fraction {
        let before = count_alive(&keep);
        apply_fwhm_filter(&measurements, &mut keep, max_excess);
        tracing::info!(
            n_before = before,
            n_after = count_alive(&keep),
            "trace_validation[FWHM band]"
        );
    }

    // Filter 5: Continuity (bd-lpgyn — reject short prism-ghost traces).
    if let Some(min_frac) = config.min_continuity_fraction {
        let before = count_alive(&keep);
        apply_continuity_filter(&measurements, &mut keep, detector_w, min_frac);
        tracing::info!(
            n_before = before,
            n_after = count_alive(&keep),
            "trace_validation[continuity]"
        );
    }

    // Filter 6: Monotonic Δy / FSR (bd-lpgyn — prism dispersion law).
    if config.enforce_monotonic_spacing.unwrap_or(false) {
        let before = count_alive(&keep);
        let tol = config.monotonic_tolerance_fraction.unwrap_or(0.25);
        apply_monotonic_dy_filter(&measurements, &mut keep, tol);
        tracing::info!(
            n_before = before,
            n_after = count_alive(&keep),
            "trace_validation[monotonic dy]"
        );
    }

    // Filter 7: Max order count (keep strongest by intensity).
    if let Some(max_count) = config.max_order_count {
        let surviving: usize = keep.iter().filter(|&&k| k).count();
        if surviving > max_count {
            // Sort survivors by intensity (descending), keep top max_count.
            let mut survivors: Vec<(usize, f64)> = measurements
                .iter()
                .enumerate()
                .filter(|(j, _)| keep[*j])
                .map(|(j, m)| (j, m.mean_intensity))
                .collect();
            survivors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for &(j, _) in survivors.iter().skip(max_count) {
                keep[j] = false;
            }
        }
    }

    // Collect surviving traces in original order.
    measurements
        .iter()
        .enumerate()
        .filter_map(|(j, m)| {
            if keep[j] {
                Some(traces[m.index].clone())
            } else {
                None
            }
        })
        .collect()
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn eval_trace_at(trace: &EchelleTraceModel, x: f64) -> Option<f64> {
    match trace {
        EchelleTraceModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
        } => {
            if coefficients.is_empty() || *domain_start >= *domain_end {
                return None;
            }
            let val = match basis {
                PolynomialBasis::Monomial => {
                    let mut acc = 0.0f64;
                    for &c in coefficients.iter().rev() {
                        acc = acc * x + c;
                    }
                    acc
                }
                PolynomialBasis::Chebyshev => {
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
            if val.is_finite() { Some(val) } else { None }
        }
    }
}

/// Numerical 2nd derivative at `x` via central differences.
fn eval_trace_curvature(trace: &EchelleTraceModel, x: f64) -> f64 {
    let h = 1.0; // 1-pixel step
    let y_minus = eval_trace_at(trace, x - h).unwrap_or(0.0);
    let y_center = eval_trace_at(trace, x).unwrap_or(0.0);
    let y_plus = eval_trace_at(trace, x + h).unwrap_or(0.0);
    (y_plus - 2.0 * y_center + y_minus) / (h * h)
}

/// Mean pixel intensity along a trace (sampled every 10 columns).
fn compute_trace_mean_intensity(
    frame: &[f32],
    w: usize,
    h: usize,
    trace: &EchelleTraceModel,
    width: u32,
    aperture_half_width: f64,
) -> f64 {
    let step = 10;
    let mut sum = 0.0f64;
    let mut count = 0u32;
    let radius = aperture_half_width.floor() as i32;

    for col in (0..width).step_by(step) {
        if let Some(center) = eval_trace_at(trace, f64::from(col)) {
            let center_px = center.round() as i32;
            for offset in -radius..=radius {
                let row = center_px + offset;
                if row >= 0 && (row as usize) < h && (col as usize) < w {
                    sum += f64::from(frame[row as usize * w + col as usize]);
                    count += 1;
                }
            }
        }
    }

    if count > 0 {
        sum / f64::from(count)
    } else {
        0.0
    }
}

/// Median intensity of a random sample of pixels (proxy for inter-order background).
fn estimate_inter_order_background(frame: &[f32], _w: usize, _h: usize) -> f64 {
    // Sample every 37th pixel (prime stride for decorrelation).
    let mut samples: Vec<f32> = frame
        .iter()
        .copied()
        .step_by(37)
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Use the 25th percentile as a robust background estimate
    // (below most trace signals but above zero-background).
    let idx = samples.len() / 4;
    f64::from(samples[idx.min(samples.len() - 1)])
}

/// Reject traces whose inter-order spacing deviates from a linear model.
fn apply_spacing_filter(measurements: &[TraceMeasurement], keep: &mut [bool], sigma_thresh: f64) {
    // Collect spacings between consecutive surviving traces.
    let alive: Vec<usize> = keep
        .iter()
        .enumerate()
        .filter(|&(_, &k)| k)
        .map(|(j, _)| j)
        .collect();
    if alive.len() < 3 {
        return;
    }

    let spacings: Vec<f64> = alive
        .windows(2)
        .map(|pair| measurements[pair[1]].center_y - measurements[pair[0]].center_y)
        .collect();

    let median_spacing = median_f64(&spacings);
    let mad = mad_f64(&spacings, median_spacing);
    // MAD → σ. When MAD = 0 (≥50% of spacings identical), fall back to
    // mean absolute deviation with a 1-pixel floor.
    let sigma = if mad > 1e-10 {
        mad * 1.4826
    } else {
        let n = spacings.len() as f64;
        let aad: f64 = spacings
            .iter()
            .map(|&s| (s - median_spacing).abs())
            .sum::<f64>()
            / n;
        aad.max(1.0)
    };

    if sigma < 1e-6 {
        return;
    }

    // Check each spacing; if an outlier, remove the trace with lower intensity.
    for pair in alive.windows(2) {
        let spacing = measurements[pair[1]].center_y - measurements[pair[0]].center_y;
        let deviation = (spacing - median_spacing).abs() / sigma;
        if deviation > sigma_thresh {
            // Remove the weaker trace.
            if measurements[pair[0]].mean_intensity < measurements[pair[1]].mean_intensity {
                keep[pair[0]] = false;
            } else {
                keep[pair[1]] = false;
            }
        }
    }
}

/// Reject traces whose curvature deviates from the population median.
fn apply_curvature_filter(measurements: &[TraceMeasurement], keep: &mut [bool], sigma_thresh: f64) {
    let curvatures: Vec<f64> = measurements
        .iter()
        .enumerate()
        .filter(|(j, _)| keep[*j])
        .map(|(_, m)| m.curvature)
        .collect();
    if curvatures.len() < 3 {
        return;
    }

    let median_curv = median_f64(&curvatures);
    let mad = mad_f64(&curvatures, median_curv);
    let sigma = mad * 1.4826;

    if sigma < 1e-10 {
        return;
    }

    for (j, m) in measurements.iter().enumerate() {
        if keep[j] {
            let deviation = (m.curvature - median_curv).abs() / sigma;
            if deviation > sigma_thresh {
                keep[j] = false;
            }
        }
    }
}

/// bd-lpgyn: reject traces whose measured FWHM exceeds the median by
/// more than `max_excess` × median. True slit images have a narrow FWHM
/// distribution (set by the entrance slit + detector pixel pitch);
/// defocused ghost orders and MCP halos sit in the right tail.
fn apply_fwhm_filter(measurements: &[TraceMeasurement], keep: &mut [bool], max_excess: f64) {
    let fwhms: Vec<f64> = measurements
        .iter()
        .enumerate()
        .filter(|(j, _)| keep[*j])
        .map(|(_, m)| m.fwhm)
        .filter(|f| f.is_finite() && *f > 0.0)
        .collect();
    if fwhms.is_empty() {
        return;
    }
    let median_fwhm = median_f64(&fwhms);
    if median_fwhm <= 0.0 {
        return;
    }
    let ceiling = median_fwhm * (1.0 + max_excess);
    for (j, m) in measurements.iter().enumerate() {
        if keep[j] && m.fwhm > ceiling {
            keep[j] = false;
        }
    }
}

/// bd-lpgyn: reject traces whose fit domain covers less than
/// `min_fraction` of the detector's dispersion-axis width. Internal
/// reflections and localised MCP halos rarely produce trajectories
/// that span ≥60% of the detector; true orders always do.
fn apply_continuity_filter(
    measurements: &[TraceMeasurement],
    keep: &mut [bool],
    detector_width: f64,
    min_fraction: f64,
) {
    if detector_width <= 0.0 {
        return;
    }
    let threshold = detector_width * min_fraction;
    for (j, m) in measurements.iter().enumerate() {
        if keep[j] && m.domain_span < threshold {
            keep[j] = false;
        }
    }
}

/// bd-lpgyn: reject traces that violate the prism's monotonic Δy rule.
///
/// For a prism cross-disperser Δy between consecutive orders grows
/// monotonically with m (toward blue). With traces sorted by `center_y`,
/// any Δy that is smaller than its predecessor by more than `tol_frac` ×
/// median Δy signals that one of the two traces is a ghost order. We
/// drop the trace whose removal *restores* monotonicity.
fn apply_monotonic_dy_filter(
    measurements: &[TraceMeasurement],
    keep: &mut [bool],
    tol_frac: f64,
) {
    // Indices of currently-surviving traces, sorted by center_y.
    let mut alive: Vec<usize> = keep
        .iter()
        .enumerate()
        .filter(|&(_, &k)| k)
        .map(|(j, _)| j)
        .collect();
    alive.sort_by(|&a, &b| {
        measurements[a]
            .center_y
            .partial_cmp(&measurements[b].center_y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if alive.len() < 3 {
        return;
    }

    // Collect consecutive Δy values to get a robust median scale.
    let dys: Vec<f64> = alive
        .windows(2)
        .map(|w| (measurements[w[1]].center_y - measurements[w[0]].center_y).abs())
        .collect();
    let median_dy = median_f64(&dys).max(1.0);
    let tol = tol_frac * median_dy;

    // Walk triples (prev, cur, next) and inspect monotonicity of Δy. If
    // Δy(cur→next) is smaller than Δy(prev→cur) by more than `tol`, one
    // of `cur` or `next` is the ghost. Remove whichever has lower
    // intensity. Re-tighten after each removal by walking again.
    //
    // Bounded-iteration safety: at most `alive.len()` rejections.
    for _ in 0..alive.len() {
        let mut violated = None;
        for w in alive.windows(3) {
            let (i0, i1, i2) = (w[0], w[1], w[2]);
            let dy01 = measurements[i1].center_y - measurements[i0].center_y;
            let dy12 = measurements[i2].center_y - measurements[i1].center_y;
            // Monotonic rule: dy12 should NOT be significantly < dy01
            // (since Δy increases with y on a prism cross-disperser).
            // A strong drop signals a ghost inserted in the middle.
            if dy12 + tol < dy01 {
                violated = Some((i0, i1, i2));
                break;
            }
        }
        let Some((_i0, i1, i2)) = violated else {
            break;
        };
        // Remove the lower-intensity trace of the two candidates.
        let drop = if measurements[i1].mean_intensity < measurements[i2].mean_intensity {
            i1
        } else {
            i2
        };
        keep[drop] = false;
        alive.retain(|&j| j != drop);
        if alive.len() < 3 {
            break;
        }
    }
}

fn domain_span_of(trace: &EchelleTraceModel) -> f64 {
    match trace {
        EchelleTraceModel::Polynomial {
            domain_start,
            domain_end,
            ..
        } => (domain_end - domain_start).max(0.0),
    }
}

fn median_f64(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut sorted = data.to_vec();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted[sorted.len() / 2]
}

fn mad_f64(data: &[f64], median: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let deviations: Vec<f64> = data.iter().map(|&x| (x - median).abs()).collect();
    median_f64(&deviations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PolynomialBasis;

    fn flat_trace(center: f64) -> OrderTrace {
        OrderTrace {
            trace: EchelleTraceModel::Polynomial {
                basis: PolynomialBasis::Monomial,
                coefficients: vec![center],
                domain_start: 0.0,
                domain_end: 1000.0,
            },
            aperture_half_width: 4.0,
            fit_rms: 0.5,
            n_samples: 100,
            order_number: None,
        }
    }

    #[test]
    fn disabled_config_returns_all_traces() {
        let traces = vec![flat_trace(50.0), flat_trace(100.0), flat_trace(150.0)];
        let config = TraceValidationConfig::default();
        assert!(config.is_empty());

        let frame = vec![100.0f32; 200 * 200];
        let result = validate_traces(&traces, &config, &frame, 200, 200, 4.0);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn max_order_count_keeps_strongest() {
        // Three traces at different Y positions; the middle one gets the
        // brightest signal because we construct the frame that way.
        let traces = vec![flat_trace(20.0), flat_trace(100.0), flat_trace(180.0)];
        let mut frame = vec![10.0f32; 200 * 200];
        // Make the middle trace (y=100) very bright.
        for col in 0..200usize {
            for offset in -4i32..=4 {
                let row = (100 + offset) as usize;
                frame[row * 200 + col] = 5000.0;
            }
        }

        let config = TraceValidationConfig {
            max_order_count: Some(1),
            ..Default::default()
        };

        let result = validate_traces(&traces, &config, &frame, 200, 200, 4.0);
        assert_eq!(result.len(), 1);
        // The surviving trace should be the bright one at y=100.
        let EchelleTraceModel::Polynomial { coefficients, .. } = &result[0].trace;
        assert!((coefficients[0] - 100.0).abs() < 0.1);
    }

    #[test]
    fn snr_filter_rejects_faint_traces() {
        let traces = vec![flat_trace(50.0), flat_trace(150.0)];
        let mut frame = vec![100.0f32; 200 * 200]; // background = 100
        // Make the first trace bright, leave the second at background level.
        for col in 0..200usize {
            for offset in -4i32..=4 {
                let row = (50 + offset) as usize;
                frame[row * 200 + col] = 2000.0;
            }
        }

        let config = TraceValidationConfig {
            min_snr: Some(3.0),
            ..Default::default()
        };

        let result = validate_traces(&traces, &config, &frame, 200, 200, 4.0);
        // Only the bright trace should survive.
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn spacing_filter_rejects_outlier() {
        // Regular traces at y=20, 50, 80, 110, 140 (spacing=30)
        // plus a spurious trace at y=60 (breaks the pattern).
        let traces = vec![
            flat_trace(20.0),
            flat_trace(50.0),
            flat_trace(60.0), // spurious
            flat_trace(80.0),
            flat_trace(110.0),
            flat_trace(140.0),
        ];
        let mut frame = vec![100.0f32; 200 * 200];
        // All traces have the same brightness.
        for trace in &traces {
            let EchelleTraceModel::Polynomial { coefficients, .. } = &trace.trace;
            let y = coefficients[0] as usize;
            for col in 0..200usize {
                for offset in 0..=8 {
                    let row = y.saturating_sub(4) + offset;
                    if row < 200 {
                        frame[row * 200 + col] = 1000.0;
                    }
                }
            }
        }

        let config = TraceValidationConfig {
            spacing_sigma: Some(2.0),
            ..Default::default()
        };

        let result = validate_traces(&traces, &config, &frame, 200, 200, 4.0);
        // The spurious trace at y=60 should be removed (it creates spacing
        // anomalies: 30, 10, 20 instead of the regular 30).
        assert!(
            result.len() < traces.len(),
            "expected fewer traces after spacing filter, got {} from {}",
            result.len(),
            traces.len()
        );
    }
}
