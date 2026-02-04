//! Mock Dover Motion driver for testing and development

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use common::capabilities::{Movable, Parameterized, TriggerOnPosition};
use common::observable::ParameterSet;
use common::parameter::Parameter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::instrument;

/// Mock Dover Motion axis driver
///
/// Provides a fully functional mock implementation for testing and development
/// without requiring Dover Motion hardware or SDK.
pub struct DoverMockDriver {
    /// Axis name for logging
    axis_name: String,

    /// Current position (mm)
    position: Arc<Mutex<f64>>,

    /// Velocity (mm/s)
    velocity: Arc<Mutex<f64>>,

    /// Acceleration (mm/s²)
    acceleration: Arc<Mutex<f64>>,

    /// Deceleration (mm/s²)
    deceleration: Arc<Mutex<f64>>,

    /// Is axis currently moving
    moving: Arc<Mutex<bool>>,

    /// Trigger-on-position enabled state
    top_enabled: Arc<Mutex<bool>>,

    /// TOP configuration (start, end, increment, bidirectional, pulse_width_ns)
    top_config: Arc<Mutex<Option<(f64, f64, f64, bool, u64)>>>,

    /// Position parameter
    position_param: Parameter<f64>,

    /// Velocity parameter
    velocity_param: Parameter<f64>,

    /// Acceleration parameter
    acceleration_param: Parameter<f64>,

    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl DoverMockDriver {
    /// Create a new mock Dover Motion axis driver.
    ///
    /// # Arguments
    ///
    /// * `axis_name` - Name of the axis (for logging/debugging)
    pub fn new(axis_name: &str) -> Self {
        let mut params = ParameterSet::new();

        let position = Parameter::new("position", 0.0)
            .with_description("Axis position")
            .with_unit("mm");

        let velocity = Parameter::new("velocity", 1.0)
            .with_description("Axis velocity")
            .with_unit("mm/s");

        let acceleration = Parameter::new("acceleration", 10.0)
            .with_description("Axis acceleration")
            .with_unit("mm/s²");

        params.register(position.clone());
        params.register(velocity.clone());
        params.register(acceleration.clone());

        Self {
            axis_name: axis_name.to_string(),
            position: Arc::new(Mutex::new(0.0)),
            velocity: Arc::new(Mutex::new(1.0)),
            acceleration: Arc::new(Mutex::new(10.0)),
            deceleration: Arc::new(Mutex::new(10.0)),
            moving: Arc::new(Mutex::new(false)),
            top_enabled: Arc::new(Mutex::new(false)),
            top_config: Arc::new(Mutex::new(None)),
            position_param: position,
            velocity_param: velocity,
            acceleration_param: acceleration,
            params: Arc::new(params),
        }
    }

    /// Set velocity.
    pub async fn set_velocity(&self, velocity: f64) -> Result<()> {
        *self.velocity.lock().await = velocity;
        self.velocity_param.inner().set(velocity);
        tracing::debug!("[MOCK] Set velocity to {} mm/s", velocity);
        Ok(())
    }

    /// Set acceleration.
    pub async fn set_acceleration(&self, acceleration: f64) -> Result<()> {
        *self.acceleration.lock().await = acceleration;
        self.acceleration_param.inner().set(acceleration);
        tracing::debug!("[MOCK] Set acceleration to {} mm/s²", acceleration);
        Ok(())
    }

    /// Set deceleration.
    pub async fn set_deceleration(&self, deceleration: f64) -> Result<()> {
        *self.deceleration.lock().await = deceleration;
        tracing::debug!("[MOCK] Set deceleration to {} mm/s²", deceleration);
        Ok(())
    }

    /// Simulate motion (for testing).
    async fn simulate_motion(&self, target: f64) {
        *self.moving.lock().await = true;

        // Simulate motion time based on distance and velocity
        let current_pos = *self.position.lock().await;
        let distance = (target - current_pos).abs();
        let velocity = *self.velocity.lock().await;
        let time_ms = ((distance / velocity) * 1000.0).max(10.0);

        tokio::time::sleep(std::time::Duration::from_millis(time_ms as u64)).await;

        *self.position.lock().await = target;
        *self.moving.lock().await = false;

        // Simulate TOP triggers if enabled
        if *self.top_enabled.lock().await {
            if let Some((start, end, inc, bidir, pulse_width)) = *self.top_config.lock().await {
                let num_triggers = ((end - start).abs() / inc).floor() as usize;
                tracing::debug!(
                    "[MOCK] TOP: Generated {} triggers ({}ns pulses)",
                    num_triggers,
                    pulse_width
                );
            }
        }
    }
}

impl Parameterized for DoverMockDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for DoverMockDriver {
    #[instrument(skip(self), fields(axis = %self.axis_name, position), err)]
    async fn move_abs(&self, position: f64) -> Result<()> {
        tracing::debug!("[MOCK] Moving to absolute position {} mm", position);
        self.simulate_motion(position).await;
        self.position_param.inner().set(position);
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name, distance), err)]
    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = *self.position.lock().await;
        let target = current + distance;
        tracing::debug!("[MOCK] Moving relative distance {} mm", distance);
        self.simulate_motion(target).await;
        self.position_param.inner().set(target);
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn position(&self) -> Result<f64> {
        let pos = *self.position.lock().await;
        self.position_param.inner().set(pos);
        Ok(pos)
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn wait_settled(&self) -> Result<()> {
        let timeout = std::time::Duration::from_secs(60);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("[MOCK] wait_settled timed out after 60 seconds"));
            }

            if !*self.moving.lock().await {
                return Ok(());
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn stop(&self) -> Result<()> {
        tracing::debug!("[MOCK] Stopping axis motion");
        *self.moving.lock().await = false;
        Ok(())
    }
}

#[async_trait]
impl TriggerOnPosition for DoverMockDriver {
    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn enable_top(
        &self,
        start_position: f64,
        end_position: f64,
        increment: f64,
        bidirectional: bool,
        pulse_width_ns: u64,
    ) -> Result<()> {
        // Validate parameters
        if increment <= 0.0 {
            return Err(anyhow!(
                "Trigger increment must be positive, got {}",
                increment
            ));
        }

        if pulse_width_ns < 50 || pulse_width_ns > 204800 {
            return Err(anyhow!(
                "Pulse width must be 50-204,800 ns, got {}",
                pulse_width_ns
            ));
        }

        if pulse_width_ns % 50 != 0 {
            return Err(anyhow!(
                "Pulse width must be a multiple of 50ns, got {}",
                pulse_width_ns
            ));
        }

        tracing::info!(
            "[MOCK] Enabled TOP: start={}, end={}, inc={}, bidir={}, pulse_width={}ns",
            start_position,
            end_position,
            increment,
            bidirectional,
            pulse_width_ns
        );

        *self.top_enabled.lock().await = true;
        *self.top_config.lock().await = Some((
            start_position,
            end_position,
            increment,
            bidirectional,
            pulse_width_ns,
        ));

        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn disable_top(&self) -> Result<()> {
        tracing::info!("[MOCK] Disabled TOP");
        *self.top_enabled.lock().await = false;
        *self.top_config.lock().await = None;
        Ok(())
    }

    #[instrument(skip(self), fields(axis = %self.axis_name), err)]
    async fn is_top_enabled(&self) -> Result<bool> {
        Ok(*self.top_enabled.lock().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_movable() {
        let driver = DoverMockDriver::new("X");

        // Test absolute move
        driver.move_abs(10.0).await.unwrap();
        assert_eq!(driver.position().await.unwrap(), 10.0);

        // Test relative move
        driver.move_rel(5.0).await.unwrap();
        assert_eq!(driver.position().await.unwrap(), 15.0);

        // Test wait_settled
        driver.wait_settled().await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_trigger_on_position() {
        let driver = DoverMockDriver::new("X");

        // Test enable TOP
        driver.enable_top(0.0, 10.0, 0.1, true, 1000).await.unwrap();

        assert!(driver.is_top_enabled().await.unwrap());

        // Test disable TOP
        driver.disable_top().await.unwrap();
        assert!(!driver.is_top_enabled().await.unwrap());
    }

    #[tokio::test]
    async fn test_mock_top_validation() {
        let driver = DoverMockDriver::new("X");

        // Test invalid increment
        assert!(driver
            .enable_top(0.0, 10.0, -0.1, true, 1000)
            .await
            .is_err());

        // Test invalid pulse width (too small)
        assert!(driver.enable_top(0.0, 10.0, 0.1, true, 25).await.is_err());

        // Test invalid pulse width (not multiple of 50)
        assert!(driver.enable_top(0.0, 10.0, 0.1, true, 175).await.is_err());

        // Test valid parameters
        assert!(driver.enable_top(0.0, 10.0, 0.1, true, 1000).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_parameters() {
        let driver = DoverMockDriver::new("X");

        // Test velocity parameter
        driver.set_velocity(5.0).await.unwrap();
        assert_eq!(*driver.velocity.lock().await, 5.0);

        // Test acceleration parameter
        driver.set_acceleration(20.0).await.unwrap();
        assert_eq!(*driver.acceleration.lock().await, 20.0);
    }
}
