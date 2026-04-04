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
