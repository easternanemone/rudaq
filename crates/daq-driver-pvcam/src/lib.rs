//! Photometrics PVCAM Camera Driver (Componentized)
//!
//! Refactored to use component architecture:
//! - Connection: Initialization and handles
//! - Acquisition: Streaming and buffers
//! - Features: Parameters and settings

pub mod components;

use anyhow::Result;
use async_trait::async_trait;
use daq_core::capabilities::{
    Commandable, ExposureControl, Frame, FrameProducer, Parameterized, Triggerable,
};
use daq_core::core::Roi;
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use daq_core::pipeline::MeasurementSource;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex;

// Re-export public types from features component
pub use crate::components::features::{
    CameraInfo, CentroidsConfig, CentroidsMode, ClearMode, ExposeOutMode, ExposureMode,
    ExposureResolution, FanSpeed, FrameFlip, FrameRotate, GainMode, PPFeature, PPParam,
    ReadoutPort, ShutterMode, ShutterStatus, SmartStreamEntry, SmartStreamMode, SpeedMode,
};
// Re-export feature functions for direct access
pub use crate::components::features::PvcamFeatures;

use crate::components::acquisition::PvcamAcquisition;
use crate::components::connection::PvcamConnection;
use crate::components::speed_table::SpeedTable;
#[cfg(feature = "pvcam_sdk")]
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
    buffer_mode: Parameter<String>,
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
    pixel_time_ns: Parameter<u32>,
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

    // Metadata (Info)
    serial_number: Parameter<String>,
    firmware_version: Parameter<String>,
    model_name: Parameter<String>,
    bit_depth: Parameter<u16>,

    params: ParameterSet,

    sensor_width: u32,
    sensor_height: u32,
    speed_table: Option<Arc<SpeedTable>>,
}

impl PvcamDriver {
    pub async fn new_async(camera_name: String) -> Result<Self> {
        tracing::info!("PvcamDriver::new_async called for camera: {}", camera_name);
        tracing::info!("pvcam_sdk feature enabled: {}", cfg!(feature = "pvcam_sdk"));

        // Run initialization in blocking task

        let connection = tokio::task::spawn_blocking({
            #[cfg(feature = "pvcam_sdk")]
            let name = camera_name.clone();
            move || -> Result<Arc<Mutex<PvcamConnection>>> {
                #[cfg(feature = "pvcam_sdk")]
                let mut conn = PvcamConnection::new();
                #[cfg(not(feature = "pvcam_sdk"))]
                let conn = PvcamConnection::new();

                #[cfg(feature = "pvcam_sdk")]
                {
                    tracing::info!("Initializing PVCAM SDK...");
                    conn.initialize()?;
                    tracing::info!("PVCAM SDK initialized, opening camera: {}", name);
                    conn.open(&name)?;
                    tracing::info!("Camera opened successfully, handle: {:?}", conn.handle());
                }
                #[cfg(not(feature = "pvcam_sdk"))]
                {
                    tracing::warn!("pvcam_sdk feature NOT enabled - using mock mode");
                }
                Ok(Arc::new(Mutex::new(conn)))
            }
        })
        .await??;

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
            #[cfg(feature = "pvcam_sdk")]
            {
                let conn = connection.lock().await;
                if let Some(hcam) = conn.handle() {
                    unsafe {
                        let mut ser: uns16 = 0;
                        let mut par: uns16 = 0;
                        // SAFETY: hcam is open; ser/par are valid out pointers for current dimensions.
                        pl_get_param(
                            hcam,
                            PARAM_SER_SIZE,
                            ATTR_CURRENT,
                            &mut ser as *mut _ as *mut _,
                        );
                        pl_get_param(
                            hcam,
                            PARAM_PAR_SIZE,
                            ATTR_CURRENT,
                            &mut par as *mut _ as *mut _,
                        );
                        if ser > 0 && par > 0 {
                            w = ser as u32;
                            h = par as u32;
                        }
                    }
                }
            }
            (w, h)
        };

        // Fetch Camera Info for metadata parameters
        let info = {
            let conn = connection.lock().await;
            PvcamFeatures::get_camera_info(&conn).unwrap_or_else(|e| {
                tracing::warn!("Failed to get camera info: {}", e);
                CameraInfo {
                    serial_number: "Unknown".to_string(),
                    firmware_version: "Unknown".to_string(),
                    chip_name: "Unknown".to_string(),
                    temperature_c: 0.0,
                    bit_depth: 0,
                    pixel_time_ns: 0,
                    pixel_size_nm: (0, 0),
                    sensor_size: (0, 0),
                    gain_name: "Unknown".to_string(),
                    speed_name: "Unknown".to_string(),
                    port_name: "Unknown".to_string(),
                    gain_index: 0,
                    speed_index: 0,
                }
            })
        };

        // Build speed table cache (bd-c4hf.3)
        let speed_table = {
            #[cfg(feature = "pvcam_sdk")]
            {
                let conn = connection.lock().await;
                if let Some(h) = conn.handle() {
                    match SpeedTable::build(h) {
                        Ok(table) => Some(Arc::new(table)),
                        Err(e) => {
                            tracing::warn!("Failed to build speed table: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            #[cfg(not(feature = "pvcam_sdk"))]
            {
                match SpeedTable::build(0) {
                    Ok(table) => Some(Arc::new(table)),
                    Err(e) => {
                        tracing::warn!("Failed to build mock speed table: {}", e);
                        None
                    }
                }
            }
        };

        // Select defaults from cached table or fall back
        let (default_port_name, default_speed_name, default_gain_name) =
            if let Some(table) = speed_table.as_ref() {
                let port = table
                    .ports
                    .iter()
                    .find(|p| p.name == info.port_name)
                    .or_else(|| table.ports.first());
                let port_name = port
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Sensitivity".to_string());

                let speed = port.and_then(|p| {
                    p.speeds
                        .iter()
                        .find(|s| s.index as u32 == info.speed_index as u32)
                        .or_else(|| p.speeds.iter().find(|s| s.name == info.speed_name))
                        .or_else(|| p.speeds.first())
                });
                let speed_name = speed
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "100 MHz".to_string());

                let gain = speed.and_then(|s| {
                    s.gains
                        .iter()
                        .find(|g| g.index as u32 == info.gain_index as u32)
                        .or_else(|| s.gains.iter().find(|g| g.name == info.gain_name))
                        .or_else(|| s.gains.first())
                });
                let gain_name = gain
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| "HDR".to_string());

                (port_name, speed_name, gain_name)
            } else {
                (
                    "Sensitivity".to_string(),
                    "100 MHz".to_string(),
                    "HDR".to_string(),
                )
            };

        let mut params = ParameterSet::new();

        // Acquisition Group
        let exposure_ms = Parameter::new("acquisition.exposure_ms", 100.0)
            .with_description("Exposure time")
            .with_unit("ms")
            .with_range(0.1, 60000.0);

        let trigger_mode = Parameter::new(
            "acquisition.trigger_mode",
            ExposureMode::Timed.as_str().to_string(),
        )
        .with_description("Trigger mode")
        .with_choices_introspectable(ExposureMode::all_choices());

        let clear_mode = Parameter::new(
            "acquisition.clear_mode",
            ClearMode::PreExposure.as_str().to_string(),
        )
        .with_description("Clear mode")
        .with_choices_introspectable(ClearMode::all_choices());

        let expose_out_mode = Parameter::new(
            "acquisition.expose_out_mode",
            ExposeOutMode::FirstRow.as_str().to_string(),
        )
        .with_description("Expose out signal mode")
        .with_choices_introspectable(ExposeOutMode::all_choices());

        let buffer_mode = Parameter::new("acquisition.buffer_mode", "Overwrite".to_string())
            .with_description("Circular buffer mode")
            .with_choices_introspectable(vec!["Overwrite".into(), "No Overwrite".into()]);

        let roi = Parameter::new(
            "acquisition.roi",
            Roi {
                x: 0,
                y: 0,
                width,
                height,
            },
        )
        .with_description("Region of interest");

        let binning =
            Parameter::new("acquisition.binning", (1u16, 1u16)).with_description("Binning (x, y)");

        let armed =
            Parameter::new("acquisition.armed", false).with_description("Camera armed for trigger");

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
        let readout_port = Parameter::new("readout.port", default_port_name)
            .with_description("Readout port selection");

        let speed_mode = Parameter::new("readout.speed_mode", default_speed_name)
            .with_description("Readout speed selection");

        let gain_mode = Parameter::new("readout.gain_mode", default_gain_name)
            .with_description("Gain mode selection");

        let adc_offset = Parameter::new("readout.adc_offset", 0i16).with_description("ADC offset");

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

        let pixel_time_ns = Parameter::new("acquisition.pixel_time_ns", 0u32)
            .with_description("Pixel time")
            .with_unit("ns")
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

        let shutter_status =
            Parameter::new("shutter.status", ShutterStatus::Closed.as_str().to_string())
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

        let smart_stream_mode = Parameter::new(
            "streaming.smart_stream_mode",
            SmartStreamMode::Exposures.as_str().to_string(),
        )
        .with_description("Smart streaming mode")
        .with_choices_introspectable(SmartStreamMode::all_choices());

        let metadata_enabled = Parameter::new("processing.metadata_enabled", false)
            .with_description("Enable per-frame metadata");

        // Host-Side Processing Group
        let host_rotate = Parameter::new(
            "processing.host_rotate",
            FrameRotate::None.as_str().to_string(),
        )
        .with_description("Host-side frame rotation")
        .with_choices_introspectable(FrameRotate::all_choices());

        let host_flip =
            Parameter::new("processing.host_flip", FrameFlip::None.as_str().to_string())
                .with_description("Host-side frame flip")
                .with_choices_introspectable(FrameFlip::all_choices());

        let host_summing_enabled = Parameter::new("processing.host_summing_enabled", false)
            .with_description("Enable host-side frame summing");

        let host_summing_count = Parameter::new("processing.host_summing_count", 1u32)
            .with_description("Number of frames to sum on host")
            .with_range(1, 1000);

        // Metadata Info Group
        let serial_number = Parameter::new("info.serial_number", info.serial_number)
            .with_description("Camera Serial Number")
            .read_only();

        let firmware_version = Parameter::new("info.firmware_version", info.firmware_version)
            .with_description("Camera Firmware Version")
            .read_only();

        let model_name = Parameter::new("info.model_name", info.chip_name)
            .with_description("Camera Model / Chip Name")
            .read_only();

        let bit_depth = Parameter::new("info.bit_depth", info.bit_depth)
            .with_description("ADC Bit Depth")
            .read_only();

        // Register all parameters
        params.register(exposure_ms.clone());
        params.register(trigger_mode.clone());
        params.register(clear_mode.clone());
        params.register(expose_out_mode.clone());
        params.register(buffer_mode.clone());
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
        params.register(pixel_time_ns.clone());
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
        params.register(serial_number.clone());
        params.register(firmware_version.clone());
        params.register(model_name.clone());
        params.register(bit_depth.clone());

        let acquisition = Arc::new(PvcamAcquisition::new(
            streaming.clone(),
            buffer_mode.clone(),
        ));

        let mut driver = Self {
            camera_name,
            acquisition,
            connection: connection.clone(),
            exposure_ms,
            trigger_mode,
            clear_mode,
            expose_out_mode,
            buffer_mode,
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
            pixel_time_ns,
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
            serial_number,
            firmware_version,
            model_name,
            bit_depth,
            params,
            sensor_width: width,
            sensor_height: height,
            speed_table: speed_table.clone(),
        };

        driver.connect_params();

        // Populate dynamic enum choices from camera (bd-c4hf.2)
        driver.populate_dynamic_choices().await;

        // Wire dependent choice updates (bd-c4hf.4)
        driver.attach_choice_listeners().await;

        // Background polling for drift values (temperature, shutter status, readout timing)
        let temperature_param = driver.temperature.clone();
        let shutter_status_param = driver.shutter_status.clone();

        let readout_time_param = driver.readout_time_us.clone();
        let pixel_time_param = driver.pixel_time_ns.clone();
        let clearing_time_param = driver.clearing_time_us.clone();
        let pre_trigger_param = driver.pre_trigger_delay_us.clone();
        let post_trigger_param = driver.post_trigger_delay_us.clone();
        let frame_time_param = driver.frame_time_us.clone();
        let exposure_param = driver.exposure_ms.clone();

        let conn_poll = connection.clone();
        // Critical fix (ADR-pvcam-continuous-acquisition, 2026-01-12):
        // Skip drift polling while streaming to prevent SDK callback corruption.
        // pl_get_param() calls during continuous acquisition corrupt callback state.
        let streaming_check = driver.streaming.clone();
        tokio::spawn(async move {
            tracing::debug!("Starting PVCAM drift polling task");
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;

                // CRITICAL: Skip all SDK calls while streaming is active (bd-nzcq)
                // Calling pl_get_param during acquisition corrupts the SDK's
                // internal callback state, causing READOUT_NOT_ACTIVE and no further callbacks.
                if streaming_check.get() {
                    tracing::trace!("Drift polling skipped (streaming active)");
                    continue;
                }

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

                // Pixel time is driven by cached speed table listeners; no periodic polling needed here.
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

        // Buffer Mode (guard against streaming changes)
        self.buffer_mode.connect_to_hardware_write({
            let streaming = self.streaming.clone();
            move |_val| {
                let streaming = streaming.clone();
                Box::pin(async move {
                    if streaming.get() {
                        Err(DaqError::Instrument(
                            "Cannot change buffer_mode while streaming".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
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
                        return Err(DaqError::Instrument(
                            "Cannot change ROI while streaming".into(),
                        ));
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
                        return Err(DaqError::Instrument(
                            "Cannot change binning while streaming".into(),
                        ));
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
                        return Err(DaqError::Instrument(
                            "Cannot change readout port while streaming".into(),
                        ));
                    }
                    let conn_guard = conn.lock().await;
                    let ports = PvcamFeatures::list_readout_ports(&conn_guard)
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    if let Some(port) = ports.iter().find(|p| p.name == name) {
                        PvcamFeatures::set_readout_port(&conn_guard, port.index)
                            .map_err(|e| DaqError::Instrument(e.to_string()))
                    } else {
                        Err(DaqError::Instrument(format!(
                            "Invalid readout port: {}",
                            name
                        )))
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
                        return Err(DaqError::Instrument(
                            "Cannot change speed mode while streaming".into(),
                        ));
                    }
                    let conn_guard = conn.lock().await;
                    let modes = PvcamFeatures::list_speed_modes(&conn_guard)
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    if let Some(mode) = modes.iter().find(|m| m.name == name) {
                        PvcamFeatures::set_speed_index(&conn_guard, mode.index)
                            .map_err(|e| DaqError::Instrument(e.to_string()))
                    } else {
                        Err(DaqError::Instrument(format!(
                            "Invalid speed mode: {}",
                            name
                        )))
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
                        return Err(DaqError::Instrument(
                            "Cannot change gain mode while streaming".into(),
                        ));
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

    /// Populate dynamic enum choices from the camera (bd-c4hf.2)
    ///
    /// Queries the camera for available options and updates parameter choices.
    /// This enables dynamic dropdowns in the GUI that reflect actual hardware
    /// capabilities rather than hardcoded values.
    async fn populate_dynamic_choices(&self) {
        self.apply_cached_choices().await;
    }

    /// Wire reactive dropdown updates using cached SpeedTable (bd-c4hf.4)
    async fn attach_choice_listeners(&self) {
        if self.speed_table.is_none() {
            return;
        }

        let table = self.speed_table.as_ref().unwrap().clone();
        let readout_port = self.readout_port.clone();
        let speed_mode = self.speed_mode.clone();
        let gain_mode = self.gain_mode.clone();
        let bit_depth = self.bit_depth.clone();
        let pixel_time_ns = self.pixel_time_ns.clone();

        // On port change -> update speeds, gain, read-only fields
        tokio::spawn(async move {
            readout_port
                .add_change_listener(move |new_port| {
                    let table = table.clone();
                    let speed_mode = speed_mode.clone();
                    let gain_mode = gain_mode.clone();
                    let bit_depth = bit_depth.clone();
                    let pixel_time_ns = pixel_time_ns.clone();
                    let new_port = new_port.clone();
                    tokio::spawn(async move {
                        if let Some(port) = table.ports.iter().find(|p| p.name == new_port) {
                            let speed_names: Vec<String> =
                                port.speeds.iter().map(|s| s.name.clone()).collect();
                            speed_mode.inner().update_choices(speed_names.clone());
                            if !speed_names.iter().any(|n| *n == speed_mode.get()) {
                                if let Some(first) = speed_names.first() {
                                    let _ = speed_mode.set(first.clone()).await;
                                }
                            }

                            // Cascade to gain
                            let selected_speed_name = speed_mode.get();
                            let selected_speed = port
                                .speeds
                                .iter()
                                .find(|s| s.name == selected_speed_name)
                                .cloned()
                                .or_else(|| port.speeds.first().cloned());

                            if let Some(speed) = selected_speed {
                                let gain_names: Vec<String> =
                                    speed.gains.iter().map(|g| g.name.clone()).collect();
                                gain_mode.inner().update_choices(gain_names.clone());
                                if !gain_names.iter().any(|n| *n == gain_mode.get()) {
                                    if let Some(first) = gain_names.first() {
                                        let _ = gain_mode.set(first.clone()).await;
                                    }
                                }

                                let _ = bit_depth.set(speed.bit_depth as u16).await;
                                let _ = pixel_time_ns.set(speed.pix_time_ns as u32).await;
                            }
                        }
                    });
                })
                .await;
        });

        let table = self.speed_table.as_ref().unwrap().clone();
        let speed_mode = self.speed_mode.clone();
        let gain_mode = self.gain_mode.clone();
        let bit_depth = self.bit_depth.clone();
        let pixel_time_ns = self.pixel_time_ns.clone();
        let readout_port = self.readout_port.clone();

        // On speed change -> update gains, read-only fields
        speed_mode
            .add_change_listener(move |new_speed| {
                let table = table.clone();
                let gain_mode = gain_mode.clone();
                let bit_depth = bit_depth.clone();
                let pixel_time_ns = pixel_time_ns.clone();
                let readout_port = readout_port.clone();
                let new_speed = new_speed.clone();
                tokio::spawn(async move {
                    let current_port = readout_port.get();
                    if let Some(port) = table.ports.iter().find(|p| p.name == current_port) {
                        if let Some(speed) =
                            port.speeds.iter().find(|s| s.name == new_speed).cloned()
                        {
                            let gain_names: Vec<String> =
                                speed.gains.iter().map(|g| g.name.clone()).collect();
                            gain_mode.inner().update_choices(gain_names.clone());
                            if !gain_names.iter().any(|n| *n == gain_mode.get()) {
                                if let Some(first) = gain_names.first() {
                                    let _ = gain_mode.set(first.clone()).await;
                                }
                            }

                            let _ = bit_depth.set(speed.bit_depth as u16).await;
                            let _ = pixel_time_ns.set(speed.pix_time_ns as u32).await;
                        }
                    }
                });
            })
            .await;
    }

    /// Apply cached choices from the SpeedTable if available; otherwise query hardware.
    async fn apply_cached_choices(&self) {
        // Fast path: use cached SpeedTable when available.
        if let Some(table) = self.speed_table.as_ref() {
            let port_names: Vec<String> = table.ports.iter().map(|p| p.name.clone()).collect();
            self.readout_port.inner().update_choices(port_names.clone());

            if !port_names.iter().any(|n| *n == self.readout_port.get()) {
                if let Some(first) = port_names.first() {
                    let _ = self.readout_port.set(first.clone()).await;
                }
            }

            let selected_port_name = self.readout_port.get();
            let selected_port = table
                .ports
                .iter()
                .find(|p| p.name == selected_port_name)
                .cloned()
                .or_else(|| table.ports.first().cloned());

            if let Some(port) = selected_port {
                let speed_names: Vec<String> = port.speeds.iter().map(|s| s.name.clone()).collect();
                self.speed_mode.inner().update_choices(speed_names.clone());
                if !speed_names.iter().any(|n| *n == self.speed_mode.get()) {
                    if let Some(first) = speed_names.first() {
                        let _ = self.speed_mode.set(first.clone()).await;
                    }
                }

                let selected_speed_name = self.speed_mode.get();
                let selected_speed = port
                    .speeds
                    .iter()
                    .find(|s| s.name == selected_speed_name)
                    .cloned()
                    .or_else(|| port.speeds.first().cloned());

                if let Some(speed) = selected_speed {
                    let gain_names: Vec<String> =
                        speed.gains.iter().map(|g| g.name.clone()).collect();
                    self.gain_mode.inner().update_choices(gain_names.clone());
                    if !gain_names.iter().any(|n| *n == self.gain_mode.get()) {
                        if let Some(first) = gain_names.first() {
                            let _ = self.gain_mode.set(first.clone()).await;
                        }
                    }

                    // Update read-only info parameters when available
                    let _ = self.bit_depth.set(speed.bit_depth as u16).await;
                    let _ = self.pixel_time_ns.set(speed.pix_time_ns as u32).await;
                }
            }

            return;
        }

        let conn_guard = self.connection.lock().await;

        // Populate readout port choices
        match PvcamFeatures::list_readout_ports(&conn_guard) {
            Ok(ports) => {
                let names: Vec<String> = ports.iter().map(|p| p.name.clone()).collect();
                tracing::debug!("Populating readout port choices: {:?}", names);
                self.readout_port.inner().update_choices(names);
            }
            Err(e) => {
                tracing::warn!("Failed to query readout ports: {}", e);
            }
        }

        // Populate speed mode choices
        match PvcamFeatures::list_speed_modes(&conn_guard) {
            Ok(modes) => {
                let names: Vec<String> = modes.iter().map(|m| m.name.clone()).collect();
                tracing::debug!("Populating speed mode choices: {:?}", names);
                self.speed_mode.inner().update_choices(names);
            }
            Err(e) => {
                tracing::warn!("Failed to query speed modes: {}", e);
            }
        }

        // Populate gain mode choices
        match PvcamFeatures::list_gain_modes(&conn_guard) {
            Ok(modes) => {
                let names: Vec<String> = modes.iter().map(|m| m.name.clone()).collect();
                tracing::debug!("Populating gain mode choices: {:?}", names);
                self.gain_mode.inner().update_choices(names);
            }
            Err(e) => {
                tracing::warn!("Failed to query gain modes: {}", e);
            }
        }

        // Populate trigger mode (exposure mode) choices
        match PvcamFeatures::list_exposure_modes(&conn_guard) {
            Ok(modes) => {
                let names: Vec<String> = modes.iter().map(|(_, name)| name.clone()).collect();
                tracing::debug!("Populating trigger mode choices: {:?}", names);
                self.trigger_mode.inner().update_choices(names);
            }
            Err(e) => {
                tracing::warn!("Failed to query exposure modes: {}", e);
            }
        }
    }

    pub async fn acquire_frame(&self) -> Result<Frame> {
        let conn = self.connection.lock().await;
        self.acquisition
            .acquire_single_frame(
                &conn,
                self.roi.get(),
                self.binning.get(),
                self.exposure_ms.get(),
            )
            .await
    }

    pub fn resolution(&self) -> (u32, u32) {
        (self.sensor_width, self.sensor_height)
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

    /// Check if the driver experienced an error that requires recovery.
    ///
    /// Returns true if the last acquisition ended due to an error (USB disconnect,
    /// SDK failure, etc.) rather than a normal stop.
    ///
    /// # Error Recovery (bd-g9po)
    ///
    /// When this returns true, the caller should:
    /// 1. Call `reset_error_state()` to clear the error
    /// 2. Optionally call `reinitialize()` to fully reset the SDK connection
    /// 3. Retry the operation
    pub fn has_error(&self) -> bool {
        self.acquisition.has_error()
    }

    /// Get the last error that occurred, if any.
    ///
    /// Returns the error type from the last failed acquisition. The error is
    /// cleared when `reset_error_state()` or `reinitialize()` is called.
    pub fn last_error(&self) -> Option<crate::components::acquisition::AcquisitionError> {
        self.acquisition.last_error()
    }

    /// Reset the error state without reinitializing the SDK.
    ///
    /// Use this for transient errors where the SDK is still functional.
    /// For more serious errors (USB disconnect), use `reinitialize()` instead.
    pub async fn reset_error_state(&self) -> Result<()> {
        self.acquisition.clear_error();
        // Reset streaming state in case it was left in inconsistent state
        if self.streaming.get() {
            let conn = self.connection.lock().await;
            self.acquisition.stop_stream(&conn).await?;
        }
        Ok(())
    }

    /// Reinitialize the driver after a fatal error.
    ///
    /// This performs a full reset of the SDK connection:
    /// 1. Stops any active streaming
    /// 2. Closes and reopens the camera connection
    /// 3. Clears all error state
    /// 4. Reloads camera parameters
    ///
    /// # Error Recovery (bd-g9po)
    ///
    /// Call this after detecting a fatal error (USB disconnect, SDK crash) to
    /// restore the driver to a functional state. The camera must still be
    /// physically connected for this to succeed.
    ///
    /// # Example
    /// ```ignore
    /// if driver.has_error() {
    ///     match driver.reinitialize().await {
    ///         Ok(()) => println!("Camera recovered"),
    ///         Err(e) => println!("Camera not available: {}", e),
    ///     }
    /// }
    /// ```
    #[cfg(feature = "pvcam_sdk")]
    pub async fn reinitialize(&self) -> Result<()> {
        use crate::components::features::PvcamFeatures;

        tracing::info!("Reinitializing PVCAM driver after error");

        // 1. Stop streaming if active
        if self.streaming.get() {
            let conn = self.connection.lock().await;
            let _ = self.acquisition.stop_stream(&conn).await;
        }

        // 2. Clear error state
        self.acquisition.clear_error();

        // 3. Close and reopen the camera
        let camera_name = self.camera_name.clone();
        let connection = self.connection.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = connection.blocking_lock();

            // Close existing connection
            conn.close();

            // Reinitialize SDK (ref counting handles multiple instances)
            conn.initialize()?;

            // Reopen camera
            conn.open(&camera_name)?;

            tracing::info!("Camera reconnected successfully");
            Ok(())
        })
        .await??;

        // 4. Reload camera info (temperature, etc. may have changed)
        let conn = self.connection.lock().await;
        if let Ok(temp) = PvcamFeatures::get_temperature(&conn) {
            let _ = self.temperature.set(temp).await;
        }

        tracing::info!("PVCAM driver reinitialized successfully");
        Ok(())
    }

    /// Reinitialize the driver after a fatal error (mock mode).
    #[cfg(not(feature = "pvcam_sdk"))]
    pub async fn reinitialize(&self) -> Result<()> {
        tracing::info!("Reinitializing PVCAM driver (mock mode)");

        // Just clear error state in mock mode
        self.acquisition.clear_error();

        if self.streaming.get() {
            let conn = self.connection.lock().await;
            let _ = self.acquisition.stop_stream(&conn).await;
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
        // If not streaming, emulate a software trigger by acquiring a single frame.
        // This supports plans like Count which trigger then read.
        if !self.streaming.get() {
            let acquisition = self.acquisition.clone();
            let connection = self.connection.clone();
            let roi = self.roi.get();
            let binning = self.binning.get();
            let exposure_ms = self.exposure_ms.get();

            tokio::spawn(async move {
                let conn = connection.lock().await;
                if let Err(e) = acquisition
                    .acquire_single_frame(&conn, roi, binning, exposure_ms)
                    .await
                {
                    tracing::error!("Trigger acquisition failed: {}", e);
                }
            });
        }
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
        self.acquisition
            .start_stream(
                &conn,
                self.roi.get(),
                self.binning.get(),
                self.exposure_ms.get(),
                self.buffer_mode.get(),
            )
            .await
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

    async fn register_output(
        &self,
        tx: tokio::sync::mpsc::Sender<Self::Output>,
    ) -> Result<(), Self::Error> {
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
                let exposures = args
                    .get("exposures")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'exposures' array argument"))?;

                let exposures_u32: Vec<u32> = exposures
                    .iter()
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
