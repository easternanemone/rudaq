//! Procedure Steps - Atomic Operations
//!
//! `ProcedureStep` represents atomic, reusable operations that compose into procedures.
//! Each step follows a validate → execute → verify pattern.

use super::ProcedureContext;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// =============================================================================
// ProcedureStep Trait
// =============================================================================

/// An atomic operation within a procedure.
///
/// Steps are the building blocks of procedures. They follow a pattern:
/// 1. **precondition()** - Check if step can execute
/// 2. **execute()** - Perform the operation
/// 3. **postcondition()** - Verify operation succeeded
/// 4. **record()** - Capture output data
///
/// # Example
///
/// ```rust,ignore
/// struct MoveToPositionStep {
///     role: String,
///     target_position: f64,
///     tolerance: f64,
/// }
///
/// #[async_trait]
/// impl ProcedureStep for MoveToPositionStep {
///     fn name(&self) -> &str { "move_to_position" }
///
///     async fn execute(&mut self, ctx: &ProcedureContext) -> Result<StepOutcome> {
///         let device = ctx.get_movable(&self.role).await
///             .ok_or_else(|| anyhow!("Device not found"))?;
///
///         device.move_abs(self.target_position).await?;
///
///         let final_pos = device.get_position().await?;
///         let error = (final_pos - self.target_position).abs();
///
///         if error <= self.tolerance {
///             Ok(StepOutcome::success().with_data("final_position", final_pos))
///         } else {
///             Ok(StepOutcome::failure(format!("Position error {:.4} exceeds tolerance", error)))
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait ProcedureStep: Send + Sync {
    /// Get the step name (for logging and progress)
    fn name(&self) -> &str;

    /// Get step description
    fn description(&self) -> &str {
        ""
    }

    /// Estimated duration (for progress calculation)
    fn estimated_duration(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Check if preconditions are met
    ///
    /// Called before execute() to verify:
    /// - Required devices are available
    /// - Hardware is in correct state
    /// - Dependencies are satisfied
    async fn precondition(&self, _ctx: &ProcedureContext) -> Result<bool> {
        Ok(true)
    }

    /// Execute the step operation
    ///
    /// The main implementation. Should:
    /// - Perform the atomic operation
    /// - Handle errors gracefully
    /// - Return outcome with any output data
    async fn execute(&mut self, ctx: &ProcedureContext) -> Result<StepOutcome>;

    /// Verify postconditions after execution
    ///
    /// Called after execute() to verify:
    /// - Operation completed correctly
    /// - Hardware is in expected state
    /// - No errors occurred
    async fn postcondition(&self, _ctx: &ProcedureContext) -> Result<bool> {
        Ok(true)
    }

    /// Record step output for results
    ///
    /// Override to capture additional data beyond what execute() returns.
    fn record(&self) -> Option<StepRecording> {
        None
    }

    /// Whether this step can be retried on failure
    fn is_retryable(&self) -> bool {
        false
    }

    /// Maximum retry attempts
    fn max_retries(&self) -> u32 {
        0
    }

    /// Delay between retries
    fn retry_delay(&self) -> Duration {
        Duration::from_millis(100)
    }
}

// =============================================================================
// Step Outcome
// =============================================================================

/// Outcome of executing a procedure step
#[derive(Debug, Clone)]
pub struct StepOutcome {
    /// Whether step succeeded
    pub success: bool,

    /// Output data from step
    pub data: std::collections::HashMap<String, StepValue>,

    /// Error message if failed
    pub error: Option<String>,

    /// Warnings generated
    pub warnings: Vec<String>,

    /// Whether to skip remaining steps
    pub abort_procedure: bool,
}

impl StepOutcome {
    /// Create a successful outcome
    pub fn success() -> Self {
        Self {
            success: true,
            data: std::collections::HashMap::new(),
            error: None,
            warnings: Vec::new(),
            abort_procedure: false,
        }
    }

    /// Create a failed outcome
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            data: std::collections::HashMap::new(),
            error: Some(error.into()),
            warnings: Vec::new(),
            abort_procedure: false,
        }
    }

    /// Create a failed outcome that aborts the procedure
    pub fn abort(error: impl Into<String>) -> Self {
        Self {
            success: false,
            data: std::collections::HashMap::new(),
            error: Some(error.into()),
            warnings: Vec::new(),
            abort_procedure: true,
        }
    }

    /// Add output data
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<StepValue>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    /// Add a warning
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }
}

// =============================================================================
// Step Value
// =============================================================================

/// Value types for step output data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    FloatArray(Vec<f64>),
}

impl From<bool> for StepValue {
    fn from(v: bool) -> Self {
        StepValue::Bool(v)
    }
}

impl From<i64> for StepValue {
    fn from(v: i64) -> Self {
        StepValue::Integer(v)
    }
}

impl From<i32> for StepValue {
    fn from(v: i32) -> Self {
        StepValue::Integer(v as i64)
    }
}

impl From<f64> for StepValue {
    fn from(v: f64) -> Self {
        StepValue::Float(v)
    }
}

impl From<f32> for StepValue {
    fn from(v: f32) -> Self {
        StepValue::Float(v as f64)
    }
}

impl From<String> for StepValue {
    fn from(v: String) -> Self {
        StepValue::String(v)
    }
}

impl From<&str> for StepValue {
    fn from(v: &str) -> Self {
        StepValue::String(v.to_string())
    }
}

impl From<Vec<f64>> for StepValue {
    fn from(v: Vec<f64>) -> Self {
        StepValue::FloatArray(v)
    }
}

// =============================================================================
// Step Progress
// =============================================================================

/// Progress update from a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepProgress {
    /// Step name
    pub step_name: String,

    /// Progress within step (0.0 - 1.0)
    pub progress: f64,

    /// Status message
    pub message: String,

    /// Current operation (for detailed progress)
    pub operation: Option<String>,
}

impl StepProgress {
    /// Create a new step progress
    pub fn new(step_name: impl Into<String>, progress: f64, message: impl Into<String>) -> Self {
        Self {
            step_name: step_name.into(),
            progress: progress.clamp(0.0, 1.0),
            message: message.into(),
            operation: None,
        }
    }

    /// Add operation detail
    pub fn with_operation(mut self, op: impl Into<String>) -> Self {
        self.operation = Some(op.into());
        self
    }
}

// =============================================================================
// Step Recording
// =============================================================================

/// Data recorded by a step for results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecording {
    /// Time-series data collected during step
    #[serde(default)]
    pub timeseries: Vec<TimeseriesPoint>,

    /// Scalar measurements
    #[serde(default)]
    pub measurements: std::collections::HashMap<String, f64>,

    /// Metadata
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

impl Default for StepRecording {
    fn default() -> Self {
        Self {
            timeseries: Vec::new(),
            measurements: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
        }
    }
}

impl StepRecording {
    /// Add a timeseries point
    pub fn add_point(&mut self, time_sec: f64, value: f64) {
        self.timeseries.push(TimeseriesPoint {
            time_sec,
            value,
            label: None,
        });
    }

    /// Add a labeled timeseries point
    pub fn add_labeled_point(&mut self, time_sec: f64, value: f64, label: impl Into<String>) {
        self.timeseries.push(TimeseriesPoint {
            time_sec,
            value,
            label: Some(label.into()),
        });
    }

    /// Add a measurement
    pub fn add_measurement(&mut self, name: impl Into<String>, value: f64) {
        self.measurements.insert(name.into(), value);
    }

    /// Add metadata
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
}

/// A point in a time series
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeseriesPoint {
    /// Time in seconds (relative to step start)
    pub time_sec: f64,
    /// Measured value
    pub value: f64,
    /// Optional label for this point
    pub label: Option<String>,
}

// =============================================================================
// Common Step Implementations
// =============================================================================

/// A step that waits for a specified duration
pub struct WaitStep {
    name: String,
    duration: Duration,
}

impl WaitStep {
    pub fn new(name: impl Into<String>, duration: Duration) -> Self {
        Self {
            name: name.into(),
            duration,
        }
    }
}

#[async_trait]
impl ProcedureStep for WaitStep {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Wait for a specified duration"
    }

    fn estimated_duration(&self) -> Duration {
        self.duration
    }

    async fn execute(&mut self, ctx: &ProcedureContext) -> Result<StepOutcome> {
        let start = std::time::Instant::now();
        let check_interval = Duration::from_millis(100);

        while start.elapsed() < self.duration {
            if ctx.is_cancelled() {
                return Ok(StepOutcome::abort("Cancelled during wait"));
            }
            tokio::time::sleep(check_interval).await;
        }

        Ok(StepOutcome::success().with_data("waited_ms", self.duration.as_millis() as i64))
    }
}

/// A step that moves a device to a position
pub struct MoveToPositionStep {
    name: String,
    role: String,
    target: f64,
    tolerance: f64,
}

impl MoveToPositionStep {
    pub fn new(
        name: impl Into<String>,
        role: impl Into<String>,
        target: f64,
        tolerance: f64,
    ) -> Self {
        Self {
            name: name.into(),
            role: role.into(),
            target,
            tolerance,
        }
    }
}

#[async_trait]
impl ProcedureStep for MoveToPositionStep {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Move device to target position"
    }

    fn estimated_duration(&self) -> Duration {
        Duration::from_secs(5) // Conservative estimate
    }

    fn is_retryable(&self) -> bool {
        true
    }

    fn max_retries(&self) -> u32 {
        2
    }

    async fn execute(&mut self, ctx: &ProcedureContext) -> Result<StepOutcome> {
        let device = ctx
            .get_movable(&self.role)
            .await
            .ok_or_else(|| anyhow::anyhow!("Device '{}' not found", self.role))?;

        // Move to target
        device.move_abs(self.target).await?;

        // Verify position
        let final_pos = device.position().await?;
        let error = (final_pos - self.target).abs();

        if error <= self.tolerance {
            Ok(StepOutcome::success()
                .with_data("target", self.target)
                .with_data("final_position", final_pos)
                .with_data("error", error))
        } else {
            Ok(StepOutcome::failure(format!(
                "Position error {:.4} exceeds tolerance {:.4}",
                error, self.tolerance
            ))
            .with_data("target", self.target)
            .with_data("final_position", final_pos)
            .with_data("error", error))
        }
    }
}

/// A step that reads a value from a readable device
pub struct ReadValueStep {
    name: String,
    role: String,
    samples: u32,
    sample_interval: Duration,
}

impl ReadValueStep {
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: role.into(),
            samples: 1,
            sample_interval: Duration::from_millis(100),
        }
    }

    /// Configure to take multiple samples and average
    pub fn with_averaging(mut self, samples: u32, interval: Duration) -> Self {
        self.samples = samples;
        self.sample_interval = interval;
        self
    }
}

#[async_trait]
impl ProcedureStep for ReadValueStep {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Read value from device"
    }

    fn estimated_duration(&self) -> Duration {
        self.sample_interval * self.samples
    }

    async fn execute(&mut self, ctx: &ProcedureContext) -> Result<StepOutcome> {
        let device = ctx
            .get_readable(&self.role)
            .await
            .ok_or_else(|| anyhow::anyhow!("Device '{}' not found", self.role))?;

        let mut values = Vec::with_capacity(self.samples as usize);

        for _ in 0..self.samples {
            if ctx.is_cancelled() {
                return Ok(StepOutcome::abort("Cancelled during read"));
            }

            let value = device.read().await?;
            values.push(value);

            if self.samples > 1 {
                tokio::time::sleep(self.sample_interval).await;
            }
        }

        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let std_dev = if values.len() > 1 {
            let variance =
                values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
            variance.sqrt()
        } else {
            0.0
        };

        Ok(StepOutcome::success()
            .with_data("value", mean)
            .with_data("std_dev", std_dev)
            .with_data("samples", self.samples as i64)
            .with_data("all_values", values))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_outcome_success() {
        let outcome = StepOutcome::success()
            .with_data("position", 45.0f64)
            .with_warning("Slight vibration");

        assert!(outcome.success);
        assert!(!outcome.abort_procedure);
        assert!(!outcome.warnings.is_empty());
    }

    #[test]
    fn test_step_outcome_failure() {
        let outcome = StepOutcome::failure("Device timeout");

        assert!(!outcome.success);
        assert!(!outcome.abort_procedure);
        assert_eq!(outcome.error, Some("Device timeout".to_string()));
    }

    #[test]
    fn test_step_outcome_abort() {
        let outcome = StepOutcome::abort("Critical error");

        assert!(!outcome.success);
        assert!(outcome.abort_procedure);
    }

    #[test]
    fn test_step_progress() {
        let progress = StepProgress::new("move_to_position", 0.75, "Moving...")
            .with_operation("Waiting for settle");

        assert_eq!(progress.step_name, "move_to_position");
        assert!((progress.progress - 0.75).abs() < 0.001);
        assert!(progress.operation.is_some());
    }

    #[test]
    fn test_step_recording() {
        let mut recording = StepRecording::default();
        recording.add_point(0.0, 1.0);
        recording.add_point(0.1, 1.1);
        recording.add_measurement("peak", 1.5);
        recording.add_metadata("units", "mm");

        assert_eq!(recording.timeseries.len(), 2);
        assert_eq!(recording.measurements.get("peak"), Some(&1.5));
        assert_eq!(recording.metadata.get("units"), Some(&"mm".to_string()));
    }
}
