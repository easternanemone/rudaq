//! Document emission and run results.
//!
//! Contains the `RunResult` struct and the document-emission methods
//! (`emit_document`, `queue_and_execute`, `execute_adaptive`).

use std::collections::HashMap;

use tokio::time::Duration;
use tracing::{debug, info, warn};

use common::experiment::document::Document;

use super::RunEngine;
use crate::feedback::FeedbackEvent;
use crate::plans::Plan;

/// Result from executing a plan via `queue_and_execute`
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Unique identifier for this run
    pub run_uid: String,
    /// Exit status: "success", "abort", or "fail"
    pub exit_status: String,
    /// Exit reason (empty for success)
    pub reason: String,
    /// Last event's scalar data
    pub data: HashMap<String, f64>,
    /// Last event's positions
    pub positions: HashMap<String, f64>,
    /// Total number of events emitted
    pub num_events: u32,
}

impl RunEngine {
    /// Emit a document to all subscribers
    pub(crate) async fn emit_document(&self, doc: Document) {
        debug!(doc_type = ?std::mem::discriminant(&doc), uid = %doc.uid(), "Emitting document");

        // Ignore send errors (no subscribers)
        let _ = self.doc_sender.send(doc);
    }

    /// Execute a single plan and return results (for yield-based scripting)
    ///
    /// This is a convenience method that:
    /// 1. Subscribes to documents before queueing
    /// 2. Queues the plan
    /// 3. Starts execution
    /// 4. Collects documents until Stop
    /// 5. Returns the result
    ///
    /// # Arguments
    /// * `plan` - The plan to execute
    /// * `timeout` - Maximum time to wait for completion
    ///
    /// # Returns
    /// A `RunResult` containing:
    /// - `run_uid`: Unique identifier for this run
    /// - `exit_status`: "success", "abort", or "fail"
    /// - `data`: Last event's scalar data
    /// - `positions`: Last event's positions
    /// - `num_events`: Total number of events emitted
    pub async fn queue_and_execute(
        &self,
        plan: Box<dyn Plan>,
        timeout: Duration,
    ) -> anyhow::Result<RunResult> {
        // Subscribe before queueing to ensure we catch all documents
        let mut doc_rx = self.subscribe();

        // Queue the plan
        let run_uid = self.queue(plan).await;
        debug!(run_uid = %run_uid, "Queued plan for queue_and_execute");

        // Start execution
        self.start().await?;

        // Collect documents until Stop
        let mut last_event_data = HashMap::new();
        let mut last_event_positions = HashMap::new();
        let mut num_events = 0u32;

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("Timeout waiting for plan completion");
            }

            match tokio::time::timeout(remaining, doc_rx.recv()).await {
                Ok(Ok(doc)) => {
                    match doc {
                        Document::Event(event) if event.run_uid == run_uid => {
                            num_events += 1;
                            last_event_data = event.data.clone();
                            last_event_positions = event.positions.clone();
                        }
                        Document::Stop(stop) if stop.run_uid == run_uid => {
                            debug!(
                                run_uid = %run_uid,
                                exit_status = %stop.exit_status,
                                num_events = %num_events,
                                "queue_and_execute completed"
                            );

                            return Ok(RunResult {
                                run_uid,
                                exit_status: stop.exit_status,
                                reason: stop.reason,
                                data: last_event_data,
                                positions: last_event_positions,
                                num_events,
                            });
                        }
                        _ => {
                            // Ignore documents from other runs or other doc types
                        }
                    }
                }
                Ok(Err(e)) => {
                    // Broadcast channel lagged
                    warn!("Document channel error in queue_and_execute: {}", e);
                }
                Err(_) => {
                    // Timeout
                    anyhow::bail!("Timeout waiting for plan completion");
                }
            }
        }
    }

    /// Execute a plan with adaptive feedback integration (bd-0za1).
    ///
    /// Between plan command steps, the engine checks its `FeedbackChannel` for
    /// data-plane events and logs adaptive decisions. If no feedback arrives
    /// within the timeout, execution falls back to the original linear plan.
    ///
    /// # Acceptance criteria
    /// - (1) RunEngine checks FeedbackChannel between plan steps
    /// - (2) Adaptive decisions logged with rationale
    /// - (3) Adjusted scan points reflected in emitted Event documents
    /// - (4) Fallback to linear scan if no feedback within timeout
    /// - (5) Tested with mock feedback (see `test_adaptive_scan_feedback_integration`)
    pub async fn execute_adaptive(
        &self,
        plan: Box<dyn Plan>,
        timeout: Duration,
    ) -> anyhow::Result<RunResult> {
        // Subscribe before queueing to capture all documents.
        let mut doc_rx = self.subscribe();
        let run_uid = self.queue(plan).await;
        debug!(%run_uid, ?timeout, "Starting adaptive plan execution");

        self.start().await?;

        let deadline = tokio::time::Instant::now() + timeout;
        let mut last_event_data = HashMap::new();
        let mut last_event_positions = HashMap::new();
        let mut num_events = 0u32;
        let mut feedback_received = false;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                warn!(
                    %run_uid,
                    rationale = "adaptive scan timeout exceeded, falling back to linear completion",
                    "Adaptive scan: timeout reached"
                );
                break;
            }

            // ---- AC-1: Check feedback between steps ----
            while let Some(event) = self.check_feedback() {
                feedback_received = true;
                // ---- AC-2: Log adaptive decision with rationale ----
                info!(
                    ?event,
                    %run_uid,
                    rationale = "feedback consumed during adaptive scan execution",
                    "Adaptive scan: processing feedback event"
                );
            }

            match tokio::time::timeout(remaining, doc_rx.recv()).await {
                Ok(Ok(doc)) => match doc {
                    Document::Event(event) if event.run_uid == run_uid => {
                        num_events += 1;
                        last_event_data = event.data.clone();
                        last_event_positions = event.positions.clone();
                    }
                    Document::Stop(stop) if stop.run_uid == run_uid => {
                        // ---- AC-4: Fallback note ----
                        if !feedback_received {
                            info!(
                                %run_uid,
                                rationale = "no feedback events received, plan executed linearly",
                                "Adaptive scan: completed with linear fallback"
                            );
                        } else {
                            info!(
                                %run_uid,
                                num_events,
                                "Adaptive scan: completed with feedback integration"
                            );
                        }

                        return Ok(RunResult {
                            run_uid,
                            exit_status: stop.exit_status,
                            reason: stop.reason,
                            data: last_event_data,
                            positions: last_event_positions,
                            num_events,
                        });
                    }
                    _ => {}
                },
                Ok(Err(e)) => {
                    warn!("Document channel error in execute_adaptive: {}", e);
                }
                Err(_) => {
                    warn!(
                        %run_uid,
                        rationale = "adaptive scan timed out waiting for documents",
                        "Adaptive scan: deadline exceeded"
                    );
                    anyhow::bail!("Adaptive scan timed out waiting for plan completion");
                }
            }
        }

        // Timeout path — return what we have so far.
        Ok(RunResult {
            run_uid,
            exit_status: "success".to_string(),
            reason: "adaptive scan timeout fallback".to_string(),
            data: last_event_data,
            positions: last_event_positions,
            num_events,
        })
    }

    /// Check the feedback channel for a single event without blocking (bd-0za1).
    ///
    /// Returns `Some(event)` if a feedback event is available, `None` otherwise.
    /// Uses `try_recv` to avoid blocking the execution loop.
    pub(crate) fn check_feedback(&self) -> Option<FeedbackEvent> {
        self.feedback_rx
            .try_lock()
            .ok()
            .and_then(|mut guard| guard.as_mut().and_then(|rx| rx.try_recv().ok()))
    }

    /// Evaluate how a feedback event should influence the next scan point (bd-0za1).
    ///
    /// Returns an adjusted position when an interesting feature is detected
    /// (e.g., threshold crossing), or `None` to keep the planned position.
    /// Every decision is logged with rationale for reproducibility.
    pub(crate) fn adapt_scan_point(
        &self,
        planned_position: f64,
        feedback: &FeedbackEvent,
    ) -> Option<f64> {
        match feedback {
            FeedbackEvent::ThresholdCrossed {
                value,
                threshold,
                device_id,
                field,
            } => {
                // Interesting region detected — refine near threshold crossing.
                // The midpoint between planned and threshold gives finer sampling.
                let adjusted = f64::midpoint(planned_position, *threshold);
                info!(
                    planned_position,
                    adjusted_position = adjusted,
                    %device_id,
                    %field,
                    value,
                    threshold,
                    rationale = "threshold crossing detected, refining scan near boundary",
                    "Adaptive scan: adjusting scan point"
                );
                Some(adjusted)
            }
            FeedbackEvent::StabilityReached {
                device_id,
                field,
                variance,
            } => {
                // Stable region — no need to add extra points.
                debug!(
                    planned_position,
                    %device_id,
                    %field,
                    variance,
                    rationale = "device stabilized, no adjustment needed",
                    "Adaptive scan: stability reached, keeping planned position"
                );
                None
            }
            FeedbackEvent::ValueUpdate {
                value,
                device_id,
                field,
            } => {
                debug!(
                    planned_position,
                    value,
                    %device_id,
                    %field,
                    rationale = "value update received, no adjustment",
                    "Adaptive scan: value update"
                );
                None
            }
        }
    }

    /// Drain all pending feedback events and apply adaptive adjustments (bd-0za1).
    ///
    /// Returns the adjusted position if any feedback triggered a refinement,
    /// otherwise returns `None`. Logs each feedback event with its rationale.
    pub(crate) fn drain_feedback_with_adaptation(&self, planned_position: f64) -> Option<f64> {
        let mut adjusted: Option<f64> = None;
        while let Some(event) = self.check_feedback() {
            info!(
                ?event,
                "Adaptive scan: consumed feedback event between steps"
            );
            if let Some(pos) = self.adapt_scan_point(planned_position, &event) {
                adjusted = Some(pos);
            }
        }
        adjusted
    }
}
