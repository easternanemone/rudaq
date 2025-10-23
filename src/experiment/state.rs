//! Experiment state management and checkpointing.
//!
//! This module provides state tracking for the RunEngine, including lifecycle
//! management and checkpoint serialization for pause/resume functionality.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Experiment execution state.
///
/// Tracks the current lifecycle state of the RunEngine. State transitions are
/// controlled by plan messages and RunEngine commands.
///
/// # State Machine
///
/// ```text
/// Idle ──BeginRun──> Running ──EndRun──> Complete
///                      │   ▲                │
///                      │   │                │
///                  Pause│   │Resume          │
///                      │   │                │
///                      ▼   │                │
///                    Paused                 │
///                      │                    │
///                      │                    │
///                  Error─────────────────────
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperimentState {
    /// No experiment running
    Idle,
    /// Experiment actively executing
    Running,
    /// Experiment paused (can be resumed)
    Paused,
    /// Experiment completed successfully
    Complete,
    /// Experiment encountered an error
    Error,
}

impl std::fmt::Display for ExperimentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExperimentState::Idle => write!(f, "Idle"),
            ExperimentState::Running => write!(f, "Running"),
            ExperimentState::Paused => write!(f, "Paused"),
            ExperimentState::Complete => write!(f, "Complete"),
            ExperimentState::Error => write!(f, "Error"),
        }
    }
}

impl ExperimentState {
    /// Check if the state allows starting a new run.
    pub fn can_begin(&self) -> bool {
        matches!(self, ExperimentState::Idle | ExperimentState::Complete | ExperimentState::Error)
    }

    /// Check if the state allows pausing.
    pub fn can_pause(&self) -> bool {
        matches!(self, ExperimentState::Running)
    }

    /// Check if the state allows resuming.
    pub fn can_resume(&self) -> bool {
        matches!(self, ExperimentState::Paused)
    }
}

/// Serializable checkpoint for experiment state.
///
/// Checkpoints capture the full state of an experiment at a specific point in time,
/// enabling pause/resume and error recovery. They include:
///
/// - Experiment metadata (run ID, start time, parameters)
/// - Plan state (serialized plan object)
/// - Execution progress (message count, current step)
/// - Error information (if checkpoint created after error)
///
/// # Storage
///
/// Checkpoints are serialized to JSON and saved to disk. The RunEngine can load
/// checkpoints to resume execution from where it left off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique run identifier
    pub run_id: String,
    /// Checkpoint creation timestamp
    pub timestamp: DateTime<Utc>,
    /// Experiment state when checkpoint was created
    pub state: ExperimentState,
    /// Run metadata (experiment name, parameters, etc.)
    pub metadata: HashMap<String, String>,
    /// Number of messages processed before checkpoint
    pub message_count: usize,
    /// Optional checkpoint label for identification
    pub label: Option<String>,
    /// Optional error message if checkpoint created after failure
    pub error: Option<String>,
    /// Serialized plan state (if plan is Serialize)
    ///
    /// This is stored as a JSON value to allow dynamic deserialization
    /// based on plan type.
    pub plan_state: Option<serde_json::Value>,
}

impl Checkpoint {
    /// Create a new checkpoint.
    pub fn new(
        run_id: String,
        state: ExperimentState,
        metadata: HashMap<String, String>,
        message_count: usize,
    ) -> Self {
        Self {
            run_id,
            timestamp: Utc::now(),
            state,
            metadata,
            message_count,
            label: None,
            error: None,
            plan_state: None,
        }
    }

    /// Set the checkpoint label.
    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the error message.
    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }

    /// Set the plan state.
    pub fn with_plan_state(mut self, plan_state: serde_json::Value) -> Self {
        self.plan_state = Some(plan_state);
        self
    }

    /// Save checkpoint to a JSON file.
    ///
    /// # Arguments
    ///
    /// * `path` - File path where checkpoint will be saved
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Parent directory doesn't exist
    /// - Serialization fails
    /// - File write fails
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize checkpoint")?;

        // Ensure parent directory exists
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)
                .context("Failed to create checkpoint directory")?;
        }

        fs::write(&path, json)
            .with_context(|| format!("Failed to write checkpoint to {:?}", path.as_ref()))?;

        Ok(())
    }

    /// Load checkpoint from a JSON file.
    ///
    /// # Arguments
    ///
    /// * `path` - File path to load checkpoint from
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - File doesn't exist
    /// - File read fails
    /// - Deserialization fails
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read checkpoint from {:?}", path.as_ref()))?;

        let checkpoint = serde_json::from_str(&json)
            .context("Failed to deserialize checkpoint")?;

        Ok(checkpoint)
    }

    /// Get the default checkpoint directory.
    ///
    /// Returns `./checkpoints/<run_id>/` relative to current directory.
    pub fn default_dir(run_id: &str) -> PathBuf {
        PathBuf::from("checkpoints").join(run_id)
    }

    /// Get the default checkpoint filename based on timestamp.
    ///
    /// Format: `checkpoint_<timestamp>.json`
    pub fn default_filename(&self) -> String {
        format!(
            "checkpoint_{}.json",
            self.timestamp.format("%Y%m%d_%H%M%S")
        )
    }

    /// Get the default full path for this checkpoint.
    pub fn default_path(&self) -> PathBuf {
        Self::default_dir(&self.run_id).join(self.default_filename())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_transitions() {
        assert!(ExperimentState::Idle.can_begin());
        assert!(!ExperimentState::Running.can_begin());

        assert!(ExperimentState::Running.can_pause());
        assert!(!ExperimentState::Idle.can_pause());

        assert!(ExperimentState::Paused.can_resume());
        assert!(!ExperimentState::Running.can_resume());
    }

    #[test]
    fn test_checkpoint_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");

        let mut metadata = HashMap::new();
        metadata.insert("experiment".to_string(), "test".to_string());

        let checkpoint = Checkpoint::new(
            "run_001".to_string(),
            ExperimentState::Paused,
            metadata.clone(),
            42,
        ).with_label("manual_pause".to_string());

        // Save
        checkpoint.save(&path).unwrap();

        // Load
        let loaded = Checkpoint::load(&path).unwrap();

        assert_eq!(loaded.run_id, "run_001");
        assert_eq!(loaded.state, ExperimentState::Paused);
        assert_eq!(loaded.message_count, 42);
        assert_eq!(loaded.label, Some("manual_pause".to_string()));
        assert_eq!(loaded.metadata, metadata);
    }

    #[test]
    fn test_checkpoint_with_plan_state() {
        let mut checkpoint = Checkpoint::new(
            "run_002".to_string(),
            ExperimentState::Running,
            HashMap::new(),
            10,
        );

        let plan_state = serde_json::json!({
            "type": "TimeSeries",
            "current_step": 5,
            "total_steps": 100,
        });

        checkpoint = checkpoint.with_plan_state(plan_state.clone());
        assert_eq!(checkpoint.plan_state, Some(plan_state));
    }
}
