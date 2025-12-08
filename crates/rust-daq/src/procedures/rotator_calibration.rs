//! Rotator Calibration Procedure
//!
//! Calibrates rotary motion stages by:
//! 1. Home position detection
//! 2. Backlash measurement (approach from both directions)
//! 3. Repeatability testing (N cycles)
//!
//! # Configuration
//!
//! ```toml
//! [procedure]
//! type = "rotator_calibration"
//! name = "Rotator Home Position Calibration"
//!
//! [params]
//! num_cycles = 3
//! angle_tolerance_deg = 0.1
//! backlash_test_angle = 5.0
//! reference_position = 0.0
//!
//! [roles.rotator]
//! device_id = "elliptec_addr2"
//! ```

use super::{
    CalibrationData, CalibrationResult, ParameterConstraints, ParameterDef, Procedure,
    ProcedureConfig, ProcedureContext, ProcedureProgress, ProcedureResult, ProcedureState,
    ProcedureTypeInfo, QualityMetrics, RoleRequirement, StepResult,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

// =============================================================================
// Configuration
// =============================================================================

/// Typed configuration for rotator calibration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotatorCalibrationConfig {
    /// Number of repeatability test cycles
    #[serde(default = "default_num_cycles")]
    pub num_cycles: u32,

    /// Acceptable position error in degrees
    #[serde(default = "default_angle_tolerance")]
    pub angle_tolerance_deg: f64,

    /// Angle to use for backlash testing
    #[serde(default = "default_backlash_angle")]
    pub backlash_test_angle: f64,

    /// Reference position for calibration (typically home/0)
    #[serde(default = "default_reference_position")]
    pub reference_position: f64,

    /// Settle time after moves (seconds)
    #[serde(default = "default_settle_time")]
    pub settle_time_sec: f64,

    /// Maximum allowed backlash before warning (degrees)
    #[serde(default = "default_max_backlash")]
    pub max_backlash_deg: f64,

    /// Maximum allowed repeatability error (degrees)
    #[serde(default = "default_max_repeatability")]
    pub max_repeatability_deg: f64,
}

fn default_num_cycles() -> u32 {
    3
}
fn default_angle_tolerance() -> f64 {
    0.1
}
fn default_backlash_angle() -> f64 {
    5.0
}
fn default_reference_position() -> f64 {
    0.0
}
fn default_settle_time() -> f64 {
    0.5
}
fn default_max_backlash() -> f64 {
    1.0
}
fn default_max_repeatability() -> f64 {
    0.2
}

impl Default for RotatorCalibrationConfig {
    fn default() -> Self {
        Self {
            num_cycles: default_num_cycles(),
            angle_tolerance_deg: default_angle_tolerance(),
            backlash_test_angle: default_backlash_angle(),
            reference_position: default_reference_position(),
            settle_time_sec: default_settle_time(),
            max_backlash_deg: default_max_backlash(),
            max_repeatability_deg: default_max_repeatability(),
        }
    }
}

impl RotatorCalibrationConfig {
    /// Load from ProcedureConfig
    pub fn from_procedure_config(config: &ProcedureConfig) -> Self {
        Self {
            num_cycles: config
                .get_i64("num_cycles")
                .map(|v| v as u32)
                .unwrap_or_else(default_num_cycles),
            angle_tolerance_deg: config
                .get_f64("angle_tolerance_deg")
                .unwrap_or_else(default_angle_tolerance),
            backlash_test_angle: config
                .get_f64("backlash_test_angle")
                .unwrap_or_else(default_backlash_angle),
            reference_position: config
                .get_f64("reference_position")
                .unwrap_or_else(default_reference_position),
            settle_time_sec: config
                .get_f64("settle_time_sec")
                .unwrap_or_else(default_settle_time),
            max_backlash_deg: config
                .get_f64("max_backlash_deg")
                .unwrap_or_else(default_max_backlash),
            max_repeatability_deg: config
                .get_f64("max_repeatability_deg")
                .unwrap_or_else(default_max_repeatability),
        }
    }
}

// =============================================================================
// Calibration Results
// =============================================================================

/// Results from the rotator calibration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotatorCalibrationResults {
    /// Measured backlash in degrees
    pub backlash_deg: f64,

    /// Repeatability measurements from each cycle
    pub repeatability_measurements: Vec<f64>,

    /// Mean repeatability error
    pub repeatability_mean: f64,

    /// Standard deviation of repeatability
    pub repeatability_std: f64,

    /// Home offset detected (if any)
    pub home_offset: f64,

    /// All position readings during test
    pub all_positions: Vec<f64>,
}

// =============================================================================
// Procedure Implementation
// =============================================================================

/// Rotator calibration procedure
pub struct RotatorCalibration {
    /// Procedure configuration
    config: ProcedureConfig,

    /// Typed configuration
    params: RotatorCalibrationConfig,

    /// Current state
    state: ProcedureState,

    /// Current progress
    progress: ProcedureProgress,

    /// Calibration results (populated during execution)
    results: Option<RotatorCalibrationResults>,
}

impl Default for RotatorCalibration {
    fn default() -> Self {
        Self::new()
    }
}

impl RotatorCalibration {
    /// Create a new rotator calibration procedure
    pub fn new() -> Self {
        Self {
            config: ProcedureConfig::new("rotator_calibration"),
            params: RotatorCalibrationConfig::default(),
            state: ProcedureState::Idle,
            progress: ProcedureProgress::default(),
            results: None,
        }
    }

    /// Helper: move to position and wait for settle
    async fn move_and_settle(&self, ctx: &ProcedureContext, target: f64) -> Result<f64> {
        let device = ctx
            .get_movable("rotator")
            .await
            .ok_or_else(|| anyhow!("Rotator device not found"))?;

        device.move_abs(target).await?;
        tokio::time::sleep(Duration::from_secs_f64(self.params.settle_time_sec)).await;

        device.position().await
    }

    /// Step 1: Move to reference position
    async fn step_move_to_reference(&mut self, ctx: &ProcedureContext) -> Result<StepResult> {
        let start = Instant::now();

        self.update_progress(
            ProcedureState::Running,
            0,
            4,
            "move_to_reference",
            "Moving to reference position",
        );

        let final_pos = self
            .move_and_settle(ctx, self.params.reference_position)
            .await?;
        let error = (final_pos - self.params.reference_position).abs();

        let success = error <= self.params.angle_tolerance_deg;

        Ok(if success {
            StepResult::success("move_to_reference", 0, start.elapsed())
                .with_data("target", self.params.reference_position)
                .with_data("final_position", final_pos)
                .with_data("error", error)
        } else {
            StepResult::failure(
                "move_to_reference",
                0,
                start.elapsed(),
                format!(
                    "Position error {:.4} deg exceeds tolerance {:.4} deg",
                    error, self.params.angle_tolerance_deg
                ),
            )
            .with_data("target", self.params.reference_position)
            .with_data("final_position", final_pos)
            .with_data("error", error)
        })
    }

    /// Step 2: Measure backlash
    async fn step_measure_backlash(&mut self, ctx: &ProcedureContext) -> Result<StepResult> {
        let start = Instant::now();

        self.update_progress(
            ProcedureState::Running,
            1,
            4,
            "measure_backlash",
            "Measuring backlash",
        );

        let ref_pos = self.params.reference_position;
        let test_angle = self.params.backlash_test_angle;

        // Move forward past reference, then back to reference
        self.move_and_settle(ctx, ref_pos + test_angle).await?;
        let pos_from_positive = self.move_and_settle(ctx, ref_pos).await?;

        // Move backward past reference, then back to reference
        self.move_and_settle(ctx, ref_pos - test_angle).await?;
        let pos_from_negative = self.move_and_settle(ctx, ref_pos).await?;

        // Backlash is the difference
        let backlash = (pos_from_positive - pos_from_negative).abs();

        // Store for final results
        if let Some(ref mut results) = self.results {
            results.backlash_deg = backlash;
            results.all_positions.push(pos_from_positive);
            results.all_positions.push(pos_from_negative);
        }

        let success = backlash <= self.params.max_backlash_deg;

        Ok(if success {
            StepResult::success("measure_backlash", 1, start.elapsed())
                .with_data("backlash_deg", backlash)
                .with_data("pos_from_positive", pos_from_positive)
                .with_data("pos_from_negative", pos_from_negative)
        } else {
            // Still return success but log warning - backlash is informational
            StepResult::success("measure_backlash", 1, start.elapsed())
                .with_data("backlash_deg", backlash)
                .with_data(
                    "warning",
                    format!(
                        "Backlash {:.4} deg exceeds max {:.4} deg",
                        backlash, self.params.max_backlash_deg
                    ),
                )
        })
    }

    /// Step 3: Repeatability test
    async fn step_repeatability_test(&mut self, ctx: &ProcedureContext) -> Result<StepResult> {
        let start = Instant::now();

        self.update_progress(
            ProcedureState::Running,
            2,
            4,
            "repeatability_test",
            "Running repeatability test",
        );

        let ref_pos = self.params.reference_position;
        let test_angle = self.params.backlash_test_angle;
        let num_cycles = self.params.num_cycles;

        let mut measurements = Vec::with_capacity(num_cycles as usize);

        for cycle in 0..num_cycles {
            // Update progress
            let cycle_progress = cycle as f64 / num_cycles as f64;
            self.update_progress(
                ProcedureState::Running,
                2,
                4,
                "repeatability_test",
                &format!("Cycle {}/{}", cycle + 1, num_cycles),
            );
            self.progress.step_progress = cycle_progress;

            // Check cancellation
            if ctx.is_cancelled() {
                return Ok(StepResult::failure(
                    "repeatability_test",
                    2,
                    start.elapsed(),
                    "Cancelled by user",
                ));
            }

            // Move away and back
            self.move_and_settle(ctx, ref_pos + test_angle).await?;
            let position = self.move_and_settle(ctx, ref_pos).await?;

            let error = (position - ref_pos).abs();
            measurements.push(error);

            if let Some(ref mut results) = self.results {
                results.all_positions.push(position);
            }
        }

        // Calculate statistics
        let mean = measurements.iter().sum::<f64>() / measurements.len() as f64;
        let variance = if measurements.len() > 1 {
            measurements.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (measurements.len() - 1) as f64
        } else {
            0.0
        };
        let std_dev = variance.sqrt();

        // Store results
        if let Some(ref mut results) = self.results {
            results.repeatability_measurements = measurements.clone();
            results.repeatability_mean = mean;
            results.repeatability_std = std_dev;
        }

        let success = mean <= self.params.max_repeatability_deg;

        Ok(if success {
            StepResult::success("repeatability_test", 2, start.elapsed())
                .with_data("repeatability_mean", mean)
                .with_data("repeatability_std", std_dev)
                .with_data("measurements", measurements)
        } else {
            StepResult::failure(
                "repeatability_test",
                2,
                start.elapsed(),
                format!(
                    "Repeatability {:.4} deg exceeds max {:.4} deg",
                    mean, self.params.max_repeatability_deg
                ),
            )
            .with_data("repeatability_mean", mean)
            .with_data("repeatability_std", std_dev)
        })
    }

    /// Step 4: Compute final results
    async fn step_compute_results(&mut self, ctx: &ProcedureContext) -> Result<StepResult> {
        let start = Instant::now();

        self.update_progress(
            ProcedureState::Finalizing,
            3,
            4,
            "compute_results",
            "Computing final results",
        );

        // Get current position as home offset reference
        let device = ctx
            .get_movable("rotator")
            .await
            .ok_or_else(|| anyhow!("Rotator device not found"))?;

        let final_position = device.position().await?;
        let home_offset = final_position - self.params.reference_position;

        if let Some(ref mut results) = self.results {
            results.home_offset = home_offset;
        }

        Ok(StepResult::success("compute_results", 3, start.elapsed())
            .with_data("home_offset", home_offset)
            .with_data("final_position", final_position))
    }

    /// Update progress and send to context
    fn update_progress(
        &mut self,
        state: ProcedureState,
        step: usize,
        total: usize,
        step_name: &str,
        message: &str,
    ) {
        self.state = state;
        self.progress = ProcedureProgress {
            state,
            current_step: step,
            total_steps: total,
            step_name: step_name.to_string(),
            step_progress: 0.0,
            overall_progress: step as f64 / total as f64,
            eta_seconds: None,
            message: message.to_string(),
        };
    }
}

#[async_trait]
impl Procedure for RotatorCalibration {
    fn type_info() -> ProcedureTypeInfo {
        ProcedureTypeInfo {
            type_id: "rotator_calibration".to_string(),
            name: "Rotator Calibration".to_string(),
            description:
                "Calibrate rotary motion stage: home position, backlash, and repeatability"
                    .to_string(),
            category: "calibration".to_string(),
            roles: vec![RoleRequirement {
                role_id: "rotator".to_string(),
                capability: "Movable".to_string(),
                optional: false,
                description: "Rotary motion stage to calibrate".to_string(),
            }],
            parameters: vec![
                ParameterDef {
                    name: "num_cycles".to_string(),
                    param_type: "i32".to_string(),
                    default: Some("3".to_string()),
                    units: None,
                    description: "Number of repeatability test cycles".to_string(),
                    constraints: Some(ParameterConstraints {
                        min: Some(1.0),
                        max: Some(100.0),
                        allowed_values: None,
                        pattern: None,
                    }),
                },
                ParameterDef {
                    name: "angle_tolerance_deg".to_string(),
                    param_type: "f64".to_string(),
                    default: Some("0.1".to_string()),
                    units: Some("deg".to_string()),
                    description: "Acceptable position error".to_string(),
                    constraints: Some(ParameterConstraints {
                        min: Some(0.001),
                        max: Some(10.0),
                        allowed_values: None,
                        pattern: None,
                    }),
                },
                ParameterDef {
                    name: "backlash_test_angle".to_string(),
                    param_type: "f64".to_string(),
                    default: Some("5.0".to_string()),
                    units: Some("deg".to_string()),
                    description: "Angle for backlash testing".to_string(),
                    constraints: Some(ParameterConstraints {
                        min: Some(1.0),
                        max: Some(180.0),
                        allowed_values: None,
                        pattern: None,
                    }),
                },
                ParameterDef {
                    name: "reference_position".to_string(),
                    param_type: "f64".to_string(),
                    default: Some("0.0".to_string()),
                    units: Some("deg".to_string()),
                    description: "Reference/home position for calibration".to_string(),
                    constraints: None,
                },
                ParameterDef {
                    name: "settle_time_sec".to_string(),
                    param_type: "f64".to_string(),
                    default: Some("0.5".to_string()),
                    units: Some("s".to_string()),
                    description: "Settle time after moves".to_string(),
                    constraints: Some(ParameterConstraints {
                        min: Some(0.0),
                        max: Some(10.0),
                        allowed_values: None,
                        pattern: None,
                    }),
                },
            ],
            version: "1.0.0".to_string(),
        }
    }

    fn type_id(&self) -> &str {
        "rotator_calibration"
    }

    fn state(&self) -> ProcedureState {
        self.state
    }

    fn progress(&self) -> ProcedureProgress {
        self.progress.clone()
    }

    fn configure(&mut self, config: &ProcedureConfig) -> Result<Vec<String>> {
        self.config = config.clone();
        self.params = RotatorCalibrationConfig::from_procedure_config(config);
        Ok(vec![])
    }

    fn get_config(&self) -> &ProcedureConfig {
        &self.config
    }

    async fn validate(&mut self, ctx: &ProcedureContext) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Check rotator device is available
        if ctx.get_movable("rotator").await.is_none() {
            return Err(anyhow!(
                "Rotator device not available. Check role assignment: {}",
                ctx.get_device_id("rotator")
                    .unwrap_or(&"<not assigned>".to_string())
            ));
        }

        // Validate parameters
        if self.params.num_cycles == 0 {
            return Err(anyhow!("num_cycles must be at least 1"));
        }

        if self.params.angle_tolerance_deg <= 0.0 {
            return Err(anyhow!("angle_tolerance_deg must be positive"));
        }

        if self.params.backlash_test_angle <= 0.0 {
            return Err(anyhow!("backlash_test_angle must be positive"));
        }

        // Warn about potentially slow execution
        if self.params.num_cycles > 10 {
            warnings.push(format!(
                "High cycle count ({}) may take a long time",
                self.params.num_cycles
            ));
        }

        self.state = ProcedureState::Validating;
        Ok(warnings)
    }

    async fn prepare(&mut self, _ctx: &ProcedureContext) -> Result<()> {
        self.state = ProcedureState::Preparing;

        // Initialize results structure
        self.results = Some(RotatorCalibrationResults {
            backlash_deg: 0.0,
            repeatability_measurements: Vec::new(),
            repeatability_mean: 0.0,
            repeatability_std: 0.0,
            home_offset: 0.0,
            all_positions: Vec::new(),
        });

        Ok(())
    }

    async fn execute(&mut self, ctx: ProcedureContext) -> Result<ProcedureResult> {
        let execution_start = Instant::now();
        self.state = ProcedureState::Running;

        let mut step_results = Vec::new();
        let mut quality = QualityMetrics::default();

        // Step 1: Move to reference
        let step1 = self.step_move_to_reference(&ctx).await?;
        if !step1.success {
            self.state = ProcedureState::Failed;
            return Ok(ProcedureResult::failure(
                "rotator_calibration",
                &ctx.procedure_id,
                step1
                    .error
                    .clone()
                    .unwrap_or_else(|| "Move to reference failed".to_string()),
            )
            .with_step(step1));
        }
        quality.add_pass(
            "move_to_reference",
            "Successfully moved to reference position",
        );
        step_results.push(step1);

        // Check cancellation
        if ctx.is_cancelled() {
            self.state = ProcedureState::Cancelled;
            return Ok(ProcedureResult::failure(
                "rotator_calibration",
                &ctx.procedure_id,
                "Cancelled by user",
            ));
        }

        // Step 2: Measure backlash
        let step2 = self.step_measure_backlash(&ctx).await?;
        let backlash = self.results.as_ref().map(|r| r.backlash_deg).unwrap_or(0.0);
        if backlash <= self.params.max_backlash_deg {
            quality.add_pass(
                "backlash",
                format!("Backlash {:.4} deg within limit", backlash),
            );
        } else {
            quality.add_warning(
                "backlash",
                format!(
                    "Backlash {:.4} deg exceeds max {:.4} deg",
                    backlash, self.params.max_backlash_deg
                ),
            );
        }
        step_results.push(step2);

        // Check cancellation
        if ctx.is_cancelled() {
            self.state = ProcedureState::Cancelled;
            return Ok(ProcedureResult::failure(
                "rotator_calibration",
                &ctx.procedure_id,
                "Cancelled by user",
            ));
        }

        // Step 3: Repeatability test
        let step3 = self.step_repeatability_test(&ctx).await?;
        if !step3.success {
            self.state = ProcedureState::Failed;
            quality.add_fail(
                "repeatability",
                step3
                    .error
                    .clone()
                    .unwrap_or_else(|| "Repeatability test failed".to_string()),
            );
            return Ok(ProcedureResult::failure(
                "rotator_calibration",
                &ctx.procedure_id,
                step3
                    .error
                    .clone()
                    .unwrap_or_else(|| "Repeatability test failed".to_string()),
            )
            .with_step(step3)
            .with_quality(quality));
        }
        let repeatability = self
            .results
            .as_ref()
            .map(|r| r.repeatability_mean)
            .unwrap_or(0.0);
        quality.add_pass(
            "repeatability",
            format!("Repeatability {:.4} deg within limit", repeatability),
        );
        step_results.push(step3);

        // Step 4: Compute results
        let step4 = self.step_compute_results(&ctx).await?;
        step_results.push(step4);

        // Build final result
        self.state = ProcedureState::Completed;

        let results = self.results.as_ref().expect("Results should be populated");

        let mut result = ProcedureResult::success("rotator_calibration", &ctx.procedure_id)
            .with_quality(quality)
            .with_data("backlash_deg", results.backlash_deg)
            .with_data("repeatability_mean", results.repeatability_mean)
            .with_data("repeatability_std", results.repeatability_std)
            .with_data("home_offset", results.home_offset)
            .with_data("num_cycles", self.params.num_cycles as i64);

        for step in step_results {
            result = result.with_step(step);
        }

        result.timing.total_duration = execution_start.elapsed();
        result.timing.execution_duration = execution_start.elapsed();

        Ok(result)
    }

    async fn finalize(&mut self, ctx: &ProcedureContext) -> Result<()> {
        // Return to reference position if possible
        if let Some(device) = ctx.get_movable("rotator").await {
            let _ = device.move_abs(self.params.reference_position).await;
        }
        Ok(())
    }

    async fn cancel(&mut self) -> Result<()> {
        self.state = ProcedureState::Cancelled;
        Ok(())
    }
}

// =============================================================================
// CalibrationResult Conversion
// =============================================================================

impl From<(ProcedureResult, &RotatorCalibrationResults)> for CalibrationResult {
    fn from((base, results): (ProcedureResult, &RotatorCalibrationResults)) -> Self {
        let mut fitted_params = std::collections::HashMap::new();
        fitted_params.insert("backlash_deg".to_string(), results.backlash_deg);
        fitted_params.insert("repeatability_mean".to_string(), results.repeatability_mean);
        fitted_params.insert("repeatability_std".to_string(), results.repeatability_std);

        CalibrationResult {
            calibration: CalibrationData {
                device_id: base.execution_id.clone(),
                calibration_type: "rotator_home".to_string(),
                before: None,
                after: Some(results.home_offset),
                improvement: None,
                fitted_params,
                reference: Some(0.0),
                tolerance_achieved: Some(results.repeatability_mean),
            },
            base,
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
    fn test_config_defaults() {
        let config = RotatorCalibrationConfig::default();
        assert_eq!(config.num_cycles, 3);
        assert!((config.angle_tolerance_deg - 0.1).abs() < 0.001);
        assert!((config.backlash_test_angle - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_config_from_procedure_config() {
        let mut proc_config = ProcedureConfig::new("rotator_calibration");
        proc_config.set_param("num_cycles", 5i64);
        proc_config.set_param("angle_tolerance_deg", 0.05f64);

        let config = RotatorCalibrationConfig::from_procedure_config(&proc_config);
        assert_eq!(config.num_cycles, 5);
        assert!((config.angle_tolerance_deg - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_type_info() {
        let info = RotatorCalibration::type_info();
        assert_eq!(info.type_id, "rotator_calibration");
        assert_eq!(info.category, "calibration");
        assert_eq!(info.roles.len(), 1);
        assert_eq!(info.roles[0].role_id, "rotator");
        assert!(!info.parameters.is_empty());
    }

    #[test]
    fn test_procedure_creation() {
        let proc = RotatorCalibration::new();
        assert_eq!(proc.type_id(), "rotator_calibration");
        assert_eq!(proc.state(), ProcedureState::Idle);
    }

    #[test]
    fn test_procedure_configure() {
        let mut proc = RotatorCalibration::new();
        let config = ProcedureConfig::new("rotator_calibration")
            .with_param("num_cycles", 10i64)
            .with_role("rotator", "test_device");

        let warnings = proc.configure(&config).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(proc.params.num_cycles, 10);
    }
}
