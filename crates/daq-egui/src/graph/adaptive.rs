//! Adaptive scan trigger evaluation.
//!
//! Provides runtime evaluation of trigger conditions for adaptive scans.
//! Triggers can detect threshold crossings or peaks in signal data.

use crate::graph::nodes::{ThresholdOp, TriggerCondition, TriggerLogic};
use find_peaks::PeakFinder;

/// Detected peak information.
#[derive(Debug, Clone)]
pub struct DetectedPeak {
    /// Index in the signal array
    pub index: usize,
    /// Signal value at peak
    pub height: f64,
    /// Position (if axis values provided)
    pub position: Option<f64>,
}

/// Detect peaks in a signal using prominence-based filtering.
///
/// # Arguments
/// * `signal` - Signal values to analyze
/// * `min_prominence` - Minimum prominence for peak detection
/// * `min_height` - Optional minimum height threshold
///
/// # Returns
/// Vector of detected peaks sorted by height (highest first)
pub fn detect_peaks(signal: &[f64], min_prominence: f64, min_height: Option<f64>) -> Vec<DetectedPeak> {
    if signal.is_empty() {
        return Vec::new();
    }

    let mut fp = PeakFinder::new(signal);
    fp.with_min_prominence(min_prominence);

    if let Some(height) = min_height {
        fp.with_min_height(height);
    }

    let mut peaks: Vec<DetectedPeak> = fp
        .find_peaks()
        .iter()
        .map(|p| DetectedPeak {
            index: p.middle_position(),
            height: p.height.unwrap_or(0.0),
            position: None,
        })
        .collect();

    // Sort by height descending
    peaks.sort_by(|a, b| b.height.partial_cmp(&a.height).unwrap_or(std::cmp::Ordering::Equal));

    peaks
}

/// Evaluate a threshold condition against current value.
pub fn evaluate_threshold(value: f64, operator: &ThresholdOp, threshold: f64) -> bool {
    match operator {
        ThresholdOp::LessThan => value < threshold,
        ThresholdOp::GreaterThan => value > threshold,
        ThresholdOp::EqualWithin { tolerance } => (value - threshold).abs() <= *tolerance,
    }
}

/// Result of trigger evaluation.
#[derive(Debug, Clone)]
pub struct TriggerResult {
    /// Whether the trigger fired
    pub fired: bool,
    /// Detected peak (if peak detection trigger)
    pub peak: Option<DetectedPeak>,
    /// Which trigger(s) fired (by index)
    pub fired_triggers: Vec<usize>,
}

/// Evaluate trigger conditions against scan data.
///
/// # Arguments
/// * `triggers` - Trigger conditions to evaluate
/// * `logic` - AND/OR logic for combining triggers
/// * `signal` - Current signal data from scan
/// * `positions` - Optional position values corresponding to signal
///
/// # Returns
/// TriggerResult indicating whether triggers fired and any detected peaks
pub fn evaluate_triggers(
    triggers: &[TriggerCondition],
    logic: &TriggerLogic,
    signal: &[f64],
    positions: Option<&[f64]>,
) -> TriggerResult {
    if triggers.is_empty() {
        return TriggerResult {
            fired: false,
            peak: None,
            fired_triggers: Vec::new(),
        };
    }

    let mut fired_triggers = Vec::new();
    let mut detected_peak = None;

    for (idx, trigger) in triggers.iter().enumerate() {
        let fired = match trigger {
            TriggerCondition::Threshold { operator, value, .. } => {
                // Check if any signal value crosses threshold
                signal.iter().any(|&v| evaluate_threshold(v, operator, *value))
            }
            TriggerCondition::PeakDetection {
                min_prominence,
                min_height,
                ..
            } => {
                let peaks = detect_peaks(signal, *min_prominence, *min_height);
                if let Some(peak) = peaks.first() {
                    // Add position if available
                    let mut peak_with_pos = peak.clone();
                    if let Some(positions) = positions {
                        if peak.index < positions.len() {
                            peak_with_pos.position = Some(positions[peak.index]);
                        }
                    }
                    detected_peak = Some(peak_with_pos);
                    true
                } else {
                    false
                }
            }
        };

        if fired {
            fired_triggers.push(idx);
        }
    }

    let overall_fired = match logic {
        TriggerLogic::Any => !fired_triggers.is_empty(),
        TriggerLogic::All => fired_triggers.len() == triggers.len(),
    };

    TriggerResult {
        fired: overall_fired,
        peak: detected_peak,
        fired_triggers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_peaks_simple() {
        // Simple signal with one clear peak
        let signal = vec![0.0, 1.0, 2.0, 5.0, 3.0, 1.0, 0.0];
        let peaks = detect_peaks(&signal, 1.0, None);

        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].index, 3); // Peak at index 3 (value 5.0)
        assert!((peaks[0].height - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_detect_peaks_multiple() {
        // Signal with two peaks
        let signal = vec![0.0, 3.0, 0.0, 5.0, 0.0, 2.0, 0.0];
        let peaks = detect_peaks(&signal, 1.0, None);

        // Should find at least 2 peaks
        assert!(peaks.len() >= 2);

        // First peak should be the highest (5.0)
        assert_eq!(peaks[0].index, 3);
    }

    #[test]
    fn test_detect_peaks_with_height_filter() {
        let signal = vec![0.0, 1.0, 0.0, 5.0, 0.0];
        let peaks = detect_peaks(&signal, 0.5, Some(3.0));

        // Only the peak at 5.0 should pass height filter
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].index, 3);
    }

    #[test]
    fn test_detect_peaks_empty() {
        let signal: Vec<f64> = vec![];
        let peaks = detect_peaks(&signal, 1.0, None);
        assert!(peaks.is_empty());
    }

    #[test]
    fn test_evaluate_threshold_less_than() {
        assert!(evaluate_threshold(5.0, &ThresholdOp::LessThan, 10.0));
        assert!(!evaluate_threshold(15.0, &ThresholdOp::LessThan, 10.0));
        assert!(!evaluate_threshold(10.0, &ThresholdOp::LessThan, 10.0)); // Not strictly less
    }

    #[test]
    fn test_evaluate_threshold_greater_than() {
        assert!(evaluate_threshold(15.0, &ThresholdOp::GreaterThan, 10.0));
        assert!(!evaluate_threshold(5.0, &ThresholdOp::GreaterThan, 10.0));
        assert!(!evaluate_threshold(10.0, &ThresholdOp::GreaterThan, 10.0)); // Not strictly greater
    }

    #[test]
    fn test_evaluate_threshold_equal_within() {
        let op = ThresholdOp::EqualWithin { tolerance: 0.1 };
        assert!(evaluate_threshold(10.05, &op, 10.0));
        assert!(evaluate_threshold(9.95, &op, 10.0));
        assert!(!evaluate_threshold(10.2, &op, 10.0));
    }

    #[test]
    fn test_evaluate_triggers_any_logic() {
        let triggers = vec![
            TriggerCondition::Threshold {
                device_id: "sensor1".to_string(),
                operator: ThresholdOp::GreaterThan,
                value: 10.0,
            },
            TriggerCondition::Threshold {
                device_id: "sensor2".to_string(),
                operator: ThresholdOp::LessThan,
                value: 5.0,
            },
        ];

        // Signal that triggers first condition
        let signal = vec![8.0, 12.0, 9.0]; // 12 > 10
        let result = evaluate_triggers(&triggers, &TriggerLogic::Any, &signal, None);

        assert!(result.fired);
        assert!(result.fired_triggers.contains(&0));
    }

    #[test]
    fn test_evaluate_triggers_all_logic() {
        let triggers = vec![
            TriggerCondition::Threshold {
                device_id: "sensor1".to_string(),
                operator: ThresholdOp::GreaterThan,
                value: 10.0,
            },
            TriggerCondition::Threshold {
                device_id: "sensor2".to_string(),
                operator: ThresholdOp::LessThan,
                value: 20.0,
            },
        ];

        // Signal that triggers both conditions
        let signal = vec![15.0]; // 15 > 10 AND 15 < 20
        let result = evaluate_triggers(&triggers, &TriggerLogic::All, &signal, None);

        assert!(result.fired);
        assert_eq!(result.fired_triggers.len(), 2);
    }

    #[test]
    fn test_evaluate_triggers_all_logic_partial() {
        let triggers = vec![
            TriggerCondition::Threshold {
                device_id: "sensor1".to_string(),
                operator: ThresholdOp::GreaterThan,
                value: 10.0,
            },
            TriggerCondition::Threshold {
                device_id: "sensor2".to_string(),
                operator: ThresholdOp::LessThan,
                value: 5.0,
            },
        ];

        // Signal that only triggers first condition
        let signal = vec![12.0]; // 12 > 10 but 12 is NOT < 5
        let result = evaluate_triggers(&triggers, &TriggerLogic::All, &signal, None);

        // With ALL logic, should NOT fire if only one matches
        assert!(!result.fired);
        assert_eq!(result.fired_triggers.len(), 1);
    }

    #[test]
    fn test_evaluate_triggers_peak_detection() {
        let triggers = vec![TriggerCondition::PeakDetection {
            device_id: "detector".to_string(),
            min_prominence: 1.0,
            min_height: None,
        }];

        let signal = vec![0.0, 1.0, 5.0, 2.0, 0.0];
        let positions = vec![0.0, 25.0, 50.0, 75.0, 100.0];

        let result = evaluate_triggers(&triggers, &TriggerLogic::Any, &signal, Some(&positions));

        assert!(result.fired);
        assert!(result.peak.is_some());

        let peak = result.peak.unwrap();
        assert_eq!(peak.index, 2);
        assert_eq!(peak.position, Some(50.0));
    }

    #[test]
    fn test_evaluate_triggers_empty() {
        let triggers: Vec<TriggerCondition> = vec![];
        let signal = vec![1.0, 2.0, 3.0];

        let result = evaluate_triggers(&triggers, &TriggerLogic::Any, &signal, None);

        assert!(!result.fired);
        assert!(result.fired_triggers.is_empty());
    }
}
