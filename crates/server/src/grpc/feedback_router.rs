//! Tap-to-feedback routing for data-plane threshold evaluation (bd-7md9).
//!
//! `FeedbackRouter` bridges ring-buffer tap consumers and the `RunEngine`
//! feedback channel.  It holds a clone of the engine's feedback sender and
//! provides lightweight methods that evaluate threshold / stability
//! conditions and, when satisfied, push a [`FeedbackEvent`] into the
//! channel.
//!
//! The router is designed to be wired into tap consumer loops in the server
//! streaming layer.  Because the feedback channel is bounded (capacity 256),
//! all sends use `try_send` so a full channel never blocks the data path.

use experiment::FeedbackEvent;
use tokio::sync::mpsc;
use tracing;

/// Routes data-plane observations into the `RunEngine` feedback channel.
///
/// Obtain an instance via [`FeedbackRouter::new`] by passing a sender
/// cloned from [`RunEngine::feedback_sender`].
///
/// # Thread Safety
///
/// `FeedbackRouter` is `Send + Sync` and cheaply cloneable (the inner
/// sender is an `mpsc::Sender` which is `Clone`).
#[derive(Debug, Clone)]
pub struct FeedbackRouter {
    feedback_tx: mpsc::Sender<FeedbackEvent>,
}

impl FeedbackRouter {
    /// Create a new router from a feedback channel sender.
    ///
    /// Typically called once at server startup:
    /// ```rust,ignore
    /// let router = FeedbackRouter::new(run_engine.feedback_sender());
    /// ```
    pub fn new(feedback_tx: mpsc::Sender<FeedbackEvent>) -> Self {
        Self { feedback_tx }
    }

    /// Evaluate a threshold condition and, if crossed, send a
    /// [`FeedbackEvent::ThresholdCrossed`].
    ///
    /// Returns `true` if the threshold was crossed (regardless of whether
    /// the event was successfully enqueued).
    pub fn check_threshold(
        &self,
        device_id: &str,
        field: &str,
        value: f64,
        threshold: f64,
    ) -> bool {
        if value >= threshold {
            let event = FeedbackEvent::ThresholdCrossed {
                device_id: device_id.into(),
                field: field.to_string(),
                value,
                threshold,
            };
            if let Err(mpsc::error::TrySendError::Full(_)) = self.feedback_tx.try_send(event) {
                tracing::warn!(
                    device_id,
                    field,
                    "Feedback channel full — dropped ThresholdCrossed event"
                );
            }
            return true;
        }
        false
    }

    /// Report a stability observation.
    ///
    /// Sends a [`FeedbackEvent::StabilityReached`] when the measured
    /// variance is at or below the given tolerance.  Returns `true` if
    /// stability was reached.
    pub fn check_stability(
        &self,
        device_id: &str,
        field: &str,
        variance: f64,
        tolerance: f64,
    ) -> bool {
        if variance <= tolerance {
            let event = FeedbackEvent::StabilityReached {
                device_id: device_id.into(),
                field: field.to_string(),
                variance,
            };
            if let Err(mpsc::error::TrySendError::Full(_)) = self.feedback_tx.try_send(event) {
                tracing::warn!(
                    device_id,
                    field,
                    "Feedback channel full — dropped StabilityReached event"
                );
            }
            return true;
        }
        false
    }

    /// Forward a raw value update into the feedback channel.
    ///
    /// Useful for tap consumers that want every reading to be visible to
    /// the `RunEngine` consumer without applying any condition.
    pub fn send_value_update(&self, device_id: &str, field: &str, value: f64) {
        let event = FeedbackEvent::ValueUpdate {
            device_id: device_id.to_string(),
            field: field.to_string(),
            value,
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = self.feedback_tx.try_send(event) {
            tracing::debug!(
                device_id,
                field,
                "Feedback channel full — dropped ValueUpdate event"
            );
        }
    }

    /// Returns `true` if the feedback channel is still open (the receiver
    /// has not been dropped).
    pub fn is_connected(&self) -> bool {
        !self.feedback_tx.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn threshold_crossed_sends_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let router = FeedbackRouter::new(tx);

        // Value below threshold -- no event
        let crossed = router.check_threshold("detector_1", "intensity", 0.5, 1.0);
        assert!(!crossed);
        // Channel should be empty
        assert!(rx.try_recv().is_err());

        // Value at threshold -- event
        let crossed = router.check_threshold("detector_1", "intensity", 1.0, 1.0);
        assert!(crossed);
        let event = rx.try_recv().expect("expected a FeedbackEvent");
        match event {
            FeedbackEvent::ThresholdCrossed {
                device_id,
                field,
                value,
                threshold,
            } => {
                assert_eq!(device_id, "detector_1");
                assert_eq!(field, "intensity");
                assert!((value - 1.0).abs() < f64::EPSILON);
                assert!((threshold - 1.0).abs() < f64::EPSILON);
            }
            other => panic!("unexpected event variant: {other:?}"),
        }

        // Value above threshold -- event
        let crossed = router.check_threshold("detector_1", "intensity", 2.5, 1.0);
        assert!(crossed);
        let event = rx.try_recv().expect("expected a FeedbackEvent");
        match event {
            FeedbackEvent::ThresholdCrossed { value, .. } => {
                assert!((value - 2.5).abs() < f64::EPSILON);
            }
            other => panic!("unexpected event variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn stability_reached_sends_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let router = FeedbackRouter::new(tx);

        // Variance above tolerance -- no event
        let stable = router.check_stability("stage_x", "position", 0.05, 0.01);
        assert!(!stable);
        assert!(rx.try_recv().is_err());

        // Variance at tolerance -- event
        let stable = router.check_stability("stage_x", "position", 0.01, 0.01);
        assert!(stable);
        let event = rx.try_recv().expect("expected a FeedbackEvent");
        match event {
            FeedbackEvent::StabilityReached {
                device_id,
                field,
                variance,
            } => {
                assert_eq!(device_id, "stage_x");
                assert_eq!(field, "position");
                assert!((variance - 0.01).abs() < f64::EPSILON);
            }
            other => panic!("unexpected event variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn value_update_sends_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let router = FeedbackRouter::new(tx);

        router.send_value_update("power_meter", "watts", 42.0);
        let event = rx.try_recv().expect("expected a FeedbackEvent");
        match event {
            FeedbackEvent::ValueUpdate {
                device_id,
                field,
                value,
            } => {
                assert_eq!(device_id, "power_meter");
                assert_eq!(field, "watts");
                assert!((value - 42.0).abs() < f64::EPSILON);
            }
            other => panic!("unexpected event variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn full_channel_does_not_block() {
        // Channel capacity of 1 -- second send should be dropped, not block
        let (tx, _rx) = mpsc::channel(1);
        let router = FeedbackRouter::new(tx);

        // Fill the channel
        let crossed = router.check_threshold("det", "val", 10.0, 1.0);
        assert!(crossed);

        // This should still return true (threshold was crossed) even though
        // the event is dropped due to backpressure
        let crossed = router.check_threshold("det", "val", 20.0, 1.0);
        assert!(crossed);
    }

    #[tokio::test]
    async fn is_connected_reflects_channel_state() {
        let (tx, rx) = mpsc::channel::<FeedbackEvent>(16);
        let router = FeedbackRouter::new(tx);

        assert!(router.is_connected());
        drop(rx);
        assert!(!router.is_connected());
    }

    #[tokio::test]
    async fn clone_shares_same_channel() {
        let (tx, mut rx) = mpsc::channel(16);
        let router = FeedbackRouter::new(tx);
        let router2 = router.clone();

        router.check_threshold("a", "v", 5.0, 1.0);
        router2.check_threshold("b", "v", 5.0, 1.0);

        let ev1 = rx.try_recv().expect("event from router 1");
        let ev2 = rx.try_recv().expect("event from router 2");
        match (&ev1, &ev2) {
            (
                FeedbackEvent::ThresholdCrossed { device_id: id1, .. },
                FeedbackEvent::ThresholdCrossed { device_id: id2, .. },
            ) => {
                assert_eq!(id1, "a");
                assert_eq!(id2, "b");
            }
            _ => panic!("unexpected event variants"),
        }
    }
}
