//! Core trait definitions for rust-daq instrument abstraction
//!
//! This crate provides the fundamental building blocks for the three-tier architecture:
//! - HardwareAdapter: Low-level I/O abstraction
//! - Instrument: Logical device abstraction with state management
//! - Meta-Instrument traits: Standardized interfaces (Camera, PowerMeter, etc.)

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

// Re-export commonly used types
pub use anyhow::{anyhow, Result};
pub use thiserror::Error;

//==============================================================================
// Core Data Types
//==============================================================================

/// A single scalar measurement from an instrument
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub value: f64,
    pub unit: String,
}

/// Spectrum data (e.g., from FFT or spectrometer)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrumData {
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub wavelengths: Vec<f64>,  // or frequencies
    pub intensities: Vec<f64>,
    pub unit_x: String,         // e.g., "nm" or "Hz"
    pub unit_y: String,         // e.g., "counts" or "dBm"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Image data from cameras or 2D sensors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<f64>,       // Row-major order
    pub unit: String,           // e.g., "counts", "photons"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Unified measurement type supporting multiple data forms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Measurement {
    Scalar(DataPoint),
    Spectrum(SpectrumData),
    Image(ImageData),
}

/// Type alias for Arc-wrapped measurements (zero-copy distribution)
pub type ArcMeasurement = Arc<Measurement>;

/// Channel sender for Arc-wrapped measurements
pub type MeasurementSender = broadcast::Sender<ArcMeasurement>;

/// Channel receiver for Arc-wrapped measurements
pub type MeasurementReceiver = broadcast::Receiver<ArcMeasurement>;

//==============================================================================
// Instrument State Management
//==============================================================================

/// Self-contained error description for use in InstrumentState
#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq)]
#[error("{message}")]
pub struct DaqError {
    pub message: String,
    pub can_recover: bool,
}

/// Explicit state tracking for all instruments
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InstrumentState {
    /// Not connected to hardware
    Disconnected,

    /// Connection in progress
    Connecting,

    /// Connected and idle, ready for commands
    Ready,

    /// Actively acquiring data
    Acquiring,

    /// Error state with embedded error information
    Error(DaqError),

    /// Shutting down gracefully
    ShuttingDown,
}

//==============================================================================
// Configuration Types
//==============================================================================

/// Generic configuration for hardware adapters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    /// Adapter-specific configuration as JSON
    pub params: serde_json::Value,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            params: serde_json::json!({}),
        }
    }
}

/// Generic configuration for instruments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentConfig {
    /// Instrument type identifier
    pub instrument_type: String,
    /// Instrument-specific parameters
    pub params: serde_json::Value,
}

/// Region of interest for cameras
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ROI {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Default for ROI {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 512,
            height: 512,
        }
    }
}

/// Power range for power meters
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PowerRange {
    Auto,
    Range(f64), // Maximum power in watts
}

//==============================================================================
// Hardware Adapter Trait
//==============================================================================

/// Base trait all hardware adapters must implement
///
/// Hardware adapters provide low-level I/O abstraction, isolating
/// platform-specific communication (serial, VISA, USB, network) from
/// instrument logic.
#[async_trait]
pub trait HardwareAdapter: Send + Sync {
    /// Connect to hardware with given configuration
    async fn connect(&mut self, config: &AdapterConfig) -> Result<()>;

    /// Disconnect from hardware
    async fn disconnect(&mut self) -> Result<()>;

    /// Reset the hardware connection (disconnect + reconnect)
    ///
    /// Default implementation disconnects and reconnects with default config.
    /// Adapters can override for hardware-specific reset procedures.
    async fn reset(&mut self) -> Result<()> {
        self.disconnect().await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        self.connect(&AdapterConfig::default()).await
    }

    /// Check if currently connected
    fn is_connected(&self) -> bool;

    /// Get adapter type identifier (e.g., "serial", "visa", "usb")
    fn adapter_type(&self) -> &str;

    /// Get human-readable adapter information
    fn info(&self) -> String {
        format!("{}Adapter", self.adapter_type())
    }
}

//==============================================================================
// Commands
//==============================================================================

/// Commands that can be sent to instruments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstrumentCommand {
    /// Request instrument to shut down gracefully
    Shutdown,

    /// Start data acquisition
    StartAcquisition,

    /// Stop data acquisition
    StopAcquisition,

    /// Set a parameter (instrument-specific)
    SetParameter {
        name: String,
        value: serde_json::Value,
    },

    /// Get a parameter (instrument-specific)
    GetParameter {
        name: String,
    },

    /// Attempt recovery from error state
    Recover,
}

//==============================================================================
// Base Instrument Trait
//==============================================================================

/// Base trait all instruments must implement
///
/// Instruments represent logical devices built on top of HardwareAdapters.
/// They provide the mid-level abstraction between hardware I/O and
/// application logic.
#[async_trait]
pub trait Instrument: Send + Sync {
    /// Get instrument identifier
    fn id(&self) -> &str;

    /// Get instrument type string
    fn instrument_type(&self) -> &str;

    /// Get current instrument state
    fn state(&self) -> InstrumentState;


    /// Initialize instrument (connect and configure)
    async fn initialize(&mut self) -> Result<()>;

    /// Shutdown instrument gracefully
    async fn shutdown(&mut self) -> Result<()>;

    /// Get measurement stream receiver
    fn measurement_stream(&self) -> MeasurementReceiver;

    /// Handle a command
    async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()>;

    /// Attempt to recover from Error state
    /// Returns Ok(()) if recovered to Ready state
    async fn recover(&mut self) -> Result<()>;

    /// Check if operation is valid in current state
    fn can_execute(&self, cmd: &InstrumentCommand) -> bool {
        match (self.state(), cmd) {
            (InstrumentState::Ready, InstrumentCommand::StartAcquisition) => true,
            (InstrumentState::Acquiring, InstrumentCommand::StopAcquisition) => true,
            (InstrumentState::Ready, InstrumentCommand::SetParameter { .. }) => true,
            (InstrumentState::Ready, InstrumentCommand::GetParameter { .. }) => true,
            (InstrumentState::Error(err), InstrumentCommand::Recover) if err.can_recover => true,
            (_, InstrumentCommand::Shutdown) => true, // Can always request shutdown
            _ => false,
        }
    }
}

//==============================================================================
// Meta-Instrument Traits
//==============================================================================

/// Camera meta-instrument interface
///
/// Provides standardized interface for all camera-like devices
/// (scientific cameras, webcams, frame grabbers, etc.)
#[async_trait]
pub trait Camera: Instrument {
    // Frame acquisition

    /// Capture a single frame
    async fn snap(&mut self) -> Result<ImageData>;

    /// Start continuous live acquisition
    async fn start_live(&mut self) -> Result<()>;

    /// Stop continuous acquisition
    async fn stop_live(&mut self) -> Result<()>;

    // Configuration

    /// Set exposure time in milliseconds
    async fn set_exposure_ms(&mut self, ms: f64) -> Result<()>;

    /// Get current exposure time
    async fn get_exposure_ms(&self) -> f64;

    /// Set region of interest
    async fn set_roi(&mut self, roi: ROI) -> Result<()>;

    /// Get current ROI
    async fn get_roi(&self) -> ROI;

    /// Set pixel binning
    async fn set_binning(&mut self, x: u16, y: u16) -> Result<()>;

    /// Get current binning
    async fn get_binning(&self) -> (u16, u16);

    // Capabilities

    /// Get sensor size in pixels (width, height)
    fn get_sensor_size(&self) -> (u32, u32);

    /// Get physical pixel size in micrometers (x, y)
    fn get_pixel_size_um(&self) -> (f64, f64);

    /// Check if hardware triggering is supported
    fn supports_hardware_trigger(&self) -> bool;
}

/// Position controller (motors, stages, rotation mounts)
///
/// For single-axis positioners
#[async_trait]
pub trait PositionController: Instrument {
    /// Move to absolute position
    async fn move_absolute(&mut self, position: f64) -> Result<()>;

    /// Move relative to current position
    async fn move_relative(&mut self, delta: f64) -> Result<()>;

    /// Get current position
    async fn get_position(&self) -> Result<f64>;

    /// Home the axis (find reference position)
    async fn home(&mut self) -> Result<()>;

    /// Emergency stop
    async fn stop(&mut self) -> Result<()>;

    /// Get valid position range (min, max)
    fn get_position_range(&self) -> (f64, f64);

    /// Get position units (e.g., "mm", "degrees")
    fn get_units(&self) -> &str;
}

/// Multi-axis position controller
///
/// For motion controllers with multiple independent axes
#[async_trait]
pub trait MultiAxisController: Instrument {
    /// Get number of axes
    fn num_axes(&self) -> usize;

    /// Move single axis to absolute position
    async fn move_absolute_axis(&mut self, axis: usize, pos: f64) -> Result<()>;

    /// Get position of single axis
    async fn get_position_axis(&self, axis: usize) -> Result<f64>;

    /// Move all axes simultaneously
    async fn move_absolute_all(&mut self, positions: &[f64]) -> Result<()>;

    /// Get all axis positions
    async fn get_positions_all(&self) -> Result<Vec<f64>>;

    /// Home single axis
    async fn home_axis(&mut self, axis: usize) -> Result<()>;

    /// Emergency stop all axes
    async fn stop_all(&mut self) -> Result<()>;
}

/// Spectrum analyzer / spectrometer
#[async_trait]
pub trait SpectrumAnalyzer: Instrument {
    /// Acquire a spectrum
    async fn acquire_spectrum(&mut self) -> Result<SpectrumData>;

    /// Set integration time in milliseconds
    async fn set_integration_time_ms(&mut self, ms: f64) -> Result<()>;

    /// Get wavelength/frequency range (min, max)
    async fn get_wavelength_range(&self) -> (f64, f64);
}

/// Power meter
#[async_trait]
pub trait PowerMeter: Instrument {
    /// Read current power
    async fn read_power(&mut self) -> Result<f64>;

    /// Set wavelength for calibration (nanometers)
    async fn set_wavelength_nm(&mut self, nm: f64) -> Result<()>;

    /// Set measurement range
    async fn set_range(&mut self, range: PowerRange) -> Result<()>;

    /// Zero/tare the sensor
    async fn zero(&mut self) -> Result<()>;
}

/// Tunable laser
#[async_trait]
pub trait TunableLaser: Instrument {
    // Wavelength control

    /// Set output wavelength in nanometers
    async fn set_wavelength_nm(&mut self, nm: f64) -> Result<()>;

    /// Get current wavelength
    async fn get_wavelength_nm(&self) -> Result<f64>;

    /// Get output power in watts
    async fn get_power_w(&self) -> Result<f64>;

    // Shutter control

    /// Control shutter (true = open, false = closed)
    async fn set_shutter(&mut self, open: bool) -> Result<()>;

    /// Get shutter state
    async fn get_shutter(&self) -> bool;

    // Laser control

    /// Turn laser on
    async fn laser_on(&mut self) -> Result<()>;

    /// Turn laser off
    async fn laser_off(&mut self) -> Result<()>;
}

/// Motion controller for multi-axis positioning systems
///
/// Provides standardized interface for motion controllers like ESP300, ESP301, etc.
/// Supports both single-axis queries and coordinated multi-axis moves.
#[async_trait]
pub trait MotionController: Instrument {
    /// Get number of axes
    fn num_axes(&self) -> usize;

    // Single-axis operations

    /// Move single axis to absolute position
    async fn move_absolute(&mut self, axis: usize, position: f64) -> Result<()>;

    /// Move single axis relative to current position
    async fn move_relative(&mut self, axis: usize, distance: f64) -> Result<()>;

    /// Get position of single axis
    async fn get_position(&self, axis: usize) -> Result<f64>;

    /// Get velocity of single axis
    async fn get_velocity(&self, axis: usize) -> Result<f64>;

    /// Set velocity for single axis (units/second)
    async fn set_velocity(&mut self, axis: usize, velocity: f64) -> Result<()>;

    /// Set acceleration for single axis (units/second²)
    async fn set_acceleration(&mut self, axis: usize, acceleration: f64) -> Result<()>;

    /// Home single axis (find reference position)
    async fn home_axis(&mut self, axis: usize) -> Result<()>;

    /// Stop single axis immediately
    async fn stop_axis(&mut self, axis: usize) -> Result<()>;

    // Multi-axis operations

    /// Move all axes to absolute positions simultaneously
    async fn move_absolute_all(&mut self, positions: &[f64]) -> Result<()>;

    /// Get all axis positions
    async fn get_positions_all(&self) -> Result<Vec<f64>>;

    /// Home all axes
    async fn home_all(&mut self) -> Result<()>;

    /// Emergency stop all axes
    async fn stop_all(&mut self) -> Result<()>;

    // Axis configuration

    /// Get position units for axis (e.g., "mm", "degrees")
    fn get_units(&self, axis: usize) -> &str;

    /// Get valid position range for axis (min, max)
    fn get_position_range(&self, axis: usize) -> (f64, f64);

    /// Check if axis is moving
    async fn is_moving(&self, axis: usize) -> Result<bool>;
}

//==============================================================================
// Helper Functions
//==============================================================================

/// Create a new Arc-wrapped Measurement
pub fn arc_measurement(m: Measurement) -> ArcMeasurement {
    Arc::new(m)
}

/// Create a measurement channel with given capacity
pub fn measurement_channel(capacity: usize) -> (MeasurementSender, MeasurementReceiver) {
    broadcast::channel(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arc_measurement_size() {
        let data = DataPoint {
            timestamp: Utc::now(),
            channel: "test".to_string(),
            value: 42.0,
            unit: "V".to_string(),
        };
        let m = arc_measurement(Measurement::Scalar(data));

        // Arc should be pointer-sized
        assert_eq!(std::mem::size_of_val(&m), std::mem::size_of::<usize>());
    }

    #[test]
    fn test_arc_measurement_zero_copy() {
        // This test verifies that cloning an ArcMeasurement does not copy the underlying Measurement data.
        let data = DataPoint {
            timestamp: Utc::now(),
            channel: "test".to_string(),
            value: 1.0,
            unit: "V".to_string(),
        };
        let m1 = arc_measurement(Measurement::Scalar(data));
        let m2 = m1.clone();

        // Both Arcs should point to the exact same memory location.
        assert!(Arc::ptr_eq(&m1, &m2), "Cloned Arc does not point to the same memory");
        // The strong count reflects the number of references.
        assert_eq!(Arc::strong_count(&m1), 2, "Strong count should be 2 after clone");
    }

    // A mock instrument implementation is needed to test the default `can_execute` logic.
    struct MockInstrument {
        state: InstrumentState,
    }

    #[async_trait]
    impl Instrument for MockInstrument {
        fn id(&self) -> &str { "mock" }
        fn instrument_type(&self) -> &str { "mock" }
        fn state(&self) -> InstrumentState { self.state.clone() }
        async fn initialize(&mut self) -> Result<()> { Ok(()) }
        async fn shutdown(&mut self) -> Result<()> { Ok(()) }
        fn measurement_stream(&self) -> MeasurementReceiver {
            let (tx, rx) = measurement_channel(1);
            // The sender is dropped, so the channel is immediately closed.
            // This is fine for testing `can_execute` which doesn't use the stream.
            drop(tx);
            rx
        }
        async fn handle_command(&mut self, _cmd: InstrumentCommand) -> Result<()> { Ok(()) }
        async fn recover(&mut self) -> Result<()> { Ok(()) }
    }

    #[test]
    fn test_can_execute_state_logic() {
        // This table-driven test verifies the state machine logic in the Instrument trait's
        // default `can_execute` implementation.
        let set_param_cmd = InstrumentCommand::SetParameter {
            name: "exposure".to_string(),
            value: serde_json::json!(100),
        };
        let get_param_cmd = InstrumentCommand::GetParameter { name: "exposure".to_string() };

        let test_cases = vec![
            // (State, Command, Expected Result, Description)
            (InstrumentState::Disconnected, InstrumentCommand::StartAcquisition, false, "Cannot start acquisition when disconnected"),
            (InstrumentState::Connecting, InstrumentCommand::StartAcquisition, false, "Cannot start acquisition while connecting"),
            (InstrumentState::Ready, InstrumentCommand::StartAcquisition, true, "Can start acquisition from Ready state"),
            (InstrumentState::Ready, InstrumentCommand::StopAcquisition, false, "Cannot stop acquisition when not acquiring"),
            (InstrumentState::Ready, set_param_cmd.clone(), true, "Can set parameter in Ready state"),
            (InstrumentState::Ready, get_param_cmd.clone(), true, "Can get parameter in Ready state"),
            (InstrumentState::Acquiring, InstrumentCommand::StartAcquisition, false, "Cannot start acquisition when already acquiring"),
            (InstrumentState::Acquiring, InstrumentCommand::StopAcquisition, true, "Can stop acquisition from Acquiring state"),
            (InstrumentState::Acquiring, set_param_cmd.clone(), false, "Cannot set parameter while acquiring"),
            (InstrumentState::Error(DaqError { message: "".into(), can_recover: true }), InstrumentCommand::Recover, true, "Can recover from a recoverable error"),
            (InstrumentState::Error(DaqError { message: "".into(), can_recover: false }), InstrumentCommand::Recover, false, "Cannot recover from a non-recoverable error"),
            (InstrumentState::Error(DaqError { message: "".into(), can_recover: true }), InstrumentCommand::StartAcquisition, false, "Cannot start acquisition from an error state"),
            (InstrumentState::ShuttingDown, InstrumentCommand::StartAcquisition, false, "Cannot start acquisition while shutting down"),
            // Shutdown should always be allowed from any state.
            (InstrumentState::Disconnected, InstrumentCommand::Shutdown, true, "Can shut down from Disconnected"),
            (InstrumentState::Ready, InstrumentCommand::Shutdown, true, "Can shut down from Ready"),
            (InstrumentState::Acquiring, InstrumentCommand::Shutdown, true, "Can shut down from Acquiring"),
            (InstrumentState::Error(DaqError { message: "".into(), can_recover: false }), InstrumentCommand::Shutdown, true, "Can shut down from Error"),
        ];

        for (state, cmd, expected, description) in test_cases {
            let instrument = MockInstrument { state: state.clone() };
            assert_eq!(
                instrument.can_execute(&cmd),
                expected,
                "Failed check: '{}'. State: {:?}, Command: {:?}",
                description,
                state,
                cmd
            );
        }
    }

    #[tokio::test]
    async fn test_measurement_channel_broadcast_to_multiple_subscribers() {
        // Verifies that multiple subscribers receive the same data via zero-copy Arcs.
        let (tx, mut rx1) = measurement_channel(2);
        let mut rx2 = tx.subscribe();

        let m1 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 1.0, unit: "V".into() }));
        let m2 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 2.0, unit: "V".into() }));

        tx.send(m1.clone()).unwrap();
        tx.send(m2.clone()).unwrap();

        // Both subscribers should receive both messages.
        let (received1_1, received1_2) = (rx1.recv().await.unwrap(), rx1.recv().await.unwrap());
        let (received2_1, received2_2) = (rx2.recv().await.unwrap(), rx2.recv().await.unwrap());

        // Verify pointer equality to confirm zero-copy.
        assert!(Arc::ptr_eq(&m1, &received1_1));
        assert!(Arc::ptr_eq(&m2, &received1_2));
        assert!(Arc::ptr_eq(&m1, &received2_1));
        assert!(Arc::ptr_eq(&m2, &received2_2));
    }

    #[tokio::test]
    async fn test_measurement_channel_drops_messages_for_lagging_subscriber() {
        // Verifies that a slow subscriber will miss messages rather than blocking the sender.
        let (tx, mut rx) = measurement_channel(1); // Small capacity to easily trigger lagging

        let m1 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 1.0, unit: "V".into() }));
        let m2 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 2.0, unit: "V".into() }));

        // Fill the channel capacity.
        tx.send(m1).unwrap();

        // This send will succeed but causes the previous message (m1) to be dropped for this subscriber.
        tx.send(m2.clone()).unwrap();

        // The first attempt to receive will report that messages were skipped.
        match rx.recv().await {
            Err(broadcast::error::RecvError::Lagged(count)) => assert_eq!(count, 1),
            other => panic!("Expected a Lagged error, but got {:?}", other),
        }

        // The receiver is now caught up and can receive the next available message.
        let received = rx.recv().await.unwrap();
        assert!(Arc::ptr_eq(&m2, &received));
    }

    #[tokio::test]
    async fn test_measurement_channel_late_subscriber_misses_old_messages() {
        // Verifies that a new subscriber does not receive messages sent before it subscribed.
        let (tx, mut _initial_rx) = measurement_channel(10);

        let m1 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 1.0, unit: "V".into() }));
        tx.send(m1).unwrap();

        // This subscriber joins *after* m1 was sent.
        let mut rx = tx.subscribe();

        // It should not see the old message. `try_recv` is non-blocking.
        assert!(matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)));

        // It will, however, see new messages sent after it subscribed.
        let m2 = arc_measurement(Measurement::Scalar(DataPoint { timestamp: Utc::now(), channel: "c1".into(), value: 2.0, unit: "V".into() }));
        tx.send(m2.clone()).unwrap();
        let received = rx.recv().await.unwrap();
        assert!(Arc::ptr_eq(&m2, &received));
    }

    #[test]
    #[should_panic(expected = "capacity cannot be zero")]
    fn test_measurement_channel_zero_capacity_panics() {
        // The underlying `tokio::sync::broadcast::channel` panics if capacity is 0.
        // This test confirms our helper function preserves that critical contract.
        let (_tx, _rx) = measurement_channel(0);
    }

    #[test]
    fn test_roi_default() {
        let roi = ROI::default();
        assert_eq!(roi.x, 0);
        assert_eq!(roi.y, 0);
        assert_eq!(roi.width, 512);
        assert_eq!(roi.height, 512);
    }
}
