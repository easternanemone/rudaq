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
use tokio::sync::Mutex;

// Re-export public types from features component
pub use crate::components::features::{
    CameraInfo, CentroidsConfig, CentroidsMode, FanSpeed, GainMode, PPFeature, PPParam, SpeedMode,
};

use crate::components::acquisition::PvcamAcquisition;
use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

/// Driver for Photometrics PVCAM cameras
#[allow(dead_code)]
pub struct PvcamDriver {
    camera_name: String,
    
    // Components
    connection: Arc<Mutex<PvcamConnection>>,
    acquisition: Arc<PvcamAcquisition>,
    
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
        // Run initialization in blocking task
        let connection = tokio::task::spawn_blocking({
            #[cfg(feature = "pvcam_hardware")]
            let name = camera_name.clone();
            move || -> Result<Arc<Mutex<PvcamConnection>>> {
                let conn = PvcamConnection::new();
                #[cfg(feature = "pvcam_hardware")]
                {
                    conn.initialize()?;
                    conn.open(&name)?;
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
            connection,
            acquisition,
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
