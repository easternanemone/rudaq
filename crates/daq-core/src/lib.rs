//! Core types and traits for the Rust DAQ system.
//!
//! This crate provides foundational types used throughout the DAQ system,
//! including high-precision timestamping with NTP synchronization.
//!
//! **V3 Migration Note**: This crate is being phased out in favor of `core_v3` module.
//! The types below are temporary re-exports to support legacy V2 instruments during migration.

pub mod timestamp;

// Re-export commonly used types
pub use timestamp::{Timestamp, TimestampSource};

// =============================================================================
// Temporary V2 compatibility layer (will be removed after V3 migration)
// =============================================================================

/// Temporary re-export for V2 instruments - use `crate::core_v3::Measurement` in new code
pub use crate::legacy_v2::{
    arc_measurement, measurement_channel, AdapterConfig, Camera, DaqError, DataPoint,
    HardwareAdapter, ImageData, Instrument, InstrumentCommand, InstrumentState, Measurement,
    MeasurementReceiver, MeasurementSender, MotionController, PixelBuffer, PowerMeter, PowerRange,
    Result, SpectrumData, TunableLaser, ROI,
};

mod legacy_v2 {
    use super::timestamp::Timestamp;
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use tokio::sync::broadcast;

    pub type Result<T> = anyhow::Result<T>;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DataPoint {
        pub timestamp: Timestamp,
        pub channel: String,
        pub value: f64,
        pub unit: String,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub enum PixelBuffer {
        U8(Vec<u8>),
        U16(Vec<u16>),
        F64(Vec<f64>),
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SpectrumData {
        pub timestamp: Timestamp,
        pub channel: String,
        pub wavelengths: Vec<f64>,
        pub intensities: Vec<f64>,
        pub unit_x: String,
        pub unit_y: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub metadata: Option<serde_json::Value>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ImageData {
        pub timestamp: Timestamp,
        pub channel: String,
        pub width: u32,
        pub height: u32,
        pub pixels: PixelBuffer,
        pub unit: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub metadata: Option<serde_json::Value>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum Measurement {
        Scalar(DataPoint),
        Spectrum(SpectrumData),
        Image(ImageData),
    }

    pub type ArcMeasurement = Arc<Measurement>;
    pub type MeasurementSender = broadcast::Sender<ArcMeasurement>;
    pub type MeasurementReceiver = broadcast::Receiver<ArcMeasurement>;

    pub fn measurement_channel(capacity: usize) -> (MeasurementSender, MeasurementReceiver) {
        broadcast::channel(capacity)
    }

    pub fn arc_measurement(m: Measurement) -> ArcMeasurement {
        Arc::new(m)
    }

    #[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, PartialEq)]
    #[error("{message}")]
    pub struct DaqError {
        pub message: String,
        pub can_recover: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum InstrumentState {
        Disconnected,
        Connecting,
        Ready,
        Acquiring,
        Error(DaqError),
        ShuttingDown,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AdapterConfig {
        pub params: serde_json::Value,
    }

    #[async_trait]
    pub trait HardwareAdapter: Send + Sync {
        async fn connect(&mut self, config: &AdapterConfig) -> Result<()>;
        async fn disconnect(&mut self) -> Result<()>;
        fn is_connected(&self) -> bool;
        fn adapter_type(&self) -> &str;
        fn as_any(&self) -> &dyn std::any::Any;
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

        async fn reset(&mut self) -> Result<()> {
            self.disconnect().await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            self.connect(&AdapterConfig::default()).await
        }

        fn info(&self) -> String {
            format!("{}Adapter", self.adapter_type())
        }
    }

    impl Default for AdapterConfig {
        fn default() -> Self {
            Self {
                params: serde_json::json!({}),
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum InstrumentCommand {
        Shutdown,
        StartAcquisition,
        StopAcquisition,
        SnapFrame,
        SetParameter {
            name: String,
            value: serde_json::Value,
        },
        GetParameter {
            name: String,
        },
        Recover,
    }

    #[async_trait]
    pub trait Instrument: Send + Sync {
        fn id(&self) -> &str;
        fn instrument_type(&self) -> &str;
        fn state(&self) -> InstrumentState;
        async fn initialize(&mut self) -> Result<()>;
        async fn shutdown(&mut self) -> Result<()>;
        fn measurement_stream(&self) -> MeasurementReceiver;
        async fn handle_command(&mut self, cmd: InstrumentCommand) -> Result<()>;
        async fn recover(&mut self) -> Result<()>;
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ROI {
        pub x: u16,
        pub y: u16,
        pub width: u16,
        pub height: u16,
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub enum PowerRange {
        Auto,
        Range(f64),
    }

    #[async_trait]
    pub trait Camera: Instrument {
        async fn snap_frame(&mut self) -> Result<ImageData>;
        async fn start_acquisition(&mut self) -> Result<()>;
        async fn stop_acquisition(&mut self) -> Result<()>;
        fn roi(&self) -> ROI;
        async fn set_roi(&mut self, roi: ROI) -> Result<()>;
        fn exposure_time(&self) -> f64;
        async fn set_exposure_time(&mut self, time_ms: f64) -> Result<()>;

        // Alias methods
        async fn snap(&mut self) -> Result<ImageData> {
            self.snap_frame().await
        }

        async fn start_live(&mut self) -> Result<()> {
            self.start_acquisition().await
        }

        async fn stop_live(&mut self) -> Result<()> {
            self.stop_acquisition().await
        }

        async fn set_exposure_ms(&mut self, ms: f64) -> Result<()> {
            self.set_exposure_time(ms).await
        }

        fn get_exposure_ms(&self) -> f64 {
            self.exposure_time()
        }

        fn get_roi(&self) -> ROI {
            self.roi()
        }

        async fn set_binning(&mut self, _x: u16, _y: u16) -> Result<()> {
            Err(anyhow::anyhow!("Binning not supported"))
        }

        fn get_binning(&self) -> (u16, u16) {
            (1, 1)
        }

        fn get_sensor_size(&self) -> (u32, u32) {
            (2048, 2048)
        }

        fn get_pixel_size_um(&self) -> (f64, f64) {
            (6.5, 6.5)
        }

        fn supports_hardware_trigger(&self) -> bool {
            false
        }
    }

    #[async_trait]
    pub trait PowerMeter: Instrument {
        async fn read_power(&mut self) -> Result<f64>;
        fn power_range(&self) -> PowerRange;
        async fn set_power_range(&mut self, range: PowerRange) -> Result<()>;

        async fn set_wavelength_nm(&mut self, _wavelength: f64) -> Result<()> {
            Err(anyhow::anyhow!("Wavelength setting not supported"))
        }

        async fn set_range(&mut self, range: PowerRange) -> Result<()> {
            self.set_power_range(range).await
        }

        async fn zero(&mut self) -> Result<()> {
            Err(anyhow::anyhow!("Zeroing not supported"))
        }
    }

    #[async_trait]
    pub trait MotionController: Instrument {
        async fn move_absolute(&mut self, position: f64) -> Result<()>;
        async fn move_relative(&mut self, distance: f64) -> Result<()>;
        async fn get_position(&self) -> Result<f64>;
        async fn home(&mut self) -> Result<()>;

        // Extended multi-axis methods
        fn num_axes(&self) -> usize {
            1 // Default to single axis
        }

        async fn get_velocity(&self, _axis: usize) -> Result<f64> {
            Err(anyhow::anyhow!("Velocity readback not supported"))
        }

        async fn set_velocity(&mut self, _axis: usize, _velocity: f64) -> Result<()> {
            Err(anyhow::anyhow!("Velocity control not supported"))
        }

        async fn set_acceleration(&mut self, _axis: usize, _acceleration: f64) -> Result<()> {
            Err(anyhow::anyhow!("Acceleration control not supported"))
        }

        async fn home_axis(&mut self, axis: usize) -> Result<()> {
            if axis == 0 && self.num_axes() == 1 {
                self.home().await
            } else {
                Err(anyhow::anyhow!("Multi-axis homing not supported"))
            }
        }

        async fn stop_axis(&mut self, _axis: usize) -> Result<()> {
            Err(anyhow::anyhow!("Stop not supported"))
        }

        async fn move_absolute_all(&mut self, _positions: Vec<f64>) -> Result<()> {
            Err(anyhow::anyhow!("Multi-axis move not supported"))
        }

        async fn get_positions_all(&self) -> Result<Vec<f64>> {
            if self.num_axes() == 1 {
                Ok(vec![self.get_position().await?])
            } else {
                Err(anyhow::anyhow!("Multi-axis position read not supported"))
            }
        }

        async fn home_all(&mut self) -> Result<()> {
            self.home().await
        }

        async fn stop_all(&mut self) -> Result<()> {
            Err(anyhow::anyhow!("Stop not supported"))
        }

        fn get_units(&self) -> String {
            "steps".to_string()
        }

        fn get_position_range(&self) -> (f64, f64) {
            (0.0, 1000.0)
        }

        fn is_moving(&self) -> bool {
            false
        }
    }

    #[async_trait]
    pub trait TunableLaser: Instrument {
        async fn set_wavelength(&mut self, wavelength_nm: f64) -> Result<()>;
        async fn get_wavelength(&self) -> Result<f64>;
        async fn enable_output(&mut self, enable: bool) -> Result<()>;

        // Alias methods for consistency
        async fn set_wavelength_nm(&mut self, wavelength: f64) -> Result<()> {
            self.set_wavelength(wavelength).await
        }

        async fn get_wavelength_nm(&self) -> Result<f64> {
            self.get_wavelength().await
        }

        async fn get_power_w(&self) -> Result<f64> {
            Err(anyhow::anyhow!("Power readback not supported"))
        }

        async fn set_shutter(&mut self, open: bool) -> Result<()> {
            self.enable_output(open).await
        }

        async fn get_shutter(&self) -> Result<bool> {
            Err(anyhow::anyhow!("Shutter state readback not supported"))
        }

        async fn laser_on(&mut self) -> Result<()> {
            self.enable_output(true).await
        }

        async fn laser_off(&mut self) -> Result<()> {
            self.enable_output(false).await
        }
    }
}
