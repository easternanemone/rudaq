//! Spectra-Physics MaiTai Tunable Ti:Sapphire Laser V2 Implementation
//!
//! This module provides a V2 implementation of the MaiTai laser
//! using the new three-tier architecture:
//! - SerialAdapter for RS-232 communication
//! - Instrument trait for state management
//! - TunableLaser trait for domain-specific methods
//!
//! ## Configuration Example
//!
//! ```toml
//! [instruments.maitai_laser]
//! type = "maitai_v2"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! wavelength = 800.0  # nm (default tuning)
//! polling_rate_hz = 1.0
//! ```
//!
//! ## Wavelength Range
//! - Ti:Sapphire: 690-1040 nm typical
//! - MaiTai specific: 690-1040 nm (model dependent)
//!
//! ## Protocol
//! - Line terminator: `\r`
//! - Response delimiter: `\r`
//! - Commands: `*IDN?`, `WAVELENGTH:xxx`, `POWER?`, `SHUTTER:0/1`, etc.

use crate::adapters::SerialAdapter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use daq_core::{
    arc_measurement, DataPoint, HardwareAdapter, Instrument, InstrumentCommand, InstrumentState,
    Measurement, MeasurementReceiver, MeasurementSender, TunableLaser,
};
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;

/// MaiTai tunable laser V2 implementation using new trait architecture
pub struct MaiTaiV2 {
    /// Instrument identifier
    id: String,

    /// Serial adapter (Arc<Mutex> for shared mutable access)
    serial: Arc<Mutex<SerialAdapter>>,

    /// Current instrument state
    state: InstrumentState,


    /// Laser configuration
    wavelength_nm: f64,
    shutter_open: bool,
    laser_on: bool,
    polling_rate_hz: f64,

    /// Valid wavelength range for this model
    wavelength_min_nm: f64,
    wavelength_max_nm: f64,

    /// Data streaming (zero-copy with Arc)
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver,

    /// Monitoring task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MaiTaiV2 {
    /// Create a new MaiTai V2 instrument with SerialAdapter
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (typically 9600)
    pub fn new(id: String, port: String, baud_rate: u32) -> Self {
        let serial = SerialAdapter::new(port, baud_rate)
            .with_timeout(Duration::from_secs(2))
            .with_line_terminator("\r".to_string())
            .with_response_delimiter('\r');

        let (tx, rx) = broadcast::channel(1024);

        Self {
            id,
            serial: Arc::new(Mutex::new(serial)),
            state: InstrumentState::Disconnected,

            wavelength_nm: 800.0,
            shutter_open: false,
            laser_on: false,
            polling_rate_hz: 1.0,
            wavelength_min_nm: 690.0,
            wavelength_max_nm: 1040.0,
            measurement_tx: tx,
            _measurement_rx_keeper: rx,
            task_handle: None,
            shutdown_tx: None,
        }
    }

    /// Send a command to the laser
    async fn send_command(&self, command: &str) -> Result<String> {
        self.serial.lock().await.send_command(command).await
    }

    /// Query a numeric value from the laser
    async fn query_value(&self, command: &str) -> Result<f64> {
        let response = self.send_command(command).await?;

        // Remove command echo if present (format: "COMMAND:value")
        let value_str = response.split(':').last().unwrap_or(&response);

        value_str.trim().parse::<f64>()
            .with_context(|| format!("Failed to parse response '{}' as float", response))
    }

    /// Configure the instrument after connection
    async fn configure(&mut self) -> Result<()> {
        // Verify connection with identity query
        let id_response = self.send_command("*IDN?")
            .await
            .context("Failed to query instrument identity")?;

        info!("MaiTai identity: {}", id_response);

        // Set initial wavelength
        self.send_command(&format!("WAVELENGTH:{}", self.wavelength_nm))
            .await
            .context("Failed to set initial wavelength")?;

        info!("Set wavelength to {} nm", self.wavelength_nm);

        // Query current shutter state
        match self.query_value("SHUTTER?").await {
            Ok(val) => {
                self.shutter_open = val > 0.5;
                info!("Current shutter state: {}", if self.shutter_open { "open" } else { "closed" });
            }
            Err(e) => {
                warn!("Failed to query initial shutter state: {}", e);
            }
        }

        Ok(())
    }

    /// Spawn monitoring task for continuous parameter polling
    fn spawn_monitoring_task(&mut self) {
        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let polling_rate = self.polling_rate_hz;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Clone serial adapter for the monitoring task
        // Note: In production, we'd need a better way to share serial access
        // For now, this demonstrates the pattern. A real implementation might use
        // a command queue or shared serial access pattern.

        self.task_handle = Some(tokio::spawn(async move {
            let interval_duration = Duration::from_secs_f64(1.0 / polling_rate);
            let mut interval = tokio::time::interval(interval_duration);

            info!("MaiTai '{}' monitoring task started at {} Hz", id, polling_rate);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // In real implementation, would query serial port here
                        // For now, generate placeholder data
                        let timestamp = Utc::now();

                        // Mock wavelength data
                        let wavelength_dp = DataPoint {
                            timestamp,
                            channel: format!("{}_wavelength", id),
                            value: 800.0, // Would be queried from instrument
                            unit: "nm".to_string(),
                        };

                        let measurement = arc_measurement(Measurement::Scalar(wavelength_dp));
                        if tx.send(measurement).is_err() {
                            warn!("No active receivers for MaiTai data");
                            break;
                        }

                        // Mock power data
                        let power_dp = DataPoint {
                            timestamp,
                            channel: format!("{}_power", id),
                            value: 1.5, // Would be queried from instrument
                            unit: "W".to_string(),
                        };

                        let _ = tx.send(arc_measurement(Measurement::Scalar(power_dp)));

                        // Mock shutter state
                        let shutter_dp = DataPoint {
                            timestamp,
                            channel: format!("{}_shutter", id),
                            value: 1.0, // Would be queried from instrument
                            unit: "state".to_string(),
                        };

                        let _ = tx.send(arc_measurement(Measurement::Scalar(shutter_dp)));
                    }
                    _ = &mut shutdown_rx => {
                        info!("MaiTai '{}' monitoring task shutting down", id);
                        break;
                    }
                }
            }
        }));
    }

    /// Validate wavelength is within instrument range
    fn validate_wavelength(&self, nm: f64) -> Result<()> {
        if nm < self.wavelength_min_nm || nm > self.wavelength_max_nm {
            return Err(anyhow!(
                "Wavelength {} nm out of range ({}-{} nm)",
                nm,
                self.wavelength_min_nm,
                self.wavelength_max_nm
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl Instrument for MaiTaiV2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "maitai_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state
    }


    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow!("Cannot initialize from state: {:?}", self.state));
        }

        info!("Initializing MaiTai '{}'", self.id);
        self.state = InstrumentState::Connecting;

        // Connect hardware adapter
        let connect_result = self.serial.lock().await.connect(&Default::default()).await;

        match connect_result {
            Ok(()) => {
                info!("MaiTai '{}' adapter connected", self.id);

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
                info!("MaiTai '{}' initialized successfully", self.id);
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
        info!("Shutting down MaiTai '{}'", self.id);
        self.state = InstrumentState::ShuttingDown;

        // Stop monitoring task if running
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        // Disconnect adapter
        self.serial.lock().await.disconnect().await?;

        self.state = InstrumentState::Disconnected;
        info!("MaiTai '{}' shut down successfully", self.id);
        Ok(())
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                info!("Attempting to recover MaiTai '{}'", self.id);

                // Disconnect and wait
                let _ = self.serial.lock().await.disconnect().await;
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Reconnect and reconfigure
                self.serial.lock().await.connect(&Default::default()).await?;
                self.configure().await?;

                self.state = InstrumentState::Ready;

                info!("MaiTai '{}' recovered successfully", self.id);
                Ok(())
            }
            InstrumentState::Error(_) => {
                Err(anyhow!("Cannot recover from unrecoverable error"))
            }
            _ => Err(anyhow!("Cannot recover from state: {:?}", self.state)),
        }
    }

    fn measurement_stream(&self) -> MeasurementReceiver {
        self.measurement_tx.subscribe()
    }

    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()> {
        match cmd {
            InstrumentCommand::StartAcquisition => self.start_monitoring().await,
            InstrumentCommand::StopAcquisition => self.stop_monitoring().await,
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::Recover => self.recover().await,
            InstrumentCommand::SetParameter { name, value } => {
                match name.as_str() {
                    "wavelength_nm" => {
                        let wavelength = value.as_f64()
                            .ok_or_else(|| anyhow!("Invalid wavelength value"))?;
                        self.set_wavelength_nm(wavelength).await
                    }
                    "shutter" => {
                        let open = value.as_bool()
                            .ok_or_else(|| anyhow!("Invalid shutter value (expected boolean)"))?;
                        self.set_shutter(open).await
                    }
                    "laser_on" => {
                        let on = value.as_bool()
                            .ok_or_else(|| anyhow!("Invalid laser_on value (expected boolean)"))?;
                        if on {
                            self.laser_on().await
                        } else {
                            self.laser_off().await
                        }
                    }
                    _ => Err(anyhow!("Unknown parameter: {}", name)),
                }
            }
            InstrumentCommand::GetParameter { name } => {
                // For GetParameter, we'd need a way to return the value
                // For now, just log it
                info!("Get parameter request for '{}' (not implemented)", name);
                Ok(())
            }
        }
    }
}

#[async_trait]
impl TunableLaser for MaiTaiV2 {
    async fn set_wavelength_nm(&mut self, nm: f64) -> Result<()> {
        self.validate_wavelength(nm)?;

        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot set wavelength from state: {:?}", self.state));
        }

        self.send_command(&format!("WAVELENGTH:{}", nm))
            .await
            .context("Failed to set wavelength")?;

        self.wavelength_nm = nm;
        info!("Set MaiTai wavelength to {} nm", nm);
        Ok(())
    }

    async fn get_wavelength_nm(&self) -> Result<f64> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot query wavelength from state: {:?}", self.state));
        }

        self.query_value("WAVELENGTH?")
            .await
            .context("Failed to query wavelength")
    }

    async fn get_power_w(&self) -> Result<f64> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot query power from state: {:?}", self.state));
        }

        self.query_value("POWER?")
            .await
            .context("Failed to query power")
    }

    async fn set_shutter(&mut self, open: bool) -> Result<()> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot control shutter from state: {:?}", self.state));
        }

        let cmd = if open { "SHUTTER:1" } else { "SHUTTER:0" };

        self.send_command(cmd)
            .await
            .context("Failed to set shutter")?;

        self.shutter_open = open;
        info!("MaiTai shutter: {}", if open { "open" } else { "closed" });
        Ok(())
    }

    async fn get_shutter(&self) -> bool {
        self.shutter_open
    }

    async fn laser_on(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot turn on laser from state: {:?}", self.state));
        }

        self.send_command("ON")
            .await
            .context("Failed to turn on laser")?;

        self.laser_on = true;
        info!("MaiTai laser: ON");
        Ok(())
    }

    async fn laser_off(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot turn off laser from state: {:?}", self.state));
        }

        self.send_command("OFF")
            .await
            .context("Failed to turn off laser")?;

        self.laser_on = false;
        info!("MaiTai laser: OFF");
        Ok(())
    }
}

// Additional MaiTai-specific methods (not in TunableLaser trait)
impl MaiTaiV2 {
    /// Start continuous parameter monitoring
    async fn start_monitoring(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow!("Cannot start monitoring from state: {:?}", self.state));
        }

        self.spawn_monitoring_task();
        self.state = InstrumentState::Acquiring;

        info!("MaiTai '{}' started monitoring", self.id);
        Ok(())
    }

    /// Stop continuous parameter monitoring
    async fn stop_monitoring(&mut self) -> Result<()> {
        if self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Not currently acquiring"));
        }

        // Stop monitoring task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        self.state = InstrumentState::Ready;
        info!("MaiTai '{}' stopped monitoring", self.id);
        Ok(())
    }

    /// Get current wavelength setting (cached value)
    pub fn get_wavelength_cached(&self) -> f64 {
        self.wavelength_nm
    }

    /// Get valid wavelength range for this instrument
    pub fn get_wavelength_range(&self) -> (f64, f64) {
        (self.wavelength_min_nm, self.wavelength_max_nm)
    }

    /// Set custom wavelength range (for different MaiTai models)
    pub fn set_wavelength_range(&mut self, min_nm: f64, max_nm: f64) -> Result<()> {
        if min_nm >= max_nm {
            return Err(anyhow!("Invalid wavelength range: min must be < max"));
        }
        self.wavelength_min_nm = min_nm;
        self.wavelength_max_nm = max_nm;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maitai_creation() {
        let instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        assert_eq!(instrument.id(), "test_laser");
        assert_eq!(instrument.instrument_type(), "maitai_v2");
        assert_eq!(instrument.state(), InstrumentState::Disconnected);

    }

    #[test]
    fn test_initial_parameters() {
        let instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Verify default parameters
        assert_eq!(instrument.wavelength_nm, 800.0);
        assert_eq!(instrument.shutter_open, false);
        assert_eq!(instrument.laser_on, false);
        assert_eq!(instrument.polling_rate_hz, 1.0);
        assert_eq!(instrument.get_wavelength_range(), (690.0, 1040.0));
    }

    #[test]
    fn test_wavelength_validation() {
        let instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Valid wavelengths
        assert!(instrument.validate_wavelength(700.0).is_ok());
        assert!(instrument.validate_wavelength(800.0).is_ok());
        assert!(instrument.validate_wavelength(1000.0).is_ok());

        // Invalid wavelengths
        assert!(instrument.validate_wavelength(689.0).is_err());
        assert!(instrument.validate_wavelength(1041.0).is_err());
        assert!(instrument.validate_wavelength(500.0).is_err());
        assert!(instrument.validate_wavelength(1500.0).is_err());
    }

    #[test]
    fn test_custom_wavelength_range() {
        let mut instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Set custom range for different model
        assert!(instrument.set_wavelength_range(710.0, 990.0).is_ok());
        assert_eq!(instrument.get_wavelength_range(), (710.0, 990.0));

        // Validate against new range
        assert!(instrument.validate_wavelength(700.0).is_err());
        assert!(instrument.validate_wavelength(750.0).is_ok());
        assert!(instrument.validate_wavelength(1000.0).is_err());

        // Invalid range
        assert!(instrument.set_wavelength_range(1000.0, 700.0).is_err());
    }

    #[test]
    fn test_cached_wavelength() {
        let mut instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        assert_eq!(instrument.get_wavelength_cached(), 800.0);

        // Modify internal state
        instrument.wavelength_nm = 900.0;
        assert_eq!(instrument.get_wavelength_cached(), 900.0);
    }

    #[test]
    fn test_shutter_state_tracking() {
        let mut instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Test internal state tracking
        assert_eq!(instrument.shutter_open, false);

        // Modify internal state
        instrument.shutter_open = true;
        assert_eq!(instrument.shutter_open, true);
    }

    #[test]
    fn test_state_transitions() {
        let instrument = MaiTaiV2::new(
            "test_laser".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Should start disconnected
        assert_eq!(instrument.state(), InstrumentState::Disconnected);

        // Note: Full state transition testing requires async and actual hardware
        // or mocking, which is better suited for integration tests
    }

    // Note: Integration tests with actual hardware or mocked SerialAdapter
    // would go in tests/ directory. These unit tests verify the structure
    // and basic functionality without hardware.
}
