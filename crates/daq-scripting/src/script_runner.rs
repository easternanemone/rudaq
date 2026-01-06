//! Script Plan Runner - Async executor for yield-based scripts (bd-94zq.4)
//!
//! This module provides `ScriptPlanRunner`, which executes Rhai scripts
//! that yield plans. It bridges the synchronous Rhai execution with
//! the async RunEngine.
//!
//! # Architecture
//!
//! ```text
//!                          ScriptPlanRunner::run()
//!                                   │
//!     ┌─────────────────────────────┼─────────────────────────────┐
//!     │                             │                             │
//!     ▼                             ▼                             ▼
//! spawn_blocking             tokio::select!               RunEngine
//! (Rhai script)         (receive from yield_rx)       (execute plans)
//!     │                             │                             │
//!     │ yield plan ─────────────────▶                             │
//!     │                             │ execute via engine ─────────▶
//!     │                             │                             │
//!     │                             │◀───────── Documents ────────│
//!     │◀───────────────── result ───│                             │
//!     │ (resume script)             │                             │
//!     │                             │                             │
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_scripting::script_runner::ScriptPlanRunner;
//!
//! let runner = ScriptPlanRunner::new(run_engine.clone());
//! let report = runner.run(script_content).await?;
//!
//! println!("Script completed with {} plans executed", report.plans_executed);
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
// mpsc and watch channels used indirectly through YieldChannelBuilder
use tracing::{debug, error, info, warn};

use daq_core::experiment::document::Document;
use daq_experiment::plans::Plan;
use daq_experiment::plans_imperative::ImperativePlan;
use daq_experiment::run_engine::RunEngine;

use crate::yield_handle::{YieldChannelBuilder, YieldResult, YieldedValue};
use crate::RhaiEngine;

/// Report from a completed script execution
#[derive(Debug, Clone)]
pub struct ScriptRunReport {
    /// Total number of plans executed
    pub plans_executed: u32,
    /// Total number of events emitted across all plans
    pub total_events: u32,
    /// Total execution time
    pub duration: Duration,
    /// Whether the script completed successfully
    pub success: bool,
    /// Error message if script failed
    pub error: Option<String>,
    /// Run UIDs of all executed plans
    pub run_uids: Vec<String>,
}

impl ScriptRunReport {
    /// Create a success report
    fn success(plans: u32, events: u32, duration: Duration, run_uids: Vec<String>) -> Self {
        Self {
            plans_executed: plans,
            total_events: events,
            duration,
            success: true,
            error: None,
            run_uids,
        }
    }

    /// Create a failure report
    fn failure(
        error: impl Into<String>,
        plans: u32,
        events: u32,
        duration: Duration,
        run_uids: Vec<String>,
    ) -> Self {
        Self {
            plans_executed: plans,
            total_events: events,
            duration,
            success: false,
            error: Some(error.into()),
            run_uids,
        }
    }
}

/// Configuration for script execution
#[derive(Debug, Clone)]
pub struct ScriptRunConfig {
    /// Maximum execution time for the entire script
    pub timeout: Duration,
    /// Maximum number of plans a script can yield
    pub max_plans: usize,
    /// Whether to continue on plan failure
    pub continue_on_error: bool,
}

impl Default for ScriptRunConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3600), // 1 hour default
            max_plans: 1000,
            continue_on_error: false,
        }
    }
}

/// Executes yield-based Rhai scripts via the RunEngine
pub struct ScriptPlanRunner {
    /// Shared RunEngine instance
    run_engine: Arc<RunEngine>,
    /// Execution configuration
    config: ScriptRunConfig,
}

impl ScriptPlanRunner {
    /// Create a new ScriptPlanRunner with the given RunEngine
    pub fn new(run_engine: Arc<RunEngine>) -> Self {
        Self {
            run_engine,
            config: ScriptRunConfig::default(),
        }
    }

    /// Create a runner with custom configuration
    pub fn with_config(run_engine: Arc<RunEngine>, config: ScriptRunConfig) -> Self {
        Self { run_engine, config }
    }

    /// Run a script and return a report
    pub async fn run(&self, script: &str) -> Result<ScriptRunReport> {
        let start_time = Instant::now();
        let mut plans_executed = 0u32;
        let mut total_events = 0u32;
        let mut run_uids = Vec::new();

        info!("Starting script execution");

        // Create yield channels
        let (yield_handle, mut yield_rx, result_tx) = YieldChannelBuilder::new().build();

        // Clone what we need for the script task
        let script_owned = script.to_string();
        let handle_for_script = yield_handle.clone();

        // Spawn the script in a blocking task
        let script_task = tokio::task::spawn_blocking(move || {
            Self::run_script_blocking(&script_owned, handle_for_script)
        });

        // Main loop: receive yielded values and execute them
        let timeout_deadline = Instant::now() + self.config.timeout;

        loop {
            // Check timeout
            if Instant::now() > timeout_deadline {
                error!("Script execution timed out");
                return Ok(ScriptRunReport::failure(
                    "Script execution timed out",
                    plans_executed,
                    total_events,
                    start_time.elapsed(),
                    run_uids,
                ));
            }

            // Check plan limit
            if plans_executed as usize >= self.config.max_plans {
                error!("Script exceeded maximum plan limit");
                return Ok(ScriptRunReport::failure(
                    format!(
                        "Script exceeded maximum plan limit of {}",
                        self.config.max_plans
                    ),
                    plans_executed,
                    total_events,
                    start_time.elapsed(),
                    run_uids,
                ));
            }

            // Wait for next yielded value with timeout
            let receive_result =
                tokio::time::timeout(Duration::from_millis(100), yield_rx.recv()).await;

            match receive_result {
                Ok(Some(yielded)) => {
                    match yielded {
                        YieldedValue::Plan(plan) => {
                            debug!("Received yielded plan: {}", plan.plan_type());
                            match self.execute_plan(plan).await {
                                Ok(result) => {
                                    plans_executed += 1;
                                    total_events += result.num_events;
                                    run_uids.push(result.run_uid.clone());

                                    // Send result back to script
                                    if result_tx.send(Some(result)).is_err() {
                                        warn!("Failed to send result to script (channel closed)");
                                    }
                                }
                                Err(e) => {
                                    error!("Plan execution failed: {}", e);
                                    let error_result =
                                        YieldResult::fail(String::new(), e.to_string(), 0);
                                    let _ = result_tx.send(Some(error_result));

                                    if !self.config.continue_on_error {
                                        return Ok(ScriptRunReport::failure(
                                            e.to_string(),
                                            plans_executed,
                                            total_events,
                                            start_time.elapsed(),
                                            run_uids,
                                        ));
                                    }
                                }
                            }
                        }
                        YieldedValue::Command(cmd) => {
                            debug!("Received yielded command: {:?}", cmd);
                            // Wrap single command in ImperativePlan
                            let plan = Box::new(ImperativePlan::new(vec![cmd]));
                            match self.execute_plan(plan).await {
                                Ok(result) => {
                                    plans_executed += 1;
                                    total_events += result.num_events;
                                    run_uids.push(result.run_uid.clone());

                                    if result_tx.send(Some(result)).is_err() {
                                        warn!("Failed to send result to script");
                                    }
                                }
                                Err(e) => {
                                    error!("Command execution failed: {}", e);
                                    let error_result =
                                        YieldResult::fail(String::new(), e.to_string(), 0);
                                    let _ = result_tx.send(Some(error_result));

                                    if !self.config.continue_on_error {
                                        return Ok(ScriptRunReport::failure(
                                            e.to_string(),
                                            plans_executed,
                                            total_events,
                                            start_time.elapsed(),
                                            run_uids,
                                        ));
                                    }
                                }
                            }
                        }
                        YieldedValue::Done => {
                            info!("Script signaled completion");
                            break;
                        }
                        YieldedValue::Error(e) => {
                            error!("Script signaled error: {}", e);
                            return Ok(ScriptRunReport::failure(
                                e,
                                plans_executed,
                                total_events,
                                start_time.elapsed(),
                                run_uids,
                            ));
                        }
                    }
                }
                Ok(None) => {
                    // Channel closed - script task finished
                    debug!("Yield channel closed, checking script task");
                    break;
                }
                Err(_) => {
                    // Timeout on receive - check if script task is done
                    if script_task.is_finished() {
                        break;
                    }
                    // Otherwise continue waiting
                    continue;
                }
            }
        }

        // Wait for script task to complete
        match script_task.await {
            Ok(Ok(())) => {
                info!(
                    "Script completed successfully: {} plans, {} events, {:?}",
                    plans_executed,
                    total_events,
                    start_time.elapsed()
                );
                Ok(ScriptRunReport::success(
                    plans_executed,
                    total_events,
                    start_time.elapsed(),
                    run_uids,
                ))
            }
            Ok(Err(e)) => {
                error!("Script execution error: {}", e);
                Ok(ScriptRunReport::failure(
                    e.to_string(),
                    plans_executed,
                    total_events,
                    start_time.elapsed(),
                    run_uids,
                ))
            }
            Err(e) => {
                error!("Script task panicked: {}", e);
                Ok(ScriptRunReport::failure(
                    format!("Script task panicked: {}", e),
                    plans_executed,
                    total_events,
                    start_time.elapsed(),
                    run_uids,
                ))
            }
        }
    }

    /// Run the Rhai script in a blocking context
    fn run_script_blocking(
        script: &str,
        yield_handle: Arc<crate::yield_handle::YieldHandle>,
    ) -> Result<()> {
        // Create Rhai engine with yield support
        let mut engine = RhaiEngine::new()?;

        // Register the yield handle
        engine.set_yield_handle(yield_handle.clone())?;

        // Execute the script
        let result = engine.eval::<()>(script);

        // Signal completion
        match result {
            Ok(()) => {
                let _ = yield_handle.signal_done();
                Ok(())
            }
            Err(e) => {
                let _ = yield_handle.signal_error(e.to_string());
                Err(anyhow!("Script error: {}", e))
            }
        }
    }

    /// Execute a single plan via the RunEngine and return the result
    async fn execute_plan(&self, plan: Box<dyn Plan>) -> Result<YieldResult> {
        let plan_type = plan.plan_type().to_string();
        debug!("Executing plan: {}", plan_type);

        // Subscribe to documents before queueing
        let mut doc_rx = self.run_engine.subscribe();

        // Queue the plan
        let run_uid = self.run_engine.queue(plan).await;
        debug!("Plan queued with run_uid: {}", run_uid);

        // Start execution
        self.run_engine.start().await?;

        // Collect documents until Stop
        let mut last_event_data = HashMap::new();
        let mut last_event_positions = HashMap::new();
        let mut num_events = 0u32;

        loop {
            match tokio::time::timeout(Duration::from_secs(300), doc_rx.recv()).await {
                Ok(Ok(doc)) => {
                    match doc {
                        Document::Event(event) if event.run_uid == run_uid => {
                            num_events += 1;
                            last_event_data = event.data.clone();
                            last_event_positions = event.positions.clone();
                        }
                        Document::Stop(stop) if stop.run_uid == run_uid => {
                            debug!(
                                "Plan {} completed: {} events, status={}",
                                run_uid, num_events, stop.exit_status
                            );

                            return match stop.exit_status.as_str() {
                                "success" => Ok(YieldResult::success(
                                    run_uid,
                                    last_event_data,
                                    last_event_positions,
                                    num_events,
                                )),
                                "abort" => Ok(YieldResult::abort(run_uid, stop.reason, num_events)),
                                _ => Ok(YieldResult::fail(run_uid, stop.reason, num_events)),
                            };
                        }
                        _ => {
                            // Ignore documents from other runs
                        }
                    }
                }
                Ok(Err(e)) => {
                    // Broadcast channel lagged - this shouldn't normally happen
                    warn!("Document channel error: {}", e);
                }
                Err(_) => {
                    // Timeout waiting for documents
                    return Err(anyhow!("Timeout waiting for plan completion"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_run_report_success() {
        let report = ScriptRunReport::success(
            5,
            100,
            Duration::from_secs(10),
            vec!["run1".into(), "run2".into()],
        );

        assert!(report.success);
        assert_eq!(report.plans_executed, 5);
        assert_eq!(report.total_events, 100);
        assert!(report.error.is_none());
    }

    #[test]
    fn test_script_run_report_failure() {
        let report = ScriptRunReport::failure(
            "Test error",
            3,
            50,
            Duration::from_secs(5),
            vec!["run1".into()],
        );

        assert!(!report.success);
        assert_eq!(report.error, Some("Test error".to_string()));
    }

    #[test]
    fn test_script_run_config_default() {
        let config = ScriptRunConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(3600));
        assert_eq!(config.max_plans, 1000);
        assert!(!config.continue_on_error);
    }
}
