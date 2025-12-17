//! Photometrics PVCAM Camera Driver (Componentized)
//!
//! Refactored to use component architecture:
//! - Connection: Initialization and handles
//! - Acquisition: Streaming and buffers
//! - Features: Parameters and settings

pub mod components;

use anyhow::Result;
use async_trait::async_trait;
use daq_core::capabilities::{ExposureControl, Frame, FrameProducer, Parameterized, Triggerable};
use daq_core::core::Roi;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use daq_core::pipeline::MeasurementSource;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

// Re-export public types from features component
pub use crate::components::features::{
    CameraInfo, CentroidsConfig, CentroidsMode, ClearMode, ExposureMode, ExposeOutMode,
    FanSpeed, GainMode, PPFeature, PPParam, ReadoutPort, SpeedMode,
};
// Re-export feature functions for direct access
pub use crate::components::features::PvcamFeatures;

use crate::components::acquisition::PvcamAcquisition;
use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

/// Driver for Photometrics PVCAM cameras
///
/// # Drop Order (bd-nq82)
///
/// Fields drop in declaration order. `acquisition` MUST drop before `connection`
/// to ensure the poll thread stops before the SDK is uninitialized. The explicit
/// Drop impl also calls stop_stream() to ensure clean shutdown.
#[allow(dead_code)]
pub struct PvcamDriver {
    camera_name: String,

    // Components - ORDER MATTERS for drop safety (bd-nq82)
    // acquisition must drop BEFORE connection to stop poll thread before SDK uninit
    acquisition: Arc<PvcamAcquisition>,
    connection: Arc<Mutex<PvcamConnection>>,
    
    // Parameters
    exposure_ms: Parameter<f64>,
    roi: Parameter<Roi>,
    binning: Parameter<(u16, u16)>,
    armed: Parameter<bool>,
    streaming: Parameter<bool>,
    temperature: Parameter<f64>,
    temperature_setpoint: Parameter<f64>,
    fan_speed: Parameter<String>,
    gain_index: Parameter<u16>,
    speed_index: Parameter<u16>,
    
    params: ParameterSet,
    
    sensor_width: u32,
    sensor_height: u32,
}

impl PvcamDriver {
    pub async fn new_async(camera_name: String) -> Result<Self> {
        tracing::info!("PvcamDriver::new_async called for camera: {}", camera_name);
        tracing::info!("pvcam_hardware feature enabled: {}", cfg!(feature = "pvcam_hardware"));

        // Run initialization in blocking task
        let connection = tokio::task::spawn_blocking({
            #[cfg(feature = "pvcam_hardware")]
            let name = camera_name.clone();
            move || -> Result<Arc<Mutex<PvcamConnection>>> {
                #[cfg(feature = "pvcam_hardware")]
                let mut conn = PvcamConnection::new();
                #[cfg(not(feature = "pvcam_hardware"))]
                let conn = PvcamConnection::new();

                #[cfg(feature = "pvcam_hardware")]
                {
                    tracing::info!("Initializing PVCAM SDK...");
                    conn.initialize()?;
                    tracing::info!("PVCAM SDK initialized, opening camera: {}", name);
                    conn.open(&name)?;
                    tracing::info!("Camera opened successfully, handle: {:?}", conn.handle());
                }
                #[cfg(not(feature = "pvcam_hardware"))]
                {
                    tracing::warn!("pvcam_hardware feature NOT enabled - using mock mode");
                }
                Ok(Arc::new(Mutex::new(conn)))
            }
        }).await??;
        
        Self::create(camera_name, connection).await
    }

    #[deprecated(note = "Use new_async()")]
    pub fn new(camera_name: &str) -> Result<Self> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(Self::new_async(camera_name.to_string()))
    }

    async fn create(camera_name: String, connection: Arc<Mutex<PvcamConnection>>) -> Result<Self> {
        // Query sensor size
        let (width, height) = {
            #[allow(unused_mut)]
            let mut w = 2048;
            #[allow(unused_mut)]
            let mut h = 2048;
            #[cfg(feature = "pvcam_hardware")]
            {
                let conn = connection.lock().await;
                if let Some(hcam) = conn.handle() {
                    unsafe {
                        let mut ser: uns16 = 0;
                        let mut par: uns16 = 0;
                        // SAFETY: hcam is open; ser/par are valid out pointers for current dimensions.
                        pl_get_param(hcam, PARAM_SER_SIZE, ATTR_CURRENT, &mut ser as *mut _ as *mut _);
                        pl_get_param(hcam, PARAM_PAR_SIZE, ATTR_CURRENT, &mut par as *mut _ as *mut _);
                        if ser > 0 && par > 0 {
                            w = ser as u32;
                            h = par as u32;
                        }
                    }
                }
            }
            (w, h)
        };

        let mut params = ParameterSet::new();
        
        let exposure_ms = Parameter::new("exposure_ms", 100.0)
            .with_description("Exposure time")
            .with_unit("ms")
            .with_range(0.1, 60000.0);
            
        let roi = Parameter::new("roi", Roi { x: 0, y: 0, width, height })
            .with_description("Region of interest");
            
        let binning = Parameter::new("binning", (1u16, 1u16))
            .with_description("Binning (x, y)");
            
        let armed = Parameter::new("armed", false).with_description("Armed");
        let streaming = Parameter::new("streaming", false).with_description("Streaming");
        let temperature = Parameter::new("temperature", 0.0).with_unit("C");
        let temperature_setpoint = Parameter::new("temperature_setpoint", -10.0).with_unit("C");
        let fan_speed = Parameter::new("fan_speed", "High".to_string());
        let gain_index = Parameter::new("gain_index", 0u16);
        let speed_index = Parameter::new("speed_index", 0u16);

        params.register(exposure_ms.clone());
        params.register(roi.clone());
        params.register(binning.clone());
        params.register(armed.clone());
        params.register(streaming.clone());
        params.register(temperature.clone());
        params.register(temperature_setpoint.clone());
        params.register(fan_speed.clone());
        params.register(gain_index.clone());
        params.register(speed_index.clone());

        let acquisition = Arc::new(PvcamAcquisition::new(streaming.clone()));

        Ok(Self {
            camera_name,
            // Field order matches struct declaration for drop safety (bd-nq82)
            acquisition,
            connection,
            exposure_ms,
            roi,
            binning,
            armed,
            streaming,
            temperature,
            temperature_setpoint,
            fan_speed,
            gain_index,
            speed_index,
            params,
            sensor_width: width,
            sensor_height: height,
        })
    }

    pub async fn acquire_frame(&self) -> Result<Frame> {
        let conn = self.connection.lock().await;
        self.acquisition.acquire_single_frame(&conn, self.roi.get(), self.binning.get(), self.exposure_ms.get()).await
    }

    pub fn resolution(&self) -> (u32, u32) {
        (self.sensor_width, self.sensor_height)
    }

    /// Register an Arrow tap to receive frames as `UInt16Array` (requires `arrow_tap` feature).
    #[cfg(feature = "arrow_tap")]
    pub async fn set_arrow_tap(
        &self,
        tx: tokio::sync::mpsc::Sender<std::sync::Arc<arrow::array::UInt16Array>>,
    ) {
        self.acquisition.set_arrow_tap(tx).await;
    }
}

#[async_trait]
impl ExposureControl for PvcamDriver {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        self.exposure_ms.set(seconds * 1000.0).await
    }
    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.exposure_ms.get() / 1000.0)
    }
}

#[async_trait]
impl Triggerable for PvcamDriver {
    async fn arm(&self) -> Result<()> {
        self.armed.set(true).await
    }
    async fn trigger(&self) -> Result<()> {
        // Software trigger logic
        Ok(())
    }
    async fn is_armed(&self) -> Result<bool> {
        Ok(self.armed.get())
    }
}

#[async_trait]
impl FrameProducer for PvcamDriver {
    async fn start_stream(&self) -> Result<()> {
        let conn = self.connection.lock().await;
        self.acquisition.start_stream(
            &conn,
            self.roi.get(),
            self.binning.get(),
            self.exposure_ms.get()
        ).await
    }

    async fn stop_stream(&self) -> Result<()> {
        let conn = self.connection.lock().await;
        self.acquisition.stop_stream(&conn).await
    }

    fn resolution(&self) -> (u32, u32) {
        (self.sensor_width, self.sensor_height)
    }

    async fn subscribe_frames(&self) -> Option<tokio::sync::broadcast::Receiver<Arc<Frame>>> {
        Some(self.acquisition.frame_tx.subscribe())
    }

    async fn is_streaming(&self) -> Result<bool> {
        Ok(self.streaming.get())
    }

    fn frame_count(&self) -> u64 {
        self.acquisition.frame_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl MeasurementSource for PvcamDriver {
    type Output = Arc<Frame>;
    type Error = anyhow::Error;

    async fn register_output(&self, tx: tokio::sync::mpsc::Sender<Self::Output>) -> Result<(), Self::Error> {
        let mut reliable = self.acquisition.reliable_tx.lock().await;
        *reliable = Some(tx);
        Ok(())
    }
}

impl Parameterized for PvcamDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

/// Drop impl ensures streaming is stopped before acquisition/connection drop (bd-nq82).
///
/// This prevents the poll thread from calling PVCAM SDK functions after uninit.
/// We use block_on to synchronously stop streaming since Drop cannot be async.
impl Drop for PvcamDriver {
    fn drop(&mut self) {
        // Only attempt cleanup if streaming is active
        if self.streaming.get() {
            tracing::debug!("PvcamDriver::drop - stopping active stream");

            // Use tokio's current runtime to block on stop_stream
            // This ensures the poll thread exits before we drop acquisition/connection
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let conn = self.connection.clone();
                let acq = self.acquisition.clone();
                let _ = handle.block_on(async move {
                    let conn_guard = conn.lock().await;
                    let _ = acq.stop_stream(&conn_guard).await;
                });
                tracing::debug!("PvcamDriver::drop - stream stopped");
            } else {
                // No tokio runtime - acquisition Drop will handle shutdown via flag
                tracing::warn!("PvcamDriver::drop - no tokio runtime, relying on acquisition Drop");
            }
        }
    }
}
