//! Feedback channel for data-plane events during plan execution (bd-7rg0)
//!
//! The `FeedbackEvent` enum represents events received from the data-plane
//! during plan execution. The `RunEngine` exposes a channel that consumers
//! can subscribe to for real-time feedback on device readings, threshold
//! crossings, and stability detection.

use common::device_id::DeviceId;

/// Events received from the data-plane during plan execution.
#[derive(Debug, Clone)]
pub enum FeedbackEvent {
    /// A device reading crossed a threshold.
    ThresholdCrossed {
        /// Device that produced the reading.
        device_id: DeviceId,
        /// Field name (e.g., "intensity", "value").
        field: String,
        /// The reading value that crossed the threshold.
        value: f64,
        /// The threshold that was crossed.
        threshold: f64,
    },
    /// A device reading became stable within tolerance.
    StabilityReached {
        /// Device that stabilized.
        device_id: DeviceId,
        /// Field name (e.g., "value").
        field: String,
        /// Measured relative variance when stability was declared.
        variance: f64,
    },
    /// A raw value update from a device.
    ValueUpdate {
        /// Device that produced the reading.
        device_id: DeviceId,
        /// Field name (e.g., "value").
        field: String,
        /// The current reading value.
        value: f64,
    },
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_event_threshold_crossed_clone() {
        let event = FeedbackEvent::ThresholdCrossed {
            device_id: DeviceId::from("pm_1"),
            field: "intensity".to_string(),
            value: 0.5,
            threshold: 0.3,
        };
        let cloned = event.clone();
        assert!(
            matches!(
                &cloned,
                FeedbackEvent::ThresholdCrossed { value, threshold, .. }
                if (*value - 0.5).abs() < f64::EPSILON && (*threshold - 0.3).abs() < f64::EPSILON
            )
        );
    }

    #[test]
    fn test_feedback_event_stability_reached_clone() {
        let event = FeedbackEvent::StabilityReached {
            device_id: DeviceId::from("stage_1"),
            field: "position".to_string(),
            variance: 0.001,
        };
        let cloned = event.clone();
        assert!(
            matches!(
                &cloned,
                FeedbackEvent::StabilityReached { variance, .. }
                if (*variance - 0.001).abs() < f64::EPSILON
            )
        );
    }

    #[test]
    fn test_feedback_event_value_update_clone() {
        let event = FeedbackEvent::ValueUpdate {
            device_id: DeviceId::from("detector"),
            field: "count".to_string(),
            value: 42.0,
        };
        let cloned = event.clone();
        assert!(
            matches!(
                &cloned,
                FeedbackEvent::ValueUpdate { value, .. }
                if (*value - 42.0).abs() < f64::EPSILON
            )
        );
    }

    #[test]
    fn test_feedback_event_debug() {
        let event = FeedbackEvent::ValueUpdate {
            device_id: DeviceId::from("pm"),
            field: "power".to_string(),
            value: 1.23,
        };
        let dbg = format!("{event:?}");
        assert!(dbg.contains("ValueUpdate"));
        assert!(dbg.contains("power"));
    }
}
