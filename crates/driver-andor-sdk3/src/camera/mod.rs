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
//! 3. Configure cooling (`TemperatureControl` enum → `SensorCooling` → `FanSpeed`)
//! 4. Configure AOI (Area of Interest) and binning
//! 5. Set trigger mode, exposure, gate mode
//! 6. Configure MCP gain and DDG timing
//! 7. Start acquisition
//!
//! ## Cooling Model
//!
//! Andor SDK3 cameras have two temperature-setting mechanisms:
//!
//! - **`TemperatureControl`** (enum): Discrete calibrated setpoints (e.g., `"0.00"`).
//!   Preferred on cameras that support it (iStar, Zyla, Marana). The SDK manages
//!   the TEC PID loop for these validated points.
//! - **`TargetSensorTemperature`** (float): On cameras with `TemperatureControl`,
//!   this is **read-only** — it reflects the setpoint the SDK selected from the
//!   enum. On simpler cameras without the enum, this float is writable.
//!
//! The `configure_cooling()` method tries the enum first, then falls back to the
//! float for cameras that don't implement `TemperatureControl`.
//! `AT_Close` does **not** disable the TEC — cooling persists at the hardware level.
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

mod configuration;
mod drop;
mod parameters;
mod sdk_features;
mod traits;

use crate::types::{
    CameraInfo, DeviceState, ElectronicShutteringMode, GateMode, InsertionDelay, TriggerMode,
};
use anyhow::Result;
use common::capabilities::{FrameObserver, LoanedFrame, ObserverHandle};
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
    // bd-zg9e: promoted from dynamic to core
    "MCPIntelligentGating",
    "MCPVoltage",
    "InsertionDelay",
    "CameraAcquiring",
    "BaselineLevel",
];

#[cfg(feature = "camera")]
use crate::error::AndorError;
#[cfg(feature = "camera")]
use std::sync::atomic::AtomicUsize;
#[cfg(feature = "camera")]
use std::sync::Mutex as StdMutex;

#[cfg(feature = "camera")]
use andor_sdk3_sys::*;

/// Camera handle type: `AT_H` (`c_int`) with the SDK feature, `i32` without.
///
/// Both aliases resolve to the same 32-bit integer; the alias exists so that
/// `new_inner` can accept the handle without a second layer of `#[cfg]` guards.
#[cfg(feature = "camera")]
type CameraHandle = AT_H;
#[cfg(not(feature = "camera"))]
type CameraHandle = i32;

// =============================================================================
// Blocking bridge helper
// =============================================================================

/// Run an SDK FFI call on `spawn_blocking` and map errors to `DaqError`.
///
/// This eliminates the repeated `.await.map_err(...)?.map_err(...)` pattern
/// used by every `Parameter<T>` hardware callback in this module.
#[cfg(feature = "camera")]
async fn sdk_blocking<F, T>(f: F) -> Result<T, DaqError>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| DaqError::Instrument(format!("spawn_blocking: {e}")))?
        .map_err(|e| DaqError::Instrument(e.to_string()))
}

/// Pause SDK acquisition, apply a parameter change, then restart (bd-4msn, bd-71sq).
///
/// Stops acquisition, **flushes both SDK buffer queues**, applies the parameter
/// change, re-queues all buffers, and restarts acquisition. The flush is
/// required by the SDK3 documentation (§2.3.9): "Failure to call AT_Flush
/// after an acquisition… may lead to undefined behaviour."
///
/// Prior to bd-71sq this function skipped the flush, which caused a recurring
/// general protection fault (GPF) in the SDK's internal `shared_ptr` bookkeeping
/// when the stale buffer queue was reused after AcquisitionStart.
///
/// The acquisition loop must tolerate the brief `AT_WaitBuffer` error this
/// causes — see the retry logic in `acquisition_loop`.
#[cfg(feature = "camera")]
fn pause_apply_restart(
    handle: AT_H,
    sdk_buffers: &Option<Arc<crate::buffer::SdkBufferSet>>,
    f: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    use crate::error::sdk_result;

    // Step 1: Stop acquisition
    unsafe {
        let stop = to_wide_string("AcquisitionStop");
        sdk_result(AT_Command(handle, stop.as_ptr()))?;
    }

    // Step 2: Flush both SDK buffer queues (required by SDK3 §2.3.9)
    unsafe {
        sdk_result(AT_Flush(handle))?;
    }
    tracing::debug!(sdk_handle = handle, "Flushed SDK buffers for param change");

    // Step 3: Apply the parameter change
    let result = f();

    // Step 4: Re-queue all buffers and restart (always, even if param change failed)
    if let Some(ref bufs) = *sdk_buffers {
        unsafe {
            for buf in bufs.iter() {
                let ret = AT_QueueBuffer(handle, buf.as_ptr(), buf.size() as std::os::raw::c_int);
                if let Err(e) = sdk_result(ret) {
                    tracing::error!("Failed to re-queue buffer after param change: {e}");
                    return Err(e.into());
                }
            }
        }
    }

    unsafe {
        let start = to_wide_string("AcquisitionStart");
        if let Err(e) = sdk_result(AT_Command(handle, start.as_ptr())) {
            tracing::error!("Failed to restart acquisition after param change: {e}");
            return Err(e.into());
        }
    }

    result
}

/// Global instance counter for SDK library lifecycle management.
#[cfg(feature = "camera")]
static LIBRARY_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "camera")]
static LIBRARY_INIT_MUTEX: StdMutex<()> = StdMutex::new(());
#[cfg(feature = "camera")]
const ANDOR_RECOVERY_ROOTS: &[&str] = &["/tmp", "/dev/shm"];
#[cfg(feature = "camera")]
const ANDOR_RECOVERY_PREFIXES: &[&str] = &[
    "andor",
    "atcore",
    "atdebug",
    "shamrock",
    "sem.andor",
    "sem.atcore",
    "sem.atdebug",
    "sem.shamrock",
];

/// Bridge between the C FFI callback thread and the async parameter update task.
#[cfg(feature = "camera")]
pub(crate) struct FeatureCallbackBridge {
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

pub(crate) struct AndorCameraInner {
    #[cfg(feature = "camera")]
    pub(crate) handle: AT_H,
    #[cfg(not(feature = "camera"))]
    pub(crate) handle: i32,

    pub(crate) info: CameraInfo,
    pub(crate) streaming: Arc<AtomicBool>,
    pub(crate) armed: AtomicBool,
    pub(crate) frame_count: AtomicU32,

    // Acquisition parameters
    pub(crate) exposure_s: Parameter<f64>,
    pub(crate) trigger_mode: Parameter<TriggerMode>,
    pub(crate) gate_mode: Parameter<GateMode>,
    pub(crate) mcp_gain: Parameter<u32>,
    pub(crate) ddg_output_delay_ps: Parameter<u64>,
    pub(crate) ddg_output_width_ps: Parameter<u64>,

    // AOI parameters
    pub(crate) roi: Parameter<Roi>,
    pub(crate) binning: Parameter<(u32, u32)>,

    // Temperature and cooling (bd-zekj)
    pub(crate) temperature_c: Parameter<f64>,
    pub(crate) cooling_enabled: AtomicBool,
    pub(crate) target_temperature_c: Parameter<f64>,

    // Electronic shuttering (bd-apwl)
    pub(crate) electronic_shuttering: Parameter<ElectronicShutteringMode>,

    // bd-zg9e: iStar intensifier features
    pub(crate) mcp_intelligate: Parameter<bool>,
    pub(crate) mcp_voltage: Parameter<u32>,
    pub(crate) insertion_delay: Parameter<InsertionDelay>,

    // bd-zg9e: per-frame metadata toggles
    pub(crate) metadata_ddg_info: Parameter<bool>,
    pub(crate) metadata_mcp_gain: Parameter<bool>,
    pub(crate) metadata_frame_info: Parameter<bool>,

    // bd-zg9e: acquisition status + diagnostics
    pub(crate) camera_acquiring: Parameter<bool>,
    pub(crate) baseline_level: Parameter<i64>,

    // bd-zg9e.10: device lifecycle state
    pub(crate) device_state: Parameter<DeviceState>,

    // Frame loss tracking (bd-fami)
    pub(crate) frames_dropped: AtomicU64,
    pub(crate) last_hw_frame_nr: std::sync::atomic::AtomicI32,

    // Hardware timestamp clock frequency in Hz (bd-z54k)
    pub(crate) hw_timestamp_freq: AtomicU64,

    // Error tracking (bd-z95k)
    pub(crate) last_error: std::sync::Mutex<Option<String>>,

    // Frame observers (bd-0dax.4)
    pub(crate) observers: Mutex<Vec<(ObserverHandle, Box<dyn FrameObserver>)>>,
    pub(crate) next_observer_id: AtomicU64,

    // Drift polling task handle (bd-j4aa)
    pub(crate) drift_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // Acquisition output channels (bd-b2kf.3, bd-b2kf.8)
    pub(crate) primary_tx: Mutex<Option<tokio::sync::mpsc::Sender<LoanedFrame>>>,
    pub(crate) frame_pool: Arc<pool::Pool<pool::FrameData>>,
    pub(crate) tap_registry: TapRegistry,
    pub(crate) acq_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // SDK buffer set for the current acquisition (bd-71sq).
    // Arc-wrapped so parameter callbacks (which capture a clone before the struct
    // is constructed) share the same mutex as start_stream/stop_stream.
    #[cfg(feature = "camera")]
    pub(crate) sdk_buffers: Arc<std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>>,

    // Feature callback lifecycle (bd-joqu / Copilot review)
    #[cfg(feature = "camera")]
    pub(crate) _callback_bridge: Mutex<Option<Box<FeatureCallbackBridge>>>,
    #[cfg(feature = "camera")]
    pub(crate) callback_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    pub(crate) callback_task_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,

    // Registered feature callback names for cleanup (bd-cytq)
    #[cfg(feature = "camera")]
    pub(crate) registered_callbacks: std::sync::Mutex<Vec<String>>,

    // Parameters
    pub(crate) params: ParameterSet,
}

// =========================================================================
// Lightweight TapRegistry (reimplemented from PVCAM pattern, ~40 lines)
// =========================================================================

/// Minimal frame tap registry for observer notification.
pub(crate) struct TapRegistry {
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

    /// Drop all registered observers, closing their sender channels.
    fn clear_all(&self) {
        let count = {
            let mut taps = self.taps.write();
            let count = taps.len();
            taps.clear();
            count
        };
        if count > 0 {
            tracing::debug!(count, "Cleared all observers on acquisition error");
        }
    }
}

impl AndorCamera {
    /// Create new mock camera instance for testing
    pub async fn new_mock() -> Result<Self> {
        #[cfg(feature = "camera")]
        let handle: CameraHandle = AT_HANDLE_UNINITIALISED;
        #[cfg(not(feature = "camera"))]
        let handle: CameraHandle = 0;
        Self::new_inner(handle, Self::mock_camera_info(0)).await
    }

    /// Create new camera instance (async, validates device identity)
    pub async fn new_async(camera_index: i32) -> Result<Self> {
        #[cfg(feature = "camera")]
        let (handle, info) =
            tokio::task::spawn_blocking(move || Self::init_hardware(camera_index)).await??;

        #[cfg(not(feature = "camera"))]
        let (handle, info) = (camera_index, Self::mock_camera_info(camera_index));

        Self::new_inner(handle, info).await
    }

    /// Shared camera construction from a resolved `(handle, info)` pair.
    async fn new_inner(handle: CameraHandle, info: CameraInfo) -> Result<Self> {
        let sensor_width = info.sensor_width;
        let sensor_height = info.sensor_height;

        #[allow(unused_mut)]
        let mut exposure_s = Parameter::new("exposure_s", 0.001)
            .with_unit("s")
            .with_description("Integration time");

        #[allow(unused_mut)]
        let mut trigger_mode =
            Parameter::new("trigger_mode", TriggerMode::Internal).with_description("Trigger mode");

        #[allow(unused_mut)]
        let mut gate_mode =
            Parameter::new("gate_mode", GateMode::CWOn).with_description("MCP gate mode");

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
            .with_unit("\u{00b0}C")
            .with_description("Sensor temperature");

        #[allow(unused_mut)]
        let mut target_temperature_c = Parameter::new("target_temperature_c", -20.0)
            .with_unit("\u{00b0}C")
            .with_description(
                "Target cooling temperature (read-only, set via TemperatureControl enum)",
            );

        #[allow(unused_mut)]
        let mut electronic_shuttering =
            Parameter::new("electronic_shuttering", ElectronicShutteringMode::Rolling)
                .with_description("Electronic shuttering mode");

        // bd-zg9e: iStar intensifier parameters
        #[allow(unused_mut)]
        let mut mcp_intelligate = Parameter::new("mcp_intelligate", false).with_description(
            "MCP Intelligate (simultaneous photocathode+MCP gating for UV safety)",
        );

        #[allow(unused_mut)]
        let mut mcp_voltage = Parameter::new("mcp_voltage", 0u32)
            .with_description("MCP high voltage read-back")
            .read_only();

        #[allow(unused_mut)]
        let mut insertion_delay = Parameter::new("insertion_delay", InsertionDelay::Normal)
            .with_description("Intensifier insertion delay (Normal ~40ns, Fast <19ns)");

        // bd-zg9e: per-frame metadata toggles
        #[allow(unused_mut)]
        let mut metadata_ddg_info = Parameter::new("metadata_ddg_info", false)
            .with_description("Include DDG timing in per-frame metadata");

        #[allow(unused_mut)]
        let mut metadata_mcp_gain_param = Parameter::new("metadata_mcp_gain", false)
            .with_description("Include MCP gain in per-frame metadata");

        #[allow(unused_mut)]
        let mut metadata_frame_info = Parameter::new("metadata_frame_info", false)
            .with_description("Include frame info in per-frame metadata");

        // bd-zg9e: acquisition status + diagnostics
        #[allow(unused_mut)]
        let mut camera_acquiring = Parameter::new("camera_acquiring", false)
            .with_description("Camera is actively acquiring")
            .read_only();

        #[allow(unused_mut)]
        let mut baseline_level = Parameter::new("baseline_level", 0i64)
            .with_description("Electronic baseline level (ADU)")
            .read_only();

        // bd-zg9e.10: device lifecycle state
        let device_state = Parameter::new("device_state", DeviceState::Initializing)
            .with_description("Device lifecycle state")
            .read_only();

        let streaming_flag = Arc::new(AtomicBool::new(false));

        // Shared buffer set reference for pause_apply_restart flush/re-queue (bd-71sq).
        // Created here so parameter callbacks can access it; populated in start_stream.
        #[cfg(feature = "camera")]
        let sdk_buffers_lock: Arc<
            std::sync::Mutex<Option<Arc<crate::buffer::SdkBufferSet>>>,
        > = Arc::new(std::sync::Mutex::new(None));

        #[cfg(feature = "camera")]
        {
            Self::attach_exposure_callback(
                &mut exposure_s,
                handle,
                streaming_flag.clone(),
                sdk_buffers_lock.clone(),
            );
            Self::attach_trigger_mode_callback(&mut trigger_mode, handle);
            Self::attach_gate_mode_callback(&mut gate_mode, handle);
            Self::attach_mcp_gain_callback(
                &mut mcp_gain,
                handle,
                streaming_flag.clone(),
                sdk_buffers_lock.clone(),
            );
            Self::attach_ddg_delay_callback(
                &mut ddg_output_delay_ps,
                handle,
                streaming_flag.clone(),
                sdk_buffers_lock.clone(),
            );
            Self::attach_ddg_width_callback(
                &mut ddg_output_width_ps,
                handle,
                streaming_flag.clone(),
                sdk_buffers_lock.clone(),
            );
            Self::attach_temperature_reader(&mut temperature_c, handle);
            Self::attach_target_temperature_reader(&mut target_temperature_c, handle);
            Self::attach_roi_callback(&mut roi, handle);
            Self::attach_binning_callback(&mut binning, handle);
            Self::attach_electronic_shuttering_callback(&mut electronic_shuttering, handle);
            // bd-zg9e: new callbacks
            Self::attach_mcp_intelligate_callback(
                &mut mcp_intelligate,
                handle,
                streaming_flag.clone(),
                sdk_buffers_lock.clone(),
            );
            Self::attach_mcp_voltage_reader(&mut mcp_voltage, handle);
            Self::attach_insertion_delay_callback(&mut insertion_delay, handle);
            Self::attach_metadata_bool_callback(&mut metadata_ddg_info, handle, "MetadataEnable");
            Self::attach_metadata_bool_callback(
                &mut metadata_mcp_gain_param,
                handle,
                "MetadataEnable",
            );
            Self::attach_metadata_bool_callback(&mut metadata_frame_info, handle, "MetadataFrame");
            Self::attach_camera_acquiring_reader(&mut camera_acquiring, handle);
            Self::attach_baseline_level_reader(&mut baseline_level, handle);
        }

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
        // bd-zg9e: register new core parameters
        params.register(mcp_intelligate.clone());
        params.register(mcp_voltage.clone());
        params.register(insertion_delay.clone());
        params.register(metadata_ddg_info.clone());
        params.register(metadata_mcp_gain_param.clone());
        params.register(metadata_frame_info.clone());
        params.register(camera_acquiring.clone());
        params.register(baseline_level.clone());
        params.register(device_state.clone());

        {
            #[cfg(not(feature = "camera"))]
            let discovered = crate::introspection::introspect_mock_features();
            #[cfg(feature = "camera")]
            let discovered = if handle == AT_HANDLE_UNINITIALISED {
                crate::introspection::introspect_mock_features()
            } else {
                crate::introspection::introspect_all_features(handle)
            };
            Self::register_dynamic_features(&discovered, &mut params, handle);
        }

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
            streaming: streaming_flag,
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
            // bd-zg9e: new core parameters
            mcp_intelligate,
            mcp_voltage,
            insertion_delay,
            metadata_ddg_info,
            metadata_mcp_gain: metadata_mcp_gain_param,
            metadata_frame_info,
            camera_acquiring,
            baseline_level,
            device_state,
            frames_dropped: AtomicU64::new(0),
            last_hw_frame_nr: std::sync::atomic::AtomicI32::new(-1),
            hw_timestamp_freq: AtomicU64::new(0),
            last_error: std::sync::Mutex::new(None),
            observers: Mutex::new(Vec::new()),
            next_observer_id: AtomicU64::new(1),
            drift_task_handle: Mutex::new(None),
            primary_tx: Mutex::new(None),
            frame_pool,
            tap_registry: TapRegistry::new(),
            acq_task_handle: Mutex::new(None),
            #[cfg(feature = "camera")]
            sdk_buffers: sdk_buffers_lock,
            #[cfg(feature = "camera")]
            _callback_bridge: Mutex::new(None),
            #[cfg(feature = "camera")]
            callback_tx: Mutex::new(None),
            callback_task_handle: Mutex::new(None),
            #[cfg(feature = "camera")]
            registered_callbacks: std::sync::Mutex::new(Vec::new()),
            params,
        });

        Ok(Self { inner })
    }

    #[cfg(feature = "camera")]
    fn init_hardware(camera_index: i32) -> Result<(AT_H, CameraInfo)> {
        let _guard = match LIBRARY_INIT_MUTEX.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Andor library init mutex poisoned during recovery, proceeding");
                poisoned.into_inner()
            }
        };

        match unsafe { Self::init_hardware_once(camera_index) } {
            Ok(result) => Ok(result),
            Err(err) => {
                let Some(andor_error) = err.downcast_ref::<AndorError>() else {
                    return Err(err);
                };
                if !andor_error.is_recoverable_init_conflict() {
                    return Err(err);
                }

                let cleanup_report =
                    crate::cleanup_runtime_artifacts(ANDOR_RECOVERY_ROOTS, ANDOR_RECOVERY_PREFIXES);
                tracing::warn!(
                    error = %andor_error,
                    removed = cleanup_report.removed_count(),
                    failed = cleanup_report.failed_count(),
                    artifacts = ?cleanup_report,
                    "Andor SDK init reported a stale runtime conflict; cleaning artifacts and retrying once"
                );

                unsafe { Self::init_hardware_once(camera_index) }.map_err(|retry_err| {
                    retry_err.context(format!(
                        "Andor SDK recovery retry failed after cleanup ({})",
                        cleanup_report.summary()
                    ))
                })
            }
        }
    }

    #[cfg(feature = "camera")]
    unsafe fn init_hardware_once(camera_index: i32) -> Result<(AT_H, CameraInfo)> {
        use crate::error::sdk_result;

        unsafe {
            let ret = AT_InitialiseLibrary();
            sdk_result(ret)?;
            LIBRARY_INSTANCE_COUNT.fetch_add(1, Ordering::SeqCst);

            let mut handle = AT_HANDLE_UNINITIALISED;
            let ret = AT_Open(camera_index, &mut handle);
            if ret != AT_SUCCESS {
                if LIBRARY_INSTANCE_COUNT.fetch_sub(1, Ordering::SeqCst) == 1 {
                    AT_FinaliseLibrary();
                }
                return Err(AndorError::from_code(ret).into());
            }

            match Self::query_camera_info(handle) {
                Ok(info) => {
                    if info.features.pixel_encoding {
                        let feat = to_wide_string("PixelEncoding");
                        let val = to_wide_string("Mono16");
                        let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
                        if ret != AT_SUCCESS {
                            tracing::warn!(
                                "Failed to set PixelEncoding=Mono16 (code {}), frames may use packed format",
                                ret
                            );
                        } else {
                            tracing::info!("Set PixelEncoding=Mono16 for reliable frame streaming");
                        }
                    }

                    // Force full-frame AOI to prevent stale hardware state from previous sessions.
                    // The SDK retains AOI across AT_Close/AT_Open cycles, so a killed daemon can
                    // leave the camera cropped (e.g. 640x540 instead of 2560x2160).
                    let sw = info.sensor_width as i64;
                    let sh = info.sensor_height as i64;
                    if let Err(e) = Self::set_int_feature(handle, "AOILeft", 1) {
                        tracing::warn!("Failed to set AOILeft=1: {e}");
                    }
                    if let Err(e) = Self::set_int_feature(handle, "AOITop", 1) {
                        tracing::warn!("Failed to set AOITop=1: {e}");
                    }
                    if let Err(e) = Self::set_int_feature(handle, "AOIWidth", sw) {
                        tracing::warn!(sensor_width = sw, "Failed to set AOIWidth: {e}");
                    }
                    if let Err(e) = Self::set_int_feature(handle, "AOIHeight", sh) {
                        tracing::warn!(sensor_height = sh, "Failed to set AOIHeight: {e}");
                    }
                    tracing::info!(
                        sensor_width = sw,
                        sensor_height = sh,
                        "Forced full-frame AOI on camera initialization"
                    );

                    Ok((handle, info))
                }
                Err(e) => {
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
    unsafe fn get_string_feature_or_default(handle: AT_H, feature: &str, default: &str) -> String {
        let feature_wide = to_wide_string(feature);
        let mut buffer = wide_string_buffer(256);
        let ret = AT_GetString(handle, feature_wide.as_ptr(), buffer.as_mut_ptr(), 256);
        if ret == AT_SUCCESS {
            from_wide_string(&buffer)
        } else {
            default.to_string()
        }
    }

    #[cfg(feature = "camera")]
    unsafe fn query_camera_info(handle: AT_H) -> Result<CameraInfo> {
        use crate::error::sdk_result;

        let sensor_width_feature = to_wide_string("SensorWidth");
        let mut sensor_width: AT_64 = 0;
        let ret = AT_GetInt(handle, sensor_width_feature.as_ptr(), &mut sensor_width);
        sdk_result(ret)?;

        let sensor_height_feature = to_wide_string("SensorHeight");
        let mut sensor_height: AT_64 = 0;
        let ret = AT_GetInt(handle, sensor_height_feature.as_ptr(), &mut sensor_height);
        sdk_result(ret)?;

        let model = Self::get_string_feature_or_default(handle, "CameraModel", "Unknown");
        let serial_number = Self::get_string_feature_or_default(handle, "SerialNumber", "Unknown");
        let firmware_version =
            Self::get_string_feature_or_default(handle, "FirmwareVersion", "Unknown");

        tracing::info!(
            sensor_width,
            sensor_height,
            model = %model,
            serial = %serial_number,
            firmware = %firmware_version,
            "Andor SDK3 sensor dimensions from hardware"
        );

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
            mcp_intelligate: check("MCPIntelligentGating"),
            mcp_voltage: check("MCPVoltage"),
            insertion_delay: check("InsertionDelay"),
            metadata_ddg_info: check("MetadataEnable"),
            metadata_mcp_gain: check("MetadataEnable"),
            metadata_frame_info: check("MetadataFrame"),
            camera_acquiring: check("CameraAcquiring"),
            baseline_level: check("BaselineLevel"),
            software_trigger: check("SoftwareTrigger"),
        }
    }

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
                ..Default::default()
            },
        }
    }

    // =========================================================================
    // Feature callbacks — reactive parameter updates (bd-oj1k)
    // =========================================================================

    #[cfg(feature = "camera")]
    pub async fn register_feature_callbacks(&self) -> Result<()> {
        use crate::introspection;

        let handle = self.inner.handle;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let bridge = Box::new(FeatureCallbackBridge { tx: tx.clone() });

        let bridge_ptr: *mut std::os::raw::c_void =
            &*bridge as *const FeatureCallbackBridge as *mut std::os::raw::c_void;

        tracing::info!(
            sdk_handle = handle,
            bridge_ptr = ?bridge_ptr,
            "Registering Andor SDK3 feature callbacks"
        );

        let known = introspection::known_features();
        let mut registered = 0u32;
        let mut attempted = 0u32;

        for (name, ftype, _group) in &known {
            if matches!(
                ftype,
                introspection::FeatureType::Command | introspection::FeatureType::Str
            ) {
                continue;
            }
            if self.inner.params.get(*name).is_none() {
                continue;
            }
            if !Self::is_feature_implemented(handle, name).unwrap_or(false) {
                continue;
            }
            attempted += 1;
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
                        sdk_handle = handle,
                        feature = name,
                        bridge_ptr = ?bridge_ptr,
                        "Failed to register feature callback: {}",
                        crate::error::AndorError::from_code(ret)
                    );
                } else {
                    tracing::debug!(
                        sdk_handle = handle,
                        feature = name,
                        bridge_ptr = ?bridge_ptr,
                        "Registered Andor feature callback"
                    );
                    if let Ok(mut cbs) = self.inner.registered_callbacks.lock() {
                        cbs.push((*name).to_string());
                    }
                    registered += 1;
                }
            }
        }

        tracing::info!(
            sdk_handle = handle,
            bridge_ptr = ?bridge_ptr,
            attempted,
            registered,
            "Andor SDK3 feature callback registration complete"
        );

        {
            let mut guard = self.inner._callback_bridge.lock().await;
            *guard = Some(bridge);
        }
        {
            let mut guard = self.inner.callback_tx.lock().await;
            *guard = Some(tx);
        }

        let inner = self.inner.clone();
        let task_handle = tokio::spawn(async move {
            let type_map: std::collections::HashMap<&str, introspection::FeatureType> =
                introspection::known_features()
                    .into_iter()
                    .map(|(name, ftype, _)| (name, ftype))
                    .collect();

            while let Some(feature_name) = rx.recv().await {
                tracing::debug!(feature = %feature_name, "Processing SDK feature change");

                let Some(&ftype) = type_map.get(feature_name.as_str()) else {
                    tracing::trace!(feature = %feature_name, "Unknown feature in callback, skipping");
                    continue;
                };

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
                    introspection::FeatureType::Str | introspection::FeatureType::Command => None,
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

        {
            let mut guard = self.inner.callback_task_handle.lock().await;
            *guard = Some(task_handle);
        }

        Ok(())
    }

    /// C-compatible callback invoked by the SDK when a feature value changes.
    #[cfg(feature = "camera")]
    pub(crate) unsafe extern "C" fn sdk_feature_callback(
        _handle: andor_sdk3_sys::AT_H,
        feature: *const andor_sdk3_sys::AT_WC,
        context: *mut std::os::raw::c_void,
    ) -> std::os::raw::c_int {
        let feature_name = if feature.is_null() {
            "unknown".to_string()
        } else {
            let mut len = 0;
            while *feature.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(feature, len);
            slice
                .iter()
                .filter_map(|&c| char::from_u32(c as u32))
                .collect()
        };

        if !context.is_null() {
            let bridge = &*(context as *const FeatureCallbackBridge);
            let _ = bridge.tx.send(feature_name);
        } else {
            tracing::debug!(feature = %feature_name, "SDK feature callback (no bridge)");
        }

        andor_sdk3_sys::AT_CALLBACK_SUCCESS
    }
}

#[cfg(test)]
#[cfg(not(feature = "camera"))]
mod tests {
    use super::*;
    use common::capabilities::Parameterized;

    #[tokio::test]
    async fn test_dynamic_features_registered() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let param_set = camera.parameters();
        let names = param_set.names();

        assert!(names.contains(&"exposure_s"));
        assert!(names.contains(&"trigger_mode"));
        assert!(names.contains(&"mcp_gain"));
        assert!(names.contains(&"temperature_c"));

        assert!(names.contains(&"FrameRate"), "FrameRate missing");
        assert!(names.contains(&"PixelEncoding"), "PixelEncoding missing");
        assert!(names.contains(&"SensorCooling"), "SensorCooling missing");
        assert!(names.contains(&"FanSpeed"), "FanSpeed missing");
        assert!(names.contains(&"CameraModel"), "CameraModel missing");

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

        assert!(
            param_set.get("ExposureTime").is_none(),
            "ExposureTime should not exist (covered by exposure_s)"
        );
        assert!(
            param_set.get("exposure_s").is_some(),
            "exposure_s core parameter should exist"
        );
    }

    #[test]
    fn test_wait_buffer_timeout_is_unsigned() {
        let _: unsafe extern "C" fn(
            andor_sdk3_sys::AT_H,
            *mut *mut u8,
            *mut std::os::raw::c_int,
            std::os::raw::c_uint,
        ) -> std::os::raw::c_int = andor_sdk3_sys::AT_WaitBuffer;
    }
}
