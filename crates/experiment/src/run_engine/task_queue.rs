//! Plan queue management.
//!
//! Encapsulates `QueuedPlan` and the queue operations (enqueue, dequeue,
//! inspect, clear) in a dedicated struct so the `RunEngine` delegates
//! rather than owning the raw `Vec` directly.

use std::collections::HashMap;

use tokio::sync::Mutex;
use tracing::info;

use crate::plans::Plan;
use common::experiment::document::new_uid;

/// A queued plan waiting to be executed.
pub(crate) struct QueuedPlan {
    pub(crate) plan: Box<dyn Plan>,
    pub(crate) metadata: HashMap<String, String>,
    pub(crate) run_uid: String,
}

/// Thread-safe plan queue.
///
/// Wraps a `Mutex<Vec<QueuedPlan>>` and exposes a focused API for
/// enqueueing, dequeueing, inspecting, and clearing plans. This avoids
/// exposing the raw collection on `RunEngine` and allows queue logic
/// (e.g., future priority ordering) to evolve independently.
pub(crate) struct TaskQueue {
    queue: Mutex<Vec<QueuedPlan>>,
}

impl TaskQueue {
    pub(crate) fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    /// Enqueue a plan with metadata and return its assigned `run_uid`.
    pub(crate) async fn enqueue(
        &self,
        plan: Box<dyn Plan>,
        metadata: HashMap<String, String>,
    ) -> String {
        let run_uid = new_uid();
        info!(run_uid = %run_uid, plan_type = %plan.plan_type(), "Queueing plan");

        let mut queue = self.queue.lock().await;
        queue.push(QueuedPlan {
            plan,
            metadata,
            run_uid: run_uid.clone(),
        });

        run_uid
    }

    /// Remove and return the next plan, or `None` if the queue is empty.
    pub(crate) async fn dequeue(&self) -> Option<QueuedPlan> {
        let mut queue = self.queue.lock().await;
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    /// Remove a specific plan by `run_uid`. Returns the removed plan, if found.
    pub(crate) async fn remove(&self, run_uid: &str) -> Option<QueuedPlan> {
        let mut queue = self.queue.lock().await;
        queue
            .iter()
            .position(|q| q.run_uid == run_uid)
            .map(|pos| queue.remove(pos))
    }

    /// Return the `run_uid` values for all queued plans, in order.
    pub(crate) async fn run_uids(&self) -> Vec<String> {
        self.queue
            .lock()
            .await
            .iter()
            .map(|q| q.run_uid.clone())
            .collect()
    }

    /// Number of plans in the queue.
    pub(crate) async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Discard all queued plans.
    pub(crate) async fn clear(&self) {
        self.queue.lock().await.clear();
    }

    /// Access the first queued plan (for readiness checks) without removing it.
    pub(crate) async fn peek_first<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&QueuedPlan>) -> R,
    {
        let queue = self.queue.lock().await;
        f(queue.first())
    }
}
