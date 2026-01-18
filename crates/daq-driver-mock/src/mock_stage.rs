//! Mock motion stage implementation.

use anyhow::Result;
use async_trait::async_trait;
use daq_core::capabilities::{Movable, Parameterized};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

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
// MockStage - Simulated Motion Stage
// =============================================================================

/// Mock motion stage with realistic timing.
///
/// Simulates a linear stage with:
/// - Configurable motion speed (default: 10mm/sec)
/// - 50ms settling time after motion
/// - Thread-safe position tracking
///
/// # Example
///
/// ```rust,ignore
/// let stage = MockStage::new();
/// stage.move_abs(10.0).await?; // Takes ~1 second
/// assert_eq!(stage.position().await?, 10.0);
/// ```
pub struct MockStage {
    position: Parameter<f64>,
    /// Holds the position state alive for the hardware callbacks.
    /// The Arc is captured by callbacks during initialization.
    #[allow(dead_code)]
    position_state: Arc<RwLock<f64>>,
    speed_mm_per_sec: f64,
    params: ParameterSet,
}

impl MockStage {
    /// Create new mock stage at position 0.0mm with default speed.
    pub fn new() -> Self {
        Self::with_config(MockStageConfig::default())
    }

    /// Create mock stage with configuration.
    pub fn with_config(config: MockStageConfig) -> Self {
        let mut params = ParameterSet::new();
        let position_state = Arc::new(RwLock::new(config.initial_position));
        let position = Parameter::new("position", config.initial_position)
            .with_description("Stage position")
            .with_unit("mm");

        let position =
            Self::attach_stage_callbacks(position, position_state.clone(), config.speed_mm_per_sec);

        params.register(position.clone());

        Self {
            position,
            position_state,
            speed_mm_per_sec: config.speed_mm_per_sec,
            params,
        }
    }

    /// Create new mock stage at specified initial position.
    ///
    /// # Arguments
    /// * `initial_position` - Starting position in mm
    pub fn with_position(initial_position: f64) -> Self {
        Self::with_config(MockStageConfig {
            initial_position,
            ..Default::default()
        })
    }

    /// Create mock stage with custom speed.
    ///
    /// # Arguments
    /// * `speed_mm_per_sec` - Motion speed in mm/sec
    pub fn with_speed(speed_mm_per_sec: f64) -> Self {
        Self::with_config(MockStageConfig {
            speed_mm_per_sec,
            ..Default::default()
        })
    }

    fn attach_stage_callbacks(
        mut position: Parameter<f64>,
        state: Arc<RwLock<f64>>,
        speed_mm_per_sec: f64,
    ) -> Parameter<f64> {
        let state_for_write = state.clone();
        position.connect_to_hardware_write(move |target| {
            let state_for_write = state_for_write.clone();
            Box::pin(async move {
                let current = *state_for_write.read().await;
                let distance = (target - current).abs();
                let delay_ms = (distance / speed_mm_per_sec * 1000.0) as u64;

                tracing::debug!(
                    "MockStage: Moving from {:.2}mm to {:.2}mm ({}ms)",
                    current,
                    target,
                    delay_ms
                );

                sleep(Duration::from_millis(delay_ms)).await;
                *state_for_write.write().await = target;
                tracing::debug!("MockStage: Reached {:.2}mm", target);
                Ok(())
            })
        });

        let state_for_read = state.clone();
        position.connect_to_hardware_read(move || {
            let state_for_read = state_for_read.clone();
            Box::pin(async move { Ok(*state_for_read.read().await) })
        });

        position
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
        tracing::debug!(
            "MockStage: command move to {:.2}mm at {:.2} mm/s",
            target,
            self.speed_mm_per_sec
        );
        self.position.set(target).await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = self.position.get();
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(self.position.get())
    }

    async fn wait_settled(&self) -> Result<()> {
        tracing::debug!("MockStage: Settling...");
        sleep(Duration::from_millis(50)).await; // 50ms settling time
        tracing::debug!("MockStage: Settled");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn test_factory_creates_stage() {
        let factory = MockStageFactory;

        assert_eq!(factory.driver_type(), "mock_stage");

        let config = toml::Value::Table(toml::map::Map::new());
        let components = factory.build(config).await.unwrap();

        assert!(components.movable.is_some());
        assert!(components.parameterized.is_some());
    }
}
