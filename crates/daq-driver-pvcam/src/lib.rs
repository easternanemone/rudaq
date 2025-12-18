//! Photometrics PVCAM Camera Driver (Componentized)
//!
//! Refactored to use component architecture:
//! - Connection: Initialization and handles
//! - Acquisition: Streaming and buffers
//! - Features: Parameters and settings

pub mod components;

use anyhow::Result;
use async_trait::async_trait;
use daq_core::capabilities::{Commandable, ExposureControl, Frame, FrameProducer, Parameterized, Triggerable};
use daq_core::core::Roi;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use daq_core::pipeline::MeasurementSource;
use daq_core::error::DaqError;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex;

// Re-export public types from features component
pub use crate::components::features::{
    CameraInfo, CentroidsConfig, CentroidsMode, ClearMode, ExposureMode, ExposureResolution, ExposeOutMode,
    FanSpeed, FrameFlip, FrameRotate, GainMode, PPFeature, PPParam, ReadoutPort, ShutterMode, ShutterStatus,
    SmartStreamEntry, SmartStreamMode, SpeedMode,
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
    
    // Acquisition Parameters
    exposure_ms: Parameter<f64>,
    trigger_mode: Parameter<String>,
    clear_mode: Parameter<String>,
    expose_out_mode: Parameter<String>,
    roi: Parameter<Roi>,
    binning: Parameter<(u16, u16)>,
    armed: Parameter<bool>,
    streaming: Parameter<bool>,

    // Thermal Parameters
    temperature: Parameter<f64>,
    temperature_setpoint: Parameter<f64>,
    fan_speed: Parameter<String>,

    // Readout Parameters
    readout_port: Parameter<String>,
    speed_mode: Parameter<String>,
    gain_mode: Parameter<String>,
    adc_offset: Parameter<i16>,
    full_well_capacity: Parameter<u32>,
    pre_mask: Parameter<u16>,
    post_mask: Parameter<u16>,
    pre_scan: Parameter<u16>,
    post_scan: Parameter<u16>,

    // Readout Timing
    readout_time_us: Parameter<u32>,
    clearing_time_us: Parameter<u32>,
    pre_trigger_delay_us: Parameter<u32>,
    post_trigger_delay_us: Parameter<u32>,
    frame_time_us: Parameter<f64>,

    // Shutter Parameters
    shutter_mode: Parameter<String>,
    shutter_status: Parameter<String>,
    shutter_open_delay: Parameter<u32>,
    shutter_close_delay: Parameter<u32>,

    // Streaming & Metadata
    smart_stream_enabled: Parameter<bool>,
    smart_stream_mode: Parameter<String>,
    metadata_enabled: Parameter<bool>,

    // Host-Side Processing
    host_rotate: Parameter<String>,
    host_flip: Parameter<String>,
    host_summing_enabled: Parameter<bool>,
    host_summing_count: Parameter<u32>,
    
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
        
        // Acquisition Group
        let exposure_ms = Parameter::new("acquisition.exposure_ms", 100.0)
            .with_description("Exposure time")
            .with_unit("ms")
            .with_range(0.1, 60000.0);
            
        let trigger_mode = Parameter::new("acquisition.trigger_mode", ExposureMode::Timed.as_str().to_string())
            .with_description("Trigger mode")
            .with_choices_introspectable(ExposureMode::all_choices());

        let clear_mode = Parameter::new("acquisition.clear_mode", ClearMode::PreExposure.as_str().to_string())
            .with_description("CCD clear mode")
            .with_choices_introspectable(ClearMode::all_choices());

        let expose_out_mode = Parameter::new("acquisition.expose_out_mode", ExposeOutMode::FirstRow.as_str().to_string())
            .with_description("Expose out signal mode")
            .with_choices_introspectable(ExposeOutMode::all_choices());

        let roi = Parameter::new("acquisition.roi", Roi { x: 0, y: 0, width, height })
            .with_description("Region of interest");
            
        let binning = Parameter::new("acquisition.binning", (1u16, 1u16))
            .with_description("Binning (x, y)");
            
        let armed = Parameter::new("acquisition.armed", false)
            .with_description("Camera armed for trigger");

        let streaming = Parameter::new("acquisition.streaming", false)
            .with_description("Camera streaming state");

        // Thermal Group
        let temperature = Parameter::new("thermal.temperature", 0.0)
            .with_description("Current sensor temperature")
            .with_unit("C")
            .read_only();

        let temperature_setpoint = Parameter::new("thermal.setpoint", -10.0)
            .with_description("Temperature setpoint")
            .with_unit("C")
            .with_range(-100.0, 50.0);

        let fan_speed = Parameter::new("thermal.fan_speed", FanSpeed::High.as_str().to_string())
            .with_description("Cooling fan speed")
            .with_choices_introspectable(FanSpeed::all_choices());

        // Readout Group
        let readout_port = Parameter::new("readout.port", "Sensitivity".to_string())
            .with_description("Readout port selection");

        let speed_mode = Parameter::new("readout.speed_mode", "100 MHz".to_string())
            .with_description("Readout speed selection");

        let gain_mode = Parameter::new("readout.gain_mode", "HDR".to_string())
            .with_description("Gain mode selection");

        let adc_offset = Parameter::new("readout.adc_offset", 0i16)
            .with_description("ADC offset");

        let full_well_capacity = Parameter::new("readout.full_well_capacity", 60000u32)
            .with_description("Full well capacity")
            .read_only();

        let pre_mask = Parameter::new("readout.pre_mask", 0u16)
            .with_description("Pre-mask pixels")
            .read_only();

        let post_mask = Parameter::new("readout.post_mask", 0u16)
            .with_description("Post-mask pixels")
            .read_only();

        let pre_scan = Parameter::new("readout.pre_scan", 0u16)
            .with_description("Pre-scan pixels")
            .read_only();

        let post_scan = Parameter::new("readout.post_scan", 0u16)
            .with_description("Post-scan pixels")
            .read_only();

        // Readout Timing
        let readout_time_us = Parameter::new("acquisition.readout_time_us", 0u32)
            .with_description("Readout time")
            .with_unit("us")
            .read_only();

        let clearing_time_us = Parameter::new("acquisition.clearing_time_us", 0u32)
            .with_description("Sensor clearing time")
            .with_unit("us")
            .read_only();

        let pre_trigger_delay_us = Parameter::new("acquisition.pre_trigger_delay_us", 0u32)
            .with_description("Pre-trigger delay")
            .with_unit("us")
            .read_only();

        let post_trigger_delay_us = Parameter::new("acquisition.post_trigger_delay_us", 0u32)
            .with_description("Post-trigger delay")
            .with_unit("us")
            .read_only();

        let frame_time_us = Parameter::new("acquisition.frame_time_us", 0.0)
            .with_description("Total frame time (Exposure + Readout)")
            .with_unit("us")
            .read_only();

        // Shutter Group
        let shutter_mode = Parameter::new("shutter.mode", ShutterMode::Normal.as_str().to_string())
            .with_description("Physical shutter mode")
            .with_choices_introspectable(ShutterMode::all_choices());

        let shutter_status = Parameter::new("shutter.status", ShutterStatus::Closed.as_str().to_string())
            .with_description("Current shutter status")
            .with_choices_introspectable(ShutterStatus::all_choices())
            .read_only();

        let shutter_open_delay = Parameter::new("shutter.open_delay", 0u32)
            .with_description("Shutter open delay")
            .with_unit("us");

        let shutter_close_delay = Parameter::new("shutter.close_delay", 0u32)
            .with_description("Shutter close delay")
            .with_unit("us");

        // Streaming & Metadata Group
        let smart_stream_enabled = Parameter::new("streaming.smart_stream_enabled", false)
            .with_description("Hardware-timed smart streaming");

        let smart_stream_mode = Parameter::new("streaming.smart_stream_mode", SmartStreamMode::Exposures.as_str().to_string())
            .with_description("Smart streaming mode")
            .with_choices_introspectable(SmartStreamMode::all_choices());

        let metadata_enabled = Parameter::new("processing.metadata_enabled", false)
            .with_description("Enable per-frame metadata");

        // Host-Side Processing Group
        let host_rotate = Parameter::new("processing.host_rotate", FrameRotate::None.as_str().to_string())
            .with_description("Host-side frame rotation")
            .with_choices_introspectable(FrameRotate::all_choices());

        let host_flip = Parameter::new("processing.host_flip", FrameFlip::None.as_str().to_string())
            .with_description("Host-side frame flip")
            .with_choices_introspectable(FrameFlip::all_choices());

        let host_summing_enabled = Parameter::new("processing.host_summing_enabled", false)
            .with_description("Enable host-side frame summing");

        let host_summing_count = Parameter::new("processing.host_summing_count", 1u32)
            .with_description("Number of frames to sum on host")
            .with_range(1, 1000);

        // Register all parameters
        params.register(exposure_ms.clone());
        params.register(trigger_mode.clone());
        params.register(clear_mode.clone());
        params.register(expose_out_mode.clone());
        params.register(roi.clone());
        params.register(binning.clone());
        params.register(armed.clone());
        params.register(streaming.clone());
        params.register(temperature.clone());
        params.register(temperature_setpoint.clone());
        params.register(fan_speed.clone());
        params.register(readout_port.clone());
        params.register(speed_mode.clone());
        params.register(gain_mode.clone());
        params.register(adc_offset.clone());
        params.register(full_well_capacity.clone());
        params.register(pre_mask.clone());
        params.register(post_mask.clone());
        params.register(pre_scan.clone());
        params.register(post_scan.clone());
        params.register(readout_time_us.clone());
        params.register(clearing_time_us.clone());
        params.register(pre_trigger_delay_us.clone());
        params.register(post_trigger_delay_us.clone());
        params.register(frame_time_us.clone());
        params.register(shutter_mode.clone());
        params.register(shutter_status.clone());
        params.register(shutter_open_delay.clone());
        params.register(shutter_close_delay.clone());
        params.register(smart_stream_enabled.clone());
        params.register(smart_stream_mode.clone());
        params.register(metadata_enabled.clone());
        params.register(host_rotate.clone());
        params.register(host_flip.clone());
        params.register(host_summing_enabled.clone());
        params.register(host_summing_count.clone());

        let acquisition = Arc::new(PvcamAcquisition::new(streaming.clone()));

        let mut driver = Self {
            camera_name,
            acquisition,
            connection: connection.clone(),
            exposure_ms,
            trigger_mode,
            clear_mode,
            expose_out_mode,
            roi,
            binning,
            armed,
            streaming,
            temperature,
            temperature_setpoint,
            fan_speed,
            readout_port,
            speed_mode,
            gain_mode,
            adc_offset,
            full_well_capacity,
            pre_mask,
            post_mask,
            pre_scan,
            post_scan,
            readout_time_us,
            clearing_time_us,
            pre_trigger_delay_us,
            post_trigger_delay_us,
            frame_time_us,
            shutter_mode,
            shutter_status,
            shutter_open_delay,
            shutter_close_delay,
            smart_stream_enabled,
            smart_stream_mode,
            metadata_enabled,
            host_rotate,
            host_flip,
            host_summing_enabled,
            host_summing_count,
            params,
            sensor_width: width,
            sensor_height: height,
        };

        driver.connect_params();

        // Background polling for drift values (temperature, shutter status, readout timing)
        let temperature_param = driver.temperature.clone();
        let shutter_status_param = driver.shutter_status.clone();
        
        let readout_time_param = driver.readout_time_us.clone();
        let clearing_time_param = driver.clearing_time_us.clone();
        let pre_trigger_param = driver.pre_trigger_delay_us.clone();
        let post_trigger_param = driver.post_trigger_delay_us.clone();
        let frame_time_param = driver.frame_time_us.clone();
        let exposure_param = driver.exposure_ms.clone();

        let conn_poll = connection.clone();
        tokio::spawn(async move {
            tracing::debug!("Starting PVCAM drift polling task");
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let conn_guard = conn_poll.lock().await;
                
                // Poll Temperature
                if let Ok(temp) = PvcamFeatures::get_temperature(&conn_guard) {
                    let _ = temperature_param.set(temp).await;
                }
                
                // Poll Shutter Status
                if let Ok(status) = PvcamFeatures::get_shutter_status(&conn_guard) {
                    let _ = shutter_status_param.set(status.as_str().to_string()).await;
                }

                // Poll Readout Timing (updates when ROI/Binning/Speed changes)
                if let Ok(val) = PvcamFeatures::get_readout_time_us(&conn_guard) {
                    let _ = readout_time_param.set(val).await;
                    // Update frame time (Exposure + Readout)
                    // Exposure is ms, readout is us.
                    let exp_ms = exposure_param.get();
                    let readout_us = val as f64;
                    let _ = frame_time_param.set(exp_ms * 1000.0 + readout_us).await;
                }
                
                if let Ok(val) = PvcamFeatures::get_clearing_time_us(&conn_guard) {
                    let _ = clearing_time_param.set(val).await;
                }

                if let Ok(val) = PvcamFeatures::get_pre_trigger_delay_us(&conn_guard) {
                    let _ = pre_trigger_param.set(val).await;
                }

                if let Ok(val) = PvcamFeatures::get_post_trigger_delay_us(&conn_guard) {
                    let _ = post_trigger_param.set(val).await;
                }
            }
        });
        
        Ok(driver)
    }

    fn connect_params(&mut self) {
        let conn = self.connection.clone();

        // Thermal Setpoint
        self.temperature_setpoint.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_temperature_setpoint(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Fan Speed
        self.fan_speed.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let speed = FanSpeed::from_str(&val);
                    PvcamFeatures::set_fan_speed(&conn_guard, speed)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Trigger Mode
        self.trigger_mode.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let mode = ExposureMode::from_str(&val);
                    PvcamFeatures::set_exposure_mode(&conn_guard, mode)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Clear Mode
        self.clear_mode.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let mode = ClearMode::from_str(&val);
                    PvcamFeatures::set_clear_mode(&conn_guard, mode)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Expose Out Mode
        self.expose_out_mode.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let mode = ExposeOutMode::from_str(&val);
                    PvcamFeatures::set_expose_out_mode(&conn_guard, mode)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Shutter Mode
        self.shutter_mode.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let mode = ShutterMode::from_str(&val);
                    PvcamFeatures::set_shutter_mode(&conn_guard, mode)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Shutter Open Delay
        self.shutter_open_delay.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_shutter_open_delay_us(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Shutter Close Delay
        self.shutter_close_delay.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_shutter_close_delay_us(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // ROI
        self.roi.connect_to_hardware_write({
            let streaming = self.streaming.clone();
            move |_val| {
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        return Err(DaqError::Instrument("Cannot change ROI while streaming".into()));
                    }
                    Ok(())
                })
            }
        });

        // Binning
        self.binning.connect_to_hardware_write({
            let streaming = self.streaming.clone();
            move |_val| {
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        return Err(DaqError::Instrument("Cannot change binning while streaming".into()));
                    }
                    Ok(())
                })
            }
        });

        // Readout Port
        self.readout_port.connect_to_hardware_write({
            let conn = conn.clone();
            let streaming = self.streaming.clone();
            move |name| {
                let conn = conn.clone();
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        return Err(DaqError::Instrument("Cannot change readout port while streaming".into()));
                    }
                    let conn_guard = conn.lock().await;
                    let ports = PvcamFeatures::list_readout_ports(&conn_guard)
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    if let Some(port) = ports.iter().find(|p| p.name == name) {
                        PvcamFeatures::set_readout_port(&conn_guard, port.index)
                            .map_err(|e| DaqError::Instrument(e.to_string()))
                    } else {
                        Err(DaqError::Instrument(format!("Invalid readout port: {}", name)))
                    }
                })
            }
        });

        // Speed Mode
        self.speed_mode.connect_to_hardware_write({
            let conn = conn.clone();
            let streaming = self.streaming.clone();
            move |name| {
                let conn = conn.clone();
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        return Err(DaqError::Instrument("Cannot change speed mode while streaming".into()));
                    }
                    let conn_guard = conn.lock().await;
                    let modes = PvcamFeatures::list_speed_modes(&conn_guard)
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    if let Some(mode) = modes.iter().find(|m| m.name == name) {
                        PvcamFeatures::set_speed_index(&conn_guard, mode.index)
                            .map_err(|e| DaqError::Instrument(e.to_string()))
                    } else {
                        Err(DaqError::Instrument(format!("Invalid speed mode: {}", name)))
                    }
                })
            }
        });

        // Gain Mode
        self.gain_mode.connect_to_hardware_write({
            let conn = conn.clone();
            let streaming = self.streaming.clone();
            move |name| {
                let conn = conn.clone();
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        return Err(DaqError::Instrument("Cannot change gain mode while streaming".into()));
                    }
                    let conn_guard = conn.lock().await;
                    let modes = PvcamFeatures::list_gain_modes(&conn_guard)
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    if let Some(mode) = modes.iter().find(|m| m.name == name) {
                        PvcamFeatures::set_gain_index(&conn_guard, mode.index)
                            .map_err(|e| DaqError::Instrument(e.to_string()))
                    } else {
                        Err(DaqError::Instrument(format!("Invalid gain mode: {}", name)))
                    }
                })
            }
        });

        // Smart Streaming Enabled
        self.smart_stream_enabled.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_smart_stream_enabled(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Smart Streaming Mode
        self.smart_stream_mode.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let mode = SmartStreamMode::from_str(&val);
                    PvcamFeatures::set_smart_stream_mode(&conn_guard, mode)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Metadata Enabled
        self.metadata_enabled.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_metadata_enabled(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // ADC Offset
        self.adc_offset.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_adc_offset(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Host Frame Rotate
        self.host_rotate.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let rotate = FrameRotate::from_str(&val);
                    PvcamFeatures::set_host_frame_rotate(&conn_guard, rotate)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Host Frame Flip
        self.host_flip.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    let flip = FrameFlip::from_str(&val);
                    PvcamFeatures::set_host_frame_flip(&conn_guard, flip)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Host Frame Summing Enabled
        self.host_summing_enabled.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_host_frame_summing_enabled(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });

        // Host Frame Summing Count
        self.host_summing_count.connect_to_hardware_write({
            let conn = conn.clone();
            move |val| {
                let conn = conn.clone();
                Box::pin(async move {
                    let conn_guard = conn.lock().await;
                    PvcamFeatures::set_host_frame_summing_count(&conn_guard, val)
                        .map_err(|e| DaqError::Instrument(e.to_string()))
                })
            }
        });
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

    /// Gracefully shutdown the driver, stopping any active streaming.
    ///
    /// This method should be called before dropping the driver when running
    /// in an async context to ensure proper cleanup. If not called, the Drop
    /// implementation will attempt best-effort cleanup but cannot perform
    /// async operations.
    ///
    /// # Example
    /// ```ignore
    /// let driver = PvcamDriver::new_async("PrimeBSI").await?;
    /// driver.start_stream().await?;
    /// // ... use the camera ...
    /// driver.shutdown().await?;  // Clean shutdown before drop
    /// ```
    ///
    /// # Safety (bd-lwg7)
    /// This method is safe to call from any async context. The Drop implementation
    /// cannot use `block_on()` as it would panic if called from within an async
    /// context, so this explicit shutdown method is preferred.
    pub async fn shutdown(&self) -> Result<()> {
        if self.streaming.get() {
            tracing::debug!("PvcamDriver::shutdown - stopping active stream");
            let conn = self.connection.lock().await;
            self.acquisition.stop_stream(&conn).await?;
            tracing::debug!("PvcamDriver::shutdown - stream stopped");
        }
        Ok(())
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

#[async_trait]
impl Commandable for PvcamDriver {
    async fn execute_command(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let conn = self.connection.lock().await;

        match command {
            "reset_pp" => {
                PvcamFeatures::reset_pp_features(&conn)?;
                Ok(serde_json::json!({ "success": true }))
            }
            "upload_smart_stream" => {
                let exposures = args.get("exposures")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'exposures' array argument"))?;
                
                let exposures_u32: Vec<u32> = exposures.iter()
                    .map(|v| v.as_u64().unwrap_or(0) as u32)
                    .collect();
                
                PvcamFeatures::upload_smart_stream(&conn, &exposures_u32)?;
                Ok(serde_json::json!({ "success": true, "count": exposures_u32.len() }))
            }
            _ => anyhow::bail!("Unknown command: {}", command),
        }
    }
}

/// Drop impl ensures streaming state is signaled for cleanup (bd-nq82, bd-lwg7).
///
/// # Important (bd-lwg7)
/// This Drop implementation does NOT call `block_on()` to avoid panicking when
/// dropped inside an async context. Instead, it relies on:
///
/// 1. Users calling `shutdown().await` before dropping (preferred)
/// 2. `PvcamAcquisition::Drop` setting shutdown flags and signaling the poll thread
///
/// The poll thread will exit on its next iteration when it sees the shutdown flag.
/// This may leave hardware in a streaming state briefly, but avoids runtime panics.
///
/// For clean shutdown, always call `driver.shutdown().await` before dropping.
impl Drop for PvcamDriver {
    fn drop(&mut self) {
        if self.streaming.get() {
            // Log warning - user should have called shutdown() first
            tracing::warn!(
                "PvcamDriver dropped while streaming was active. \
                 Call driver.shutdown().await before dropping for clean shutdown. \
                 Relying on PvcamAcquisition::Drop for best-effort cleanup."
            );
            // PvcamAcquisition::Drop will:
            // 1. Set shutdown flag (atomic)
            // 2. Signal callback context
            // 3. Abort poll handle
            // This is non-blocking and safe from any context.
        }
    }
}
