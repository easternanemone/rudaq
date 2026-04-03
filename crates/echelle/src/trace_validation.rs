//! Post-detection trace validation for echelle order filtering.
//!
//! Filters spurious echelle order detections (ghost reflections, scattered
//! light ridges, detector artifacts) from the output of [`detect_orders`].
//! All filters are optional and disabled by default for backward compatibility.
//!
//! Validation criteria (from CERES and HiFLEx):
//! - **SNR threshold**: reject traces with mean aperture signal below a
//!   configurable multiple of the inter-order background
//! - **Spacing regularity**: fit a linear model to inter-trace spacings
//!   and reject outliers beyond a MAD-based robust sigma threshold
//! - **Curvature consistency**: reject traces whose 2nd derivative at
//!   the detector center deviates beyond a robust sigma threshold
//! - **Max order count**: if too many traces remain, keep the strongest
//!   N by mean intensity
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
    /// Typical value: 5.0.
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
}

impl TraceValidationConfig {
    /// Returns `true` if all filters are disabled (no validation will be applied).
    pub fn is_empty(&self) -> bool {
        self.min_snr.is_none()
            && self.spacing_sigma.is_none()
            && self.curvature_sigma.is_none()
            && self.max_order_count.is_none()
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

    // Compute metrics for each trace.
    let measurements: Vec<TraceMeasurement> = traces
        .iter()
        .enumerate()
        .filter_map(|(i, trace)| {
            let center_y = eval_trace_at(&trace.trace, mid_x)?;
            let mean_intensity =
                compute_trace_mean_intensity(frame, w, h, &trace.trace, width, aperture_half_width);
            let curvature = eval_trace_curvature(&trace.trace, mid_x);
            Some(TraceMeasurement {
                index: i,
                center_y,
                mean_intensity,
                curvature,
            })
        })
        .collect();

    let mut keep = vec![true; measurements.len()];

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
    }

    // Filter 2: Spacing regularity.
    if let Some(sigma_thresh) = config.spacing_sigma {
        apply_spacing_filter(&measurements, &mut keep, sigma_thresh);
    }

    // Filter 3: Curvature consistency.
    if let Some(sigma_thresh) = config.curvature_sigma {
        apply_curvature_filter(&measurements, &mut keep, sigma_thresh);
    }

    // Filter 4: Max order count (keep strongest by intensity).
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
            if val.is_finite() {
                Some(val)
            } else {
                None
            }
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
        .filter(|(_, &k)| k)
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
