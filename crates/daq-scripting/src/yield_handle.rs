//! Yield Handle - Communication channel between script and ScriptPlanRunner (bd-94zq.4)
//!
//! This module provides the types needed for yield-based plan scripting.
//! Scripts yield plans to the runner, which executes them via the shared
//! RunEngine and returns results back to the script.
//!
//! # Architecture
//!
//! ```text
//! Script (Rhai)              YieldHandle              ScriptPlanRunner
//!       │                         │                         │
//!       │ yield plan              │                         │
//!       │────────────────────────▶│ send to plan_tx         │
//!       │                         │────────────────────────▶│
//!       │                         │                         │ execute via RunEngine
//!       │                         │                         │
//!       │                         │     send YieldResult    │
//!       │                         │◀────────────────────────│
//!       │◀────────────────────────│                         │
//!       │ resume with result      │                         │
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use daq_experiment::plans::{Plan, PlanCommand};
use tokio::sync::{mpsc, watch};

/// Value yielded from script to runner
pub enum YieldedValue {
    /// A plan to execute
    Plan(Box<dyn Plan>),
    /// A single imperative command (wrapped as ImperativePlan internally)
    Command(PlanCommand),
    /// Script completed successfully
    Done,
    /// Script encountered an error
    Error(String),
}

impl std::fmt::Debug for YieldedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YieldedValue::Plan(_) => write!(f, "YieldedValue::Plan(...)"),
            YieldedValue::Command(cmd) => write!(f, "YieldedValue::Command({:?})", cmd),
            YieldedValue::Done => write!(f, "YieldedValue::Done"),
            YieldedValue::Error(e) => write!(f, "YieldedValue::Error({})", e),
        }
    }
}

/// Result returned to script after plan execution
#[derive(Debug, Clone, Default)]
pub struct YieldResult {
    /// Unique identifier for the run
    pub run_uid: String,
    /// Exit status: "success", "abort", or "fail"
    pub exit_status: String,
    /// Last event's scalar data (key -> value)
    pub data: HashMap<String, f64>,
    /// Last event's device positions (device_id -> position)
    pub positions: HashMap<String, f64>,
    /// Total number of events emitted
    pub num_events: u32,
    /// Optional error message if exit_status is "fail"
    pub error: Option<String>,
}

impl YieldResult {
    /// Create a successful result
    pub fn success(
        run_uid: String,
        data: HashMap<String, f64>,
        positions: HashMap<String, f64>,
        num_events: u32,
    ) -> Self {
        Self {
            run_uid,
            exit_status: "success".to_string(),
            data,
            positions,
            num_events,
            error: None,
        }
    }

    /// Create a failed result
    pub fn fail(run_uid: String, error: impl Into<String>, num_events: u32) -> Self {
        Self {
            run_uid,
            exit_status: "fail".to_string(),
            data: HashMap::new(),
            positions: HashMap::new(),
            num_events,
            error: Some(error.into()),
        }
    }

    /// Create an aborted result
    pub fn abort(run_uid: String, reason: impl Into<String>, num_events: u32) -> Self {
        Self {
            run_uid,
            exit_status: "abort".to_string(),
            data: HashMap::new(),
            positions: HashMap::new(),
            num_events,
            error: Some(reason.into()),
        }
    }

    /// Check if the run succeeded
    pub fn is_success(&self) -> bool {
        self.exit_status == "success"
    }

    /// Check if the run was aborted
    pub fn is_abort(&self) -> bool {
        self.exit_status == "abort"
    }

    /// Check if the run failed
    pub fn is_fail(&self) -> bool {
        self.exit_status == "fail"
    }
}

/// Handle for script-side yield communication
///
/// This is passed to Rhai scripts as a global variable and provides
/// the mechanism for scripts to yield plans and receive results.
pub struct YieldHandle {
    /// Channel to send yielded plans to the runner
    plan_tx: mpsc::Sender<YieldedValue>,
    /// Channel to receive results from the runner
    result_rx: watch::Receiver<Option<YieldResult>>,
    /// Flag to track if we're waiting for a result
    waiting_for_result: std::sync::atomic::AtomicBool,
}

impl YieldHandle {
    /// Create a new YieldHandle with the given channels
    pub fn new(
        plan_tx: mpsc::Sender<YieldedValue>,
        result_rx: watch::Receiver<Option<YieldResult>>,
    ) -> Self {
        Self {
            plan_tx,
            result_rx,
            waiting_for_result: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Yield a plan and wait for the result
    ///
    /// This is called from Rhai scripts via the `yield` binding.
    /// It blocks the script thread until the plan completes.
    pub fn yield_plan(&self, plan: Box<dyn Plan>) -> Result<YieldResult, String> {
        self.yield_value(YieldedValue::Plan(plan))
    }

    /// Yield a single command and wait for the result
    ///
    /// Used for imperative-style commands that get wrapped in ImperativePlan.
    pub fn yield_command(&self, command: PlanCommand) -> Result<YieldResult, String> {
        self.yield_value(YieldedValue::Command(command))
    }

    /// Internal yield implementation
    fn yield_value(&self, value: YieldedValue) -> Result<YieldResult, String> {
        use std::sync::atomic::Ordering;

        // Mark that we're waiting
        self.waiting_for_result.store(true, Ordering::SeqCst);

        // Send the yielded value to the runner
        self.plan_tx
            .blocking_send(value)
            .map_err(|e| format!("Failed to send yielded value: {}", e))?;

        // Block waiting for result
        // This is called from spawn_blocking, so blocking is OK
        loop {
            // Check if a result is available
            {
                let result = self.result_rx.borrow();
                if let Some(ref res) = *result {
                    let cloned = res.clone();
                    self.waiting_for_result.store(false, Ordering::SeqCst);
                    return Ok(cloned);
                }
            }

            // Small sleep to avoid busy waiting
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Signal that the script has completed successfully
    pub fn signal_done(&self) -> Result<(), String> {
        self.plan_tx
            .blocking_send(YieldedValue::Done)
            .map_err(|e| format!("Failed to signal done: {}", e))
    }

    /// Signal that the script encountered an error
    pub fn signal_error(&self, error: String) -> Result<(), String> {
        self.plan_tx
            .blocking_send(YieldedValue::Error(error))
            .map_err(|e| format!("Failed to signal error: {}", e))
    }

    /// Check if currently waiting for a result
    pub fn is_waiting(&self) -> bool {
        self.waiting_for_result
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Builder for creating YieldHandle pairs
///
/// Creates both ends of the yield communication channel.
pub struct YieldChannelBuilder {
    /// Buffer size for the plan channel
    plan_buffer_size: usize,
}

impl Default for YieldChannelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl YieldChannelBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self {
            plan_buffer_size: 16,
        }
    }

    /// Set the buffer size for the plan channel
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.plan_buffer_size = size;
        self
    }

    /// Build the channel pair
    ///
    /// Returns:
    /// - YieldHandle for the script side
    /// - mpsc::Receiver for receiving yielded values
    /// - watch::Sender for sending results back
    pub fn build(
        self,
    ) -> (
        Arc<YieldHandle>,
        mpsc::Receiver<YieldedValue>,
        watch::Sender<Option<YieldResult>>,
    ) {
        let (plan_tx, plan_rx) = mpsc::channel(self.plan_buffer_size);
        let (result_tx, result_rx) = watch::channel(None);

        let handle = Arc::new(YieldHandle::new(plan_tx, result_rx));

        (handle, plan_rx, result_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yield_result_success() {
        let mut data = HashMap::new();
        data.insert("power".to_string(), 42.0);

        let result = YieldResult::success("run_123".to_string(), data, HashMap::new(), 10);

        assert!(result.is_success());
        assert!(!result.is_fail());
        assert!(!result.is_abort());
        assert_eq!(result.num_events, 10);
    }

    #[test]
    fn test_yield_result_fail() {
        let result = YieldResult::fail("run_123".to_string(), "Hardware error", 5);

        assert!(result.is_fail());
        assert!(!result.is_success());
        assert!(result.error.is_some());
    }

    #[test]
    fn test_yield_channel_builder() {
        let (handle, _rx, _tx) = YieldChannelBuilder::new().with_buffer_size(32).build();

        assert!(!handle.is_waiting());
    }
}
