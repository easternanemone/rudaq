//! RunEngine - State machine for experiment orchestration (bd-73yh.1)
//!
//! The RunEngine executes plans, manages pause/resume, and emits documents.
//! It provides a clean abstraction between experiment logic (plans) and
//! hardware operations.
//!
//! # Architecture
//!
//! RunEngine delegates to composed sub-components rather than owning
//! all state directly:
//!
//! - [`TaskQueue`](task_queue::TaskQueue) — plan queue management (enqueue, dequeue, clear)
//! - [`WatchdogManager`](watchdog::WatchdogManager) — orphan-plan detection (activity timestamp, timeout)
//!
//! # State Machine
//!
//! ```text
//! ┌──────┐   start()   ┌─────────┐
//! │ Idle │────────────▶│ Running │
//! └──────┘             └────┬────┘
//!    ▲                      │
//!    │  completed           │ pause() at checkpoint
//!    │                      ▼
//!    │                 ┌────────┐
//!    │◀────resume()────│ Paused │
//!    │                 └────────┘
//!    │
//!    │  abort()/halt()
//!    └────────────────────────────
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! let engine = RunEngine::new(device_registry);
//!
//! // Subscribe to documents
//! let mut docs = engine.subscribe();
//!
//! // Queue and run a plan
//! let run_uid = engine.queue(plan).await?;
//! engine.start().await?;
//!
//! // Process documents as they arrive
//! while let Some(doc) = docs.recv().await {
//!     match doc {
//!         Document::Event(e) => {
//!             println!("Data: {:?}", e.data);
//!             println!("Frames: {:?}", e.arrays.keys());
//!         }
//!         Document::Stop(_) => break,
//!         _ => {}
//!     }
//! }
//! ```

pub(crate) mod command_dispatch;
pub(crate) mod context;
mod documents;
mod executor;
mod readiness;
mod state_machine;
pub(crate) mod task_queue;
#[cfg(test)]
mod tests;
pub(crate) mod watchdog;

pub use documents::RunResult;
pub use readiness::{CalibrationFreshness, CalibrationWavelengthCoverage, RunReadinessIssue};
pub use state_machine::EngineState;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tokio::time::Duration;
use tracing::{info, instrument, warn};

use super::feedback::FeedbackEvent;
use super::lifecycle::RunLifecycleHook;
use super::plans::Plan;
use common::experiment::document::Document;
use hardware::registry::DeviceRegistry;

use context::RunContext;
use task_queue::TaskQueue;
use watchdog::WatchdogManager;

/// The RunEngine orchestrates experiment execution.
///
/// Composes [`TaskQueue`] for plan queue management and
/// [`WatchdogManager`] for orphan-plan detection, rather than
/// owning the raw primitives directly.
pub struct RunEngine {
    /// Current engine state
    pub(crate) state: RwLock<EngineState>,

    /// Device registry for hardware operations
    pub(crate) device_registry: Arc<DeviceRegistry>,

    /// Plan queue (composed component)
    pub(crate) task_queue: TaskQueue,

    /// Orphan-plan watchdog (composed component)
    pub(crate) watchdog: WatchdogManager,

    /// Document broadcast channel
    pub(crate) doc_sender: broadcast::Sender<Document>,

    /// State-change broadcast channel (push-based state updates, bd-sz76)
    pub(crate) state_sender: broadcast::Sender<EngineState>,

    /// Pause request flag
    pub(crate) pause_requested: RwLock<bool>,

    /// Abort request flag
    pub(crate) abort_requested: RwLock<bool>,

    /// Current run context (when running)
    pub(crate) run_context: Mutex<Option<RunContext>>,

    /// Last checkpoint label (for resume)
    pub(crate) last_checkpoint: RwLock<Option<String>>,

    /// Optional lifecycle hook for heartbeat and other cross-cutting concerns.
    ///
    /// When set, the engine spawns a background task during plan execution
    /// that calls `on_heartbeat()` every ~10 seconds. This enables the
    /// reconciler to detect stale runs (crashed daemons).
    pub(crate) lifecycle_hook: Option<Arc<dyn RunLifecycleHook>>,

    /// Freshness metadata for calibrations that should gate run starts.
    pub(crate) readiness: readiness::ReadinessManager,

    /// Feedback channel sender for data-plane events (bd-7rg0).
    pub(crate) feedback_tx: mpsc::Sender<FeedbackEvent>,

    /// Feedback channel receiver (taken by a single consumer via `subscribe_feedback`).
    pub(crate) feedback_rx: Mutex<Option<mpsc::Receiver<FeedbackEvent>>>,
}

impl RunEngine {
    /// Create a new RunEngine
    pub fn new(device_registry: Arc<DeviceRegistry>) -> Self {
        let (doc_sender, _) = broadcast::channel(1024);
        let (state_sender, _) = broadcast::channel(16);
        let (feedback_tx, feedback_rx) = mpsc::channel(256);

        Self {
            state: RwLock::new(EngineState::Idle),
            device_registry,
            task_queue: TaskQueue::new(),
            watchdog: WatchdogManager::new(),
            doc_sender,
            state_sender,
            pause_requested: RwLock::new(false),
            abort_requested: RwLock::new(false),
            run_context: Mutex::new(None),
            last_checkpoint: RwLock::new(None),
            lifecycle_hook: None,
            readiness: readiness::ReadinessManager::new(),

            feedback_tx,
            feedback_rx: Mutex::new(Some(feedback_rx)),
        }
    }

    // ---- Feedback channel ----

    /// Subscribe to the feedback channel for data-plane events (bd-7rg0).
    ///
    /// Only one consumer can subscribe; subsequent calls return `None`.
    /// The receiver yields `FeedbackEvent` values as the engine detects
    /// threshold crossings, stability events, and value updates.
    pub async fn subscribe_feedback(&self) -> Option<mpsc::Receiver<FeedbackEvent>> {
        self.feedback_rx.lock().await.take()
    }

    /// Clone the feedback sender for external producers (bd-7md9).
    ///
    /// This allows server-layer components (e.g., `FeedbackRouter`) to push
    /// `FeedbackEvent` values into the same channel the `RunEngine` uses
    /// internally. The sender uses `try_send` semantics — callers should
    /// handle a full channel gracefully.
    pub fn feedback_sender(&self) -> mpsc::Sender<FeedbackEvent> {
        self.feedback_tx.clone()
    }

    // ---- Watchdog delegation ----

    /// Set the watchdog timeout for orphaned plan detection.
    ///
    /// If a plan has been Running or Paused with no meaningful activity
    /// (MoveTo, Read, Trigger, etc.) for longer than this duration, the
    /// watchdog will abort it automatically.
    pub fn set_watchdog_timeout(&mut self, timeout: Duration) {
        self.watchdog.set_timeout(timeout);
    }

    /// Set the lifecycle hook for heartbeat monitoring and other events.
    ///
    /// When set, the engine will spawn a background task during plan execution
    /// that calls `on_heartbeat()` every ~10 seconds.
    pub fn set_lifecycle_hook(&mut self, hook: Arc<dyn RunLifecycleHook>) {
        self.lifecycle_hook = Some(hook);
    }

    /// Record meaningful activity (resets the watchdog timer).
    ///
    /// Called internally on every command execution (MoveTo, Read, Trigger,
    /// EmitEvent, Set) and on state transitions (start, pause, resume).
    /// Also available to the gRPC layer so that external client requests
    /// (e.g. `get_engine_status`) can prove liveness.
    pub async fn touch_activity(&self) {
        self.watchdog.touch().await;
    }

    /// Spawn a background watchdog task that periodically checks for orphaned plans.
    ///
    /// A plan is considered orphaned when the engine has been in `Running` or
    /// `Paused` state with no meaningful activity for longer than the configured
    /// `watchdog_timeout` (default: 5 minutes).
    ///
    /// When an orphaned plan is detected, the watchdog aborts it and logs a
    /// warning. The check interval is one-tenth of the timeout (minimum 1s).
    ///
    /// The returned `JoinHandle` can be used to abort the watchdog on shutdown.
    pub fn spawn_watchdog(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);
        let timeout = engine.watchdog.timeout();
        let check_interval = engine.watchdog.check_interval();

        info!(
            timeout_secs = timeout.as_secs(),
            check_interval_secs = check_interval.as_secs(),
            "RunEngine orphan-plan watchdog started"
        );

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                interval.tick().await;

                let state = *engine.state.read().await;
                match state {
                    EngineState::Running | EngineState::Paused => {
                        if engine.watchdog.is_expired().await {
                            let elapsed = engine.watchdog.elapsed().await;
                            let run_uid = engine.current_run_uid().await;
                            warn!(
                                state = %state,
                                elapsed_secs = elapsed.as_secs(),
                                timeout_secs = timeout.as_secs(),
                                run_uid = ?run_uid,
                                "Orphaned plan detected: no activity for {} seconds, aborting",
                                elapsed.as_secs()
                            );
                            if let Err(e) = engine
                                .abort("watchdog: orphaned plan (no client activity)")
                                .await
                            {
                                warn!(error = %e, "Watchdog failed to abort orphaned plan");
                            }
                        }
                    }
                    // Idle or Aborting: nothing to watch
                    EngineState::Idle | EngineState::Aborting => {}
                }
            }
        })
    }

    // ---- Document subscription ----

    /// Subscribe to document stream
    pub fn subscribe(&self) -> broadcast::Receiver<Document> {
        self.doc_sender.subscribe()
    }

    // ---- State subscription (bd-sz76) ----

    /// Subscribe to engine state changes (push-based).
    pub fn subscribe_state(&self) -> broadcast::Receiver<EngineState> {
        self.state_sender.subscribe()
    }

    /// Set engine state and broadcast the change to all subscribers.
    pub(crate) async fn set_state(&self, new_state: EngineState) {
        *self.state.write().await = new_state;
        let _ = self.state_sender.send(new_state);
    }

    // ---- State queries ----

    /// Get current engine state
    pub async fn state(&self) -> EngineState {
        *self.state.read().await
    }

    /// Get the start time (Unix nanoseconds) of the current run, if any
    pub async fn current_run_start_ns(&self) -> Option<u64> {
        self.run_context
            .lock()
            .await
            .as_ref()
            .map(|ctx| ctx.run_start_ns)
    }

    /// Get the current run UID (if running)
    pub async fn current_run_uid(&self) -> Option<String> {
        self.run_context
            .lock()
            .await
            .as_ref()
            .map(|ctx| ctx.run_uid.clone())
    }

    /// Get current progress (events emitted so far)
    pub async fn current_progress(&self) -> Option<u32> {
        self.run_context
            .lock()
            .await
            .as_ref()
            .map(|ctx| ctx.seq_num)
    }

    // ---- Queue delegation ----

    /// Get the run_uids of all queued plans
    pub async fn queued_run_uids(&self) -> Vec<String> {
        self.task_queue.run_uids().await
    }

    /// Queue a plan for execution
    pub async fn queue(&self, plan: Box<dyn Plan>) -> String {
        self.task_queue.enqueue(plan, HashMap::new()).await
    }

    /// Queue a plan with user-provided metadata
    pub async fn queue_with_metadata(
        &self,
        plan: Box<dyn Plan>,
        metadata: HashMap<String, String>,
    ) -> String {
        self.task_queue.enqueue(plan, metadata).await
    }

    /// Get the number of queued plans
    pub async fn queue_len(&self) -> usize {
        self.task_queue.len().await
    }

    /// Clear all queued plans
    pub async fn clear_queue(&self) {
        self.task_queue.clear().await;
    }

    // ---- Engine control ----

    /// Start executing queued plans
    #[instrument(skip(self), err)]
    pub async fn start(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Idle {
            anyhow::bail!("Cannot start: engine is {}", current_state);
        }

        let readiness_issues = self.next_plan_readiness_issues().await;
        if let Some(issue) = readiness_issues.iter().find(|issue| issue.blocking) {
            warn!(
                target: "calibration_staleness",
                device_type = issue.device_type.as_deref().unwrap_or("unknown"),
                age_hours = issue.age_hours.unwrap_or_default(),
                max_age_hours = issue.max_age_hours.unwrap_or_default(),
                "{}",
                issue.message
            );
            anyhow::bail!("{}", issue.message);
        }

        // Reset flags
        *self.pause_requested.write().await = false;
        *self.abort_requested.write().await = false;

        // Get next plan from queue
        let queued = self
            .task_queue
            .dequeue()
            .await
            .ok_or_else(|| anyhow::anyhow!("No plans in queue"))?;

        self.set_state(EngineState::Running).await;
        self.watchdog.touch().await;
        info!("Engine started");

        // Execute the plan
        self.execute_plan(queued).await
    }

    /// Request pause at next checkpoint
    #[instrument(skip(self), err)]
    pub async fn pause(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Running {
            anyhow::bail!("Cannot pause: engine is {}", current_state);
        }

        info!("Pause requested");
        *self.pause_requested.write().await = true;
        self.watchdog.touch().await;
        Ok(())
    }

    /// Resume from paused state
    #[instrument(skip(self), err)]
    pub async fn resume(&self) -> anyhow::Result<()> {
        let current_state = *self.state.read().await;
        if current_state != EngineState::Paused {
            anyhow::bail!("Cannot resume: engine is {}", current_state);
        }

        info!("Resuming from pause");
        *self.pause_requested.write().await = false;
        self.set_state(EngineState::Running).await;
        self.watchdog.touch().await;
        Ok(())
    }

    /// Abort a plan by run_uid or the current plan if run_uid is None/empty
    ///
    /// - If `run_uid` is None or empty, aborts the currently executing plan
    /// - If `run_uid` matches the current run, aborts it
    /// - If `run_uid` matches a queued plan, removes it from the queue
    /// - Returns error if `run_uid` is specified but not found
    #[instrument(skip(self), fields(reason), err)]
    pub async fn abort(&self, reason: &str) -> anyhow::Result<()> {
        self.abort_run(None, reason).await
    }

    /// Abort a specific run by run_uid, or current if None/empty (bd-vi16.3)
    #[instrument(skip(self), fields(run_uid, reason), err)]
    pub async fn abort_run(&self, run_uid: Option<&str>, reason: &str) -> anyhow::Result<()> {
        let target_uid = run_uid.filter(|s| !s.is_empty());

        match target_uid {
            None => {
                // Abort current run (existing behavior)
                let current_state = *self.state.read().await;
                match current_state {
                    EngineState::Running | EngineState::Paused => {
                        info!(reason = %reason, "Abort requested for current run");
                        *self.abort_requested.write().await = true;
                        self.set_state(EngineState::Aborting).await;
                        Ok(())
                    }
                    _ => anyhow::bail!("Cannot abort: engine is {}", current_state),
                }
            }
            Some(uid) => {
                // Check if it matches current run
                let current_run_uid = self.current_run_uid().await;
                if current_run_uid.as_deref() == Some(uid) {
                    info!(run_uid = %uid, reason = %reason, "Abort requested for current run");
                    *self.abort_requested.write().await = true;
                    self.set_state(EngineState::Aborting).await;
                    return Ok(());
                }

                // Check if it matches a queued plan
                if let Some(removed) = self.task_queue.remove(uid).await {
                    info!(
                        run_uid = %uid,
                        plan_type = %removed.plan.plan_type(),
                        reason = %reason,
                        "Removed queued plan"
                    );
                    return Ok(());
                }

                // Not found
                anyhow::bail!("Run '{}' not found (not current and not queued)", uid)
            }
        }
    }

    /// Halt immediately (emergency stop)
    pub async fn halt(&self) -> anyhow::Result<()> {
        warn!("HALT requested - emergency stop");
        *self.abort_requested.write().await = true;
        self.set_state(EngineState::Aborting).await;
        // In a real implementation, this would also send stop commands to all hardware
        Ok(())
    }

    // ---- CommandDispatcher delegation ----

    /// Evaluate an `EvalCondition` by reading from the device registry.
    ///
    /// Convenience wrapper that creates a [`CommandDispatcher`] and delegates.
    /// Used by tests and by code that doesn't already have a dispatcher.
    pub(crate) async fn evaluate_condition(&self, condition: &crate::plans::EvalCondition) -> bool {
        let dispatcher = command_dispatch::CommandDispatcher {
            registry: &self.device_registry,
            feedback_tx: &self.feedback_tx,
        };
        dispatcher.evaluate_condition(condition).await
    }
}
