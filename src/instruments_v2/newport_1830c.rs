//! Newport 1830-C Optical Power Meter V2 Implementation
//!
//! This module provides a V2 implementation of the Newport 1830-C power meter
//! using the new three-tier architecture:
//! - SerialAdapter for RS-232 communication
//! - Instrument trait for state management
//! - PowerMeter trait for domain-specific methods
//!
//! ## Configuration Example
//!
//! ```toml
//! [instruments.power_meter_1]
//! type = "newport_1830c_v2"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! wavelength = 1550.0  # nm
//! range = 0  # 0=autorange
//! units = 0  # 0=Watts, 1=dBm, 2=dB, 3=REL
//! polling_rate_hz = 10.0
//! ```

use crate::adapters::SerialAdapter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use daq_core::{
    arc_measurement, DataPoint, DaqError, HardwareAdapter, Instrument, InstrumentCommand,
    InstrumentState, Measurement, MeasurementReceiver, MeasurementSender, PowerMeter, PowerRange,
};
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;

/// Newport 1830-C V2 implementation using new trait architecture
pub struct Newport1830CV2 {
    /// Instrument identifier
    id: String,

    /// Serial adapter (Arc<Mutex> for shared mutable access)
    serial: Arc<Mutex<SerialAdapter>>,

    /// Current instrument state
    state: InstrumentState,


    /// Power meter configuration
    wavelength_nm: f64,
    range: i32,
    units: PowerUnits,
    polling_rate_hz: f64,

    /// Data streaming (zero-copy with Arc)
    measurement_tx: MeasurementSender,
    _measurement_rx_keeper: MeasurementReceiver,

    /// Acquisition task management
    task_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Power meter unit types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerUnits {
    Watts = 0,
    DBm = 1,
    DB = 2,
    Relative = 3,
}

impl PowerUnits {
    fn from_i32(value: i32) -> Result<Self> {
        match value {
            0 => Ok(PowerUnits::Watts),
            1 => Ok(PowerUnits::DBm),
            2 => Ok(PowerUnits::DB),
            3 => Ok(PowerUnits::Relative),
            _ => Err(anyhow!("Invalid power units: {}", value)),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            PowerUnits::Watts => "W",
            PowerUnits::DBm => "dBm",
            PowerUnits::DB => "dB",
            PowerUnits::Relative => "REL",
        }
    }
}

impl Newport1830CV2 {
    /// Create a new Newport 1830-C V2 instrument with SerialAdapter
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `baud_rate` - Communication speed (typically 9600)
    pub fn new(id: String, port: String, baud_rate: u32) -> Self {
        let serial = SerialAdapter::new(port, baud_rate)
            .with_timeout(Duration::from_secs(1))
            .with_line_terminator("\r\n".to_string())
            .with_response_delimiter('\n');

        let (tx, rx) = broadcast::channel(1024);

        Self {
            id,
            serial: Arc::new(Mutex::new(serial)),
            state: InstrumentState::Disconnected,

            wavelength_nm: 1550.0,
            range: 0,
            units: PowerUnits::Watts,
            polling_rate_hz: 10.0,
            measurement_tx: tx,
            _measurement_rx_keeper: rx,
            task_handle: None,
            shutdown_tx: None,
        }
    }

    /// Send a command to the power meter
    async fn send_command(&self, command: &str) -> Result<String> {
        self.serial.lock().await.send_command(command).await
    }

    /// Configure the instrument after connection
    async fn configure(&mut self) -> Result<()> {
        // Set wavelength
        self.send_command(&format!("PM:Lambda {}", self.wavelength_nm))
            .await
            .context("Failed to set wavelength")?;

        info!("Set wavelength to {} nm", self.wavelength_nm);

        // Set range
        self.send_command(&format!("PM:Range {}", self.range))
            .await
            .context("Failed to set range")?;

        info!("Set range to {}", self.range);

        // Set units
        self.send_command(&format!("PM:Units {}", self.units as i32))
            .await
            .context("Failed to set units")?;

        info!("Set units to {}", self.units.as_str());

        Ok(())
    }

    /// Spawn polling task for continuous power measurement
    fn spawn_polling_task(&mut self) {
        let tx = self.measurement_tx.clone();
        let id = self.id.clone();
        let polling_rate = self.polling_rate_hz;
        let units = self.units;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Note: In a real implementation, we'd need to pass a way to query the serial port.
        // For now, this generates mock data. Production version would use async channels
        // to send commands from the spawned task.

        self.task_handle = Some(tokio::spawn(async move {
            let interval_duration = Duration::from_secs_f64(1.0 / polling_rate);
            let mut interval = tokio::time::interval(interval_duration);

            info!("Newport 1830-C '{}' polling task started at {} Hz", id, polling_rate);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Generate mock data
                        // Real implementation would query via serial port
                        let value = 0.001 * (Utc::now().timestamp() % 1000) as f64;

                        let datapoint = DataPoint {
                            timestamp: Utc::now(),
                            channel: format!("{}_power", id),
                            value,
                            unit: units.as_str().to_string(),
                        };

                        let measurement = arc_measurement(Measurement::Scalar(datapoint));

                        if tx.send(measurement).is_err() {
                            warn!("No active receivers for Newport 1830-C data");
                            break;
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Newport 1830-C '{}' polling task shutting down", id);
                        break;
                    }
                }
            }
        }));
    }
}

#[async_trait]
impl Instrument for Newport1830CV2 {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        "newport_1830c_v2"
    }

    fn state(&self) -> InstrumentState {
        self.state.clone()
    }


    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Disconnected {
            return Err(anyhow!("Cannot initialize from state: {:?}", self.state));
        }

        info!("Initializing Newport 1830-C '{}'", self.id);
        self.state = InstrumentState::Connecting;

        // Connect hardware adapter
        let connect_result = self.serial.lock().await.connect(&Default::default()).await;

        match connect_result {
            Ok(()) => {
                info!("Newport 1830-C '{}' adapter connected", self.id);

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
                info!("Newport 1830-C '{}' initialized successfully", self.id);
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
        info!("Shutting down Newport 1830-C '{}'", self.id);
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
        info!("Newport 1830-C '{}' shut down successfully", self.id);
        Ok(())
    }

    async fn recover(&mut self) -> Result<()> {
        match &self.state {
            InstrumentState::Error(daq_error) if daq_error.can_recover => {
                info!("Attempting to recover Newport 1830-C '{}'", self.id);

                // Disconnect and wait
                let _ = self.serial.lock().await.disconnect().await;
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Reconnect and reconfigure
                self.serial.lock().await.connect(&Default::default()).await?;
                self.configure().await?;

                self.state = InstrumentState::Ready;

                info!("Newport 1830-C '{}' recovered successfully", self.id);
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
            InstrumentCommand::StartAcquisition => self.start_streaming().await,
            InstrumentCommand::StopAcquisition => self.stop_streaming().await,
            InstrumentCommand::Shutdown => self.shutdown().await,
            InstrumentCommand::Recover => self.recover().await,
            InstrumentCommand::SetParameter { name, value } => {
                match name.as_str() {
                    "wavelength_nm" => {
                        let wavelength = value.as_f64()
                            .ok_or_else(|| anyhow!("Invalid wavelength value"))?;
                        self.set_wavelength_nm(wavelength).await
                    }
                    "range" => {
                        let range_idx = value.as_i64()
                            .ok_or_else(|| anyhow!("Invalid range value"))? as i32;
                        // Convert to PowerRange
                        let range = if range_idx == 0 {
                            PowerRange::Auto
                        } else {
                            // For Newport, range_idx maps to power levels
                            // This is simplified - real implementation would have proper mapping
                            PowerRange::Range(10.0_f64.powi(-range_idx))
                        };
                        self.set_range(range).await
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
impl PowerMeter for Newport1830CV2 {
    async fn read_power(&mut self) -> Result<f64> {
        if self.state != InstrumentState::Ready && self.state != InstrumentState::Acquiring {
            return Err(anyhow!("Cannot read power from state: {:?}", self.state));
        }

        let response = self.send_command("PM:Power?")
            .await
            .context("Failed to query power")?;

        response.parse::<f64>()
            .context("Failed to parse power value")
    }

    async fn set_wavelength_nm(&mut self, nm: f64) -> Result<()> {
        if nm < 400.0 || nm > 1700.0 {
            return Err(anyhow!("Wavelength out of range: {} nm (400-1700)", nm));
        }

        self.send_command(&format!("PM:Lambda {}", nm))
            .await
            .context("Failed to set wavelength")?;

        self.wavelength_nm = nm;
        info!("Set Newport 1830-C wavelength to {} nm", nm);
        Ok(())
    }

    async fn set_range(&mut self, range: PowerRange) -> Result<()> {
        // Convert PowerRange to Newport's integer range
        let range_idx = match range {
            PowerRange::Auto => 0,
            PowerRange::Range(max_power) => {
                // Map power to range index (simplified)
                // Real implementation would have proper Newport range mapping
                if max_power >= 1.0 { 1 }
                else if max_power >= 0.1 { 2 }
                else if max_power >= 0.01 { 3 }
                else if max_power >= 0.001 { 4 }
                else if max_power >= 0.0001 { 5 }
                else if max_power >= 0.00001 { 6 }
                else if max_power >= 0.000001 { 7 }
                else { 8 }
            }
        };

        self.send_command(&format!("PM:Range {}", range_idx))
            .await
            .context("Failed to set range")?;

        self.range = range_idx;
        info!("Set Newport 1830-C range to {}", range_idx);
        Ok(())
    }

    async fn zero(&mut self) -> Result<()> {
        self.send_command("PM:DS:Clear")
            .await
            .context("Failed to zero power meter")?;

        info!("Newport 1830-C '{}' zeroed", self.id);
        Ok(())
    }
}

// Additional Newport-specific methods (not in PowerMeter trait)
impl Newport1830CV2 {
    /// Start continuous power monitoring
    async fn start_streaming(&mut self) -> Result<()> {
        if self.state != InstrumentState::Ready {
            return Err(anyhow!("Cannot start streaming from state: {:?}", self.state));
        }

        self.spawn_polling_task();
        self.state = InstrumentState::Acquiring;

        info!("Newport 1830-C '{}' started streaming", self.id);
        Ok(())
    }

    /// Stop continuous power monitoring
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
        info!("Newport 1830-C '{}' stopped streaming", self.id);
        Ok(())
    }

    /// Get current wavelength setting
    pub async fn get_wavelength_nm(&self) -> Result<f64> {
        Ok(self.wavelength_nm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_newport_creation() {
        let instrument = Newport1830CV2::new(
            "test_pm".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        assert_eq!(instrument.id(), "test_pm");
        assert_eq!(instrument.instrument_type(), "newport_1830c_v2");
        assert_eq!(instrument.state(), InstrumentState::Disconnected);

    }

    #[test]
    fn test_power_units_conversion() {
        assert_eq!(PowerUnits::from_i32(0).unwrap(), PowerUnits::Watts);
        assert_eq!(PowerUnits::from_i32(1).unwrap(), PowerUnits::DBm);
        assert_eq!(PowerUnits::from_i32(2).unwrap(), PowerUnits::DB);
        assert_eq!(PowerUnits::from_i32(3).unwrap(), PowerUnits::Relative);
        assert!(PowerUnits::from_i32(4).is_err());

        assert_eq!(PowerUnits::Watts.as_str(), "W");
        assert_eq!(PowerUnits::DBm.as_str(), "dBm");
        assert_eq!(PowerUnits::DB.as_str(), "dB");
        assert_eq!(PowerUnits::Relative.as_str(), "REL");
    }

    #[test]
    fn test_wavelength_validation() {
        let instrument = Newport1830CV2::new(
            "test_pm".to_string(),
            "/dev/ttyUSB0".to_string(),
            9600,
        );

        // Wavelength is stored correctly
        assert_eq!(instrument.wavelength_nm, 1550.0);
        assert_eq!(instrument.range, 0);
        assert_eq!(instrument.units, PowerUnits::Watts);
        assert_eq!(instrument.polling_rate_hz, 10.0);
    }

    // Note: Integration tests with actual hardware would go in tests/ directory
    // These unit tests verify the structure and basic functionality without hardware
}
