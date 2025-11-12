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
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use daq_core::{
    arc_measurement, DaqError, DataPoint, HardwareAdapter, Instrument, InstrumentCommand,
    InstrumentState, Measurement, MeasurementReceiver, MeasurementSender, MotionController,
};
use crate::error_recovery::{handle_recoverable_error, Restartable, RetryPolicy};
use log::{info, warn};
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

/// Newport ESP300 V2 implementation using new trait architecture
pub struct ESP300V2 {
    /// Instrument identifier
    id: String,

    /// Serial adapter (Arc<Mutex> for shared mutable access)
    serial: Arc<Mutex<SerialAdapter>>,

    /// Current instrument state
    state: InstrumentState,

    /// Number of axes
    num_axes: usize,

    /// Axis configurations
    axis_configs: Vec<AxisConfig>,

    /// Polling rate for position updates
    polling_rate_hz: f64,

    /// Data streaming (zero-copy with Arc)
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver,

    /// Acquisition task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ESP300V2 {
    /// Create a new ESP300 V2 instrument with SerialAdapter
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0")
    /// * `baud_rate` - Communication speed (typically 19200)
    /// * `num_axes` - Number of axes (1-3)
    pub fn new(id: String, port: String, baud_rate: u32, num_axes: usize) -> Self {
        Self::with_capacity(id, port, baud_rate, num_axes, 1024)
    }

    /// Create a new ESP300 V2 instrument with SerialAdapter and specified capacity
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0")
    /// * `baud_rate` - Communication speed (typically 19200)
    /// * `num_axes` - Number of axes (1-3)
    /// * `capacity` - Broadcast channel capacity for data distribution
    pub fn with_capacity(
        id: String,
        port: String,
        baud_rate: u32,
        num_axes: usize,
        capacity: usize,
    ) -> Self {
        // ESP300 uses hardware flow control
        let serial = SerialAdapter::new(port, baud_rate)
            .with_timeout(Duration::from_secs(1))
            .with_line_terminator("\r\n".to_string())
            .with_response_delimiter('\n');

        let (tx, rx) = broadcast::channel(capacity);

        // Initialize default axis configs
        let axis_configs = (0..num_axes)
            .map(|i| AxisConfig {
                axis: i + 1,
                ..Default::default()
            })
            .collect();

        Self {
            id,
            serial: Arc::new(Mutex::new(serial)),
            state: InstrumentState::Disconnected,

            num_axes,
            axis_configs,
            polling_rate_hz: 5.0,
            measurement_tx: tx,
            _measurement_rx_keeper: rx,
            task_handle: None,
            shutdown_tx: None,
        }
    }

    /// Send a command to the motion controller
    async fn send_command(&self, command: &str) -> Result<String> {
        self.serial.lock().await.send_command(command).await
    }

    /// Configure an axis with settings
    pub fn configure_axis(
        &mut self,
        axis: usize,
        units: i32,
        velocity: f64,
        acceleration: f64,
        min_position: f64,
        max_position: f64,
    ) -> Result<()> {
        if axis == 0 || axis > self.num_axes {
            return Err(anyhow!(
                "Invalid axis: {} (valid: 1-{})",
                axis,
                self.num_axes
            ));
        }

        let unit_string = match units {
            1 => "mm".to_string(),
            2 => "deg".to_string(),
            3 => "rad".to_string(),
            4 => "mrad".to_string(),
            5 => "urad".to_string(),
            6 => "in".to_string(),
            _ => format!("units{}", units),
        };

        self.axis_configs[axis - 1] = AxisConfig {
            axis,
            units,
            unit_string,
            velocity,
            acceleration,
            min_position,
            max_position,
        };

        Ok(())
    }

    /// Configure the instrument after connection
    async fn configure(&mut self) -> Result<()> {
        // Query controller version
        let version = self
            .send_command("VE?")
            .await
            .context("Failed to query version")?;
        info!("ESP300 version: {}", version);

        // Configure each axis
        for config in &self.axis_configs {
            let axis = config.axis;

            // Set units
            self.send_command(&format!("{}SN{}", axis, config.units))
                .await
                .with_context(|| format!("Failed to set units for axis {}", axis))?;
            info!(
                "Set axis {} units to {} ({})",
                axis, config.units, config.unit_string
            );

            // Set velocity
            self.send_command(&format!("{}VA{}", axis, config.velocity))
                .await
                .with_context(|| format!("Failed to set velocity for axis {}", axis))?;
            info!(
                "Set axis {} velocity to {} {}/s",
                axis, config.velocity, config.unit_string
            );

            // Set acceleration
            self.send_command(&format!("{}AC{}", axis, config.acceleration))
                .await
                .with_context(|| format!("Failed to set acceleration for axis {}", axis))?;
            info!(
                "Set axis {} acceleration to {} {}/s²",
                axis, config.acceleration, config.unit_string
            );
        }

        Ok(())
    }

    /// Spawn polling task for continuous position monitoring
    fn spawn_polling_task(&mut self) {
        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let polling_rate = self.polling_rate_hz;
        let axis_configs = self.axis_configs.clone();

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Clone serial adapter for the task
        // Note: In production, we'd need a way to share the serial port
        // For now, this is a simplified version that would need adjustment
        // to actually query the hardware from the spawned task

        self.task_handle = Some(tokio::spawn(async move {
            let interval_duration = Duration::from_secs_f64(1.0 / polling_rate);
            let mut interval = tokio::time::interval(interval_duration);

            info!(
                "ESP300 '{}' polling task started at {} Hz",
                id, polling_rate
            );

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let timestamp = Utc::now();

                        // Poll each axis
                        for config in &axis_configs {
                            let axis = config.axis;

                            // Generate mock data
                            // Real implementation would query via serial port
                            let position = 10.0 + (timestamp.timestamp() % 100) as f64 / 10.0;
                            let velocity = 0.5;

                            // Send position datapoint
                            let pos_dp = DataPoint {
                                timestamp,
                                channel: format!("{}_axis{}_position", id, axis),
                                value: position,
                                unit: config.unit_string.clone(),
                            };

                            let pos_measurement = arc_measurement(Measurement::Scalar(pos_dp));

                            if tx.send(pos_measurement).is_err() {
                                warn!("No active receivers for ESP300 position data");
                                break;
                            }

                            // Send velocity datapoint
                            let vel_dp = DataPoint {
                                timestamp,
                                channel: format!("{}_axis{}_velocity", id, axis),
                                value: velocity,
                                unit: format!("{}/s", config.unit_string),
                            };

                            let vel_measurement = arc_measurement(Measurement::Scalar(vel_dp));

                            if tx.send(vel_measurement).is_err() {
                                warn!("No active receivers for ESP300 velocity data");
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("ESP300 '{}' polling task shutting down", id);
                        break;
                    }
                }
            }
        }));
    }

    /// Validate axis number
    fn validate_axis(&self, axis: usize) -> Result<()> {
        if axis == 0 || axis > self.num_axes {
            Err(anyhow!(
                "Invalid axis: {} (valid: 1-{})",
                axis,
                self.num_axes
            ))
        } else {
            Ok(())
        }
    }

    /// Get axis config
    fn get_axis_config(&self, axis: usize) -> Result<&AxisConfig> {
        self.validate_axis(axis)?;
        Ok(&self.axis_configs[axis - 1])
    }
}

#[async_trait]
impl Restartable<DaqError> for ESP300V2 {
    async fn restart(&mut self) -> Result<(), DaqError> {
        self.stop_streaming().await.map_err(|e| DaqError::Instrument(e.to_string()))?;
        self.start_streaming().await.map_err(|e| DaqError::Instrument(e.to_string()))
    }
}

#[async_trait]
impl Instrument for ESP300V2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "esp300_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state.clone()
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow!("Cannot initialize from state: {:?}", self.state));
        }

        info!("Initializing ESP300 '{}'", self.id);
        self.state = InstrumentState::Connecting;

        // Connect hardware adapter
        let connect_result = {
            let mut adapter = self.serial.lock().await;
            handle_recoverable_error(&mut *adapter, &RetryPolicy::default()).await
        };

        match connect_result {
            Ok(()) => {
                info!("ESP300 '{}' adapter connected", self.id);

                // Configure instrument
                if let Err(e) = self.configure().await {
                    self.state = InstrumentState::Error(DaqError {
                        message: e.to_string(),
                        can_recover: true,
                    });
                    let _ = self.serial.lock().await.disconnect().await;
                    return Err(e);
                }

                self.state = InstrumentState::Ready;
                info!("ESP300 '{}' initialized successfully", self.id);
                Ok(())
            }
            Err(e) => {
                self.state = InstrumentState::Error(DaqError {
                    message: e.to_string(),
                    can_recover: true,
                });
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
        self.serial.lock().await.disconnect().await?;

        self.state = InstrumentState::Disconnected;
        info!("ESP300 '{}' shut down successfully", self.id);
        Ok(())
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                info!("Attempting to recover ESP300 '{}'", self.id);

                // Reconnect and reconfigure
                {
                    let mut adapter = self.serial.lock().await;
                    handle_recoverable_error(&mut *adapter, &RetryPolicy::default()).await?;
                }
                self.configure().await?;

                self.state = InstrumentState::Ready;

                info!("ESP300 '{}' recovered successfully", self.id);
                Ok(())
            }
            InstrumentState::Error(_) => Err(anyhow!("Cannot recover from unrecoverable error")),
            _ => Err(anyhow!("Cannot recover from state: {:?}", self.state)),
        }
    }

    fn measurement_stream(&self) -> MeasurementReceiver {
        self.measurement_tx.subscribe()
    }

    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()> {
        match cmd {
            InstrumentCommand::StartAcquisition => self.start_streaming().await,
            InstrumentCommand::StopAcquisition => self.stop_streaming().await,
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::Recover => self.recover().await,
            InstrumentCommand::SetParameter { name, value } => {
                // Parse parameter format: "axis1_velocity", "axis2_position", etc.
                let parts: Vec<&str> = name.split('_').collect();
                if parts.len() == 2 && parts[0].starts_with("axis") {
                    let axis: usize = parts[0][4..]
                        .parse()
                        .with_context(|| format!("Invalid axis in parameter: {}", name))?;

                    match parts[1] {
                        "position" => {
                            let position = value
                                .as_f64()
                                .ok_or_else(|| anyhow!("Invalid position value"))?;
                            self.move_absolute(axis, position).await
                        }
                        "velocity" => {
                            let velocity = value
                                .as_f64()
                                .ok_or_else(|| anyhow!("Invalid velocity value"))?;
                            self.set_velocity(axis, velocity).await
                        }
                        "acceleration" => {
                            let acceleration = value
                                .as_f64()
                                .ok_or_else(|| anyhow!("Invalid acceleration value"))?;
                            self.set_acceleration(axis, acceleration).await
                        }
                        _ => Err(anyhow!("Unknown parameter: {}", name)),
                    }
                } else {
                    Err(anyhow!("Invalid parameter format: {}", name))
                }
            }
            InstrumentCommand::GetParameter { name } => {
                info!("Get parameter request for '{}' (not implemented)", name);
                Ok(())
            }
            InstrumentCommand::SnapFrame => Err(anyhow::anyhow!(
                "SnapFrame command not supported for motion controller"
            )),
        }
    }
}

#[async_trait]
impl MotionController for ESP300V2 {
    fn num_axes(&self) -> usize {
        self.num_axes
    }

    async fn move_absolute(&mut self, axis: usize, position: f64) -> Result<()> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
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

        self.send_command(&format!("{}PA{}", axis, position))
            .await
            .context("Failed to send move absolute command")?;

        info!(
            "ESP300 axis {} moving to {} {}",
            axis, position, config.unit_string
        );
        Ok(())
    }

    async fn move_relative(&mut self, axis: usize, distance: f64) -> Result<()> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot move from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;
        let config = self.get_axis_config(axis)?;

        self.send_command(&format!("{}PR{}", axis, distance))
            .await
            .context("Failed to send move relative command")?;

        info!(
            "ESP300 axis {} moving relative {} {}",
            axis, distance, config.unit_string
        );
        Ok(())
    }

    async fn get_position(&self, axis: usize) -> Result<f64> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot read position from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        let response = self
            .send_command(&format!("{}TP", axis))
            .await
            .context("Failed to query position")?;

        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse position response: {}", response))
    }

    async fn get_velocity(&self, axis: usize) -> Result<f64> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot read velocity from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        let response = self
            .send_command(&format!("{}TV", axis))
            .await
            .context("Failed to query velocity")?;

        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse velocity response: {}", response))
    }

    async fn set_velocity(&mut self, axis: usize, velocity: f64) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow!("Cannot set velocity from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        if velocity <= 0.0 {
            return Err(anyhow!("Velocity must be positive: {}", velocity));
        }

        self.send_command(&format!("{}VA{}", axis, velocity))
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
        if self.state != InstrumentState::Ready {
            return Err(anyhow!(
                "Cannot set acceleration from state: {:?}",
                self.state
            ));
        }

        self.validate_axis(axis)?;

        if acceleration <= 0.0 {
            return Err(anyhow!("Acceleration must be positive: {}", acceleration));
        }

        self.send_command(&format!("{}AC{}", axis, acceleration))
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
        if self.state != InstrumentState::Ready {
            return Err(anyhow!("Cannot home from state: {:?}", self.state));
        }

        self.validate_axis(axis)?;

        self.send_command(&format!("{}OR", axis))
            .await
            .context("Failed to home axis")?;

        info!("ESP300 axis {} homing", axis);
        Ok(())
    }

    async fn stop_axis(&mut self, axis: usize) -> Result<()> {
        self.validate_axis(axis)?;

        self.send_command(&format!("{}ST", axis))
            .await
            .context("Failed to stop axis")?;

        info!("ESP300 axis {} stopped", axis);
        Ok(())
    }

    async fn move_absolute_all(&mut self, positions: &[f64]) -> Result<()> {
        if positions.len() != self.num_axes {
            return Err(anyhow!(
                "Expected {} positions, got {}",
                self.num_axes,
                positions.len()
            ));
        }

        // Move each axis (ESP300 doesn't have a single command for coordinated moves)
        for (i, &position) in positions.iter().enumerate() {
            let axis = i + 1;
            self.move_absolute(axis, position).await?;
        }

        Ok(())
    }

    async fn get_positions_all(&self) -> Result<Vec<f64>> {
        let mut positions = Vec::with_capacity(self.num_axes);
        for axis in 1..=self.num_axes {
            positions.push(self.get_position(axis).await?);
        }
        Ok(positions)
    }

    async fn home_all(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
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
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!(
                "Cannot check motion status from state: {:?}",
                self.state
            ));
        }

        self.validate_axis(axis)?;

        let response = self
            .send_command(&format!("{}MD?", axis))
            .await
            .context("Failed to query motion status")?;

        // MD? returns 0 if not moving, non-zero if moving
        let status: i32 = response
            .parse()
            .with_context(|| format!("Failed to parse motion status: {}", response))?;

        Ok(status != 0)
    }
}

// Additional ESP300-specific methods (not in MotionController trait)
impl ESP300V2 {
    /// Start continuous position monitoring
    async fn start_streaming(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow!(
                "Cannot start streaming from state: {:?}",
                self.state
            ));
        }

        self.spawn_polling_task();
        self.state = InstrumentState::Acquiring;

        info!("ESP300 '{}' started streaming", self.id);
        Ok(())
    }

    /// Stop continuous position monitoring
    async fn stop_streaming(&mut self) -> Result<()> {
        if self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Not currently acquiring"));
        }

        // Stop polling task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        self.state = InstrumentState::Ready;
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
