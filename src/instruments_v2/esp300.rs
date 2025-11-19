//! Newport ESP300 3-axis Motion Controller V2 Implementation
//!
//! This module provides a V2 implementation of the Newport ESP300 motion controller
//! using the new three-tier architecture:
//! - SerialAdapter for RS-232 communication
//! - Instrument trait for state management
//! - MotionController trait for domain-specific methods
//!
//! ## Configuration Example
//!
//! ```toml
//! [instruments.motion_controller]
//! type = "esp300_v2"
//! port = "/dev/ttyUSB0"
//! baud_rate = 19200
//! num_axes = 3
//! polling_rate_hz = 5.0
//!
//! [instruments.motion_controller.axis1]
//! units = 1  # 1=millimeters, 2=degrees, etc.
//! velocity = 5.0  # mm/s or deg/s
//! acceleration = 10.0  # mm/s² or deg/s²
//! min_position = 0.0
//! max_position = 100.0
//!
//! [instruments.motion_controller.axis2]
//! units = 1
//! velocity = 5.0
//! acceleration = 10.0
//! min_position = 0.0
//! max_position = 100.0
//!
//! [instruments.motion_controller.axis3]
//! units = 1
//! velocity = 5.0
//! acceleration = 10.0
//! min_position = 0.0
//! max_position = 100.0
//! ```

use crate::adapters::SerialAdapter;

use crate::core_v3::{

    Command, Instrument, InstrumentState, Measurement, MotionController, ParameterBase, Response,

};

use crate::hardware::adapter::HardwareAdapter;

use anyhow::{anyhow, Context, Result};

use async_trait::async_trait;

use chrono::Utc;

use log::{info, warn};

use std::collections::HashMap;

use std::sync::Arc;

use std::time::Duration;

use tokio::sync::{broadcast, Mutex};

use tokio::task::JoinHandle;

/// Axis configuration for ESP300
#[derive(Debug, Clone)]
struct AxisConfig {
    /// Axis number (1-3)
    axis: usize,
    /// Unit code (1=mm, 2=degrees, etc.)
    units: i32,
    /// Unit string for display
    unit_string: String,
    /// Velocity in units/second
    velocity: f64,
    /// Acceleration in units/second²
    acceleration: f64,
    /// Minimum position
    min_position: f64,
    /// Maximum position
    max_position: f64,
}

impl Default for AxisConfig {
    fn default() -> Self {
        Self {
            axis: 1,
            units: 1, // millimeters
            unit_string: "mm".to_string(),
            velocity: 5.0,
            acceleration: 10.0,
            min_position: 0.0,
            max_position: 100.0,
        }
    }
}

// ... (imports)

// ... (AxisConfig struct)

pub struct ESP300V2 {
    /// Instrument identifier
    id: String,

    /// Hardware adapter (Arc<Mutex> for shared mutable access)
    adapter: Arc<Mutex<dyn HardwareAdapter + Send + Sync>>,

    /// Current instrument state
    state: InstrumentState,

    /// Number of axes
    num_axes: usize,

    /// Axis configurations
    axis_configs: Vec<AxisConfig>,

    /// Polling rate for position updates
    polling_rate_hz: f64,

    /// Data streaming
    measurement_tx: broadcast::Sender<Measurement>,

    /// Acquisition task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ESP300V2 {
    /// Create a new ESP300 V2 instrument with a given hardware adapter
    pub fn new(
        id: String,
        adapter: Box<dyn HardwareAdapter + Send + Sync>,
        num_axes: usize,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        let axis_configs = (0..num_axes)
            .map(|i| AxisConfig {
                axis: i + 1,
                ..Default::default()
            })
            .collect();

        Self {
            id,
            adapter: Arc::new(Mutex::new(adapter)),
            state: InstrumentState::Uninitialized,
            num_axes,
            axis_configs,
            polling_rate_hz: 5.0,
            measurement_tx: tx,
            task_handle: None,
            shutdown_tx: None,
        }
    }

    /// Send a command to the motion controller and get a response
    async fn query(&self, command: &str) -> Result<String> {
        self.adapter.lock().await.query(command).await
    }

    /// Send a command to the motion controller without waiting for a response
    async fn send(&self, command: &str) -> Result<()> {
        self.adapter.lock().await.send(command).await
    }
    
    // ... (configure_axis, validate_axis, get_axis_config, configure methods)
    
    /// Spawn polling task for continuous position monitoring
    fn spawn_polling_task(&mut self) {
        let tx = self.measurement_tx.clone();
        // ... (rest of the method)
    }
    
    /// Start continuous position monitoring
    async fn start_streaming(&mut self) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!(
                "Cannot start streaming from state: {:?}",
                self.state
            ));
        }

        self.spawn_polling_task();
        self.state = InstrumentState::Running;

        info!("ESP300 '{}' started streaming", self.id);
        Ok(())
    }

    /// Stop continuous position monitoring
    async fn stop_streaming(&mut self) -> Result<()> {
        if self.state != InstrumentState::Running {
            return Err(anyhow!("Not currently acquiring"));
        }

        // Stop polling task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        self.state = InstrumentState::Idle;
        info!("ESP300 '{}' stopped streaming", self.id);
        Ok(())
    }

    /// Set polling rate for position updates
    pub fn set_polling_rate_hz(&mut self, rate: f64) -> Result<()> {
        if rate <= 0.0 {
            return Err(anyhow!("Polling rate must be positive: {}", rate));
        }
        self.polling_rate_hz = rate;
        Ok(())
    }
}








#[async_trait]
impl Instrument for ESP300V2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> InstrumentState {
        self.state
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Uninitialized {
            return Err(anyhow!("Cannot initialize from state: {:?}", self.state));
        }

        info!("Initializing ESP300 '{}'", self.id);
        self.state = InstrumentState::Connecting;

        // Connect hardware adapter
        let connect_result = {
            let mut adapter = self.adapter.lock().await;
            let config = adapter.default_config();
            adapter.connect(&config).await
        };

        match connect_result {
            Ok(()) => {
                info!("ESP300 '{}' adapter connected", self.id);

                // Configure instrument
                if let Err(e) = self.configure().await {
                    self.state = InstrumentState::Error(e.to_string());
                    let _ = self.adapter.lock().await.disconnect().await;
                    return Err(e);
                }

                self.state = InstrumentState::Idle;
                info!("ESP300 '{}' initialized successfully", self.id);
                Ok(())
            }
            Err(e) => {
                self.state = InstrumentState::Error(e.to_string());
                Err(e)
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down ESP300 '{}'", self.id);
        self.state = InstrumentState::ShuttingDown;

        // Stop acquisition task if running
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        // Disconnect adapter
        self.adapter.lock().await.disconnect().await?;

        self.state = InstrumentState::Idle;
        info!("ESP300 '{}' shut down successfully", self.id);
        Ok(())
    }

    fn data_channel(&self) -> broadcast::Receiver<Measurement> {
        self.measurement_tx.subscribe()
    }

    async fn execute(&mut self, cmd: Command) -> Result<Response> {
        // This will be implemented later
        Ok(Response::Ok)
    }

    fn parameters(&self) -> &std::collections::HashMap<String, Box<dyn ParameterBase>> {
        // This will be implemented later
        unimplemented!()
    }

    fn parameters_mut(
        &mut self,
    ) -> &mut std::collections::HashMap<String, Box<dyn ParameterBase>> {
        // This will be implemented later
        unimplemented!()
    }
}

#[async_trait]
impl MotionController for ESP300V2 {
    fn num_axes(&self) -> usize {
        self.num_axes
    }

    async fn move_absolute(&mut self, axis: usize, position: f64) -> Result<()> {
        if self.state != InstrumentState::Idle && self.state != InstrumentState::Running {
            return Err(anyhow!("Cannot move from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        // Check position limits
        let config = self.get_axis_config(axis)?;
        if position < config.min_position || position > config.max_position {
            return Err(anyhow!(
                "Position {} out of range [{}, {}] for axis {}",
                position,
                config.min_position,
                config.max_position,
                axis
            ));
        }

        self.send(&format!("{}PA{}", axis, position))
            .await
            .context("Failed to send move absolute command")?;

        info!(
            "ESP300 axis {} moving to {} {}",
            axis, position, config.unit_string
        );
        Ok(())
    }

    async fn move_relative(&mut self, axis: usize, distance: f64) -> Result<()> {
        if self.state != InstrumentState::Idle && self.state != InstrumentState::Running {
            return Err(anyhow!("Cannot move from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;
        let config = self.get_axis_config(axis)?;

        self.send(&format!("{}PR{}", axis, distance))
            .await
            .context("Failed to send move relative command")?;

        info!(
            "ESP300 axis {} moving relative {} {}",
            axis, distance, config.unit_string
        );
        Ok(())
    }

    async fn get_position(&self, axis: usize) -> Result<f64> {
        if self.state != InstrumentState::Idle && self.state != InstrumentState::Running {
            return Err(anyhow!("Cannot read position from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        let response = self
            .query(&format!("{}TP", axis))
            .await
            .context("Failed to query position")?;

        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse position response: {}", response))
    }

    async fn get_velocity(&self, axis: usize) -> Result<f64> {
        if self.state != InstrumentState::Idle && self.state != InstrumentState::Running {
            return Err(anyhow!("Cannot read velocity from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        let response = self
            .query(&format!("{}TV", axis))
            .await
            .context("Failed to query velocity")?;

        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse velocity response: {}", response))
    }

    async fn set_velocity(&mut self, axis: usize, velocity: f64) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!("Cannot set velocity from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        if velocity <= 0.0 {
            return Err(anyhow!("Velocity must be positive: {}", velocity));
        }

        self.send(&format!("{}VA{}", axis, velocity))
            .await
            .context("Failed to set velocity")?;

        self.axis_configs[axis - 1].velocity = velocity;
        let config = self.get_axis_config(axis)?;
        info!(
            "Set ESP300 axis {} velocity to {} {}/s",
            axis, velocity, config.unit_string
        );
        Ok(())
    }

    async fn set_acceleration(&mut self, axis: usize, acceleration: f64) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!(
                "Cannot set acceleration from state: {:?}",
                self.state
            ));
        }

        self.validate_axis(axis)?;

        if acceleration <= 0.0 {
            return Err(anyhow!("Acceleration must be positive: {}", acceleration));
        }

        self.send(&format!("{}AC{}", axis, acceleration))
            .await
            .context("Failed to set acceleration")?;

        self.axis_configs[axis - 1].acceleration = acceleration;
        let config = self.get_axis_config(axis)?;
        info!(
            "Set ESP300 axis {} acceleration to {} {}/s²",
            axis, acceleration, config.unit_string
        );
        Ok(())
    }

    async fn home_axis(&mut self, axis: usize) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!("Cannot home from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        self.send(&format!("{}OR", axis))
            .await
            .context("Failed to home axis")?;

        info!("ESP300 axis {} homing", axis);
        Ok(())
    }

    async fn stop_axis(&mut self, axis: usize) -> Result<()> {
        self.validate_axis(axis)?;

        self.send(&format!("{}ST", axis))
            .await
            .context("Failed to stop axis")?;

        info!("ESP300 axis {} stopped", axis);
        Ok(())
    }

    async fn home_all(&mut self) -> Result<()> {
        if self.state != InstrumentState::Idle {
            return Err(anyhow!("Cannot home from state: {:?}", self.state));
        }

        for axis in 1..=self.num_axes {
            self.home_axis(axis).await?;
        }

        info!("ESP300 all axes homing");
        Ok(())
    }

    async fn stop_all(&mut self) -> Result<()> {
        for axis in 1..=self.num_axes {
            // Continue stopping other axes even if one fails
            let _ = self.stop_axis(axis).await;
        }

        info!("ESP300 all axes stopped");
        Ok(())
    }

    fn get_units(&self, axis: usize) -> &str {
        if axis == 0 || axis > self.num_axes {
            "unknown"
        } else {
            &self.axis_configs[axis - 1].unit_string
        }
    }

    fn get_position_range(&self, axis: usize) -> (f64, f64) {
        if axis == 0 || axis > self.num_axes {
            (0.0, 0.0)
        } else {
            let config = &self.axis_configs[axis - 1];
            (config.min_position, config.max_position)
        }
    }

    async fn is_moving(&self, axis: usize) -> Result<bool> {
        if self.state != InstrumentState::Idle && self.state != InstrumentState::Running {
            return Err(anyhow!(
                "Cannot check motion status from state: {:?}",
                self.state
            ));
        }

        self.validate_axis(axis)?;

        let response = self
            .query(&format!("{}MD?", axis))
            .await
            .context("Failed to query motion status")?;

        // MD? returns 0 if not moving, non-zero if moving
        let status: i32 = response
            .parse()
            .with_context(|| format!("Failed to parse motion status: {}", response))?;

        Ok(status != 0)
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_esp300_creation() {
        let instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            3,
        );

        assert_eq!(instrument.id(), "test_motion");
        assert_eq!(instrument.instrument_type(), "esp300_v2");
        assert_eq!(instrument.state(), InstrumentState::Disconnected);
        assert_eq!(instrument.num_axes(), 3);
    }

    #[test]
    fn test_axis_validation() {
        let instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            3,
        );

        assert!(instrument.validate_axis(0).is_err());
        assert!(instrument.validate_axis(1).is_ok());
        assert!(instrument.validate_axis(2).is_ok());
        assert!(instrument.validate_axis(3).is_ok());
        assert!(instrument.validate_axis(4).is_err());
    }

    #[test]
    fn test_axis_configuration() {
        let mut instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            3,
        );

        // Configure axis 1
        instrument
            .configure_axis(1, 1, 10.0, 20.0, -50.0, 50.0)
            .unwrap();
        let config = instrument.get_axis_config(1).unwrap();
        assert_eq!(config.units, 1);
        assert_eq!(config.unit_string, "mm");
        assert_eq!(config.velocity, 10.0);
        assert_eq!(config.acceleration, 20.0);
        assert_eq!(config.min_position, -50.0);
        assert_eq!(config.max_position, 50.0);

        // Invalid axis
        assert!(instrument
            .configure_axis(0, 1, 10.0, 20.0, 0.0, 100.0)
            .is_err());
        assert!(instrument
            .configure_axis(4, 1, 10.0, 20.0, 0.0, 100.0)
            .is_err());
    }

    #[test]
    fn test_unit_conversion() {
        let mut instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            3,
        );

        instrument
            .configure_axis(1, 1, 10.0, 20.0, 0.0, 100.0)
            .unwrap();
        assert_eq!(instrument.get_units(1), "mm");

        instrument
            .configure_axis(2, 2, 10.0, 20.0, 0.0, 360.0)
            .unwrap();
        assert_eq!(instrument.get_units(2), "deg");

        instrument
            .configure_axis(3, 3, 10.0, 20.0, 0.0, 6.28)
            .unwrap();
        assert_eq!(instrument.get_units(3), "rad");

        // Invalid axis
        assert_eq!(instrument.get_units(0), "unknown");
        assert_eq!(instrument.get_units(4), "unknown");
    }

    #[test]
    fn test_position_range() {
        let mut instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            2,
        );

        instrument
            .configure_axis(1, 1, 10.0, 20.0, -50.0, 50.0)
            .unwrap();
        assert_eq!(instrument.get_position_range(1), (-50.0, 50.0));

        instrument
            .configure_axis(2, 2, 5.0, 10.0, 0.0, 360.0)
            .unwrap();
        assert_eq!(instrument.get_position_range(2), (0.0, 360.0));

        // Invalid axis
        assert_eq!(instrument.get_position_range(0), (0.0, 0.0));
        assert_eq!(instrument.get_position_range(3), (0.0, 0.0));
    }

    #[test]
    fn test_polling_rate_configuration() {
        let mut instrument = ESP300V2::new(
            "test_motion".to_string(),
            "/dev/ttyUSB0".to_string(),
            19200,
            3,
        );

        assert_eq!(instrument.polling_rate_hz, 5.0);

        instrument.set_polling_rate_hz(10.0).unwrap();
        assert_eq!(instrument.polling_rate_hz, 10.0);

        // Invalid rate
        assert!(instrument.set_polling_rate_hz(0.0).is_err());
        assert!(instrument.set_polling_rate_hz(-1.0).is_err());
    }

    // Note: Integration tests with actual hardware would go in tests/ directory
    // These unit tests verify the structure and basic functionality without hardware
}