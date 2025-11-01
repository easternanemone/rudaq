//! RunEngine for executing experiment plans.
//!
//! The RunEngine is the core executor that processes plan message streams and
//! translates them into module/instrument commands via the DaqManagerActor.

use super::plan::{LogLevel, Message, Plan};
use super::state::{Checkpoint, ExperimentState};
use crate::messages::DaqCommand;
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration as StdDuration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

/// RunEngine status information.
///
/// Provides real-time visibility into experiment execution progress.
#[derive(Debug, Clone)]
pub struct RunEngineStatus {
    /// Current experiment state
    pub state: ExperimentState,
    /// Unique run identifier (None if no run active)
    pub run_id: Option<String>,
    /// Run metadata (experiment name, parameters, etc.)
    pub metadata: HashMap<String, String>,
    /// Number of messages processed in current run
    pub message_count: usize,
    /// Last error message (if state is Error)
    pub last_error: Option<String>,
}

impl Default for RunEngineStatus {
    fn default() -> Self {
        Self {
            state: ExperimentState::Idle,
            run_id: None,
            metadata: HashMap::new(),
            message_count: 0,
            last_error: None,
        }
    }
}

/// Experiment executor that processes plan message streams.
///
/// The RunEngine is responsible for:
/// - Executing plans by consuming their message streams
/// - Translating plan messages into DaqCommand actions
/// - Managing experiment lifecycle (begin/end/pause/resume)
/// - Creating checkpoints for state recovery
/// - Error handling and reporting
///
/// # Architecture
///
/// ```text
/// Plan → Message Stream → RunEngine → DaqCommand → DaqManagerActor → Module/Instrument
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::experiment::{RunEngine, TimeSeriesPlan};
/// use std::time::Duration;
///
/// let mut engine = RunEngine::new(actor_tx);
///
/// let plan = TimeSeriesPlan::new(Duration::from_secs(60), Duration::from_secs(1));
/// engine.run(Box::new(plan)).await?;
///
/// // Check status
/// let status = engine.status();
/// println!("State: {}, Messages: {}", status.state, status.message_count);
/// ```
pub struct RunEngine {
    /// Channel to send commands to DaqManagerActor
    command_tx: mpsc::Sender<DaqCommand>,
    /// Current execution status
    status: RunEngineStatus,
    /// Auto-checkpointing enabled
    auto_checkpoint: bool,
    /// Auto-checkpoint interval (messages between checkpoints)
    checkpoint_interval: usize,
}

impl RunEngine {
    /// Create a new RunEngine.
    ///
    /// # Arguments
    ///
    /// * `command_tx` - Channel to send DaqCommand messages to the actor
    pub fn new(command_tx: mpsc::Sender<DaqCommand>) -> Self {
        Self {
            command_tx,
            status: RunEngineStatus::default(),
            auto_checkpoint: false,
            checkpoint_interval: 100,
        }
    }

    /// Enable automatic checkpointing.
    ///
    /// When enabled, the engine will automatically create checkpoints every
    /// `interval` messages during plan execution.
    ///
    /// # Arguments
    ///
    /// * `interval` - Number of messages between auto-checkpoints
    pub fn with_auto_checkpoint(mut self, interval: usize) -> Self {
        self.auto_checkpoint = true;
        self.checkpoint_interval = interval;
        self
    }

    /// Get the current RunEngine status.
    pub fn status(&self) -> RunEngineStatus {
        self.status.clone()
    }

    /// Execute a plan.
    ///
    /// This is the main entry point for running experiments. The engine will:
    /// 1. Validate the plan can execute
    /// 2. Consume the plan's message stream
    /// 3. Process each message (set parameters, trigger acquisition, etc.)
    /// 4. Create checkpoints (if auto-checkpoint enabled)
    /// 5. Handle errors and state transitions
    ///
    /// # Arguments
    ///
    /// * `plan` - Boxed plan to execute
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Engine is not in Idle state
    /// - Plan validation fails
    /// - Message processing fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let plan = TimeSeriesPlan::new(Duration::from_secs(60), Duration::from_secs(1));
    /// engine.run(Box::new(plan)).await?;
    /// ```
    pub async fn run(&mut self, mut plan: Box<dyn Plan>) -> Result<()> {
        // Check engine is idle
        if !self.status.state.can_begin() {
            bail!(
                "Cannot start run: engine in {} state (expected Idle/Complete/Error)",
                self.status.state
            );
        }

        // Validate plan
        plan.validate().context("Plan validation failed")?;

        // Get plan metadata
        let (plan_name, plan_description) = plan.metadata();
        info!("Starting experiment: {} - {}", plan_name, plan_description);

        // Initialize run state
        let run_id = Uuid::new_v4().to_string();
        self.status.state = ExperimentState::Idle; // Will transition to Running on BeginRun message
        self.status.run_id = Some(run_id.clone());
        self.status.message_count = 0;
        self.status.last_error = None;

        // Execute plan stream
        let mut stream = plan.execute();
        let mut run_metadata = HashMap::new();

        while let Some(message_result) = stream.next().await {
            // Handle stream errors
            let message = match message_result {
                Ok(msg) => msg,
                Err(e) => {
                    self.handle_error(format!("Plan stream error: {}", e))
                        .await?;
                    break;
                }
            };

            // Process message
            if let Err(e) = self.process_message(message, &mut run_metadata).await {
                self.handle_error(format!("Message processing error: {}", e))
                    .await?;
                break;
            }

            // Auto-checkpoint if enabled
            if self.auto_checkpoint
                && self.status.message_count % self.checkpoint_interval == 0
                && self.status.message_count > 0
            {
                if let Err(e) = self.create_checkpoint(None, &run_metadata).await {
                    warn!("Auto-checkpoint failed: {}", e);
                }
            }

            // Check for pause
            if self.status.state == ExperimentState::Paused {
                info!("Experiment paused at message {}", self.status.message_count);
                // Wait for resume (this is simplified - real impl would use channel)
                while self.status.state == ExperimentState::Paused {
                    sleep(StdDuration::from_millis(100)).await;
                }
                info!("Experiment resumed");
            }
        }

        // Finalize if completed successfully
        if self.status.state == ExperimentState::Running {
            self.status.state = ExperimentState::Complete;
            info!(
                "Experiment completed: {} messages processed",
                self.status.message_count
            );
        }

        Ok(())
    }

    /// Process a single plan message.
    async fn process_message(
        &mut self,
        message: Message,
        run_metadata: &mut HashMap<String, String>,
    ) -> Result<()> {
        match message {
            Message::BeginRun { metadata } => {
                *run_metadata = metadata.clone();
                self.status.metadata = metadata.clone();
                self.status.state = ExperimentState::Running;
                info!("Run started: {:?}", metadata);
            }

            Message::EndRun => {
                self.status.state = ExperimentState::Complete;
                info!("Run ended");
            }

            Message::Set {
                target,
                param,
                value,
            } => {
                info!("Set {}.{} = {}", target, param, value);
                // TODO: Send InstrumentCommand to set parameter
                // This requires extending InstrumentCommand enum or using module-specific commands
            }

            Message::Trigger { module_id } => {
                info!("Trigger module: {}", module_id);
                let (cmd, rx) = DaqCommand::start_module(module_id.clone());
                self.command_tx
                    .send(cmd)
                    .await
                    .context("Failed to send start_module command")?;
                rx.await
                    .context("Failed to receive start_module response")??;
            }

            Message::Read { module_id } => {
                info!("Read from module: {}", module_id);
                // TODO: Subscribe to module data stream and wait for next sample
                // This requires extending DaqCommand or using module-specific query
            }

            Message::Sleep { duration_secs } => {
                info!("Sleep for {} seconds", duration_secs);
                sleep(StdDuration::from_secs_f64(duration_secs)).await;
            }

            Message::Checkpoint { label } => {
                self.create_checkpoint(label, run_metadata).await?;
            }

            Message::Pause => {
                if self.status.state.can_pause() {
                    self.status.state = ExperimentState::Paused;
                    info!("Experiment paused");
                } else {
                    warn!("Cannot pause: state is {}", self.status.state);
                }
            }

            Message::Resume => {
                if self.status.state.can_resume() {
                    self.status.state = ExperimentState::Running;
                    info!("Experiment resumed");
                } else {
                    warn!("Cannot resume: state is {}", self.status.state);
                }
            }

            Message::Log { level, message } => match level {
                LogLevel::Info => info!("[Plan] {}", message),
                LogLevel::Warn => warn!("[Plan] {}", message),
                LogLevel::Error => error!("[Plan] {}", message),
            },
        }

        self.status.message_count += 1;
        Ok(())
    }

    /// Create a checkpoint of the current experiment state.
    async fn create_checkpoint(
        &self,
        label: Option<String>,
        run_metadata: &HashMap<String, String>,
    ) -> Result<()> {
        let run_id = self
            .status
            .run_id
            .as_ref()
            .ok_or_else(|| anyhow!("No active run"))?;

        let mut checkpoint = Checkpoint::new(
            run_id.clone(),
            self.status.state,
            run_metadata.clone(),
            self.status.message_count,
        );

        if let Some(label) = label {
            checkpoint = checkpoint.with_label(label.clone());
            info!("Creating checkpoint: {}", label);
        } else {
            info!(
                "Creating auto-checkpoint at message {}",
                self.status.message_count
            );
        }

        let path = checkpoint.default_path();
        checkpoint
            .save(&path)
            .with_context(|| format!("Failed to save checkpoint to {:?}", path))?;

        info!("Checkpoint saved: {:?}", path);
        Ok(())
    }

    /// Handle an error during execution.
    async fn handle_error(&mut self, error_message: String) -> Result<()> {
        error!("Experiment error: {}", error_message);
        self.status.state = ExperimentState::Error;
        self.status.last_error = Some(error_message.clone());

        // Create error checkpoint
        if let Some(run_id) = &self.status.run_id {
            let checkpoint = Checkpoint::new(
                run_id.clone(),
                ExperimentState::Error,
                self.status.metadata.clone(),
                self.status.message_count,
            )
            .with_error(error_message)
            .with_label("error_checkpoint".to_string());

            if let Err(e) = checkpoint.save(checkpoint.default_path()) {
                warn!("Failed to save error checkpoint: {}", e);
            }
        }

        Ok(())
    }

    /// Resume execution from a checkpoint.
    ///
    /// # Note
    ///
    /// This is a placeholder for resumption logic. Full implementation requires:
    /// - Deserializing plan state from checkpoint
    /// - Reconstructing plan object
    /// - Fast-forwarding to checkpoint message count
    /// - Resuming stream consumption
    ///
    /// This is complex and plan-specific, so it's left for future enhancement.
    pub async fn resume_from_checkpoint(&mut self, _checkpoint: Checkpoint) -> Result<()> {
        bail!("Checkpoint resumption not yet implemented");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::plan::PlanStream;
    use futures::stream;

    struct SimplePlan {
        steps: Vec<String>,
    }

    impl Plan for SimplePlan {
        fn execute(&mut self) -> PlanStream<'_> {
            let steps = self.steps.clone();
            Box::pin(stream::iter(steps.into_iter().map(|step| {
                Ok(Message::Log {
                    level: LogLevel::Info,
                    message: step,
                })
            })))
        }

        fn metadata(&self) -> (String, String) {
            (
                "SimplePlan".to_string(),
                format!("{} steps", self.steps.len()),
            )
        }
    }

    #[tokio::test]
    async fn test_run_engine_creation() {
        let (tx, _rx) = mpsc::channel(32);
        let engine = RunEngine::new(tx);
        assert_eq!(engine.status().state, ExperimentState::Idle);
        assert_eq!(engine.status().message_count, 0);
    }

    #[tokio::test]
    async fn test_run_engine_auto_checkpoint() {
        let (tx, _rx) = mpsc::channel(32);
        let engine = RunEngine::new(tx).with_auto_checkpoint(10);
        assert!(engine.auto_checkpoint);
        assert_eq!(engine.checkpoint_interval, 10);
    }
}
