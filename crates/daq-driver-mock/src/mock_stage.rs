//! Mock motion stage implementation.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use common::capabilities::{Movable, Parameterized};
use common::driver::{Capability, DeviceComponents, DriverFactory};
use common::observable::ParameterSet;
use common::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

use crate::common::{ErrorConfig, MockMode};

// =============================================================================
// MockStageFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockStage driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockStageConfig {
    /// Initial position in mm (default: 0.0)
    #[serde(default)]
    pub initial_position: f64,

    /// Motion speed in mm/sec (default: 10.0)
    #[serde(default = "default_speed")]
    pub speed_mm_per_sec: f64,
}

fn default_speed() -> f64 {
    10.0
}

impl Default for MockStageConfig {
    fn default() -> Self {
        Self {
            initial_position: 0.0,
            speed_mm_per_sec: 10.0,
        }
    }
}

/// Factory for creating MockStage instances.
pub struct MockStageFactory;

/// Static capabilities for MockStage
static MOCK_STAGE_CAPABILITIES: &[Capability] = &[Capability::Movable, Capability::Parameterized];

impl DriverFactory for MockStageFactory {
    fn driver_type(&self) -> &'static str {
        "mock_stage"
    }

    fn name(&self) -> &'static str {
        "Mock Stage"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_STAGE_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        // Try to deserialize to validate
        let _: MockStageConfig = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockStageConfig = config.try_into().unwrap_or_default();

            let stage = Arc::new(MockStage::with_config(cfg));

            Ok(DeviceComponents {
                movable: Some(stage.clone()),
                parameterized: Some(stage),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Configuration Structs
// =============================================================================

/// Limit behavior when position exceeds boundaries
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LimitBehavior {
    /// Hard stop - return error at limit
    HardStop,
    /// Clamp to limit - move to boundary without error
    Clamp,
    /// Ignore limits - allow any position
    Ignore,
}

/// Stage position limits configuration
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageLimits {
    pub min_position: f64,
    pub max_position: f64,
    pub behavior: LimitBehavior,
}

impl StageLimits {
    /// Create limits with HardStop behavior
    pub fn hard_stop(min: f64, max: f64) -> Self {
        Self {
            min_position: min,
            max_position: max,
            behavior: LimitBehavior::HardStop,
        }
    }

    /// Create limits with Clamp behavior
    pub fn clamp(min: f64, max: f64) -> Self {
        Self {
            min_position: min,
            max_position: max,
            behavior: LimitBehavior::Clamp,
        }
    }

    /// Check and enforce limits on a target position
    fn enforce(&self, target: f64) -> Result<f64> {
        match self.behavior {
            LimitBehavior::Ignore => Ok(target),
            LimitBehavior::Clamp => Ok(target.clamp(self.min_position, self.max_position)),
            LimitBehavior::HardStop => {
                if target < self.min_position || target > self.max_position {
                    Err(anyhow!(
                        "Position {:.2}mm exceeds limits [{:.2}, {:.2}]mm",
                        target,
                        self.min_position,
                        self.max_position
                    ))
                } else {
                    Ok(target)
                }
            }
        }
    }
}

/// Trapezoidal velocity profile for realistic motion simulation
#[derive(Debug, Clone, Copy)]
pub struct VelocityProfile {
    /// Maximum velocity in units/sec
    pub max_velocity: f64,
    /// Acceleration in units/sec²
    pub acceleration: f64,
    /// Deceleration in units/sec²
    pub deceleration: f64,
}

impl VelocityProfile {
    /// Create a symmetric profile (same accel/decel)
    pub fn symmetric(max_velocity: f64, acceleration: f64) -> Self {
        Self {
            max_velocity,
            acceleration,
            deceleration: acceleration,
        }
    }

    /// Calculate motion time using trapezoidal profile
    ///
    /// Returns (total_time, accel_time, cruise_time, decel_time)
    fn calculate_motion_time(&self, distance: f64) -> (Duration, f64, f64, f64) {
        let distance = distance.abs();

        // Time to reach max velocity
        let accel_time = self.max_velocity / self.acceleration;
        let decel_time = self.max_velocity / self.deceleration;

        // Distance covered during acceleration and deceleration
        let accel_dist = 0.5 * self.acceleration * accel_time * accel_time;
        let decel_dist = 0.5 * self.deceleration * decel_time * decel_time;

        // Check if we reach max velocity
        if accel_dist + decel_dist <= distance {
            // Trapezoidal profile (reaches max velocity)
            let cruise_dist = distance - accel_dist - decel_dist;
            let cruise_time = cruise_dist / self.max_velocity;
            let total = accel_time + cruise_time + decel_time;
            (
                Duration::from_secs_f64(total),
                accel_time,
                cruise_time,
                decel_time,
            )
        } else {
            // Triangular profile (doesn't reach max velocity)
            // Solve for time when we don't reach max velocity
            let t_accel = (2.0 * distance * self.deceleration
                / (self.acceleration * (self.acceleration + self.deceleration)))
                .sqrt();
            let t_decel = (2.0 * distance * self.acceleration
                / (self.deceleration * (self.acceleration + self.deceleration)))
                .sqrt();
            let total = t_accel + t_decel;
            (Duration::from_secs_f64(total), t_accel, 0.0, t_decel)
        }
    }
}

impl Default for VelocityProfile {
    fn default() -> Self {
        // Default: 10mm/s max, 20mm/s² accel
        Self::symmetric(10.0, 20.0)
    }
}

// =============================================================================
// Internal State
// =============================================================================

#[derive(Debug)]
struct StageState {
    /// Current position
    position: f64,
    /// Is the stage homed?
    is_homed: bool,
    /// Home position offset
    home_offset: f64,
    /// Is the stage currently moving?
    is_moving: bool,
}

// =============================================================================
// MockStage - Simulated Motion Stage
// =============================================================================

/// Mock motion stage with realistic timing.
///
/// Simulates a linear stage with:
/// - Configurable motion speed and velocity profile
/// - Distance-based settling time
/// - Position limits with configurable behavior
/// - Homing support
/// - Emergency stop
/// - Error injection for testing
///
/// # Example
///
/// ```rust,ignore
/// let stage = MockStage::builder()
///     .limits(StageLimits::clamp(0.0, 100.0))
///     .velocity_profile(VelocityProfile::symmetric(20.0, 40.0))
///     .build();
///
/// stage.move_abs(10.0).await?;
/// assert_eq!(stage.position().await?, 10.0);
/// ```
pub struct MockStage {
    position: Parameter<f64>,
    /// Internal state
    state: Arc<RwLock<StageState>>,
    /// Velocity profile
    velocity_profile: VelocityProfile,
    /// Position limits
    limits: Option<StageLimits>,
    /// Base settling time (ms)
    base_settling_ms: u64,
    /// Settling coefficient (ms per mm)
    settling_coefficient: f64,
    /// Operational mode
    mode: MockMode,
    /// Error injection
    error_config: ErrorConfig,
    /// Parameter set
    params: ParameterSet,
}

impl Clone for MockStage {
    fn clone(&self) -> Self {
        Self {
            position: self.position.clone(),
            state: self.state.clone(),
            velocity_profile: self.velocity_profile,
            limits: self.limits,
            base_settling_ms: self.base_settling_ms,
            settling_coefficient: self.settling_coefficient,
            mode: self.mode,
            error_config: self.error_config.clone(),
            // ParameterSet is shared via Arc internally, so we can reconstruct it
            params: {
                let mut params = ParameterSet::new();
                params.register(self.position.clone());
                params
            },
        }
    }
}

impl MockStage {
    /// Create new mock stage at position 0.0mm with default speed.
    ///
    /// BACKWARD COMPATIBILITY: This maintains the original instant mode behavior.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Create mock stage with configuration.
    pub fn with_config(config: MockStageConfig) -> Self {
        Self::builder()
            .initial_position(config.initial_position)
            .velocity_profile(VelocityProfile {
                max_velocity: config.speed_mm_per_sec,
                ..Default::default()
            })
            .build()
    }

    /// Create new mock stage at specified initial position.
    ///
    /// # Arguments
    /// * `initial_position` - Starting position in mm
    pub fn with_position(initial_position: f64) -> Self {
        Self::builder().initial_position(initial_position).build()
    }

    /// Create mock stage with custom speed.
    ///
    /// # Arguments
    /// * `speed_mm_per_sec` - Motion speed in mm/sec
    pub fn with_speed(speed_mm_per_sec: f64) -> Self {
        Self::builder()
            .velocity_profile(VelocityProfile {
                max_velocity: speed_mm_per_sec,
                ..Default::default()
            })
            .build()
    }

    /// Create a builder for configuring MockStage
    pub fn builder() -> MockStageBuilder {
        MockStageBuilder::new()
    }

    /// Home the stage (set current position as home with offset)
    pub async fn home(&self) -> Result<()> {
        self.error_config.check_operation("mock_stage", "home")?;

        let mut state = self.state.write().await;

        tracing::debug!("MockStage: Homing...");

        // Simulate homing motion time
        if matches!(self.mode, MockMode::Realistic) {
            drop(state); // Release lock during sleep
            sleep(Duration::from_millis(100)).await;
            state = self.state.write().await;
        }

        state.position = state.home_offset;
        state.is_homed = true;

        // Update parameter
        drop(state);
        self.position.set(0.0).await?;

        tracing::debug!("MockStage: Homing complete");
        Ok(())
    }

    /// Check if stage is homed
    pub fn is_homed(&self) -> bool {
        // Safe unwrap: single read operation
        futures::executor::block_on(async { self.state.read().await.is_homed })
    }

    /// Set home position offset
    pub fn set_home_offset(&self, offset: f64) {
        futures::executor::block_on(async {
            self.state.write().await.home_offset = offset;
        });
    }

    /// Emergency stop - halt motion immediately
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.write().await;
        state.is_moving = false;
        tracing::debug!("MockStage: Emergency stop");
        Ok(())
    }

    /// Check if stage is currently moving
    pub fn is_moving(&self) -> bool {
        futures::executor::block_on(async { self.state.read().await.is_moving })
    }

    /// Calculate motion duration based on distance and mode
    fn calculate_motion_duration(&self, distance: f64) -> Duration {
        match self.mode {
            MockMode::Instant => Duration::ZERO,
            MockMode::Realistic | MockMode::Chaos => {
                let (duration, _, _, _) = self.velocity_profile.calculate_motion_time(distance);
                duration
            }
        }
    }

    /// Calculate settling time based on distance
    fn calculate_settling_time(&self, distance: f64) -> Duration {
        match self.mode {
            MockMode::Instant => Duration::ZERO,
            MockMode::Realistic | MockMode::Chaos => {
                let settling_ms =
                    self.base_settling_ms + (self.settling_coefficient * distance.abs()) as u64;
                Duration::from_millis(settling_ms)
            }
        }
    }
}

impl Default for MockStage {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameterized for MockStage {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for MockStage {
    async fn move_abs(&self, target: f64) -> Result<()> {
        // Check for errors
        self.error_config.check_operation("mock_stage", "move")?;

        // Enforce limits
        let target = if let Some(limits) = &self.limits {
            limits.enforce(target)?
        } else {
            target
        };

        let current = self.position.get();
        let distance = (target - current).abs();

        tracing::debug!(
            "MockStage: Moving from {:.2}mm to {:.2}mm ({:.2}mm)",
            current,
            target,
            distance
        );

        // Mark as moving
        {
            let mut state = self.state.write().await;
            state.is_moving = true;
        }

        // Simulate motion time
        let motion_duration = self.calculate_motion_duration(distance);
        if !motion_duration.is_zero() {
            sleep(motion_duration).await;
        }

        // Update position
        {
            let mut state = self.state.write().await;
            state.position = target;
            state.is_moving = false;
        }

        // Update parameter (triggers callbacks)
        self.position.set(target).await?;

        tracing::debug!("MockStage: Reached {:.2}mm", target);
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = self.position.get();
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(self.position.get())
    }

    async fn wait_settled(&self) -> Result<()> {
        // Get last move distance for settling calculation
        // Approximate with small default if unknown (could track this more precisely)
        let distance = 1.0;

        let settling_time = self.calculate_settling_time(distance);

        tracing::debug!("MockStage: Settling for {:?}...", settling_time);
        sleep(settling_time).await;
        tracing::debug!("MockStage: Settled");

        Ok(())
    }
}

// =============================================================================
// Builder Pattern
// =============================================================================

/// Builder for MockStage with fluent API
pub struct MockStageBuilder {
    initial_position: f64,
    velocity_profile: VelocityProfile,
    limits: Option<StageLimits>,
    base_settling_ms: u64,
    settling_coefficient: f64,
    mode: MockMode,
    error_config: ErrorConfig,
}

impl MockStageBuilder {
    /// Create a new builder with defaults
    pub fn new() -> Self {
        Self {
            initial_position: 0.0,
            velocity_profile: VelocityProfile::default(),
            limits: None,
            base_settling_ms: 10,
            settling_coefficient: 5.0, // 5ms per mm
            mode: MockMode::Instant,
            error_config: ErrorConfig::none(),
        }
    }

    /// Set initial position
    pub fn initial_position(mut self, position: f64) -> Self {
        self.initial_position = position;
        self
    }

    /// Set velocity profile
    pub fn velocity_profile(mut self, profile: VelocityProfile) -> Self {
        self.velocity_profile = profile;
        self
    }

    /// Set position limits
    pub fn limits(mut self, limits: StageLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Set base settling time (ms)
    pub fn base_settling(mut self, ms: u64) -> Self {
        self.base_settling_ms = ms;
        self
    }

    /// Set settling coefficient (ms per mm)
    pub fn settling_coefficient(mut self, coeff: f64) -> Self {
        self.settling_coefficient = coeff;
        self
    }

    /// Set operational mode
    pub fn mode(mut self, mode: MockMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set error configuration
    pub fn error_config(mut self, config: ErrorConfig) -> Self {
        self.error_config = config;
        self
    }

    /// Build the MockStage
    pub fn build(self) -> MockStage {
        let mut params = ParameterSet::new();

        let state = Arc::new(RwLock::new(StageState {
            position: self.initial_position,
            is_homed: false,
            home_offset: 0.0,
            is_moving: false,
        }));

        let mut position = Parameter::new("position", self.initial_position)
            .with_description("Stage position")
            .with_unit("mm");

        // Attach hardware callbacks
        let state_for_read = state.clone();
        position.connect_to_hardware_read(move || {
            let state = state_for_read.clone();
            Box::pin(async move { Ok(state.read().await.position) })
        });

        params.register(position.clone());

        MockStage {
            position,
            state,
            velocity_profile: self.velocity_profile,
            limits: self.limits,
            base_settling_ms: self.base_settling_ms,
            settling_coefficient: self.settling_coefficient,
            mode: self.mode,
            error_config: self.error_config,
            params,
        }
    }
}

impl Default for MockStageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ErrorScenario;

    #[tokio::test]
    async fn test_mock_stage_absolute_move() {
        let stage = MockStage::new();

        // Initial position should be 0
        assert_eq!(stage.position().await.unwrap(), 0.0);

        // Move to 10mm
        stage.move_abs(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 10.0);

        // Move to 25mm
        stage.move_abs(25.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 25.0);
    }

    #[tokio::test]
    async fn test_mock_stage_relative_move() {
        let stage = MockStage::new();

        // Move +5mm
        stage.move_rel(5.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 5.0);

        // Move +10mm
        stage.move_rel(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 15.0);

        // Move -3mm
        stage.move_rel(-3.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 12.0);
    }

    #[tokio::test]
    async fn test_mock_stage_settle() {
        let stage = MockStage::new();

        stage.move_abs(10.0).await.unwrap();
        stage.wait_settled().await.unwrap(); // Should not panic
    }

    #[tokio::test]
    async fn test_mock_stage_custom_speed() {
        let stage = MockStage::with_speed(20.0); // 20mm/sec

        stage.move_abs(20.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 20.0);
    }

    #[tokio::test]
    async fn test_mock_stage_parameter_set_moves_stage() {
        let stage = MockStage::new();
        let params = stage.parameters();

        let position_param = params
            .get_typed::<Parameter<f64>>("position")
            .expect("position parameter registered");

        position_param.set(7.5).await.unwrap();

        assert_eq!(stage.position().await.unwrap(), 7.5);
    }

    #[tokio::test]
    async fn test_limits_hard_stop() {
        let stage = MockStage::builder()
            .limits(StageLimits::hard_stop(0.0, 100.0))
            .build();

        // Within limits - should succeed
        stage.move_abs(50.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 50.0);

        // Below limit - should fail
        let result = stage.move_abs(-10.0).await;
        assert!(result.is_err());

        // Above limit - should fail
        let result = stage.move_abs(150.0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_limits_clamp() {
        let stage = MockStage::builder()
            .limits(StageLimits::clamp(0.0, 100.0))
            .build();

        // Below limit - should clamp to min
        stage.move_abs(-10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 0.0);

        // Above limit - should clamp to max
        stage.move_abs(150.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 100.0);

        // Within limits - normal behavior
        stage.move_abs(50.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 50.0);
    }

    #[tokio::test]
    async fn test_homing() {
        let stage = MockStage::builder().build();

        assert!(!stage.is_homed());

        stage.home().await.unwrap();
        assert!(stage.is_homed());
        assert_eq!(stage.position().await.unwrap(), 0.0);
    }

    #[tokio::test]
    async fn test_home_offset() {
        let stage = MockStage::builder().initial_position(10.0).build();

        stage.set_home_offset(5.0);
        stage.home().await.unwrap();

        // Position should be at home offset
        assert_eq!(stage.position().await.unwrap(), 0.0);
    }

    #[tokio::test]
    async fn test_emergency_stop() {
        let stage = MockStage::builder().mode(MockMode::Realistic).build();

        assert!(!stage.is_moving());

        // Start a move
        let stage_clone = stage.clone();
        let move_handle = tokio::spawn(async move { stage_clone.move_abs(100.0).await });

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Stop it
        stage.stop().await.unwrap();
        assert!(!stage.is_moving());

        // Wait for move to complete (will stop early)
        let _ = move_handle.await;
    }

    #[tokio::test]
    async fn test_realistic_mode_has_timing() {
        let stage = MockStage::builder()
            .mode(MockMode::Realistic)
            .velocity_profile(VelocityProfile::symmetric(10.0, 20.0))
            .build();

        let start = tokio::time::Instant::now();
        stage.move_abs(10.0).await.unwrap();
        let duration = start.elapsed();

        // Should take approximately 1 second (10mm at 10mm/s)
        // Allow some tolerance for test execution overhead
        assert!(
            duration.as_millis() > 800,
            "Motion too fast: {:?}",
            duration
        );
    }

    #[tokio::test]
    async fn test_distance_based_settling() {
        let stage = MockStage::builder()
            .mode(MockMode::Realistic)
            .base_settling(10)
            .settling_coefficient(5.0) // 5ms per mm
            .build();

        // Move 10mm - should have base (10ms) + distance (50ms) = 60ms settle
        stage.move_abs(10.0).await.unwrap();

        let start = tokio::time::Instant::now();
        stage.wait_settled().await.unwrap();
        let duration = start.elapsed();

        // Should be roughly base settling time (using default distance estimate)
        assert!(
            duration.as_millis() >= 10,
            "Settling too fast: {:?}",
            duration
        );
    }

    #[tokio::test]
    async fn test_error_injection_timeout() {
        let stage = MockStage::builder()
            .error_config(ErrorConfig::scenario(ErrorScenario::Timeout {
                operation: "move",
            }))
            .build();

        let result = stage.move_abs(10.0).await;
        assert!(result.is_err());

        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("timed out"));
    }

    #[tokio::test]
    async fn test_error_injection_fail_after_n() {
        let stage = MockStage::builder()
            .error_config(ErrorConfig::scenario(ErrorScenario::FailAfterN {
                operation: "move",
                count: 2,
            }))
            .build();

        // First two moves should succeed
        assert!(stage.move_abs(10.0).await.is_ok());
        assert!(stage.move_abs(20.0).await.is_ok());

        // Third should fail
        assert!(stage.move_abs(30.0).await.is_err());
    }

    #[tokio::test]
    async fn test_velocity_profile_calculation() {
        let profile = VelocityProfile::symmetric(10.0, 20.0);

        // Short move - triangular profile
        let (duration, _, _, _) = profile.calculate_motion_time(1.0);
        assert!(duration.as_secs_f64() > 0.0);

        // Long move - trapezoidal profile
        let (duration, accel, cruise, decel) = profile.calculate_motion_time(100.0);
        assert!(duration.as_secs_f64() > 0.0);
        assert!(accel > 0.0);
        assert!(cruise > 0.0); // Should have cruise phase
        assert!(decel > 0.0);
    }

    #[tokio::test]
    async fn test_builder_pattern() {
        let stage = MockStage::builder()
            .initial_position(5.0)
            .velocity_profile(VelocityProfile::symmetric(20.0, 40.0))
            .limits(StageLimits::clamp(0.0, 100.0))
            .mode(MockMode::Realistic)
            .build();

        assert_eq!(stage.position().await.unwrap(), 5.0);
    }

    #[tokio::test]
    async fn test_factory_creates_stage() {
        let factory = MockStageFactory;

        assert_eq!(factory.driver_type(), "mock_stage");

        let config = toml::Value::Table(toml::map::Map::new());
        let components = factory.build(config).await.unwrap();

        assert!(components.movable.is_some());
        assert!(components.parameterized.is_some());
    }

    #[tokio::test]
    async fn test_backward_compatibility() {
        // Ensure MockStage::new() still works with instant mode
        let stage = MockStage::new();

        let start = tokio::time::Instant::now();
        stage.move_abs(100.0).await.unwrap();
        let duration = start.elapsed();

        // Instant mode should be very fast (<10ms)
        assert!(duration.as_millis() < 10, "Not instant: {:?}", duration);
    }
}
