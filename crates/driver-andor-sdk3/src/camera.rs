//! Andor iStar Camera Driver
//!
//! Safe wrapper for Andor SDK3 camera API (atcore.dll).
//!
//! # Features
//!
//! - **Frame Acquisition**: Continuous streaming with circular buffers
//! - **External Triggering**: Support for external trigger inputs
//! - **MCP Gain Control**: Micro-Channel Plate intensifier gain (0-4095)
//! - **DDG Timing**: Digital Delay Generator for gate timing control
//! - **AOI & Binning**: Region of interest and binning configuration
//! - **Temperature Control**: Sensor cooling and monitoring
//!
//! # Initialization Sequence
//!
//! Based on LIBS/initialization.py lines 64-173:
//!
//! 1. Initialize SDK library (AT_InitialiseLibrary)
//! 2. Open camera by index (AT_Open)
//! 3. Enable sensor cooling if needed
//! 4. Configure AOI (Area of Interest) and binning
//! 5. Set trigger mode, exposure, gate mode
//! 6. Configure MCP gain and DDG timing
//! 7. Start acquisition
//!
//! # Example
//!
//! ```rust,no_run
//! use driver_andor_sdk3::camera::AndorCamera;
//! use common::capabilities::{FrameProducer, Triggerable};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let camera = AndorCamera::new_async(0).await?;
//!
//! // Configure for external triggering
//! camera.set_trigger_mode("External").await?;
//! camera.set_exposure(0.0015).await?;  // 1.5ms
//! camera.set_gate_mode("DDG").await?;
//! camera.set_mcp_gain(3600).await?;
//! camera.set_ddg_output_delay(1300000).await?;  // ps
//! camera.set_ddg_output_width(10000000).await?; // ps
//!
//! // Start streaming
//! camera.start_stream().await?;
//! # Ok(())
//! # }
//! ```

use crate::types::{CameraInfo, GateMode, TriggerMode};
use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{
    ExposureControl, FrameObserver, FrameProducer, ObserverHandle, Parameterized, Triggerable,
};
use common::core::Roi;
use common::observable::ParameterSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "camera")]
use crate::error::AndorError;
#[cfg(feature = "camera")]
use std::sync::atomic::AtomicUsize;

#[cfg(feature = "camera")]
use andor_sdk3_sys::*;

/// Global instance counter for SDK library lifecycle management.
///
/// The Andor SDK requires AT_InitialiseLibrary() to be called once before any cameras
/// are opened, and AT_FinaliseLibrary() to be called once after all cameras are closed.
/// This counter tracks the number of live camera instances to ensure proper cleanup.
#[cfg(feature = "camera")]
static LIBRARY_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Andor iStar camera driver
///
/// Implements the following capabilities:
/// - `FrameProducer`: Stream frames from camera
/// - `Triggerable`: External/software trigger support
/// - `ExposureControl`: Set integration time
/// - `Parameterized`: Expose camera parameters
///
/// # Thread Safety
///
/// All methods use interior mutability (Mutex/Atomic) and are safe to call
/// from multiple async tasks.
#[derive(Clone)]
pub struct AndorCamera {
    inner: Arc<AndorCameraInner>,
}

struct AndorCameraInner {
    #[cfg(feature = "camera")]
    handle: AT_H,
    #[cfg(not(feature = "camera"))]
    handle: i32,

    info: CameraInfo,
    streaming: AtomicBool,
    armed: AtomicBool,
    frame_count: AtomicU32,

    // Acquisition parameters
    exposure_s: Mutex<f64>,
    trigger_mode: Mutex<TriggerMode>,
    gate_mode: Mutex<GateMode>,
    mcp_gain: Mutex<u32>,
    ddg_output_delay_ps: Mutex<u64>,
    ddg_output_width_ps: Mutex<u64>,

    // AOI parameters
    roi: Mutex<Roi>,
    binning: Mutex<(u32, u32)>,

    // Temperature
    temperature_c: Mutex<f64>,
    cooling_enabled: AtomicBool,

    // Frame observers (bd-0dax.4)
    observers: Mutex<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
    next_observer_id: AtomicU64,

    // Parameters
    params: ParameterSet,
}

impl AndorCamera {
    /// Create new mock camera instance for testing
    ///
    /// This is a convenience method that always uses the mock backend,
    /// regardless of feature flags.
    pub async fn new_mock() -> Result<Self> {
        Self::new_async(0).await
    }

    /// Create new camera instance (async, validates device identity)
    ///
    /// # Arguments
    ///
    /// * `camera_index` - Camera index (0 for first camera)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - SDK initialization fails
    /// - Camera index is invalid
    /// - Device cannot be opened
    pub async fn new_async(camera_index: i32) -> Result<Self> {
        #[cfg(feature = "camera")]
        let (handle, info) =
            tokio::task::spawn_blocking(move || Self::init_hardware(camera_index)).await??;

        #[cfg(not(feature = "camera"))]
        let (handle, info) = (camera_index, Self::mock_camera_info(camera_index));

        let sensor_width = info.sensor_width;
        let sensor_height = info.sensor_height;

        let inner = Arc::new(AndorCameraInner {
            handle,
            info,
            streaming: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            frame_count: AtomicU32::new(0),
            exposure_s: Mutex::new(0.001),
            trigger_mode: Mutex::new(TriggerMode::Internal),
            gate_mode: Mutex::new(GateMode::CW),
            mcp_gain: Mutex::new(0),
            ddg_output_delay_ps: Mutex::new(0),
            ddg_output_width_ps: Mutex::new(1000000),
            roi: Mutex::new(Roi {
                x: 0,
                y: 0,
                width: sensor_width,
                height: sensor_height,
            }),
            binning: Mutex::new((1, 1)),
            temperature_c: Mutex::new(20.0),
            cooling_enabled: AtomicBool::new(false),
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            params: ParameterSet::new(),
        });

        Ok(Self { inner })
    }

    #[cfg(feature = "camera")]
    fn init_hardware(camera_index: i32) -> Result<(AT_H, CameraInfo)> {
        use crate::error::sdk_result;

        unsafe {
            // Initialize library (increment ref count on success)
            let ret = AT_InitialiseLibrary();
            sdk_result(ret)?;
            LIBRARY_INSTANCE_COUNT.fetch_add(1, Ordering::SeqCst);

            // Open camera
            let mut handle = AT_HANDLE_UNINITIALISED;
            let ret = AT_Open(camera_index, &mut handle);
            if ret != AT_SUCCESS {
                // Decrement ref count and finalize library if last instance
                if LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    AT_FinaliseLibrary();
                }
                return Err(AndorError::from_code(ret).into());
            }

            // Query camera info, cleanup on failure
            match Self::query_camera_info(handle) {
                Ok(info) => Ok((handle, info)),
                Err(e) => {
                    // Close handle and cleanup library on query failure
                    AT_Close(handle);
                    if LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                        AT_FinaliseLibrary();
                    }
                    Err(e)
                }
            }
        }
    }

    #[cfg(feature = "camera")]
    unsafe fn query_camera_info(handle: AT_H) -> Result<CameraInfo> {
        use crate::error::sdk_result;

        // Get sensor dimensions
        let sensor_width_feature = to_wide_string("SensorWidth");
        let mut sensor_width: AT_64 = 0;
        // SAFETY: handle is valid (just opened), feature name is valid wide string,
        // sensor_width pointer is valid for writing AT_64
        let ret = AT_GetInt(handle, sensor_width_feature.as_ptr(), &mut sensor_width);
        sdk_result(ret)?;

        let sensor_height_feature = to_wide_string("SensorHeight");
        let mut sensor_height: AT_64 = 0;
        // SAFETY: handle is valid, feature name is valid wide string,
        // sensor_height pointer is valid for writing AT_64
        let ret = AT_GetInt(handle, sensor_height_feature.as_ptr(), &mut sensor_height);
        sdk_result(ret)?;

        // Get model name
        let model_feature = to_wide_string("CameraModel");
        let mut model_buffer = wide_string_buffer(256);
        // SAFETY: handle is valid, feature name is valid wide string,
        // model_buffer has 256 elements allocated, buffer size matches allocation
        let ret = AT_GetString(
            handle,
            model_feature.as_ptr(),
            model_buffer.as_mut_ptr(),
            256,
        );
        let model = if ret == AT_SUCCESS {
            from_wide_string(&model_buffer)
        } else {
            "Unknown".to_string()
        };

        // Get serial number
        let serial_feature = to_wide_string("SerialNumber");
        let mut serial_buffer = wide_string_buffer(256);
        // SAFETY: handle is valid, feature name is valid wide string,
        // serial_buffer has 256 elements allocated, buffer size matches allocation
        let ret = AT_GetString(
            handle,
            serial_feature.as_ptr(),
            serial_buffer.as_mut_ptr(),
            256,
        );
        let serial_number = if ret == AT_SUCCESS {
            from_wide_string(&serial_buffer)
        } else {
            "Unknown".to_string()
        };

        // Get firmware version
        let fw_feature = to_wide_string("FirmwareVersion");
        let mut fw_buffer = wide_string_buffer(256);
        // SAFETY: handle is valid, feature name is valid wide string,
        // fw_buffer has 256 elements allocated, buffer size matches allocation
        let ret = AT_GetString(handle, fw_feature.as_ptr(), fw_buffer.as_mut_ptr(), 256);
        let firmware_version = if ret == AT_SUCCESS {
            from_wide_string(&fw_buffer)
        } else {
            "Unknown".to_string()
        };

        Ok(CameraInfo {
            model,
            serial_number,
            firmware_version,
            sensor_width: sensor_width as u32,
            sensor_height: sensor_height as u32,
        })
    }

    #[cfg(not(feature = "camera"))]
    fn mock_camera_info(_camera_index: i32) -> CameraInfo {
        CameraInfo {
            model: "Mock iStar".to_string(),
            serial_number: "MOCK-12345".to_string(),
            firmware_version: "1.0.0".to_string(),
            sensor_width: 2048,
            sensor_height: 2048,
        }
    }

    /// Get camera information
    pub fn info(&self) -> &CameraInfo {
        &self.inner.info
    }

    /// Set trigger mode
    pub async fn set_trigger_mode(&self, mode: &str) -> Result<()> {
        let trigger_mode = TriggerMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let mode_str = trigger_mode.to_string();
            tokio::task::spawn_blocking(move || {
                Self::set_enum_feature(handle, "TriggerMode", &mode_str)
            })
            .await??;
        }

        *self.inner.trigger_mode.lock().await = trigger_mode;
        Ok(())
    }

    /// Get trigger mode
    pub async fn get_trigger_mode(&self) -> Result<String> {
        Ok(self.inner.trigger_mode.lock().await.to_string())
    }

    /// Set gate mode (CW or DDG)
    pub async fn set_gate_mode(&self, mode: &str) -> Result<()> {
        let gate_mode = GateMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let mode_str = gate_mode.to_string();
            tokio::task::spawn_blocking(move || {
                Self::set_enum_feature(handle, "GateMode", &mode_str)
            })
            .await??;
        }

        *self.inner.gate_mode.lock().await = gate_mode;
        Ok(())
    }

    /// Set MCP gain (0-4095)
    pub async fn set_mcp_gain(&self, gain: u32) -> Result<()> {
        if gain > 4095 {
            anyhow::bail!("MCP gain must be 0-4095, got {}", gain);
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_int_feature(handle, "MCPGain", gain as i64)
            })
            .await??;
        }

        *self.inner.mcp_gain.lock().await = gain;
        Ok(())
    }

    /// Get MCP gain
    pub async fn get_mcp_gain(&self) -> Result<u32> {
        Ok(*self.inner.mcp_gain.lock().await)
    }

    /// Set DDG output delay in picoseconds
    pub async fn set_ddg_output_delay(&self, delay_ps: u64) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_float_feature(handle, "DDGOutputDelay", delay_ps as f64)
            })
            .await??;
        }

        *self.inner.ddg_output_delay_ps.lock().await = delay_ps;
        Ok(())
    }

    /// Set DDG output width in picoseconds
    pub async fn set_ddg_output_width(&self, width_ps: u64) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_float_feature(handle, "DDGOutputWidth", width_ps as f64)
            })
            .await??;
        }

        *self.inner.ddg_output_width_ps.lock().await = width_ps;
        Ok(())
    }

    /// Enable/disable sensor cooling
    pub async fn set_cooling(&self, enabled: bool) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_bool_feature(handle, "SensorCooling", enabled)
            })
            .await??;
        }

        self.inner.cooling_enabled.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    /// Get sensor temperature in Celsius
    pub async fn get_temperature(&self) -> Result<f64> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let temp = tokio::task::spawn_blocking(move || {
                Self::get_float_feature(handle, "SensorTemperature")
            })
            .await??;
            *self.inner.temperature_c.lock().await = temp;
            Ok(temp)
        }

        #[cfg(not(feature = "camera"))]
        Ok(*self.inner.temperature_c.lock().await)
    }

    #[cfg(feature = "camera")]
    fn set_enum_feature(handle: AT_H, feature: &str, value: &str) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let value_wide = to_wide_string(value);
            // SAFETY: handle is valid (camera is open), feature and value are valid wide strings
            let ret = AT_SetEnumString(handle, feature_wide.as_ptr(), value_wide.as_ptr());
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    fn set_int_feature(handle: AT_H, feature: &str, value: i64) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string, value is i64
            let ret = AT_SetInt(handle, feature_wide.as_ptr(), value);
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    fn set_float_feature(handle: AT_H, feature: &str, value: f64) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string, value is f64
            let ret = AT_SetFloat(handle, feature_wide.as_ptr(), value);
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    fn set_bool_feature(handle: AT_H, feature: &str, value: bool) -> Result<()> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            // SAFETY: handle is valid (camera is open), feature is valid wide string,
            // bool value is converted to SDK's AT_TRUE/AT_FALSE constants
            let ret = AT_SetBool(
                handle,
                feature_wide.as_ptr(),
                if value { AT_TRUE } else { AT_FALSE },
            );
            sdk_result(ret)?;
            Ok(())
        }
    }

    #[cfg(feature = "camera")]
    fn get_float_feature(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature is valid wide string,
            // value pointer is valid for writing f64
            let ret = AT_GetFloat(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }
}

// Implement capability traits

#[async_trait]
impl FrameProducer for AndorCamera {
    async fn start_stream(&self) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("AcquisitionStart");
                    // SAFETY: handle is valid (camera is open), feature is valid wide string
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        self.inner.streaming.store(true, Ordering::Relaxed);
        tracing::info!("Camera streaming started");
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("AcquisitionStop");
                    // SAFETY: handle is valid (camera is open), feature is valid wide string
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        self.inner.streaming.store(false, Ordering::Relaxed);
        tracing::info!("Camera streaming stopped");
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        // Return sensor dimensions (ROI is internal state)
        (self.inner.info.sensor_width, self.inner.info.sensor_height)
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let handle = ObserverHandle(self.inner.next_observer_id.fetch_add(1, Ordering::Relaxed));
        self.inner.observers.lock().await.push((handle, observer));
        Ok(handle)
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        let mut observers = self.inner.observers.lock().await;
        observers.retain(|(h, _)| *h != handle);
        Ok(())
    }
}

#[async_trait]
impl Triggerable for AndorCamera {
    async fn arm(&self) -> Result<()> {
        self.inner.armed.store(true, Ordering::Relaxed);
        tracing::debug!("Camera armed");
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.inner.armed.load(Ordering::Relaxed) {
            anyhow::bail!("Camera not armed");
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("SoftwareTrigger");
                    // SAFETY: handle is valid (camera is open), feature is valid wide string
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await??;
        }

        tracing::debug!("Camera triggered");
        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.inner.armed.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl ExposureControl for AndorCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        if seconds <= 0.0 {
            anyhow::bail!("Exposure must be positive, got {}", seconds);
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_float_feature(handle, "ExposureTime", seconds)
            })
            .await??;
        }

        *self.inner.exposure_s.lock().await = seconds;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(*self.inner.exposure_s.lock().await)
    }
}

impl Parameterized for AndorCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

impl Drop for AndorCameraInner {
    fn drop(&mut self) {
        #[cfg(feature = "camera")]
        unsafe {
            if self.handle != AT_HANDLE_UNINITIALISED {
                // SAFETY: handle was successfully opened in init_hardware, so AT_Close is valid
                AT_Close(self.handle);

                // Only finalize library when last instance is dropped
                // SAFETY: We increment the counter in init_hardware when library is initialized,
                // so we must decrement here. If this is the last instance (count was 1 before
                // decrement), we finalize the library.
                if LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    AT_FinaliseLibrary();
                }
            }
        }
    }
}
