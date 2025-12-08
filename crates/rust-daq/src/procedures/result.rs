//! Procedure Results and Quality Metrics
//!
//! Structured output types for procedure execution results,
//! including calibration data, quality metrics, and summaries.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// =============================================================================
// ProcedureResult
// =============================================================================

/// Result of a procedure execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureResult {
    /// Procedure type that produced this result
    pub procedure_type: String,

    /// Unique execution ID
    pub execution_id: String,

    /// Whether execution completed successfully
    pub success: bool,

    /// Overall quality metrics
    pub quality: QualityMetrics,

    /// Execution timing
    pub timing: ExecutionTiming,

    /// Output data (procedure-specific)
    #[serde(default)]
    pub data: HashMap<String, ResultValue>,

    /// Warnings generated during execution
    #[serde(default)]
    pub warnings: Vec<String>,

    /// Error message if failed
    #[serde(default)]
    pub error: Option<String>,

    /// Step-by-step breakdown
    #[serde(default)]
    pub steps: Vec<StepResult>,
}

impl ProcedureResult {
    /// Create a new successful result
    pub fn success(procedure_type: impl Into<String>, execution_id: impl Into<String>) -> Self {
        Self {
            procedure_type: procedure_type.into(),
            execution_id: execution_id.into(),
            success: true,
            quality: QualityMetrics::default(),
            timing: ExecutionTiming::default(),
            data: HashMap::new(),
            warnings: Vec::new(),
            error: None,
            steps: Vec::new(),
        }
    }

    /// Create a failed result
    pub fn failure(
        procedure_type: impl Into<String>,
        execution_id: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            procedure_type: procedure_type.into(),
            execution_id: execution_id.into(),
            success: false,
            quality: QualityMetrics::default(),
            timing: ExecutionTiming::default(),
            data: HashMap::new(),
            warnings: Vec::new(),
            error: Some(error.into()),
            steps: Vec::new(),
        }
    }

    /// Add a data value to the result
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<ResultValue>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    /// Add a warning
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Set quality metrics
    pub fn with_quality(mut self, quality: QualityMetrics) -> Self {
        self.quality = quality;
        self
    }

    /// Set timing information
    pub fn with_timing(mut self, timing: ExecutionTiming) -> Self {
        self.timing = timing;
        self
    }

    /// Add a step result
    pub fn with_step(mut self, step: StepResult) -> Self {
        self.steps.push(step);
        self
    }

    /// Generate a human-readable summary
    pub fn summary(&self) -> String {
        let status = if self.success { "SUCCESS" } else { "FAILED" };
        let quality_str = format!(
            "Quality: {:.1}% (Pass: {}, Warn: {}, Fail: {})",
            self.quality.overall_score * 100.0,
            self.quality.checks_passed,
            self.quality.checks_warned,
            self.quality.checks_failed
        );

        if self.success {
            format!(
                "{} - {} - Duration: {:.1}s - {}",
                self.procedure_type,
                status,
                self.timing.total_duration.as_secs_f64(),
                quality_str
            )
        } else {
            format!(
                "{} - {} - Error: {}",
                self.procedure_type,
                status,
                self.error.as_deref().unwrap_or("Unknown")
            )
        }
    }
}

// =============================================================================
// Quality Metrics
// =============================================================================

/// Quality metrics for procedure results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Overall quality score (0.0 - 1.0)
    pub overall_score: f64,

    /// Number of quality checks that passed
    pub checks_passed: u32,

    /// Number of quality checks with warnings
    pub checks_warned: u32,

    /// Number of quality checks that failed
    pub checks_failed: u32,

    /// Individual quality check results
    #[serde(default)]
    pub checks: Vec<QualityCheck>,
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            overall_score: 1.0,
            checks_passed: 0,
            checks_warned: 0,
            checks_failed: 0,
            checks: Vec::new(),
        }
    }
}

impl QualityMetrics {
    /// Add a passed check
    pub fn add_pass(&mut self, name: impl Into<String>, message: impl Into<String>) {
        self.checks_passed += 1;
        self.checks.push(QualityCheck {
            name: name.into(),
            status: CheckStatus::Pass,
            message: message.into(),
            value: None,
            threshold: None,
        });
        self.recalculate_score();
    }

    /// Add a warning check
    pub fn add_warning(&mut self, name: impl Into<String>, message: impl Into<String>) {
        self.checks_warned += 1;
        self.checks.push(QualityCheck {
            name: name.into(),
            status: CheckStatus::Warning,
            message: message.into(),
            value: None,
            threshold: None,
        });
        self.recalculate_score();
    }

    /// Add a failed check
    pub fn add_fail(&mut self, name: impl Into<String>, message: impl Into<String>) {
        self.checks_failed += 1;
        self.checks.push(QualityCheck {
            name: name.into(),
            status: CheckStatus::Fail,
            message: message.into(),
            value: None,
            threshold: None,
        });
        self.recalculate_score();
    }

    /// Add a check with measured value
    pub fn add_check_with_value(
        &mut self,
        name: impl Into<String>,
        value: f64,
        threshold: f64,
        passed: bool,
    ) {
        let status = if passed {
            self.checks_passed += 1;
            CheckStatus::Pass
        } else {
            self.checks_failed += 1;
            CheckStatus::Fail
        };

        self.checks.push(QualityCheck {
            name: name.into(),
            status,
            message: format!("Value: {:.4}, Threshold: {:.4}", value, threshold),
            value: Some(value),
            threshold: Some(threshold),
        });
        self.recalculate_score();
    }

    fn recalculate_score(&mut self) {
        let total = self.checks_passed + self.checks_warned + self.checks_failed;
        if total == 0 {
            self.overall_score = 1.0;
        } else {
            // Pass = 1.0, Warning = 0.5, Fail = 0.0
            let score =
                (self.checks_passed as f64 + self.checks_warned as f64 * 0.5) / total as f64;
            self.overall_score = score;
        }
    }
}

/// Individual quality check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityCheck {
    /// Check name
    pub name: String,
    /// Check status
    pub status: CheckStatus,
    /// Human-readable message
    pub message: String,
    /// Measured value (if applicable)
    pub value: Option<f64>,
    /// Threshold value (if applicable)
    pub threshold: Option<f64>,
}

/// Status of a quality check
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    /// Check passed
    Pass,
    /// Check passed with warning
    Warning,
    /// Check failed
    Fail,
}

// =============================================================================
// Execution Timing
// =============================================================================

/// Timing information for procedure execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTiming {
    /// Total execution duration
    #[serde(with = "duration_serde")]
    pub total_duration: Duration,

    /// Time spent in preparation phase
    #[serde(with = "duration_serde")]
    pub preparation_duration: Duration,

    /// Time spent executing steps
    #[serde(with = "duration_serde")]
    pub execution_duration: Duration,

    /// Time spent in finalization
    #[serde(with = "duration_serde")]
    pub finalization_duration: Duration,

    /// Start timestamp (Unix epoch seconds)
    pub start_time: f64,

    /// End timestamp (Unix epoch seconds)
    pub end_time: f64,
}

impl Default for ExecutionTiming {
    fn default() -> Self {
        Self {
            total_duration: Duration::ZERO,
            preparation_duration: Duration::ZERO,
            execution_duration: Duration::ZERO,
            finalization_duration: Duration::ZERO,
            start_time: 0.0,
            end_time: 0.0,
        }
    }
}

/// Serde helper for Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs_f64().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = f64::deserialize(deserializer)?;
        Ok(Duration::from_secs_f64(secs))
    }
}

// =============================================================================
// Step Result
// =============================================================================

/// Result of an individual procedure step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step name
    pub name: String,

    /// Step index (0-based)
    pub index: usize,

    /// Whether step succeeded
    pub success: bool,

    /// Step duration
    #[serde(with = "duration_serde")]
    pub duration: Duration,

    /// Output data from step
    #[serde(default)]
    pub data: HashMap<String, ResultValue>,

    /// Error message if step failed
    #[serde(default)]
    pub error: Option<String>,
}

impl StepResult {
    /// Create a successful step result
    pub fn success(name: impl Into<String>, index: usize, duration: Duration) -> Self {
        Self {
            name: name.into(),
            index,
            success: true,
            duration,
            data: HashMap::new(),
            error: None,
        }
    }

    /// Create a failed step result
    pub fn failure(
        name: impl Into<String>,
        index: usize,
        duration: Duration,
        error: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            index,
            success: false,
            duration,
            data: HashMap::new(),
            error: Some(error.into()),
        }
    }

    /// Add output data
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<ResultValue>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

// =============================================================================
// Result Value
// =============================================================================

/// A value that can be stored in procedure results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResultValue {
    /// Boolean value
    Bool(bool),
    /// Integer value
    Integer(i64),
    /// Float value
    Float(f64),
    /// String value
    String(String),
    /// Array of floats (common for measurement data)
    FloatArray(Vec<f64>),
    /// Generic JSON value
    Json(serde_json::Value),
}

impl From<bool> for ResultValue {
    fn from(v: bool) -> Self {
        ResultValue::Bool(v)
    }
}

impl From<i64> for ResultValue {
    fn from(v: i64) -> Self {
        ResultValue::Integer(v)
    }
}

impl From<i32> for ResultValue {
    fn from(v: i32) -> Self {
        ResultValue::Integer(v as i64)
    }
}

impl From<f64> for ResultValue {
    fn from(v: f64) -> Self {
        ResultValue::Float(v)
    }
}

impl From<f32> for ResultValue {
    fn from(v: f32) -> Self {
        ResultValue::Float(v as f64)
    }
}

impl From<String> for ResultValue {
    fn from(v: String) -> Self {
        ResultValue::String(v)
    }
}

impl From<&str> for ResultValue {
    fn from(v: &str) -> Self {
        ResultValue::String(v.to_string())
    }
}

impl From<Vec<f64>> for ResultValue {
    fn from(v: Vec<f64>) -> Self {
        ResultValue::FloatArray(v)
    }
}

// =============================================================================
// Calibration Result (Specialized)
// =============================================================================

/// Specialized result type for calibration procedures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationResult {
    /// Base result
    #[serde(flatten)]
    pub base: ProcedureResult,

    /// Calibration-specific data
    pub calibration: CalibrationData,
}

/// Calibration-specific output data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationData {
    /// Device that was calibrated
    pub device_id: String,

    /// Type of calibration performed
    pub calibration_type: String,

    /// Before/after comparison values
    pub before: Option<f64>,
    pub after: Option<f64>,

    /// Improvement percentage (if applicable)
    pub improvement: Option<f64>,

    /// Fitted parameters (if applicable)
    #[serde(default)]
    pub fitted_params: HashMap<String, f64>,

    /// Reference position or value
    pub reference: Option<f64>,

    /// Tolerance achieved
    pub tolerance_achieved: Option<f64>,
}

impl Default for CalibrationData {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            calibration_type: String::new(),
            before: None,
            after: None,
            improvement: None,
            fitted_params: HashMap::new(),
            reference: None,
            tolerance_achieved: None,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_procedure_result_success() {
        let result = ProcedureResult::success("test_procedure", "exec-001")
            .with_data("position", 45.0f64)
            .with_warning("Minor drift detected");

        assert!(result.success);
        assert_eq!(result.procedure_type, "test_procedure");
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_procedure_result_failure() {
        let result = ProcedureResult::failure("test_procedure", "exec-002", "Device timeout");

        assert!(!result.success);
        assert_eq!(result.error, Some("Device timeout".to_string()));
    }

    #[test]
    fn test_quality_metrics() {
        let mut quality = QualityMetrics::default();

        quality.add_pass("position_accuracy", "Within tolerance");
        quality.add_warning("settling_time", "Slightly slow");
        quality.add_fail("repeatability", "Out of spec");

        assert_eq!(quality.checks_passed, 1);
        assert_eq!(quality.checks_warned, 1);
        assert_eq!(quality.checks_failed, 1);
        assert!((quality.overall_score - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_step_result() {
        let step = StepResult::success("move_to_reference", 0, Duration::from_millis(500))
            .with_data("final_position", 0.0f64);

        assert!(step.success);
        assert_eq!(step.name, "move_to_reference");
    }
}
