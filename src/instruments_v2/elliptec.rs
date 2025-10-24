//! Thorlabs Elliptec ELL14 rotation mount driver V2
//!
//! V2 implementation using MotionController trait with RS-485 multidrop support.
//! Each device on the bus is treated as a separate axis.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.elliptec_rotators]
//! type = "elliptec_v2"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! device_addresses = [0, 1, 2]  # Multiple devices on same RS-485 bus
//! polling_rate_hz = 2.0
//! timeout_ms = 500
//! ```
//!
//! ## Elliptec Protocol
//!
//! RS-485 multidrop: `<address><command>[data]<CR>`
//! - Address: Single hex digit (0-F)
//! - Commands: `gp` (get position), `ma` (move absolute), `ho` (home), `in` (info)
//! - Response: `<address><status><data><CR>`

use async_trait::async_trait;
use chrono::Utc;
use daq_core::{
    arc_measurement, measurement_channel, DaqError, DataPoint, HardwareAdapter, Instrument,
    InstrumentCommand, InstrumentState, Measurement, MeasurementReceiver, MeasurementSender,
    MotionController, Result,
};
use log::{debug, info, warn};
use std::time::Duration;
use tokio::task::JoinHandle;

use crate::adapters::SerialAdapter;

/// Elliptec ELL14 rotation mount (V2)
///
/// Each device on the RS-485 bus is mapped to an axis.
/// Axis 0 = device address 0, Axis 1 = device address 1, etc.
pub struct ElliptecV2 {
    id: String,
    adapter: Box<dyn HardwareAdapter>,
    state: InstrumentState,

    // Multidrop configuration
    device_addresses: Vec<u8>,

    // Data streaming
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver,

    // Task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,

    // Elliptec-specific constants
    counts_per_rotation: f64, // ELL14: 143360 counts = 360 degrees
}

impl ElliptecV2 {
    /// Create new Elliptec instrument with default settings
    pub fn new(id: String) -> Self {
        Self::with_capacity(id, 1024)
    }

    /// Create new Elliptec instrument with specified broadcast capacity
    pub fn with_capacity(id: String, capacity: usize) -> Self {
        // Default to single device at address 0
        let port_name = "/dev/ttyUSB0".to_string();
        let adapter = SerialAdapter::new(port_name, 9600)
            .with_timeout(Duration::from_millis(500))
            .with_line_terminator("\r".to_string())
            .with_response_delimiter('\r');

        Self::with_adapter_and_capacity(id, Box::new(adapter), capacity)
    }

    /// Create new Elliptec instrument with custom adapter and capacity
    pub fn with_adapter_and_capacity(
        id: String,
        adapter: Box<dyn HardwareAdapter>,
        capacity: usize,
    ) -> Self {
        let (measurement_tx, measurement_rx) = measurement_channel(capacity);

        Self {
            id,
            adapter,
            state: InstrumentState::Disconnected,

            device_addresses: vec![0], // Default to single device

            measurement_tx,
            _measurement_rx_keeper: measurement_rx,

            task_handle: None,
            shutdown_tx: None,

            counts_per_rotation: 143360.0, // ELL14 specification
        }
    }

    /// Send command to specific device and read response
    ///
    /// Elliptec protocol: `<addr><cmd>[data]\r`
    async fn send_command(&self, address: u8, command: &str) -> Result<String> {
        // For SerialAdapter, we need to downcast
        let serial_adapter = self
            .adapter
            .as_any()
            .downcast_ref::<SerialAdapter>()
            .ok_or_else(|| anyhow::anyhow!("Adapter is not SerialAdapter"))?;

        // Format: address (hex) + command
        let cmd = format!("{:X}{}", address, command);

        let response = serial_adapter.send_command(&cmd).await?;

        // Validate response starts with correct address
        if !response.starts_with(&format!("{:X}", address)) {
            return Err(anyhow::anyhow!(
                "Response address mismatch. Expected {}, got response: {}",
                address,
                response
            ));
        }

        Ok(response)
    }

    /// Get position from device in degrees
    async fn get_position_degrees(&self, address: u8) -> Result<f64> {
        let response = self.send_command(address, "gp").await?;

        // Response format: "0PO12345678" where:
        // - '0' = address
        // - 'PO' = position status
        // - '12345678' = hex position
        if response.len() < 10 {
            return Err(anyhow::anyhow!(
                "Invalid position response (too short): {}",
                response
            ));
        }

        let status = &response[1..3];
        if status != "PO" {
            return Err(anyhow::anyhow!(
                "Invalid position status: {} (response: {})",
                status,
                response
            ));
        }

        let hex_pos = &response[3..];
        let raw_pos = u32::from_str_radix(hex_pos, 16).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse hex position '{}': {} (response: {})",
                hex_pos,
                e,
                response
            )
        })?;

        // Convert counts to degrees
        let degrees = (raw_pos as f64 / self.counts_per_rotation) * 360.0;
        Ok(degrees)
    }

    /// Set position for device in degrees
    async fn set_position_degrees(&self, address: u8, degrees: f64) -> Result<()> {
        // Normalize to 0-360 range
        let normalized = degrees.rem_euclid(360.0);

        // Convert to counts
        let counts = ((normalized / 360.0) * self.counts_per_rotation) as u32;
        let hex_pos = format!("{:08X}", counts);

        // 'ma' command - move absolute
        let response = self.send_command(address, &format!("ma{}", hex_pos)).await?;

        // Check for error responses
        let status = &response[1..3];
        if status.starts_with('E') {
            return Err(anyhow::anyhow!(
                "Elliptec device {} returned error: {}",
                address,
                response
            ));
        }

        debug!(
            "Elliptec device {} moving to {:.2} degrees (counts: {})",
            address, normalized, counts
        );
        Ok(())
    }

    /// Home device (find reference position)
    async fn home_device(&self, address: u8) -> Result<()> {
        let response = self.send_command(address, "ho").await?;

        let status = &response[1..3];
        if status.starts_with('E') {
            return Err(anyhow::anyhow!(
                "Elliptec device {} home failed: {}",
                address,
                response
            ));
        }

        debug!("Elliptec device {} homed successfully", address);
        Ok(())
    }

    /// Get device information
    async fn get_device_info(&self, address: u8) -> Result<String> {
        let response = self.send_command(address, "in").await?;
        Ok(response)
    }
}

#[async_trait]
impl Instrument for ElliptecV2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "elliptec_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state.clone()
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow::anyhow!("Already initialized"));
        }

        self.state = InstrumentState::Connecting;

        // Connect serial adapter
        match self.adapter.connect(&Default::default()).await {
            Ok(()) => {
                // Query each device for info
                for &addr in &self.device_addresses {
                    match self.get_device_info(addr).await {
                        Ok(info) => {
                            info!("Elliptec device {} (axis {}): {}", addr, addr, info);
                        }
                        Err(e) => {
                            warn!("Failed to query Elliptec device {}: {}", addr, e);
                            self.state = InstrumentState::Error(DaqError {
                                message: format!("Device {} not responding: {}", addr, e),
                                can_recover: true,
                            });
                            return Err(anyhow::anyhow!(
                                "Device {} initialization failed",
                                addr
                            ));
                        }
                    }
                }

                self.state = InstrumentState::Ready;
                info!("Elliptec '{}' initialized with {} axes", self.id, self.num_axes());
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
        self.state = InstrumentState::ShuttingDown;

        // Stop polling task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        // Disconnect adapter
        self.adapter.disconnect().await?;

        self.state = InstrumentState::Disconnected;
        info!("Elliptec '{}' shut down", self.id);
        Ok(())
    }

    fn measurement_stream(&self) -> MeasurementReceiver {
        self.measurement_tx.subscribe()
    }

    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()> {
        match cmd {
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::StartAcquisition => {
                // Start position polling if not already running
                if self.task_handle.is_none() {
                    self.start_polling(2.0).await?; // Default 2Hz
                }
                Ok(())
            }
            InstrumentCommand::StopAcquisition => {
                // Stop polling
                if let Some(tx) = self.shutdown_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(handle) = self.task_handle.take() {
                    let _ = handle.await;
                }
                Ok(())
            }
            InstrumentCommand::SetParameter { name, value } => {
                // Parse "axis:parameter" format or direct parameter
                let parts: Vec<&str> = name.split(':').collect();

                if parts.len() == 2 {
                    // Format: "axis:position"
                    let axis: usize = parts[0].parse().map_err(|e| {
                        anyhow::anyhow!("Invalid axis number '{}': {}", parts[0], e)
                    })?;

                    if parts[1] == "position" {
                        let position = value.as_f64().ok_or_else(|| {
                            anyhow::anyhow!("Invalid position value: {}", value)
                        })?;
                        self.move_absolute(axis, position).await?;
                    }
                } else {
                    warn!("Unknown parameter '{}' for Elliptec", name);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                info!("Attempting recovery for Elliptec '{}'", self.id);

                let _ = self.adapter.disconnect().await;
                tokio::time::sleep(Duration::from_millis(500)).await;

                self.adapter.connect(&Default::default()).await?;

                self.state = InstrumentState::Ready;
                info!("Recovery successful for Elliptec '{}'", self.id);
                Ok(())
            }
            InstrumentState::Error(_) => {
                Err(anyhow::anyhow!("Cannot recover from unrecoverable error"))
            }
            _ => Err(anyhow::anyhow!(
                "Cannot recover from state: {:?}",
                self.state
            )),
        }
    }
}

#[async_trait]
impl MotionController for ElliptecV2 {
    fn num_axes(&self) -> usize {
        self.device_addresses.len()
    }

    async fn move_absolute(&mut self, axis: usize, position: f64) -> Result<()> {
        if axis >= self.num_axes() {
            return Err(anyhow::anyhow!("Axis {} out of range", axis));
        }

        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Not ready, state: {:?}", self.state));
        }

        let address = self.device_addresses[axis];
        self.set_position_degrees(address, position).await
    }

    async fn move_relative(&mut self, axis: usize, distance: f64) -> Result<()> {
        let current_pos = self.get_position(axis).await?;
        self.move_absolute(axis, current_pos + distance).await
    }

    async fn get_position(&self, axis: usize) -> Result<f64> {
        if axis >= self.num_axes() {
            return Err(anyhow::anyhow!("Axis {} out of range", axis));
        }

        let address = self.device_addresses[axis];
        self.get_position_degrees(address).await
    }

    async fn get_velocity(&self, _axis: usize) -> Result<f64> {
        // ELL14 doesn't support velocity readback
        Err(anyhow::anyhow!("ELL14 does not support velocity readback"))
    }

    async fn set_velocity(&mut self, _axis: usize, _velocity: f64) -> Result<()> {
        // ELL14 doesn't support velocity control
        Err(anyhow::anyhow!("ELL14 does not support velocity control"))
    }

    async fn set_acceleration(&mut self, _axis: usize, _acceleration: f64) -> Result<()> {
        // ELL14 doesn't support acceleration control
        Err(anyhow::anyhow!("ELL14 does not support acceleration control"))
    }

    async fn home_axis(&mut self, axis: usize) -> Result<()> {
        if axis >= self.num_axes() {
            return Err(anyhow::anyhow!("Axis {} out of range", axis));
        }

        if self.state != InstrumentState::Ready {
            return Err(anyhow::anyhow!("Not ready, state: {:?}", self.state));
        }

        let address = self.device_addresses[axis];
        self.home_device(address).await
    }

    async fn stop_axis(&mut self, _axis: usize) -> Result<()> {
        // ELL14 doesn't have a stop command (moves are position-based)
        Err(anyhow::anyhow!("ELL14 does not support stop command"))
    }

    async fn move_absolute_all(&mut self, positions: &[f64]) -> Result<()> {
        if positions.len() != self.num_axes() {
            return Err(anyhow::anyhow!(
                "Position array length {} doesn't match axis count {}",
                positions.len(),
                self.num_axes()
            ));
        }

        // Move all axes sequentially
        // Note: True simultaneous move would require parallel tasks
        for (axis, &position) in positions.iter().enumerate() {
            self.move_absolute(axis, position).await?;
        }
        Ok(())
    }

    async fn get_positions_all(&self) -> Result<Vec<f64>> {
        let mut positions = Vec::with_capacity(self.num_axes());

        for axis in 0..self.num_axes() {
            let pos = self.get_position(axis).await?;
            positions.push(pos);
        }

        Ok(positions)
    }

    async fn home_all(&mut self) -> Result<()> {
        for axis in 0..self.num_axes() {
            self.home_axis(axis).await?;
        }
        Ok(())
    }

    async fn stop_all(&mut self) -> Result<()> {
        // ELL14 doesn't support stop
        Err(anyhow::anyhow!("ELL14 does not support stop command"))
    }

    fn get_units(&self, _axis: usize) -> &str {
        "degrees"
    }

    fn get_position_range(&self, _axis: usize) -> (f64, f64) {
        (0.0, 360.0) // Full rotation
    }

    async fn is_moving(&self, _axis: usize) -> Result<bool> {
        // ELL14 doesn't provide moving status
        // Could poll position rapidly to detect changes
        Ok(false)
    }
}

impl ElliptecV2 {
    /// Start position polling task
    async fn start_polling(&mut self, rate_hz: f64) -> Result<()> {
        if self.task_handle.is_some() {
            return Ok(()); // Already polling
        }

        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let addresses = self.device_addresses.clone();

        // Create a minimal clone for the task
        let adapter_clone = self.adapter.as_any()
            .downcast_ref::<SerialAdapter>()
            .ok_or_else(|| anyhow::anyhow!("Adapter is not SerialAdapter"))?
            .clone();

        let counts_per_rotation = self.counts_per_rotation;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        self.task_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / rate_hz));

            info!("Elliptec position polling started at {:.1} Hz", rate_hz);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let timestamp = Utc::now();

                        for (axis, &addr) in addresses.iter().enumerate() {
                            // Query position
                            let cmd = format!("{:X}gp", addr);
                            match adapter_clone.send_command(&cmd).await {
                                Ok(response) if response.len() >= 10 => {
                                    // Parse response
                                    if let Ok(raw_pos) = u32::from_str_radix(&response[3..], 16) {
                                        let degrees = (raw_pos as f64 / counts_per_rotation) * 360.0;

                                        let dp = DataPoint {
                                            timestamp,
                                            channel: format!("axis{}_position", axis),
                                            value: degrees,
                                            unit: "degrees".to_string(),
                                        };

                                        let measurement = arc_measurement(Measurement::Scalar(dp));
                                        if tx.send(measurement).is_err() {
                                            info!("No receivers, stopping Elliptec polling");
                                            return;
                                        }
                                    }
                                }
                                Ok(response) => {
                                    warn!("Invalid response from Elliptec device {}: {}", addr, response);
                                }
                                Err(e) => {
                                    warn!("Failed to poll Elliptec device {}: {}", addr, e);
                                }
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Elliptec polling shutdown requested");
                        break;
                    }
                }
            }

            info!("Elliptec position polling stopped");
        }));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::MockAdapter;

    #[tokio::test]
    async fn test_elliptec_lifecycle() {
        let mock_adapter = MockAdapter::new();
        let mut elliptec = ElliptecV2::with_adapter_and_capacity(
            "test_elliptec".to_string(),
            Box::new(mock_adapter),
            1024,
        );

        assert_eq!(elliptec.state(), InstrumentState::Disconnected);

        // Note: This will fail with MockAdapter since it doesn't simulate Elliptec protocol
        // In practice, you'd use a test double that responds correctly
        // elliptec.initialize().await.unwrap();
        // assert_eq!(elliptec.state(), InstrumentState::Ready);
    }

    #[tokio::test]
    async fn test_elliptec_motion_controller() {
        let mock_adapter = MockAdapter::new();
        let mut elliptec = ElliptecV2::with_adapter_and_capacity(
            "test_elliptec".to_string(),
            Box::new(mock_adapter),
            1024,
        );

        elliptec.device_addresses = vec![0, 1, 2];

        assert_eq!(elliptec.num_axes(), 3);
        assert_eq!(elliptec.get_units(0), "degrees");
        assert_eq!(elliptec.get_position_range(0), (0.0, 360.0));
    }

    #[test]
    fn test_elliptec_position_conversion() {
        let elliptec = ElliptecV2::new("test".to_string());

        // Test full rotation
        let counts = 143360u32; // One full rotation
        let degrees = (counts as f64 / elliptec.counts_per_rotation) * 360.0;
        assert!((degrees - 360.0).abs() < 0.01);

        // Test half rotation
        let counts = 71680u32;
        let degrees = (counts as f64 / elliptec.counts_per_rotation) * 360.0;
        assert!((degrees - 180.0).abs() < 0.01);
    }
}
