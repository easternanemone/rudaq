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
//! use common::capabilities::{ExposureControl, FrameProducer, Triggerable};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let camera = AndorCamera::new_async(0).await?;
//!
//! // Configure for external triggering (set modes before exposure)
//! camera.set_trigger_mode("External").await?;
//! camera.set_gate_mode("DDG").await?;
//!
//! // Query dynamic exposure range (changes with trigger/gate mode)
//! let (exp_min, _exp_max) = camera.get_exposure_range().await?;
//! camera.set_exposure(exp_min * 1.1).await?;
//!
//! camera.set_mcp_gain(3600).await?;
//! camera.set_ddg_output_delay(1300000).await?;  // ps (MCP gate delay)
//! camera.set_ddg_output_width(10000000).await?; // ps (MCP gate width)
//!
//! // Start streaming
//! camera.start_stream().await?;
//! # Ok(())
//! # }
//! ```

use crate::types::{CameraInfo, ElectronicShutteringMode, GateMode, TriggerMode};
use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{
    Commandable, ExposureControl, FrameObserver, FrameProducer, LoanedFrame, ObserverHandle,
    Parameterized, Triggerable,
};
use common::core::Roi;
use common::data::FrameView;
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// SDK3 feature names already represented by core typed parameters.
///
/// Dynamic parameter creation skips these to avoid duplicate registrations.
/// Core parameters use Rust enum types (`TriggerMode`, `GateMode`, etc.) for
/// type-safe trait implementations; dynamic parameters use generic types
/// (`f64`, `i64`, `bool`, `String`) for features that don't need specialized Rust types.
const CORE_FEATURE_NAMES: &[&str] = &[
    "ExposureTime",
    "TriggerMode",
    "GateMode",
    "MCPGain",
    "DDGOutputDelay",
    "DDGOutputWidth",
    "AOIWidth",
    "AOIHeight",
    "AOILeft",
    "AOITop",
    "AOIHBin",
    "AOIVBin",
    "SensorTemperature",
    "TargetSensorTemperature",
    "ElectronicShutteringMode",
];

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

/// Bridge between the C FFI callback thread and the async parameter update task.
///
/// Passed as the `context` pointer to `AT_RegisterFeatureCallback`. The SDK
/// callback posts feature names through the sender; a spawned tokio task
/// receives them and updates Parameters.
///
/// # Lifecycle
/// Owned by `AndorCameraInner` via `callback_bridge` field. A raw pointer to
/// the heap allocation is given to the SDK via `&*box as *const _ as *mut c_void`.
/// When the camera is dropped, `Drop` for `AndorCameraInner`:
///   1. Drops `callback_tx` → closes channel → receiver task exits gracefully
///   2. Aborts `callback_task_handle` as a backstop
///   3. `AT_Close(handle)` unregisters all SDK callbacks, invalidating the raw ptr
///   4. The `Box<FeatureCallbackBridge>` is dropped, freeing the allocation
#[cfg(feature = "camera")]
struct FeatureCallbackBridge {
    tx: tokio::sync::mpsc::UnboundedSender<String>,
}

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
    exposure_s: Parameter<f64>,
    trigger_mode: Parameter<TriggerMode>,
    gate_mode: Parameter<GateMode>,
    mcp_gain: Parameter<u32>,
    ddg_output_delay_ps: Parameter<u64>,
    ddg_output_width_ps: Parameter<u64>,

    // AOI parameters
    roi: Parameter<Roi>,
    binning: Parameter<(u32, u32)>,

    // Temperature and cooling (bd-zekj)
    temperature_c: Parameter<f64>,
    cooling_enabled: AtomicBool,
    target_temperature_c: Parameter<f64>,

    // Electronic shuttering (bd-apwl)
    electronic_shuttering: Parameter<ElectronicShutteringMode>,

    // Frame loss tracking (bd-fami)
    frames_dropped: AtomicU64,
    last_hw_frame_nr: std::sync::atomic::AtomicI32,

    // Error tracking (bd-z95k)
    last_error: std::sync::Mutex<Option<String>>,

    // Frame observers (bd-0dax.4)
    observers: Mutex<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
    next_observer_id: AtomicU64,

    // Drift polling task handle (bd-j4aa)
    drift_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // Acquisition output channels (bd-b2kf.3, bd-b2kf.8)
    /// Primary frame output — single consumer receives LoanedFrame ownership.
    primary_tx: Mutex<Option<tokio::sync::mpsc::Sender<LoanedFrame>>>,
    /// Pre-allocated frame pool for zero-allocation frame delivery.
    frame_pool: Arc<pool::Pool<pool::FrameData>>,
    /// Tap registry for notifying secondary observers.
    tap_registry: TapRegistry,
    /// Handle to the running acquisition loop task (if any).
    acq_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // Feature callback lifecycle (bd-joqu / Copilot review)
    /// Keeps the bridge allocation alive; raw ptr given to SDK via `&*box`.
    /// Dropped AFTER `AT_Close(handle)` invalidates SDK callbacks.
    #[cfg(feature = "camera")]
    _callback_bridge: Mutex<Option<Box<FeatureCallbackBridge>>>,
    /// Dropped before the bridge to close the channel and signal the receiver.
    #[cfg(feature = "camera")]
    callback_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    /// Receiver task handle — aborted in Drop as a backstop.
    callback_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // Parameters
    params: ParameterSet,
}

// =========================================================================
// Lightweight TapRegistry (reimplemented from PVCAM pattern, ~40 lines)
// =========================================================================

/// Minimal frame tap registry for observer notification.
///
/// Uses parking_lot::RwLock for low-overhead read access in the hot path.
/// Observers are called synchronously on each frame in the acquisition loop.
struct TapRegistry {
    taps: parking_lot::RwLock<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
}

impl TapRegistry {
    fn new() -> Self {
        Self {
            taps: parking_lot::RwLock::new(Vec::new()),
        }
    }

    fn register(&self, handle: ObserverHandle, observer: Box<dyn FrameObserver>) {
        self.taps.write().push((handle, observer));
    }

    fn unregister(&self, handle: ObserverHandle) {
        self.taps.write().retain(|(h, _)| *h != handle);
    }

    /// Notify all registered observers with a frame view.
    fn notify(&self, view: &FrameView<'_>) {
        let taps = self.taps.read();
        for (_, observer) in taps.iter() {
            observer.on_frame(view);
        }
    }
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

        // Create parameters with descriptive metadata
        #[allow(unused_mut)]
        let mut exposure_s = Parameter::new("exposure_s", 0.001)
            .with_unit("s")
            .with_description("Integration time");

        #[allow(unused_mut)]
        let mut trigger_mode =
            Parameter::new("trigger_mode", TriggerMode::Internal).with_description("Trigger mode");

        #[allow(unused_mut)]
        let mut gate_mode =
            Parameter::new("gate_mode", GateMode::CW).with_description("MCP gate mode");

        #[allow(unused_mut)]
        let mut mcp_gain = Parameter::new("mcp_gain", 0u32)
            .with_description("MCP gain (0-4095)")
            .with_range(0, 4095);

        #[allow(unused_mut)]
        let mut ddg_output_delay_ps = Parameter::new("ddg_output_delay_ps", 0u64)
            .with_unit("ps")
            .with_description("DDG output delay");

        #[allow(unused_mut)]
        let mut ddg_output_width_ps = Parameter::new("ddg_output_width_ps", 1_000_000u64)
            .with_unit("ps")
            .with_description("DDG output width");

        #[allow(unused_mut)]
        let mut roi = Parameter::new(
            "roi",
            Roi {
                x: 0,
                y: 0,
                width: sensor_width,
                height: sensor_height,
            },
        )
        .with_description("Region of interest");

        #[allow(unused_mut)]
        let mut binning =
            Parameter::new("binning", (1u32, 1u32)).with_description("Pixel binning (x, y)");

        #[allow(unused_mut)]
        let mut temperature_c = Parameter::new("temperature_c", 20.0)
            .with_unit("°C")
            .with_description("Sensor temperature");

        let target_temperature_c = Parameter::new("target_temperature_c", -20.0)
            .with_unit("°C")
            .with_description("Target cooling temperature");

        #[allow(unused_mut)]
        let mut electronic_shuttering =
            Parameter::new("electronic_shuttering", ElectronicShutteringMode::Rolling)
                .with_description("Electronic shuttering mode");

        // Connect hardware callbacks when SDK is available
        #[cfg(feature = "camera")]
        {
            Self::attach_exposure_callback(&mut exposure_s, handle);
            Self::attach_trigger_mode_callback(&mut trigger_mode, handle);
            Self::attach_gate_mode_callback(&mut gate_mode, handle);
            Self::attach_mcp_gain_callback(&mut mcp_gain, handle);
            Self::attach_ddg_delay_callback(&mut ddg_output_delay_ps, handle);
            Self::attach_ddg_width_callback(&mut ddg_output_width_ps, handle);
            Self::attach_temperature_reader(&mut temperature_c, handle);
            Self::attach_roi_callback(&mut roi, handle);
            Self::attach_binning_callback(&mut binning, handle);
            Self::attach_electronic_shuttering_callback(&mut electronic_shuttering, handle);
        }

        // Register all parameters for GUI/API exposure
        let mut params = ParameterSet::new();
        params.register(exposure_s.clone());
        params.register(trigger_mode.clone());
        params.register(gate_mode.clone());
        params.register(mcp_gain.clone());
        params.register(ddg_output_delay_ps.clone());
        params.register(ddg_output_width_ps.clone());
        params.register(roi.clone());
        params.register(binning.clone());
        params.register(temperature_c.clone());
        params.register(target_temperature_c.clone());
        params.register(electronic_shuttering.clone());

        // Register dynamic parameters from SDK3 feature introspection.
        // These cover SDK3 features NOT already handled by the 11 core
        // typed parameters above (see CORE_FEATURE_NAMES).
        {
            #[cfg(not(feature = "camera"))]
            let discovered = crate::introspection::introspect_mock_features();
            #[cfg(feature = "camera")]
            let discovered = crate::introspection::introspect_all_features(handle);
            Self::register_dynamic_features(&discovered, &mut params, handle);
        }

        // Create frame pool sized for full sensor (worst case).
        // Each slot holds sensor_width * sensor_height * 2 bytes (16-bit pixels).
        let frame_capacity = (sensor_width as usize) * (sensor_height as usize) * 2;
        let pool_size = crate::buffer::DEFAULT_BUFFER_COUNT;
        let frame_pool = pool::Pool::new_with_reset(
            pool_size,
            move || pool::FrameData::with_capacity(frame_capacity),
            |frame| frame.reset(),
        );

        let inner = Arc::new(AndorCameraInner {
            handle,
            info,
            streaming: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            frame_count: AtomicU32::new(0),
            exposure_s,
            trigger_mode,
            gate_mode,
            mcp_gain,
            ddg_output_delay_ps,
            ddg_output_width_ps,
            roi,
            binning,
            temperature_c,
            cooling_enabled: AtomicBool::new(false),
            target_temperature_c,
            electronic_shuttering,
            frames_dropped: AtomicU64::new(0),
            last_hw_frame_nr: std::sync::atomic::AtomicI32::new(-1),
            last_error: std::sync::Mutex::new(None),
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            drift_task_handle: Mutex::new(None),
            primary_tx: Mutex::new(None),
            frame_pool,
            tap_registry: TapRegistry::new(),
            acq_task_handle: Mutex::new(None),
            #[cfg(feature = "camera")]
            _callback_bridge: Mutex::new(None),
            #[cfg(feature = "camera")]
            callback_tx: Mutex::new(None),
            callback_task_handle: Mutex::new(None),
            params,
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

        // Query feature support (non-fatal — defaults to false)
        let features = Self::query_feature_support(handle);

        Ok(CameraInfo {
            model,
            serial_number,
            firmware_version,
            sensor_width: sensor_width as u32,
            sensor_height: sensor_height as u32,
            features,
        })
    }

    /// Query which optional SDK3 features are implemented on this camera.
    ///
    /// Uses AT_IsImplemented for each feature. Non-fatal — returns false
    /// for any feature that fails to query.
    #[cfg(feature = "camera")]
    fn query_feature_support(handle: AT_H) -> crate::types::FeatureSupport {
        let check =
            |name: &str| -> bool { Self::is_feature_implemented(handle, name).unwrap_or(false) };
        crate::types::FeatureSupport {
            mcp_gain: check("MCPGain"),
            gate_mode: check("GateMode"),
            ddg_output_delay: check("DDGOutputDelay"),
            ddg_output_width: check("DDGOutputWidth"),
            sensor_cooling: check("SensorCooling"),
            sensor_temperature: check("SensorTemperature"),
            pixel_encoding: check("PixelEncoding"),
            external_trigger_modes: check("TriggerMode"),
            electronic_shuttering_mode: check("ElectronicShutteringMode"),
            frame_count: check("FrameCount"),
        }
    }

    #[cfg(not(feature = "camera"))]
    fn mock_camera_info(_camera_index: i32) -> CameraInfo {
        use crate::types::FeatureSupport;
        CameraInfo {
            model: "Mock iStar".to_string(),
            serial_number: "MOCK-12345".to_string(),
            firmware_version: "1.0.0".to_string(),
            sensor_width: 2048,
            sensor_height: 2048,
            features: FeatureSupport {
                mcp_gain: true,
                gate_mode: true,
                ddg_output_delay: true,
                ddg_output_width: true,
                sensor_cooling: true,
                sensor_temperature: true,
                pixel_encoding: true,
                external_trigger_modes: true,
                electronic_shuttering_mode: true,
                frame_count: true,
            },
        }
    }

    /// Register dynamic SDK3 feature parameters from introspection results.
    ///
    /// For each discovered feature NOT already covered by core typed parameters
    /// (see [`CORE_FEATURE_NAMES`]), creates a `Parameter<T>` with the appropriate
    /// type, introspectable metadata (ranges, enum values), and — in hardware
    /// mode — an SDK write callback via `spawn_blocking`.
    ///
    /// Feature type mapping:
    /// - `Float` → `Parameter<f64>` with `with_range_introspectable`
    /// - `Int`   → `Parameter<i64>` with `with_range_introspectable`
    /// - `Bool`  → `Parameter<bool>` with `dtype = "bool"`
    /// - `Enum`  → `Parameter<String>` with `with_choices_introspectable`
    /// - `Str`   → `Parameter<String>` with `dtype = "string"` (typically read-only)
    /// - `Command` → skipped (handled by `Commandable`/`Triggerable` traits)
    fn register_dynamic_features(
        features: &[crate::introspection::DiscoveredFeature],
        params: &mut ParameterSet,
        handle: i32,
    ) {
        use crate::introspection::FeatureType;

        let mut count = 0u32;

        for feat in features {
            if !feat.is_displayable() || CORE_FEATURE_NAMES.contains(&feat.name.as_str()) {
                continue;
            }
            if feat.feature_type == FeatureType::Command {
                continue;
            }

            match feat.feature_type {
                FeatureType::Float => {
                    let mut param = Parameter::new(feat.name.clone(), 0.0f64)
                        .with_description(format!("SDK3: {}", feat.name));
                    if let Some((min, max)) = feat.float_range {
                        param = param.with_range_introspectable(min, max);
                    } else {
                        param = param.with_dtype("float");
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: f64| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_float_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Int => {
                    let mut param = Parameter::new(feat.name.clone(), 0i64)
                        .with_description(format!("SDK3: {}", feat.name));
                    if let Some((min, max)) = feat.int_range {
                        param = param.with_range_introspectable(min, max);
                    } else {
                        param = param.with_dtype("int");
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: i64| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_int_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Bool => {
                    let mut param = Parameter::new(feat.name.clone(), false)
                        .with_description(format!("SDK3: {}", feat.name))
                        .with_dtype("bool");
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: bool| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_bool_feature(handle, &fname, val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Enum => {
                    let default_val = feat.enum_values.first().cloned().unwrap_or_default();
                    let mut param = Parameter::new(feat.name.clone(), default_val)
                        .with_description(format!("SDK3: {}", feat.name));
                    if !feat.enum_values.is_empty() {
                        param = param.with_choices_introspectable(feat.enum_values.clone());
                    }
                    if !feat.writable {
                        param = param.read_only();
                    }
                    #[cfg(feature = "camera")]
                    {
                        if feat.writable {
                            let fname = feat.name.clone();
                            param.connect_to_hardware_write(move |val: String| {
                                let fname = fname.clone();
                                Box::pin(async move {
                                    tokio::task::spawn_blocking(move || {
                                        AndorCamera::set_enum_feature(handle, &fname, &val)
                                    })
                                    .await
                                    .map_err(|e| {
                                        DaqError::Instrument(format!("spawn_blocking: {e}"))
                                    })?
                                    .map_err(|e| DaqError::Instrument(e.to_string()))
                                })
                            });
                        }
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    params.register(param);
                }

                FeatureType::Str => {
                    let mut param = Parameter::new(feat.name.clone(), String::new())
                        .with_description(format!("SDK3: {}", feat.name))
                        .with_dtype("string");
                    if !feat.writable {
                        param = param.read_only();
                    }
                    if let Some(ref group) = feat.group {
                        param.with_metadata(|m| m.group_name = Some(group.clone()));
                    }
                    // SDK3 string features are typically read-only identity fields
                    // (CameraModel, SerialNumber, etc.) — no hardware write callbacks.
                    params.register(param);
                }

                FeatureType::Command => unreachable!("Commands filtered above"),
            }

            count += 1;
        }

        tracing::info!(count, "Registered dynamic SDK3 feature parameters");
    }

    /// Get camera information
    pub fn info(&self) -> &CameraInfo {
        &self.inner.info
    }

    /// Set trigger mode (cannot change during acquisition)
    pub async fn set_trigger_mode(&self, mode: &str) -> Result<()> {
        self.check_not_streaming()?;
        let trigger_mode = TriggerMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner.trigger_mode.set(trigger_mode).await?;
        Ok(())
    }

    /// Get trigger mode
    pub async fn get_trigger_mode(&self) -> Result<String> {
        Ok(self.inner.trigger_mode.get().to_string())
    }

    /// Set gate mode (CW or DDG)
    pub async fn set_gate_mode(&self, mode: &str) -> Result<()> {
        let gate_mode = GateMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner.gate_mode.set(gate_mode).await?;
        Ok(())
    }

    /// Set MCP gain (0-4095)
    pub async fn set_mcp_gain(&self, gain: u32) -> Result<()> {
        self.inner.mcp_gain.set(gain).await?;
        Ok(())
    }

    /// Get MCP gain
    pub async fn get_mcp_gain(&self) -> Result<u32> {
        Ok(self.inner.mcp_gain.get())
    }

    /// Set DDG output delay in picoseconds
    pub async fn set_ddg_output_delay(&self, delay_ps: u64) -> Result<()> {
        self.inner.ddg_output_delay_ps.set(delay_ps).await?;
        Ok(())
    }

    /// Set DDG output width in picoseconds
    pub async fn set_ddg_output_width(&self, width_ps: u64) -> Result<()> {
        self.inner.ddg_output_width_ps.set(width_ps).await?;
        Ok(())
    }

    /// Query the current valid range for ExposureTime (in seconds).
    ///
    /// The valid range changes dynamically based on trigger mode, gate mode,
    /// and other camera settings. Always query after changing modes.
    pub async fn get_exposure_range(&self) -> Result<(f64, f64)> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let (min, max) = tokio::task::spawn_blocking(move || {
                let min = Self::get_float_min(handle, "ExposureTime")?;
                let max = Self::get_float_max(handle, "ExposureTime")?;
                Ok::<(f64, f64), anyhow::Error>((min, max))
            })
            .await??;
            return Ok((min, max));
        }

        #[cfg(not(feature = "camera"))]
        Ok((0.0000001, 30.0))
    }

    /// Check if a named SDK feature is implemented on this camera model.
    ///
    /// This is a static capability check (`AT_IsImplemented`). Use it to
    /// determine whether the camera model supports a feature at all, not
    /// whether the feature is currently writable in the present state.
    pub async fn is_feature_implemented_on_camera(&self, feature: &str) -> Result<bool> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let feature = feature.to_string();
            return tokio::task::spawn_blocking(move || {
                Self::is_feature_implemented(handle, &feature)
            })
            .await?;
        }

        #[cfg(not(feature = "camera"))]
        {
            let _ = feature;
            Ok(true)
        }
    }

    /// Set electronic shuttering mode (cannot change during acquisition)
    pub async fn set_electronic_shuttering(&self, mode: &str) -> Result<()> {
        self.check_not_streaming()?;
        let shuttering_mode =
            ElectronicShutteringMode::try_from(mode).map_err(|e| anyhow::anyhow!(e))?;
        self.inner
            .electronic_shuttering
            .set(shuttering_mode)
            .await?;
        Ok(())
    }

    /// Set ROI (cannot change during acquisition)
    pub async fn set_roi(&self, roi: Roi) -> Result<()> {
        self.check_not_streaming()?;
        self.inner.roi.set(roi).await?;
        Ok(())
    }

    /// Set binning (cannot change during acquisition)
    pub async fn set_binning(&self, x: u32, y: u32) -> Result<()> {
        self.check_not_streaming()?;
        self.inner.binning.set((x, y)).await?;
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
        self.inner.temperature_c.read_from_hardware().await?;

        Ok(self.inner.temperature_c.get())
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

    #[cfg(feature = "camera")]
    pub(crate) fn get_float_min(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // value is a valid aligned f64 pointer
            let ret = AT_GetFloatMin(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn get_float_max(handle: AT_H, feature: &str) -> Result<f64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: f64 = 0.0;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // value is a valid aligned f64 pointer
            let ret = AT_GetFloatMax(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    fn get_int_feature(handle: AT_H, feature: &str) -> Result<i64> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: AT_64 = 0;
            let ret = AT_GetInt(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value)
        }
    }

    #[cfg(feature = "camera")]
    fn get_bool_feature(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut value: AT_BOOL = 0;
            let ret = AT_GetBool(handle, feature_wide.as_ptr(), &mut value);
            sdk_result(ret)?;
            Ok(value != AT_FALSE)
        }
    }

    #[cfg(feature = "camera")]
    fn get_enum_string(handle: AT_H, feature: &str) -> Result<String> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut index: std::os::raw::c_int = 0;
            let ret = AT_GetEnumIndex(handle, feature_wide.as_ptr(), &mut index);
            sdk_result(ret)?;

            let mut buffer = wide_string_buffer(256);
            let ret = AT_GetEnumStringByIndex(
                handle,
                feature_wide.as_ptr(),
                index,
                buffer.as_mut_ptr(),
                256,
            );
            sdk_result(ret)?;
            Ok(from_wide_string(&buffer))
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn is_feature_implemented(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut implemented: AT_BOOL = AT_FALSE;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // implemented is a valid aligned AT_BOOL pointer
            let ret = AT_IsImplemented(handle, feature_wide.as_ptr(), &mut implemented);
            sdk_result(ret)?;
            Ok(implemented != AT_FALSE)
        }
    }

    #[cfg(feature = "camera")]
    pub(crate) fn is_feature_writable(handle: AT_H, feature: &str) -> Result<bool> {
        use crate::error::sdk_result;

        unsafe {
            let feature_wide = to_wide_string(feature);
            let mut writable: AT_BOOL = AT_FALSE;
            // SAFETY: handle is valid (camera is open), feature_wide is NUL-terminated,
            // writable is a valid aligned AT_BOOL pointer
            let ret = AT_IsWritable(handle, feature_wide.as_ptr(), &mut writable);
            sdk_result(ret)?;
            Ok(writable != AT_FALSE)
        }
    }

    // =========================================================================
    // Parameter<T> hardware callback attachment methods
    // =========================================================================
    // spawn_blocking moves the FFI call off the async runtime to avoid blocking.
    // It does not serialize concurrent calls.

    #[cfg(feature = "camera")]
    fn attach_exposure_callback(param: &mut Parameter<f64>, handle: AT_H) {
        param.connect_to_hardware_write(move |val: f64| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    AndorCamera::set_float_feature(handle, "ExposureTime", val)
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_trigger_mode_callback(param: &mut Parameter<TriggerMode>, handle: AT_H) {
        param.connect_to_hardware_write(move |mode: TriggerMode| {
            Box::pin(async move {
                let mode_str = mode.to_string();
                tokio::task::spawn_blocking(move || {
                    AndorCamera::set_enum_feature(handle, "TriggerMode", &mode_str)
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_gate_mode_callback(param: &mut Parameter<GateMode>, handle: AT_H) {
        param.connect_to_hardware_write(move |mode: GateMode| {
            Box::pin(async move {
                let mode_str = mode.to_string();
                tokio::task::spawn_blocking(move || -> Result<(), anyhow::Error> {
                    AndorCamera::set_enum_feature(handle, "GateMode", &mode_str)?;
                    // When DDG mode is active, select the Gater (MCP intensifier)
                    // as the DDG output target so that DDGOutputDelay/Width
                    // control the MCP gate timing.
                    if mode == GateMode::DDG {
                        AndorCamera::set_enum_feature(handle, "DDGOutputSelector", "Gater")?;
                    }
                    Ok(())
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e: anyhow::Error| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_mcp_gain_callback(param: &mut Parameter<u32>, handle: AT_H) {
        param.connect_to_hardware_write(move |gain: u32| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    AndorCamera::set_int_feature(handle, "MCPGain", gain as i64)
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_ddg_delay_callback(param: &mut Parameter<u64>, handle: AT_H) {
        param.connect_to_hardware_write(move |delay_ps: u64| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    // SDK3 DDGOutputDelay is in seconds; parameter stores picoseconds
                    AndorCamera::set_float_feature(
                        handle,
                        "DDGOutputDelay",
                        delay_ps as f64 * 1e-12,
                    )
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_ddg_width_callback(param: &mut Parameter<u64>, handle: AT_H) {
        param.connect_to_hardware_write(move |width_ps: u64| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    // SDK3 DDGOutputWidth is in seconds; parameter stores picoseconds
                    AndorCamera::set_float_feature(
                        handle,
                        "DDGOutputWidth",
                        width_ps as f64 * 1e-12,
                    )
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_temperature_reader(param: &mut Parameter<f64>, handle: AT_H) {
        param.connect_to_hardware_read(move || {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    AndorCamera::get_float_feature(handle, "SensorTemperature")
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_roi_callback(param: &mut Parameter<Roi>, handle: AT_H) {
        param.connect_to_hardware_write(move |roi: Roi| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    // SDK3 AOI features use 1-based coordinates
                    AndorCamera::set_int_feature(handle, "AOILeft", roi.x as i64 + 1)?;
                    AndorCamera::set_int_feature(handle, "AOITop", roi.y as i64 + 1)?;
                    AndorCamera::set_int_feature(handle, "AOIWidth", roi.width as i64)?;
                    AndorCamera::set_int_feature(handle, "AOIHeight", roi.height as i64)?;
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_binning_callback(param: &mut Parameter<(u32, u32)>, handle: AT_H) {
        param.connect_to_hardware_write(move |bin: (u32, u32)| {
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    AndorCamera::set_int_feature(handle, "AOIHBin", bin.0 as i64)?;
                    AndorCamera::set_int_feature(handle, "AOIVBin", bin.1 as i64)?;
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    #[cfg(feature = "camera")]
    fn attach_electronic_shuttering_callback(
        param: &mut Parameter<ElectronicShutteringMode>,
        handle: AT_H,
    ) {
        param.connect_to_hardware_write(move |mode: ElectronicShutteringMode| {
            Box::pin(async move {
                let mode_str = mode.to_string();
                tokio::task::spawn_blocking(move || {
                    AndorCamera::set_enum_feature(handle, "ElectronicShutteringMode", &mode_str)
                })
                .await
                .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
                .map_err(|e| DaqError::Instrument(e.to_string()))
            })
        });
    }

    // =========================================================================
    // Write guards — reject parameter writes during acquisition (bd-iwoq)
    // =========================================================================

    /// Check if streaming and bail if so. Used by parameter setters that
    /// cannot safely change while the SDK acquisition loop is running.
    fn check_not_streaming(&self) -> Result<()> {
        if self.inner.streaming.load(Ordering::Relaxed) {
            anyhow::bail!(
                "Cannot change parameter while acquisition is running. Stop stream first."
            );
        }
        Ok(())
    }

    // =========================================================================
    // Temperature and cooling management (bd-zekj)
    // =========================================================================

    /// Set target cooling temperature in Celsius.
    pub async fn set_target_temperature(&self, target_c: f64) -> Result<()> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || {
                Self::set_float_feature(handle, "TargetSensorTemperature", target_c)
            })
            .await??;
        }

        self.inner.target_temperature_c.set(target_c).await?;
        Ok(())
    }

    /// Get the cooling status from the SDK.
    pub async fn get_cooling_status(&self) -> Result<crate::types::CoolingStatus> {
        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            let status_str = tokio::task::spawn_blocking(move || {
                Self::get_enum_string(handle, "TemperatureStatus")
            })
            .await??;

            return match status_str.as_str() {
                "Cooler Off" => Ok(crate::types::CoolingStatus::Off),
                "Stabilised" => Ok(crate::types::CoolingStatus::Stabilised),
                "Cooling" | "Not Stabilised" => Ok(crate::types::CoolingStatus::Stabilising),
                "Fault" => Ok(crate::types::CoolingStatus::Fault),
                other => {
                    tracing::warn!(status = other, "Unknown cooling status, treating as Off");
                    Ok(crate::types::CoolingStatus::Off)
                }
            };
        }

        #[cfg(not(feature = "camera"))]
        {
            if self.inner.cooling_enabled.load(Ordering::Relaxed) {
                Ok(crate::types::CoolingStatus::Stabilised)
            } else {
                Ok(crate::types::CoolingStatus::Off)
            }
        }
    }

    /// Get total frames dropped during current/last acquisition.
    pub fn frames_dropped(&self) -> u64 {
        self.inner.frames_dropped.load(Ordering::Relaxed)
    }

    // =========================================================================
    // Drift polling — periodic temperature/status reads (bd-j4aa)
    // =========================================================================

    /// Start a background task that periodically reads sensor temperature.
    ///
    /// Runs every `interval` and updates the `temperature_c` parameter.
    /// Stops when the camera is dropped or `stop_drift_polling` is called.
    pub async fn start_drift_polling(&self, interval: std::time::Duration) {
        // Stop existing task if any
        self.stop_drift_polling().await;

        let inner = self.inner.clone();
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;

                #[cfg(feature = "camera")]
                {
                    if let Ok(()) = inner.temperature_c.read_from_hardware().await {
                        // Parameter updated in-place
                    }
                }

                #[cfg(not(feature = "camera"))]
                {
                    // Mock: simulate slow drift towards target
                    let current = inner.temperature_c.get();
                    let target = inner.target_temperature_c.get();
                    let step = (target - current) * 0.1;
                    if step.abs() > 0.01 {
                        let _ = inner.temperature_c.set(current + step).await;
                    }
                }
            }
        });

        *self.inner.drift_task_handle.lock().await = Some(handle);
    }

    /// Stop the background drift polling task.
    pub async fn stop_drift_polling(&self) {
        if let Some(handle) = self.inner.drift_task_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
    }

    // =========================================================================
    // Error recovery (bd-z95k)
    // =========================================================================

    /// Returns true if the last acquisition ended due to an error.
    pub fn has_error(&self) -> bool {
        self.inner
            .last_error
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Returns the error message from the last failed acquisition.
    pub fn last_error(&self) -> Option<String> {
        self.inner
            .last_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Clear the error state. Call before retrying acquisition.
    pub fn clear_error(&self) {
        if let Ok(mut guard) = self.inner.last_error.lock() {
            *guard = None;
        }
    }

    /// Record an acquisition error (used internally by the acq loop).
    fn set_error(inner: &AndorCameraInner, msg: String) {
        if let Ok(mut guard) = inner.last_error.lock() {
            *guard = Some(msg);
        }
    }

    // =========================================================================
    // iStar feature guards (bd-sfxr)
    // =========================================================================

    /// Check if MCP gain is supported (iStar only).
    pub fn supports_mcp_gain(&self) -> bool {
        self.inner.info.features.mcp_gain
    }

    /// Check if gate mode is supported (iStar only).
    pub fn supports_gate_mode(&self) -> bool {
        self.inner.info.features.gate_mode
    }

    /// Check if DDG output is supported (iStar only).
    pub fn supports_ddg(&self) -> bool {
        self.inner.info.features.ddg_output_delay && self.inner.info.features.ddg_output_width
    }

    /// Set MCP gain with feature guard. Returns error on non-iStar cameras.
    pub async fn set_mcp_gain_guarded(&self, gain: u32) -> Result<()> {
        if !self.supports_mcp_gain() {
            anyhow::bail!(
                "MCP gain not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_mcp_gain(gain).await
    }

    /// Set gate mode with feature guard. Returns error on non-iStar cameras.
    pub async fn set_gate_mode_guarded(&self, mode: &str) -> Result<()> {
        if !self.supports_gate_mode() {
            anyhow::bail!(
                "Gate mode not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_gate_mode(mode).await
    }

    /// Set DDG output delay with feature guard. Returns error on non-iStar cameras.
    pub async fn set_ddg_output_delay_guarded(&self, delay_ps: u64) -> Result<()> {
        if !self.supports_ddg() {
            anyhow::bail!(
                "DDG output not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_ddg_output_delay(delay_ps).await
    }

    /// Set DDG output width with feature guard. Returns error on non-iStar cameras.
    pub async fn set_ddg_output_width_guarded(&self, width_ps: u64) -> Result<()> {
        if !self.supports_ddg() {
            anyhow::bail!(
                "DDG output not available on camera model '{}'",
                self.inner.info.model
            );
        }
        self.set_ddg_output_width(width_ps).await
    }

    // =========================================================================
    // Feature callbacks — reactive parameter updates (bd-oj1k)
    // =========================================================================

    /// Register SDK feature callbacks for reactive parameter updates.
    ///
    /// For each implemented feature in the introspection catalog, registers a
    /// C callback with `AT_RegisterFeatureCallback`. When the SDK internally
    /// changes a feature (e.g., changing binning recalculates exposure limits),
    /// the callback posts the feature name through a channel. A spawned receiver
    /// task re-reads the SDK value and updates the corresponding `Parameter<T>`.
    ///
    /// The bridge architecture avoids async/FFI issues:
    /// ```text
    /// SDK thread → C callback → UnboundedSender<String> → tokio task → Parameter::set_json()
    /// ```
    #[cfg(feature = "camera")]
    pub fn register_feature_callbacks(&self) -> Result<()> {
        use crate::introspection;

        let handle = self.inner.handle;

        // Create the callback bridge — owned, not leaked.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let bridge = Box::new(FeatureCallbackBridge { tx: tx.clone() });

        // Raw pointer for the SDK callback context. The Box stays alive in
        // `inner._callback_bridge`; the pointer is valid until AT_Close().
        let bridge_ptr: *mut std::os::raw::c_void =
            &*bridge as *const FeatureCallbackBridge as *mut std::os::raw::c_void;

        // Only register callbacks for features that have a corresponding
        // Parameter AND are observable types (skip Command and Str).
        let known = introspection::known_features();
        let mut registered = 0u32;

        for (name, ftype, _group) in &known {
            // Skip types that the receiver ignores anyway.
            if matches!(
                ftype,
                introspection::FeatureType::Command | introspection::FeatureType::Str
            ) {
                continue;
            }
            // Only register for features that have a Parameter entry.
            if self.inner.params.get(*name).is_none() {
                continue;
            }
            if !Self::is_feature_implemented(handle, name).unwrap_or(false) {
                continue;
            }
            unsafe {
                let feature_wide = andor_sdk3_sys::to_wide_string(name);
                let ret = andor_sdk3_sys::AT_RegisterFeatureCallback(
                    handle,
                    feature_wide.as_ptr(),
                    Some(Self::sdk_feature_callback),
                    bridge_ptr,
                );
                if ret != andor_sdk3_sys::AT_SUCCESS {
                    tracing::warn!(
                        feature = name,
                        "Failed to register feature callback: {}",
                        crate::error::AndorError::from_code(ret)
                    );
                } else {
                    registered += 1;
                }
            }
        }

        tracing::info!(registered, "SDK feature callbacks registered");

        // Store bridge ownership and sender in Inner so Drop can clean up.
        {
            let mut guard = self.inner._callback_bridge.lock().unwrap();
            *guard = Some(bridge);
        }
        {
            let mut guard = self.inner.callback_tx.lock().unwrap();
            *guard = Some(tx);
        }

        // Spawn receiver task that processes feature change notifications.
        let inner = self.inner.clone();
        let task_handle = tokio::spawn(async move {
            // Build a lookup table: feature name → FeatureType for efficient dispatch.
            let type_map: std::collections::HashMap<&str, introspection::FeatureType> =
                introspection::known_features()
                    .into_iter()
                    .map(|(name, ftype, _)| (name, ftype))
                    .collect();

            while let Some(feature_name) = rx.recv().await {
                tracing::debug!(feature = %feature_name, "Processing SDK feature change");

                // Look up the feature type.
                let Some(&ftype) = type_map.get(feature_name.as_str()) else {
                    tracing::trace!(feature = %feature_name, "Unknown feature in callback, skipping");
                    continue;
                };

                // Re-read the current value from SDK and update the Parameter.
                let handle = inner.handle;
                let value = match ftype {
                    introspection::FeatureType::Float => {
                        Self::get_float_feature(handle, &feature_name)
                            .ok()
                            .map(|v| serde_json::json!(v))
                    }
                    introspection::FeatureType::Int => Self::get_int_feature(handle, &feature_name)
                        .ok()
                        .map(|v| serde_json::json!(v)),
                    introspection::FeatureType::Bool => {
                        Self::get_bool_feature(handle, &feature_name)
                            .ok()
                            .map(|v| serde_json::json!(v))
                    }
                    introspection::FeatureType::Enum => {
                        Self::get_enum_string(handle, &feature_name)
                            .ok()
                            .map(|v| serde_json::json!(v))
                    }
                    introspection::FeatureType::Str | introspection::FeatureType::Command => {
                        None // Filtered at registration, but defensive.
                    }
                };

                if let Some(json_value) = value {
                    if let Some(param) = inner.params.get(&feature_name) {
                        if let Err(e) = param.set_json(json_value) {
                            tracing::trace!(
                                feature = %feature_name,
                                error = %e,
                                "Failed to update parameter from SDK callback"
                            );
                        }
                    }
                }
            }

            tracing::debug!("Feature callback receiver task exited");
        });

        // Store task handle so Drop can abort it.
        {
            let mut guard = self.inner.callback_task_handle.lock().unwrap();
            *guard = Some(task_handle);
        }

        Ok(())
    }

    /// C-compatible callback invoked by the SDK when a feature value changes.
    ///
    /// Posts the feature name through the `FeatureCallbackBridge` channel.
    /// The actual value read + Parameter update happens on the receiver task.
    #[cfg(feature = "camera")]
    unsafe extern "C" fn sdk_feature_callback(
        _handle: andor_sdk3_sys::AT_H,
        feature: *const andor_sdk3_sys::AT_WC,
        context: *mut std::os::raw::c_void,
    ) -> std::os::raw::c_int {
        let feature_name = if feature.is_null() {
            "unknown".to_string()
        } else {
            // Read wide string — find null terminator
            let mut len = 0;
            while *feature.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(feature, len);
            String::from_utf16_lossy(slice)
        };

        // Post to the bridge channel (non-blocking, fire-and-forget).
        if !context.is_null() {
            let bridge = &*(context as *const FeatureCallbackBridge);
            let _ = bridge.tx.send(feature_name);
        } else {
            tracing::debug!(feature = %feature_name, "SDK feature callback (no bridge)");
        }

        andor_sdk3_sys::AT_CALLBACK_SUCCESS
    }
}

// Implement capability traits

#[async_trait]
impl FrameProducer for AndorCamera {
    async fn start_stream(&self) -> Result<()> {
        if self.inner.streaming.load(Ordering::Relaxed) {
            anyhow::bail!("Camera is already streaming");
        }

        self.inner.streaming.store(true, Ordering::SeqCst);
        self.inner.frame_count.store(0, Ordering::Relaxed);
        self.inner.frames_dropped.store(0, Ordering::Relaxed);
        self.inner.last_hw_frame_nr.store(-1, Ordering::Relaxed);

        #[cfg(feature = "camera")]
        {
            let inner = self.inner.clone();
            let handle = inner.handle;

            // Query current frame dimensions and pixel encoding from SDK
            let (image_size, aoi_width, aoi_height, aoi_stride, pixel_encoding) =
                tokio::task::spawn_blocking(move || -> Result<(usize, u32, u32, usize, String)> {
                    let img_bytes = Self::get_int_feature(handle, "ImageSizeBytes")? as usize;
                    let w = Self::get_int_feature(handle, "AOIWidth")? as u32;
                    let h = Self::get_int_feature(handle, "AOIHeight")? as u32;
                    let stride = Self::get_int_feature(handle, "AOIStride")? as usize;
                    let encoding = Self::get_enum_string(handle, "PixelEncoding")
                        .unwrap_or_else(|_| "Unknown".to_string());
                    Ok((img_bytes, w, h, stride, encoding))
                })
                .await??;

            // Determine bytes per pixel from encoding
            let bytes_per_pixel: usize = match pixel_encoding.as_str() {
                "Mono16" => 2,
                "Mono12" => 2, // 12-bit stored in 16-bit container
                "Mono12Packed" => {
                    tracing::warn!(
                        "Mono12Packed encoding detected — stride-aware extraction needed"
                    );
                    2 // approximate; actual extraction requires stride handling
                }
                "Mono32" => 4,
                other => {
                    tracing::warn!(
                        pixel_encoding = other,
                        "Unknown pixel encoding, assuming 16-bit"
                    );
                    2
                }
            };

            tracing::info!(
                image_size,
                aoi_width,
                aoi_height,
                aoi_stride,
                %pixel_encoding,
                bytes_per_pixel,
                "SDK3 acquisition parameters"
            );

            // Allocate SDK buffers and queue them
            let buffer_count = crate::buffer::DEFAULT_BUFFER_COUNT;
            let sdk_buffers = Arc::new(crate::buffer::SdkBufferSet::new(buffer_count, image_size));

            tokio::task::spawn_blocking({
                let sdk_buffers = sdk_buffers.clone();
                move || -> Result<()> {
                    use crate::error::sdk_result;
                    unsafe {
                        for buf in sdk_buffers.iter() {
                            let ret = AT_QueueBuffer(
                                handle,
                                buf.as_ptr(),
                                buf.size() as std::os::raw::c_int,
                            );
                            sdk_result(ret)?;
                        }
                    }
                    Ok(())
                }
            })
            .await??;

            // Start SDK acquisition
            tokio::task::spawn_blocking(move || -> Result<()> {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("AcquisitionStart");
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;
                }
                Ok(())
            })
            .await??;

            // Spawn the acquisition loop on a blocking thread
            let acq_inner = self.inner.clone();
            let acq_handle = tokio::task::spawn(Self::acquisition_loop(
                acq_inner,
                sdk_buffers,
                aoi_width,
                aoi_height,
                bytes_per_pixel,
            ));
            *self.inner.acq_task_handle.lock().await = Some(acq_handle);
        }

        #[cfg(not(feature = "camera"))]
        {
            // Mock: spawn a simple frame generation loop
            let inner = self.inner.clone();
            let acq_handle = tokio::task::spawn(Self::mock_acquisition_loop(inner));
            *self.inner.acq_task_handle.lock().await = Some(acq_handle);
        }

        tracing::info!("Camera streaming started");
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        if !self.inner.streaming.load(Ordering::Relaxed) {
            return Ok(()); // Already stopped
        }

        // Signal the loop to stop
        self.inner.streaming.store(false, Ordering::SeqCst);

        // Abort the acquisition task
        if let Some(handle) = self.inner.acq_task_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await; // Ignore JoinError from abort
        }

        #[cfg(feature = "camera")]
        {
            let handle = self.inner.handle;
            tokio::task::spawn_blocking(move || -> Result<()> {
                use crate::error::sdk_result;
                unsafe {
                    let feature = to_wide_string("AcquisitionStop");
                    let ret = AT_Command(handle, feature.as_ptr());
                    sdk_result(ret)?;

                    // Flush all queued buffers
                    let ret = AT_Flush(handle);
                    sdk_result(ret)?;
                }
                Ok(())
            })
            .await??;
        }

        tracing::info!("Camera streaming stopped");
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (self.inner.info.sensor_width, self.inner.info.sensor_height)
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let handle = ObserverHandle(self.inner.next_observer_id.fetch_add(1, Ordering::Relaxed));
        self.inner.tap_registry.register(handle, observer);
        Ok(handle)
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        self.inner.tap_registry.unregister(handle);
        Ok(())
    }

    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<LoanedFrame>,
    ) -> Result<()> {
        *self.inner.primary_tx.lock().await = Some(tx);
        tracing::debug!("Primary output registered");
        Ok(())
    }
}

impl AndorCamera {
    /// Hardware acquisition loop (runs on a tokio task, uses spawn_blocking for FFI).
    ///
    /// Flow: AT_WaitBuffer → identify buffer → pool.acquire → copy → metadata → send → notify → re-queue
    #[cfg(feature = "camera")]
    async fn acquisition_loop(
        inner: Arc<AndorCameraInner>,
        sdk_buffers: Arc<crate::buffer::SdkBufferSet>,
        aoi_width: u32,
        aoi_height: u32,
        bytes_per_pixel: usize,
    ) {
        let handle = inner.handle;
        let timeout_ms: std::os::raw::c_int = 10_000; // 10s timeout per frame

        while inner.streaming.load(Ordering::SeqCst) {
            // Wait for next frame from SDK (blocking FFI call)
            let wait_result = tokio::task::spawn_blocking({
                move || -> Result<(*mut u8, usize), crate::error::AndorError> {
                    unsafe {
                        let mut ptr: *mut u8 = std::ptr::null_mut();
                        let mut size: std::os::raw::c_int = 0;
                        let ret = AT_WaitBuffer(handle, &mut ptr, &mut size, timeout_ms);
                        if ret != 0 {
                            return Err(crate::error::AndorError::from_code(ret));
                        }
                        Ok((ptr, size as usize))
                    }
                }
            })
            .await;

            // Handle join error (task cancelled)
            let wait_result = match wait_result {
                Ok(r) => r,
                Err(_) => break, // Task aborted
            };

            // Handle SDK wait result
            let (frame_ptr, frame_size) = match wait_result {
                Ok((ptr, size)) => (ptr, size),
                Err(e) if e.is_timeout() => {
                    tracing::warn!("AT_WaitBuffer timeout, retrying");
                    continue;
                }
                Err(e) => {
                    if inner.streaming.load(Ordering::Relaxed) {
                        let msg = format!("AT_WaitBuffer error: {e}");
                        tracing::error!("{msg}");
                        Self::set_error(&inner, msg);
                    }
                    break;
                }
            };

            // Get frame number
            let frame_nr = inner.frame_count.fetch_add(1, Ordering::Relaxed) as u64;

            // Frame loss detection (bd-fami): check for SDK frame count gaps
            {
                let hw_nr = Self::get_int_feature(handle, "FrameCount").unwrap_or(-1) as i32;
                if hw_nr >= 0 {
                    let prev = inner.last_hw_frame_nr.swap(hw_nr, Ordering::Relaxed);
                    if prev >= 0 && hw_nr > prev + 1 {
                        let lost = (hw_nr - prev - 1) as u64;
                        inner.frames_dropped.fetch_add(lost, Ordering::Relaxed);
                        tracing::warn!(hw_prev = prev, hw_now = hw_nr, lost, "Frame loss detected");
                    }
                }
            }

            // Acquire a pool slot and copy frame data
            let pool_result = inner.frame_pool.try_acquire();
            match pool_result {
                Some(mut loaned) => {
                    // Copy pixel data from SDK buffer into pool slot
                    let pixel_bytes =
                        (aoi_width as usize) * (aoi_height as usize) * bytes_per_pixel;
                    let copy_len = pixel_bytes.min(frame_size).min(loaned.capacity());

                    // SAFETY: frame_ptr is valid (just returned by AT_WaitBuffer),
                    // copy_len is bounded by both frame_size and pool capacity
                    unsafe {
                        loaned.copy_from_sdk(frame_ptr as *const u8, copy_len);
                    }

                    // Fill metadata (bd-v8je)
                    loaned.frame_number = frame_nr;
                    loaned.width = aoi_width;
                    loaned.height = aoi_height;
                    loaned.bit_depth = 16;
                    loaned.timestamp_ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64;
                    loaned.exposure_ms = inner.exposure_s.get() * 1000.0;
                    loaned.temperature_c = Some(inner.temperature_c.get());
                    let bin = inner.binning.get();
                    loaned.binning = Some((bin.0 as u16, bin.1 as u16));

                    // Notify secondary observers via tap registry
                    let view = FrameView::new(
                        loaned.width,
                        loaned.height,
                        loaned.bit_depth,
                        loaned.pixel_data(),
                        loaned.frame_number,
                        loaned.timestamp_ns,
                    )
                    .with_exposure(loaned.exposure_ms);
                    inner.tap_registry.notify(&view);

                    // Send to primary consumer
                    if let Some(tx) = inner.primary_tx.lock().await.as_ref() {
                        if tx.try_send(loaned).is_err() {
                            tracing::trace!("Primary output full, frame {frame_nr} dropped");
                        }
                    }
                }
                None => {
                    tracing::warn!("Frame pool exhausted, frame {frame_nr} dropped");
                }
            }

            // Re-queue the SDK buffer
            let requeue_result = tokio::task::spawn_blocking({
                let sdk_buffers = sdk_buffers.clone();
                move || -> Result<()> {
                    use crate::error::sdk_result;
                    if let Some(idx) = sdk_buffers.index_for_ptr(frame_ptr as *const u8) {
                        if let Some(buf) = sdk_buffers.get(idx) {
                            unsafe {
                                let ret = AT_QueueBuffer(
                                    handle,
                                    buf.as_ptr(),
                                    buf.size() as std::os::raw::c_int,
                                );
                                sdk_result(ret)?;
                            }
                        }
                    } else {
                        tracing::error!("WaitBuffer returned unknown pointer");
                    }
                    Ok(())
                }
            })
            .await;

            if let Err(e) = requeue_result {
                tracing::error!("Failed to re-queue buffer: {e}");
                break;
            }
            if let Ok(Err(e)) = requeue_result {
                tracing::error!("AT_QueueBuffer failed: {e}");
                break;
            }
        }

        tracing::debug!("Acquisition loop exited");
    }

    /// Mock acquisition loop for non-hardware builds.
    #[cfg(not(feature = "camera"))]
    async fn mock_acquisition_loop(inner: Arc<AndorCameraInner>) {
        use tokio::time::Duration;

        let width = inner.info.sensor_width;
        let height = inner.info.sensor_height;

        while inner.streaming.load(Ordering::Relaxed) {
            let exposure = inner.exposure_s.get();
            tokio::time::sleep(Duration::from_secs_f64(exposure)).await;

            let frame_nr = inner.frame_count.fetch_add(1, Ordering::Relaxed) as u64;

            if let Some(mut loaned) = inner.frame_pool.try_acquire() {
                // Generate synthetic gradient pattern
                let pixel_count = (width as usize) * (height as usize);
                let byte_count = pixel_count * 2;
                let actual_len = byte_count.min(loaned.capacity());

                // Write gradient directly into pool buffer
                let offset = (frame_nr % 100) as u16;
                let buf = &mut loaned.pixels[..actual_len];
                for y in 0..height {
                    for x in 0..width {
                        let idx = ((y * width + x) as usize) * 2;
                        if idx + 1 < actual_len {
                            let value = ((x + y + offset as u32) % 65535) as u16;
                            buf[idx] = value as u8;
                            buf[idx + 1] = (value >> 8) as u8;
                        }
                    }
                }
                loaned.actual_len = actual_len;

                // Fill metadata (bd-v8je)
                loaned.frame_number = frame_nr;
                loaned.width = width;
                loaned.height = height;
                loaned.bit_depth = 16;
                loaned.timestamp_ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                loaned.exposure_ms = exposure * 1000.0;
                loaned.temperature_c = Some(inner.temperature_c.get());
                let bin = inner.binning.get();
                loaned.binning = Some((bin.0 as u16, bin.1 as u16));

                // Notify observers
                let view = FrameView::new(
                    loaned.width,
                    loaned.height,
                    loaned.bit_depth,
                    loaned.pixel_data(),
                    loaned.frame_number,
                    loaned.timestamp_ns,
                )
                .with_exposure(loaned.exposure_ms);
                inner.tap_registry.notify(&view);

                // Send to primary consumer
                if let Some(tx) = inner.primary_tx.lock().await.as_ref() {
                    if tx.try_send(loaned).is_err() {
                        tracing::trace!("Primary output full, mock frame {frame_nr} dropped");
                    }
                }
            }
        }

        tracing::debug!("Mock acquisition loop exited");
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

        self.inner.exposure_s.set(seconds).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.inner.exposure_s.get())
    }
}

impl Parameterized for AndorCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.inner.params
    }
}

#[async_trait]
impl Commandable for AndorCamera {
    async fn execute_command(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match command {
            "set_cooling" => {
                let enabled = args
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .ok_or_else(|| anyhow::anyhow!("Missing bool 'enabled' argument"))?;
                self.set_cooling(enabled).await?;
                Ok(serde_json::json!({"cooling": enabled}))
            }
            "set_target_temperature" => {
                let temp = args
                    .get("temperature_c")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow::anyhow!("Missing float 'temperature_c' argument"))?;
                self.set_target_temperature(temp).await?;
                Ok(serde_json::json!({"target_temperature_c": temp}))
            }
            "get_cooling_status" => {
                let status = self.get_cooling_status().await?;
                Ok(serde_json::json!({"status": format!("{:?}", status)}))
            }
            "get_temperature" => {
                let temp = self.get_temperature().await?;
                Ok(serde_json::json!({"temperature_c": temp}))
            }
            "get_frames_dropped" => {
                let dropped = self.frames_dropped();
                Ok(serde_json::json!({"frames_dropped": dropped}))
            }
            _ => anyhow::bail!("Unknown command: {command}"),
        }
    }
}

impl Drop for AndorCameraInner {
    fn drop(&mut self) {
        // Abort drift polling task if running
        if let Ok(mut guard) = self.drift_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        // Abort acquisition task if running
        if let Ok(mut guard) = self.acq_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        // Shut down feature callback pipeline:
        // 1. Drop the sender to close the channel (receiver task will exit)
        // 2. Abort the receiver task as a backstop
        // The bridge Box is dropped automatically with the struct fields,
        // AFTER AT_Close() below unregisters all SDK callbacks.
        #[cfg(feature = "camera")]
        {
            if let Ok(mut guard) = self.callback_tx.try_lock() {
                guard.take(); // Drop the sender → channel closes
            }
        }
        if let Ok(mut guard) = self.callback_task_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }

        #[cfg(feature = "camera")]
        unsafe {
            if self.handle != AT_HANDLE_UNINITIALISED {
                // Stop acquisition if still streaming
                if self.streaming.load(Ordering::Relaxed) {
                    let feature = to_wide_string("AcquisitionStop");
                    let _ = AT_Command(self.handle, feature.as_ptr());
                    self.streaming.store(false, Ordering::Relaxed);
                }

                // Flush all queued buffers before closing
                let _ = AT_Flush(self.handle);

                AT_Close(self.handle);

                // Only finalize library when last instance is dropped
                if LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    AT_FinaliseLibrary();
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(feature = "camera"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dynamic_features_registered() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();
        let names = param_set.names();

        // Core parameters should exist
        assert!(names.contains(&"exposure_s"));
        assert!(names.contains(&"trigger_mode"));
        assert!(names.contains(&"mcp_gain"));
        assert!(names.contains(&"temperature_c"));

        // Dynamic parameters should also exist
        assert!(names.contains(&"FrameRate"), "FrameRate missing");
        assert!(names.contains(&"PixelEncoding"), "PixelEncoding missing");
        assert!(names.contains(&"SensorCooling"), "SensorCooling missing");
        assert!(names.contains(&"FanSpeed"), "FanSpeed missing");
        assert!(names.contains(&"CameraModel"), "CameraModel missing");

        // Total: 11 core + ~50 dynamic
        assert!(
            names.len() > 50,
            "Expected 50+ parameters (11 core + dynamic), got {}",
            names.len()
        );
    }

    #[tokio::test]
    async fn test_dynamic_enum_has_choices() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();

        let param = param_set
            .get("PixelEncoding")
            .expect("PixelEncoding should exist");
        let meta = param.metadata();
        assert_eq!(meta.dtype, "enum");
        assert!(!meta.enum_values.is_empty());
        assert!(meta.enum_values.contains(&"Mono16".to_string()));
    }

    #[tokio::test]
    async fn test_dynamic_float_has_range() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();

        let param = param_set.get("FrameRate").expect("FrameRate should exist");
        let meta = param.metadata();
        assert_eq!(meta.dtype, "float");
        assert!(meta.min_value.is_some());
        assert!(meta.max_value.is_some());
    }

    #[tokio::test]
    async fn test_dynamic_readonly_rejects_set() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();

        let param = param_set.get("FrameRate").expect("FrameRate should exist");
        assert!(param.metadata().read_only, "FrameRate should be read-only");

        let result = param.set_json(serde_json::json!(100.0));
        assert!(result.is_err(), "Setting read-only parameter should fail");
    }

    #[tokio::test]
    async fn test_no_duplicate_parameter_names() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();
        let mut names = param_set.names();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), original_len, "Found duplicate parameter names");
    }

    #[tokio::test]
    async fn test_core_features_not_duplicated() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();

        // SDK3 name should NOT be registered (covered by core Rust-typed parameter)
        assert!(
            param_set.get("ExposureTime").is_none(),
            "ExposureTime should not exist (covered by exposure_s)"
        );
        assert!(
            param_set.get("exposure_s").is_some(),
            "exposure_s core parameter should exist"
        );
    }
}
