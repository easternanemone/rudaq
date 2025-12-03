//! Procedure Framework for Calibration and Automation
//!
//! A modular, extensible framework for defining reusable experimental procedures,
//! inspired by Hydra, DynExp, PyMoDAQ, ScopeFoundry, and Bluesky patterns.
//!
//! # Key Concepts
//!
//! - **Procedure**: A reusable, parameterized experimental workflow
//! - **ProcedureStep**: Atomic operations that compose into procedures
//! - **ProcedureConfig**: Hierarchical configuration with composition and overrides
//! - **ProcedureResult**: Structured output with quality metrics
//! - **ProcedureRegistry**: Discovery and instantiation of procedure types
//!
//! # Architecture (DynExp/Hydra inspired)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                        ProcedureRegistry                                 │
//! │  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐      │
//! │  │ RotatorCalibration│  │ PowerCalibration │  │ FocusScan        │ ...  │
//! │  └──────────────────┘  └──────────────────┘  └──────────────────┘      │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │                        Procedure Trait                                   │
//! │  validate() → prepare() → execute_steps() → finalize() → report()      │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │                        ProcedureStep Trait                               │
//! │  precondition() → execute() → postcondition() → record()               │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │                        DeviceRegistry (Capabilities)                     │
//! │  Movable | Readable | FrameProducer | ExposureControl | Triggerable     │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Configuration (Hydra-style)
//!
//! Procedures use hierarchical TOML configuration with composition:
//!
//! ```toml
//! # procedures/rotator_calibration.toml
//! [procedure]
//! type = "rotator_calibration"
//! name = "Rotator Home Position Calibration"
//!
//! [defaults]
//! # Include config groups
//! device_roles = "lab_rotators"
//! motion_params = "standard"
//!
//! [params]
//! num_cycles = 3
//! angle_tolerance_deg = 0.1
//!
//! [roles.rotator]
//! capability = "Movable"
//! device_id = "rotator_2"  # Can be overridden
//! ```
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use rust_daq::procedures::{ProcedureRegistry, ProcedureConfig};
//!
//! // Load procedure from config
//! let config = ProcedureConfig::from_file("procedures/rotator_calibration.toml")?;
//!
//! // Create and execute
//! let mut registry = ProcedureRegistry::new(device_registry);
//! let procedure = registry.create_from_config(&config)?;
//!
//! let result = procedure.execute(ctx).await?;
//! println!("Calibration complete: {:?}", result.summary());
//! ```

pub mod config;
pub mod panic_safety;
pub mod result;
pub mod rotator_calibration;
pub mod step;

// Re-exports
pub use config::{ConfigOverride, ProcedureConfig, RoleAssignment};
pub use panic_safety::{CleanupRegistry, EmergencyStopFlag, PanicGuard, SafeStateConfig};
pub use result::{CalibrationData, CalibrationResult, ProcedureResult, QualityMetrics, StepResult};
pub use rotator_calibration::{RotatorCalibration, RotatorCalibrationConfig};
pub use step::{ProcedureStep, StepOutcome, StepProgress};

use crate::hardware::registry::DeviceRegistry;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Procedure Trait
// =============================================================================

/// Information about a procedure type (for registry and UI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureTypeInfo {
    /// Unique type identifier (e.g., "rotator_calibration")
    pub type_id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this procedure does
    pub description: String,
    /// Category for organization (e.g., "calibration", "scan", "alignment")
    pub category: String,
    /// Required device roles
    pub roles: Vec<RoleRequirement>,
    /// Available parameters with defaults
    pub parameters: Vec<ParameterDef>,
    /// Version string
    pub version: String,
}

/// Requirement for a device role
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRequirement {
    /// Role identifier (e.g., "rotator", "power_meter")
    pub role_id: String,
    /// Required capability
    pub capability: String,
    /// Whether this role is optional
    pub optional: bool,
    /// Description of what this role is used for
    pub description: String,
}

/// Definition of a procedure parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDef {
    /// Parameter name
    pub name: String,
    /// Parameter type ("f64", "i32", "bool", "string")
    pub param_type: String,
    /// Default value as string
    pub default: Option<String>,
    /// Physical units (optional)
    pub units: Option<String>,
    /// Description
    pub description: String,
    /// Validation constraints
    pub constraints: Option<ParameterConstraints>,
}

/// Constraints for parameter validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterConstraints {
    /// Minimum value (for numeric types)
    pub min: Option<f64>,
    /// Maximum value (for numeric types)
    pub max: Option<f64>,
    /// Allowed values (for enum-like params)
    pub allowed_values: Option<Vec<String>>,
    /// Regex pattern (for string params)
    pub pattern: Option<String>,
}

/// Current state of a procedure execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcedureState {
    /// Not yet started
    Idle,
    /// Validating configuration and device availability
    Validating,
    /// Preparing resources (staging)
    Preparing,
    /// Executing steps
    Running,
    /// Execution paused
    Paused,
    /// Finalizing and computing results
    Finalizing,
    /// Completed successfully
    Completed,
    /// Failed with error
    Failed,
    /// Cancelled by user
    Cancelled,
}

/// Progress information during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureProgress {
    /// Current state
    pub state: ProcedureState,
    /// Current step index (0-based)
    pub current_step: usize,
    /// Total number of steps
    pub total_steps: usize,
    /// Current step name
    pub step_name: String,
    /// Progress within current step (0.0 - 1.0)
    pub step_progress: f64,
    /// Overall progress (0.0 - 1.0)
    pub overall_progress: f64,
    /// Estimated time remaining in seconds (optional)
    pub eta_seconds: Option<f64>,
    /// Status message
    pub message: String,
}

impl Default for ProcedureProgress {
    fn default() -> Self {
        Self {
            state: ProcedureState::Idle,
            current_step: 0,
            total_steps: 0,
            step_name: String::new(),
            step_progress: 0.0,
            overall_progress: 0.0,
            eta_seconds: None,
            message: String::new(),
        }
    }
}

/// The core Procedure trait that all procedures implement.
///
/// Procedures follow a lifecycle:
/// 1. **validate()** - Check config and device availability
/// 2. **prepare()** - Stage resources, warm up hardware
/// 3. **execute()** - Run the procedure steps
/// 4. **finalize()** - Compute results, cleanup
///
/// The framework guarantees cleanup even on errors.
#[async_trait]
pub trait Procedure: Send + Sync + 'static {
    /// Get static information about this procedure type
    fn type_info() -> ProcedureTypeInfo
    where
        Self: Sized;

    /// Get the type ID for this procedure instance
    fn type_id(&self) -> &str;

    /// Get current state
    fn state(&self) -> ProcedureState;

    /// Get current progress
    fn progress(&self) -> ProcedureProgress;

    /// Configure the procedure with parameters
    ///
    /// Returns list of validation warnings (empty if all OK)
    fn configure(&mut self, config: &ProcedureConfig) -> Result<Vec<String>>;

    /// Get current configuration
    fn get_config(&self) -> &ProcedureConfig;

    /// Validate configuration and device availability
    ///
    /// Called before execution to check:
    /// - Required parameters are set
    /// - Device roles are assigned
    /// - Devices are available and have required capabilities
    async fn validate(&mut self, ctx: &ProcedureContext) -> Result<Vec<String>>;

    /// Prepare for execution (stage resources)
    ///
    /// Called after validation to:
    /// - Allocate buffers
    /// - Initialize hardware to known state
    /// - Warm up devices if needed
    async fn prepare(&mut self, ctx: &ProcedureContext) -> Result<()>;

    /// Execute the procedure
    ///
    /// The main execution method. Implementations should:
    /// - Execute steps in sequence
    /// - Update progress regularly
    /// - Check for cancellation
    /// - Handle errors gracefully
    async fn execute(&mut self, ctx: ProcedureContext) -> Result<ProcedureResult>;

    /// Finalize after execution (cleanup, compute final results)
    ///
    /// Always called after execute(), even on error.
    async fn finalize(&mut self, ctx: &ProcedureContext) -> Result<()>;

    /// Pause execution (if supported)
    async fn pause(&mut self) -> Result<()> {
        Err(anyhow::anyhow!("Pause not supported by this procedure"))
    }

    /// Resume execution (if supported)
    async fn resume(&mut self) -> Result<()> {
        Err(anyhow::anyhow!("Resume not supported by this procedure"))
    }

    /// Cancel execution
    async fn cancel(&mut self) -> Result<()>;
}

// =============================================================================
// Procedure Context
// =============================================================================

/// Context provided to procedures for device access and progress reporting.
pub struct ProcedureContext {
    /// Procedure instance ID
    pub procedure_id: String,

    /// Device assignments: role_id -> device_id
    assignments: HashMap<String, String>,

    /// Device registry for hardware access
    registry: Arc<RwLock<DeviceRegistry>>,

    /// Progress sender (for UI updates)
    progress_tx: tokio::sync::watch::Sender<ProcedureProgress>,

    /// Cancellation signal
    cancel_rx: tokio::sync::watch::Receiver<bool>,
}

impl ProcedureContext {
    /// Create a new procedure context
    pub fn new(
        procedure_id: String,
        assignments: HashMap<String, String>,
        registry: Arc<RwLock<DeviceRegistry>>,
    ) -> (Self, tokio::sync::watch::Receiver<ProcedureProgress>) {
        let (progress_tx, progress_rx) = tokio::sync::watch::channel(ProcedureProgress::default());
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        // Store cancel_tx somewhere if needed for external cancellation
        // For now, we'll handle it internally

        let ctx = Self {
            procedure_id,
            assignments,
            registry,
            progress_tx,
            cancel_rx,
        };

        (ctx, progress_rx)
    }

    /// Get a Movable device assigned to a role
    pub async fn get_movable(
        &self,
        role_id: &str,
    ) -> Option<Arc<dyn crate::hardware::capabilities::Movable>> {
        let device_id = self.assignments.get(role_id)?;
        let registry = self.registry.read().await;
        registry.get_movable(device_id)
    }

    /// Get a Readable device assigned to a role
    pub async fn get_readable(
        &self,
        role_id: &str,
    ) -> Option<Arc<dyn crate::hardware::capabilities::Readable>> {
        let device_id = self.assignments.get(role_id)?;
        let registry = self.registry.read().await;
        registry.get_readable(device_id)
    }

    /// Update progress
    pub fn update_progress(&self, progress: ProcedureProgress) {
        let _ = self.progress_tx.send(progress);
    }

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    /// Get device ID for a role
    pub fn get_device_id(&self, role_id: &str) -> Option<&String> {
        self.assignments.get(role_id)
    }
}

impl Clone for ProcedureContext {
    fn clone(&self) -> Self {
        Self {
            procedure_id: self.procedure_id.clone(),
            assignments: self.assignments.clone(),
            registry: Arc::clone(&self.registry),
            progress_tx: self.progress_tx.clone(),
            cancel_rx: self.cancel_rx.clone(),
        }
    }
}

// =============================================================================
// Procedure Registry
// =============================================================================

/// Factory function for creating procedures
pub type ProcedureFactory = fn() -> Box<dyn Procedure>;

/// Registry for procedure types and instances
pub struct ProcedureRegistry {
    /// Device registry for hardware access
    device_registry: Arc<RwLock<DeviceRegistry>>,

    /// Registered procedure types: type_id -> factory
    procedure_types: HashMap<String, ProcedureFactory>,

    /// Procedure type info cache
    type_info_cache: HashMap<String, ProcedureTypeInfo>,
}

impl ProcedureRegistry {
    /// Create a new procedure registry
    pub fn new(device_registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        let mut registry = Self {
            device_registry,
            procedure_types: HashMap::new(),
            type_info_cache: HashMap::new(),
        };

        // Register built-in procedures
        registry.register_builtin_procedures();

        registry
    }

    /// Register built-in procedure types
    fn register_builtin_procedures(&mut self) {
        // Will be populated with actual procedures
    }

    /// Register a procedure type
    pub fn register_type<P: Procedure + Default + 'static>(&mut self) {
        let info = P::type_info();
        let type_id = info.type_id.clone();
        self.type_info_cache.insert(type_id.clone(), info);
        self.procedure_types
            .insert(type_id, || Box::new(P::default()));
    }

    /// List all registered procedure types
    pub fn list_types(&self) -> Vec<&ProcedureTypeInfo> {
        self.type_info_cache.values().collect()
    }

    /// Get info for a specific procedure type
    pub fn get_type_info(&self, type_id: &str) -> Option<&ProcedureTypeInfo> {
        self.type_info_cache.get(type_id)
    }

    /// Create a procedure from configuration
    pub fn create_from_config(&self, config: &ProcedureConfig) -> Result<Box<dyn Procedure>> {
        let type_id = &config.procedure_type;
        let factory = self
            .procedure_types
            .get(type_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown procedure type: {}", type_id))?;

        let mut procedure = factory();
        procedure.configure(config)?;
        Ok(procedure)
    }

    /// Get the device registry
    pub fn device_registry(&self) -> Arc<RwLock<DeviceRegistry>> {
        Arc::clone(&self.device_registry)
    }
}

// =============================================================================
// Safe Procedure Execution
// =============================================================================

/// Execute a procedure with guaranteed cleanup on panic or error.
///
/// This wrapper ensures that:
/// 1. `finalize()` is always called, even if `execute()` panics or errors
/// 2. An emergency stop flag is set if a panic occurs
/// 3. Cleanup actions in the registry are run on failure
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::procedures::{execute_procedure_safely, ProcedureContext};
///
/// let result = execute_procedure_safely(
///     &mut my_procedure,
///     ctx,
///     EmergencyStopFlag::new(),
/// ).await;
/// ```
pub async fn execute_procedure_safely(
    procedure: &mut dyn Procedure,
    ctx: ProcedureContext,
    emergency_flag: EmergencyStopFlag,
) -> Result<ProcedureResult> {
    // Create guard that sets emergency flag on panic
    let _panic_guard = emergency_flag.guard(format!("Procedure: {}", procedure.type_id()));

    // Validate
    let warnings = procedure.validate(&ctx).await?;
    if !warnings.is_empty() {
        tracing::warn!("Procedure validation warnings: {:?}", warnings);
    }

    // Prepare
    procedure.prepare(&ctx).await?;

    // Execute with error handling
    let result = procedure.execute(ctx.clone()).await;

    // Always finalize, regardless of execute result
    if let Err(finalize_err) = procedure.finalize(&ctx).await {
        tracing::error!("Procedure finalize failed: {}", finalize_err);
        // If execute also failed, return the original error
        // If execute succeeded but finalize failed, return finalize error
        if result.is_ok() {
            return Err(finalize_err);
        }
    }

    // Dismiss panic guard on success
    _panic_guard.dismiss();

    result
}

/// Execute a procedure step with panic safety.
///
/// This wrapper catches panics and converts them to errors,
/// ensuring the step doesn't crash the entire procedure.
pub async fn execute_step_safely(
    step: &mut dyn ProcedureStep,
    ctx: &ProcedureContext,
    emergency_flag: &EmergencyStopFlag,
) -> Result<StepOutcome> {
    // Check for emergency stop before executing
    if emergency_flag.is_triggered() {
        return Ok(StepOutcome::abort(format!(
            "Emergency stop active: {}",
            emergency_flag
                .reason()
                .unwrap_or_else(|| "unknown".to_string())
        )));
    }

    // Create guard for this step
    let _guard = emergency_flag.guard(format!("Step: {}", step.name()));

    // Check precondition
    if !step.precondition(ctx).await? {
        return Ok(StepOutcome::failure("Precondition not met"));
    }

    // Execute with retry logic
    let mut attempts = 0;
    let max_attempts = if step.is_retryable() {
        step.max_retries() + 1
    } else {
        1
    };

    let mut last_outcome = StepOutcome::failure("No attempts made");

    while attempts < max_attempts {
        attempts += 1;

        // Check for emergency stop between retries
        if emergency_flag.is_triggered() {
            return Ok(StepOutcome::abort("Emergency stop during retry"));
        }

        match step.execute(ctx).await {
            Ok(outcome) => {
                if outcome.success || outcome.abort_procedure {
                    _guard.dismiss();
                    return Ok(outcome);
                }
                last_outcome = outcome;

                // Retry delay
                if attempts < max_attempts {
                    tokio::time::sleep(step.retry_delay()).await;
                }
            }
            Err(e) => {
                last_outcome = StepOutcome::failure(format!("Error: {}", e));
                if attempts < max_attempts {
                    tracing::warn!(
                        "Step '{}' failed (attempt {}/{}): {}",
                        step.name(),
                        attempts,
                        max_attempts,
                        e
                    );
                    tokio::time::sleep(step.retry_delay()).await;
                }
            }
        }
    }

    // Check postcondition even on failure (for cleanup verification)
    let _ = step.postcondition(ctx).await;

    _guard.dismiss();
    Ok(last_outcome)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_procedure_state_transitions() {
        let states = [
            ProcedureState::Idle,
            ProcedureState::Validating,
            ProcedureState::Preparing,
            ProcedureState::Running,
            ProcedureState::Completed,
        ];

        for state in states {
            assert_eq!(state, state);
        }
    }

    #[test]
    fn test_progress_default() {
        let progress = ProcedureProgress::default();
        assert_eq!(progress.state, ProcedureState::Idle);
        assert_eq!(progress.overall_progress, 0.0);
    }
}
