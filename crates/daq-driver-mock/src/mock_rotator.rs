//! Mock rotary stage implementation (ELL14-like).

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use daq_core::capabilities::{Movable, Parameterized};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

// =============================================================================
// MockRotatorFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockRotator driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockRotatorConfig {
    /// Initial position in degrees (default: 0.0)
    #[serde(default)]
    pub initial_position: f64,

    /// Velocity as percentage (0-100, default: 100)
    #[serde(default = "default_velocity")]
    pub velocity_percent: u8,

    /// Pulses per degree calibration (default: 143.4)
    #[serde(default = "default_pulses_per_degree")]
    pub pulses_per_degree: f64,

    /// Minimum position in degrees (default: 0.0)
    #[serde(default)]
    pub min_position: f64,

    /// Maximum position in degrees (default: 360.0)
    #[serde(default = "default_max_position")]
    pub max_position: f64,
}

fn default_velocity() -> u8 {
    100
}

fn default_pulses_per_degree() -> f64 {
    143.4
}

fn default_max_position() -> f64 {
    360.0
}

impl Default for MockRotatorConfig {
    fn default() -> Self {
        Self {
            initial_position: 0.0,
            velocity_percent: default_velocity(),
            pulses_per_degree: default_pulses_per_degree(),
            min_position: 0.0,
            max_position: default_max_position(),
        }
    }
}

/// Factory for creating MockRotator instances.
pub struct MockRotatorFactory;

/// Static capabilities for MockRotator
static MOCK_ROTATOR_CAPABILITIES: &[Capability] = &[Capability::Movable, Capability::Parameterized];

impl DriverFactory for MockRotatorFactory {
    fn driver_type(&self) -> &'static str {
        "mock_rotator"
    }

    fn name(&self) -> &'static str {
        "Mock Rotary Stage"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_ROTATOR_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: MockRotatorConfig = config.clone().try_into()?;

        // Validate velocity range
        if cfg.velocity_percent > 100 {
            return Err(anyhow!("Velocity must be 0-100%"));
        }

        // Validate position is within range
        if !(cfg.min_position..=cfg.max_position).contains(&cfg.initial_position) {
            return Err(anyhow!(
                "Initial position {} outside range ({}-{})",
                cfg.initial_position,
                cfg.min_position,
                cfg.max_position
            ));
        }

        // Validate pulses per degree is positive
        if cfg.pulses_per_degree <= 0.0 {
            return Err(anyhow!("Pulses per degree must be positive"));
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockRotatorConfig = config.try_into().unwrap_or_default();

            let rotator = Arc::new(MockRotator::with_config(cfg));

            Ok(DeviceComponents {
                movable: Some(rotator.clone()),
                parameterized: Some(rotator),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// ELL14 Status Codes (Mirrored from real driver)
// =============================================================================

/// ELL14 status/error codes.
///
/// Mirrored from the real ELL14 driver for compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum Ell14StatusCode {
    /// No error - command completed successfully
    Ok = 0x00,
    /// Communication timeout - no response from motor
    CommunicationTimeout = 0x01,
    /// Mechanical timeout - motor didn't reach target position in time
    MechanicalTimeout = 0x02,
    /// Command error - invalid or malformed command
    CommandError = 0x03,
    /// Value out of range - parameter outside valid bounds
    ValueOutOfRange = 0x04,
    /// Module isolated - device not responding on bus
    ModuleIsolated = 0x05,
    /// Module out of isolation - device recovered
    ModuleOutOfIsolation = 0x06,
    /// Initializing error - startup sequence failed
    InitializingError = 0x07,
    /// Thermal error - temperature out of range
    ThermalError = 0x08,
    /// Busy - device is executing another command
    Busy = 0x09,
    /// Sensor error - encoder or position sensor fault
    SensorError = 0x0A,
    /// Motor error - driver or coil fault
    MotorError = 0x0B,
    /// Out of range - position outside travel limits
    OutOfRange = 0x0C,
    /// Over current error - excessive motor current
    OverCurrentError = 0x0D,
}

impl Ell14StatusCode {
    /// Convert from u8 to status code.
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::Ok,
            0x01 => Self::CommunicationTimeout,
            0x02 => Self::MechanicalTimeout,
            0x03 => Self::CommandError,
            0x04 => Self::ValueOutOfRange,
            0x05 => Self::ModuleIsolated,
            0x06 => Self::ModuleOutOfIsolation,
            0x07 => Self::InitializingError,
            0x08 => Self::ThermalError,
            0x09 => Self::Busy,
            0x0A => Self::SensorError,
            0x0B => Self::MotorError,
            0x0C => Self::OutOfRange,
            0x0D => Self::OverCurrentError,
            _ => Self::CommandError, // Unknown codes map to command error
        }
    }

    /// Check if this is an error status.
    pub fn is_error(self) -> bool {
        !matches!(self, Self::Ok | Self::ModuleOutOfIsolation)
    }
}

// =============================================================================
// MockRotator - Simulated Rotary Stage
// =============================================================================

/// Mock rotary stage with realistic behavior.
///
/// Simulates an ELL14 rotation mount with:
/// - Position control (0-360 degrees, or custom range)
/// - Velocity control (0-100%)
/// - Calibration values (pulses per degree)
/// - Status codes (14 error states)
/// - Jog operations
/// - Homing capability
///
/// # Example
///
/// ```rust,ignore
/// let rotator = MockRotator::new();
/// rotator.move_abs(90.0).await?; // Move to 90 degrees
/// assert_eq!(rotator.position().await?, 90.0);
/// ```
pub struct MockRotator {
    /// Current position in degrees
    position_degrees: RwLock<f64>,

    /// Position limits
    min_position: f64,
    max_position: f64,

    /// Velocity percentage (0-100)
    velocity_percent: AtomicU8,

    /// Calibration: pulses per degree (reserved for future use)
    #[allow(dead_code)]
    pulses_per_degree: f64,

    /// Status code
    status: AtomicU8,

    /// Homing state
    is_homed: AtomicBool,

    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl MockRotator {
    /// Create a new MockRotator with default configuration.
    pub fn new() -> Self {
        Self::with_config(MockRotatorConfig::default())
    }

    /// Create a new MockRotator with custom configuration.
    pub fn with_config(config: MockRotatorConfig) -> Self {
        let mut params = ParameterSet::new();

        let position_param = Parameter::new("position_degrees", config.initial_position)
            .with_description("Rotary stage position")
            .with_unit("deg")
            .with_range(config.min_position, config.max_position);

        let velocity_param = Parameter::new("velocity_percent", config.velocity_percent as f64)
            .with_description("Motor velocity")
            .with_unit("%")
            .with_range(0.0, 100.0);

        params.register(position_param);
        params.register(velocity_param);

        Self {
            position_degrees: RwLock::new(config.initial_position),
            min_position: config.min_position,
            max_position: config.max_position,
            velocity_percent: AtomicU8::new(config.velocity_percent),
            pulses_per_degree: config.pulses_per_degree,
            status: AtomicU8::new(Ell14StatusCode::Ok as u8),
            is_homed: AtomicBool::new(false),
            params: Arc::new(params),
        }
    }

    /// Get the current velocity percentage.
    pub fn velocity(&self) -> u8 {
        self.velocity_percent.load(Ordering::Relaxed)
    }

    /// Set the velocity percentage (0-100).
    pub fn set_velocity(&self, velocity: u8) -> Result<()> {
        if velocity > 100 {
            self.status
                .store(Ell14StatusCode::ValueOutOfRange as u8, Ordering::Relaxed);
            return Err(anyhow!("Velocity must be 0-100%"));
        }

        self.velocity_percent.store(velocity, Ordering::Relaxed);
        self.status
            .store(Ell14StatusCode::Ok as u8, Ordering::Relaxed);

        Ok(())
    }

    /// Get the current status code.
    pub fn status_code(&self) -> Ell14StatusCode {
        Ell14StatusCode::from_u8(self.status.load(Ordering::Relaxed))
    }

    /// Check if the device is homed.
    pub fn is_homed(&self) -> bool {
        self.is_homed.load(Ordering::Relaxed)
    }

    /// Home the device to mechanical zero.
    pub async fn home(&self) -> Result<()> {
        // Simulate homing motion
        let velocity = self.velocity_percent.load(Ordering::Relaxed);
        let duration_ms = (1000.0 * (100.0 / velocity.max(1) as f64)) as u64;
        sleep(Duration::from_millis(duration_ms)).await;

        // Set position to zero
        *self.position_degrees.write().await = 0.0;

        self.is_homed.store(true, Ordering::Relaxed);
        self.status
            .store(Ell14StatusCode::Ok as u8, Ordering::Relaxed);

        Ok(())
    }

    /// Jog forward by a fixed step (default: 5 degrees).
    pub async fn jog_forward(&self, step_degrees: f64) -> Result<()> {
        let current = *self.position_degrees.read().await;
        let target = current + step_degrees;

        self.move_abs(target).await
    }

    /// Jog backward by a fixed step (default: 5 degrees).
    pub async fn jog_backward(&self, step_degrees: f64) -> Result<()> {
        let current = *self.position_degrees.read().await;
        let target = current - step_degrees;

        self.move_abs(target).await
    }

    /// Calculate movement duration based on distance and velocity.
    fn calculate_duration(&self, distance_degrees: f64) -> Duration {
        let velocity = self.velocity_percent.load(Ordering::Relaxed);

        // Base speed: 1 degree per 10ms at 100% velocity
        // Scale by velocity percentage
        let base_ms_per_degree = 10.0;
        let velocity_factor = velocity.max(1) as f64 / 100.0;
        let ms_per_degree = base_ms_per_degree / velocity_factor;

        let duration_ms = (distance_degrees.abs() * ms_per_degree) as u64;

        Duration::from_millis(duration_ms)
    }
}

impl Default for MockRotator {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameterized for MockRotator {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for MockRotator {
    async fn move_abs(&self, position: f64) -> Result<()> {
        // Validate range
        if !(self.min_position..=self.max_position).contains(&position) {
            self.status
                .store(Ell14StatusCode::OutOfRange as u8, Ordering::Relaxed);
            return Err(anyhow!(
                "Position {} outside range ({}-{})",
                position,
                self.min_position,
                self.max_position
            ));
        }

        let current = *self.position_degrees.read().await;
        let distance = (position - current).abs();

        // Simulate motion time based on distance and velocity
        let duration = self.calculate_duration(distance);
        sleep(duration).await;

        // Update position
        *self.position_degrees.write().await = position;

        self.status
            .store(Ell14StatusCode::Ok as u8, Ordering::Relaxed);

        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = *self.position_degrees.read().await;
        let target = current + distance;

        self.move_abs(target).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(*self.position_degrees.read().await)
    }

    async fn wait_settled(&self) -> Result<()> {
        // Mock rotator settles immediately (no separate settling time)
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factory_driver_type() {
        let factory = MockRotatorFactory;
        assert_eq!(factory.driver_type(), "mock_rotator");
        assert_eq!(factory.name(), "Mock Rotary Stage");
    }

    #[tokio::test]
    async fn test_factory_capabilities() {
        let factory = MockRotatorFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Movable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = MockRotatorFactory;

        // Valid config
        let valid = toml::toml! {
            initial_position = 90.0
            velocity_percent = 50
        };
        assert!(factory.validate(&toml::Value::Table(valid)).is_ok());

        // Invalid velocity (too high)
        let invalid_velocity = toml::toml! {
            velocity_percent = 150
        };
        assert!(
            factory
                .validate(&toml::Value::Table(invalid_velocity))
                .is_err()
        );

        // Invalid position (out of range)
        let invalid_position = toml::toml! {
            initial_position = 400.0
            max_position = 360.0
        };
        assert!(
            factory
                .validate(&toml::Value::Table(invalid_position))
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_position_range_validation() -> Result<()> {
        let rotator = MockRotator::new();

        // Valid positions
        rotator.move_abs(0.0).await?;
        rotator.move_abs(180.0).await?;
        rotator.move_abs(360.0).await?;

        // Invalid positions
        assert!(rotator.move_abs(-1.0).await.is_err());
        assert!(rotator.move_abs(361.0).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_velocity_control() -> Result<()> {
        let rotator = MockRotator::new();

        // Valid velocities
        rotator.set_velocity(50)?;
        assert_eq!(rotator.velocity(), 50);

        rotator.set_velocity(100)?;
        assert_eq!(rotator.velocity(), 100);

        rotator.set_velocity(0)?;
        assert_eq!(rotator.velocity(), 0);

        // Invalid velocity
        assert!(rotator.set_velocity(101).is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_homing() -> Result<()> {
        let rotator = MockRotator::new();

        // Move away from zero
        rotator.move_abs(90.0).await?;
        assert_eq!(rotator.position().await?, 90.0);

        // Home should return to zero
        assert!(!rotator.is_homed());
        rotator.home().await?;
        assert!(rotator.is_homed());
        assert_eq!(rotator.position().await?, 0.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_jog_operations() -> Result<()> {
        let rotator = MockRotator::new();

        // Start at zero
        rotator.move_abs(0.0).await?;

        // Jog forward
        rotator.jog_forward(5.0).await?;
        assert_eq!(rotator.position().await?, 5.0);

        // Jog forward again
        rotator.jog_forward(10.0).await?;
        assert_eq!(rotator.position().await?, 15.0);

        // Jog backward
        rotator.jog_backward(5.0).await?;
        assert_eq!(rotator.position().await?, 10.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_relative_movement() -> Result<()> {
        let rotator = MockRotator::new();

        rotator.move_abs(50.0).await?;
        rotator.move_rel(10.0).await?;
        assert_eq!(rotator.position().await?, 60.0);

        rotator.move_rel(-20.0).await?;
        assert_eq!(rotator.position().await?, 40.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_status_codes() -> Result<()> {
        let rotator = MockRotator::new();

        // Initially OK
        assert_eq!(rotator.status_code(), Ell14StatusCode::Ok);

        // Out of range sets error
        let _ = rotator.move_abs(500.0).await;
        assert_eq!(rotator.status_code(), Ell14StatusCode::OutOfRange);

        // Successful move clears error
        rotator.move_abs(100.0).await?;
        assert_eq!(rotator.status_code(), Ell14StatusCode::Ok);

        Ok(())
    }

    #[tokio::test]
    async fn test_velocity_affects_duration() -> Result<()> {
        let rotator = MockRotator::new();

        // At 100% velocity
        rotator.set_velocity(100)?;
        let start = std::time::Instant::now();
        rotator.move_abs(10.0).await?;
        let fast_duration = start.elapsed();

        // Reset position
        rotator.move_abs(0.0).await?;

        // At 50% velocity (should take approximately twice as long)
        rotator.set_velocity(50)?;
        let start = std::time::Instant::now();
        rotator.move_abs(10.0).await?;
        let slow_duration = start.elapsed();

        // Slow should be approximately 2x fast (with some tolerance)
        let ratio = slow_duration.as_millis() as f64 / fast_duration.as_millis() as f64;
        assert!(ratio > 1.5 && ratio < 2.5, "Ratio: {}", ratio);

        Ok(())
    }
}
