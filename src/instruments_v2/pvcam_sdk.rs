// PVCAM SDK Trait Abstraction
// This module provides a safe, testable interface to the PVCAM SDK

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    ffi::c_void,
    fmt,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// Represents a handle to an opened camera.
/// This wraps the raw `i16` handle from `pvcam-sys` for type safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraHandle(pub i16);

/// Represents a single acquired frame from the camera.
#[derive(Debug)]
pub struct Frame {
    /// The pixel data of the frame.
    pub data: Vec<u16>,
    /// The frame number in the sequence.
    pub frame_number: u32,
    /// Hardware timestamp in microseconds (from camera clock).
    pub hardware_timestamp: Option<i64>,
    /// Software capture time (when frame was received).
    pub software_timestamp: chrono::DateTime<chrono::Utc>,
    /// Actual exposure time in milliseconds.
    pub exposure_time_ms: f64,
    /// Readout duration in milliseconds (time to transfer frame).
    pub readout_time_ms: Option<f64>,
    /// Sensor temperature in degrees Celsius.
    pub sensor_temperature_c: Option<f64>,
    /// Region of interest: (x, y, width, height).
    pub roi: (u16, u16, u16, u16),
}

/// Represents possible errors that can occur during PVCAM SDK operations.
#[derive(Debug, thiserror::Error)]
pub enum PvcamError {
    #[error("Failed to initialize PVCAM SDK: {0}")]
    InitFailed(String),
    
    #[error("Camera not found: {0}")]
    CameraNotFound(String),
    
    #[error("Camera with handle {camera:?} is not open")]
    CameraNotOpen { camera: CameraHandle },
    
    #[error("Camera disconnected: {camera}")]
    CameraDisconnected { camera: String },
    
    #[error("Invalid parameter '{param}': {reason}")]
    InvalidParameter { param: String, reason: String },
    
    #[error("Parameter {param} out of range: value={value}, valid range={valid_range}")]
    OutOfRange {
        param: String,
        value: String,
        valid_range: String,
    },
    
    #[error("Acquisition error for camera {camera}: {reason}")]
    AcquisitionError { camera: String, reason: String },
    
    #[error("Operation timed out: {operation}")]
    Timeout { operation: String },
    
    #[error("PVCAM SDK operation failed with error code: {0}")]
    SdkSpecific(i16),
    
    #[error("Invalid parameter value for {param:?}: {value}")]
    InvalidParamValue {
        param: PvcamParam,
        value: String,
    },
    
    #[error("Parameter {0:?} is not supported or cannot be accessed")]
    ParamNotSupported(PvcamParam),
    
    #[error("Type mismatch for parameter {param:?}. Expected {expected}, got type ID: {actual_type_id:?}")]
    TypeMismatch {
        param: PvcamParam,
        expected: String,
        actual_type_id: TypeId,
    },
    
    #[error("Internal mock error: {0}")]
    MockError(String),
    
    #[error("SDK is not initialized")]
    NotInitialized,
    
    #[error("SDK is already initialized")]
    AlreadyInitialized,
    
    #[error("Acquisition is already in progress for handle {0:?}")]
    AcquisitionInProgress(CameraHandle),
    
    #[error("Failed to convert C string to Rust string: {0}")]
    StringConversionError(#[from] std::str::Utf8Error),
    
    #[error("Failed to allocate memory: {0}")]
    AllocationError(String),
    
    #[error("Buffer overflow: attempted to write {attempted} bytes to {capacity} byte buffer")]
    BufferOverflow { attempted: usize, capacity: usize },
    
    #[error("Frame number gap detected: expected {expected}, got {actual}. Dropped {dropped} frames")]
    DroppedFrames {
        expected: u32,
        actual: u32,
        dropped: u32,
    },
}

/// Enum representing PVCAM parameters.
/// Each variant corresponds to a specific PVCAM parameter ID and has an expected Rust type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PvcamParam {
    Exposure,          // Expected type: u16 (milliseconds)
    Gain,              // Expected type: u16
    Roi,               // Expected type: PxRegion (struct)
    SensorTemperature, // Expected type: i16 (degrees Celsius)
    PixelSize,         // Expected type: u16 (micrometers)
    ExposureMode,      // Expected type: u16 (trigger mode type)
    EdgeTrigger,       // Expected type: u16 (trigger edge - rising/falling)
}

impl PvcamParam {
    /// Returns the `TypeId` of the expected Rust type for this parameter.
    fn expected_type_id(&self) -> TypeId {
        match self {
            PvcamParam::Exposure => TypeId::of::<u16>(),
            PvcamParam::Gain => TypeId::of::<u16>(),
            PvcamParam::Roi => TypeId::of::<PxRegion>(),
            PvcamParam::SensorTemperature => TypeId::of::<i16>(),
            PvcamParam::PixelSize => TypeId::of::<u16>(),
            PvcamParam::ExposureMode => TypeId::of::<u16>(),
            PvcamParam::EdgeTrigger => TypeId::of::<u16>(),
        }
    }

    /// Returns a string representation of the expected type for this parameter.
    fn expected_type_name(&self) -> &'static str {
        match self {
            PvcamParam::Exposure => "u16",
            PvcamParam::Gain => "u16",
            PvcamParam::Roi => "PxRegion",
            PvcamParam::SensorTemperature => "i16",
            PvcamParam::PixelSize => "u16",
            PvcamParam::ExposureMode => "u16",
            PvcamParam::EdgeTrigger => "u16",
        }
    }
}

/// Placeholder for PxRegion struct (PVCAM ROI definition).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PxRegion {
    pub s1: u16,
    pub s2: u16,
    pub sbin: u16,
    pub p1: u16,
    pub p2: u16,
    pub pbin: u16,
}

/// Trigger mode for camera acquisition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TriggerMode {
    /// Free-running acquisition (default) - TIMED_MODE
    Timed = 0,
    /// External trigger for first frame only - TRIGGER_FIRST_MODE
    TriggerFirst = 1,
    /// External trigger per frame - STROBED_MODE
    Strobed = 2,
    /// Exposure controlled by trigger duration - BULB_MODE
    Bulb = 3,
    /// Software-triggered acquisition
    SoftwareEdge = 4,
}

impl TriggerMode {
    /// Convert from u16 value to TriggerMode
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(TriggerMode::Timed),
            1 => Some(TriggerMode::TriggerFirst),
            2 => Some(TriggerMode::Strobed),
            3 => Some(TriggerMode::Bulb),
            4 => Some(TriggerMode::SoftwareEdge),
            _ => None,
        }
    }

    /// Convert TriggerMode to u16 value
    pub fn as_u16(&self) -> u16 {
        *self as u16
    }

    /// Convert from string name to TriggerMode
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "timed" => Some(TriggerMode::Timed),
            "trigger_first" => Some(TriggerMode::TriggerFirst),
            "strobed" => Some(TriggerMode::Strobed),
            "bulb" => Some(TriggerMode::Bulb),
            "software_edge" => Some(TriggerMode::SoftwareEdge),
            _ => None,
        }
    }

    /// Convert TriggerMode to string name
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerMode::Timed => "timed",
            TriggerMode::TriggerFirst => "trigger_first",
            TriggerMode::Strobed => "strobed",
            TriggerMode::Bulb => "bulb",
            TriggerMode::SoftwareEdge => "software_edge",
        }
    }
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// RAII guard that stops camera acquisition when dropped.
///
/// This ensures that `stop_acquisition` is always called, even if the
/// acquisition task panics or is cancelled.
pub struct AcquisitionGuard {
    sdk: Arc<dyn PvcamSdk>,
    handle: CameraHandle,
}

impl Drop for AcquisitionGuard {
    fn drop(&mut self) {
        if let Err(e) = self.sdk.stop_acquisition(self.handle) {
            log::error!(
                "Failed to stop acquisition for handle {:?}: {}",
                self.handle,
                e
            );
        }
    }
}

/// Trait defining the PVCAM SDK abstraction.
///
/// Note: All methods take `&self` to be object-safe when used with `Arc<dyn PvcamSdk>`
/// in the `AcquisitionGuard`. Implementations must use internal mutability.
pub trait PvcamSdk: Send + Sync {
    /// Initializes the PVCAM SDK.
    fn init(&self) -> Result<(), PvcamError>;
    /// Uninitializes the PVCAM SDK.
    fn uninit(&self) -> Result<(), PvcamError>;
    /// Enumerates available cameras by name.
    fn enumerate_cameras(&self) -> Result<Vec<String>, PvcamError>;
    /// Opens a camera by its name, returning a `CameraHandle`.
    fn open_camera(&self, name: &str) -> Result<CameraHandle, PvcamError>;
    /// Closes an opened camera using its `CameraHandle`.
    fn close_camera(&self, handle: CameraHandle) -> Result<(), PvcamError>;

    /// Get u16 parameter (Exposure, Gain, PixelSize)
    fn get_param_u16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<u16, PvcamError>;
    /// Set u16 parameter (Exposure, Gain, PixelSize)
    fn set_param_u16(&self, handle: &CameraHandle, param: PvcamParam, value: u16)
        -> Result<(), PvcamError>;

    /// Get i16 parameter (SensorTemperature)
    fn get_param_i16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<i16, PvcamError>;
    /// Set i16 parameter (SensorTemperature)
    fn set_param_i16(&self, handle: &CameraHandle, param: PvcamParam, value: i16)
        -> Result<(), PvcamError>;

    /// Get PxRegion parameter (ROI)
    fn get_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
    ) -> Result<PxRegion, PvcamError>;
    /// Set PxRegion parameter (ROI)
    fn set_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        value: PxRegion,
    ) -> Result<(), PvcamError>;

    /// Starts a continuous acquisition stream for the given camera.
    ///
    /// Returns a tuple containing:
    /// 1. An `mpsc::Receiver<Frame>` to receive acquired frames.
    /// 2. An `AcquisitionGuard` that will automatically stop the acquisition when dropped.
    fn start_acquisition(
        self: Arc<Self>,
        handle: CameraHandle,
    ) -> Result<(mpsc::Receiver<Frame>, AcquisitionGuard), PvcamError>;

    /// Stops the acquisition for the given camera.
    ///
    /// This is typically called by the `AcquisitionGuard`'s `Drop` implementation.
    fn stop_acquisition(&self, handle: CameraHandle) -> Result<(), PvcamError>;
}

/// Send-safe wrapper for raw C void pointer
/// SAFETY: The pointer is only accessed from async tasks and properly managed
struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

struct RealPvcamSdkInner {
    is_initialized: bool,
    open_handles: HashMap<CameraHandle, String>,
    active_acquisitions: HashMap<CameraHandle, SendPtr>,
}

/// Real implementation of `PvcamSdk` using `pvcam-sys` FFI.
/// NOTE: This implementation is a placeholder until pvcam-sys has SDK feature enabled
pub struct RealPvcamSdk {
    inner: Mutex<RealPvcamSdkInner>,
}

impl Default for RealPvcamSdk {
    fn default() -> Self {
        RealPvcamSdk {
            inner: Mutex::new(RealPvcamSdkInner {
                is_initialized: false,
                open_handles: HashMap::new(),
                active_acquisitions: HashMap::new(),
            }),
        }
    }
}

impl RealPvcamSdk {
    /// Create a new RealPvcamSdk instance
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "pvcam-sdk")]
/// C callback function invoked by the PVCAM SDK for each new frame.
///
/// # Safety
///
/// - `frame_info_ptr` must be a valid pointer to a `FRAME_INFO` struct.
/// - `context` must be a valid `*mut c_void` pointing to a `Box<mpsc::Sender<Frame>>`.
/// - The caller (PVCAM SDK) is responsible for the validity of `frame_info_ptr`.
/// - The Rust code that sets up the callback is responsible for the validity of `context`.
unsafe extern "C" fn pvcam_frame_callback(
    frame_info_ptr: *mut pvcam_sys::FRAME_INFO,
    context: *mut c_void,
) {
    if context.is_null() {
        log::error!("PVCAM callback received a null context pointer.");
        return;
    }
    if frame_info_ptr.is_null() {
        log::error!("PVCAM callback received a null FRAME_INFO pointer.");
        return;
    }

    let frame_info = &*frame_info_ptr;
    let sender = &*(context as *const mpsc::Sender<Frame>);

    let pixel_count = frame_info.width as usize * frame_info.height as usize;
    let frame_data =
        std::slice::from_raw_parts(frame_info.buffer as *const u16, pixel_count).to_vec();

    // Extract metadata from FRAME_INFO
    // TODO: Replace with actual FRAME_INFO field access when pvcam-sdk is available
    let hardware_timestamp = None; // frame_info.timeStamp as i64 in microseconds
    let software_timestamp = chrono::Utc::now();
    let exposure_time_ms = 0.0; // Extract from frame_info.expTime or camera state
    let readout_time_ms = None; // Calculate from frame timing if available
    let sensor_temperature_c = None; // Query from camera params if available
    let roi = (0, 0, frame_info.width, frame_info.height); // Extract from frame_info ROI fields

    let frame = Frame {
        data: frame_data,
        frame_number: frame_info.frameNr,
        hardware_timestamp,
        software_timestamp,
        exposure_time_ms,
        readout_time_ms,
        sensor_temperature_c,
        roi,
    };

    // Use try_send to avoid blocking the SDK callback thread.
    if let Err(e) = sender.try_send(frame) {
        log::warn!("Failed to send frame from PVCAM callback: {}", e);
    }
}

impl PvcamSdk for RealPvcamSdk {
    fn init(&self) -> Result<(), PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.is_initialized {
            return Err(PvcamError::AlreadyInitialized);
        }
        // TODO: Call pvcam_sys::pl_pvcam_init() when pvcam-sdk feature is enabled
        #[cfg(feature = "pvcam-sdk")]
        {
            // use pvcam_sys::*;
            // let status = unsafe { pl_pvcam_init() };
            // if status != PV_OK { 
            //     return Err(PvcamError::InitFailed(
            //         format!("SDK initialization failed with code {}", status)
            //     )); 
            // }
        }
        inner.is_initialized = true;
        log::info!("PVCAM SDK initialized successfully");
        Ok(())
    }

    fn uninit(&self) -> Result<(), PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        // Best effort to close any remaining handles
        for handle in inner.open_handles.keys().copied().collect::<Vec<_>>() {
            if let Err(e) = self.close_camera(handle) {
                log::warn!("Failed to close camera {:?} during uninit: {}", handle, e);
            }
        }
        inner.open_handles.clear();

        #[cfg(feature = "pvcam-sdk")]
        {
            // use pvcam_sys::*;
            // let status = unsafe { pl_pvcam_uninit() };
            // if status != PV_OK { 
            //     return Err(PvcamError::InitFailed(
            //         format!("SDK uninitialization failed with code {}", status)
            //     )); 
            // }
        }
        inner.is_initialized = false;
        log::info!("PVCAM SDK uninitialized successfully");
        Ok(())
    }

    fn enumerate_cameras(&self) -> Result<Vec<String>, PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::debug!("Enumerating cameras (mock)");
        Ok(vec!["PrimeBSI".to_string()])
    }

    fn open_camera(&self, name: &str) -> Result<CameraHandle, PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        let handle = CameraHandle(1);
        inner.open_handles.insert(handle, name.to_string());
        log::info!("Opened camera '{}' with handle {:?}", name, handle);
        Ok(handle)
    }

    fn close_camera(&self, handle: CameraHandle) -> Result<(), PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        let camera_name = inner.open_handles.remove(&handle).ok_or_else(|| {
            PvcamError::CameraNotOpen { camera: handle }
        })?;
        // TODO: Implement with pvcam-sys when feature is enabled
        log::info!("Closed camera '{}' with handle {:?}", camera_name, handle);
        Ok(())
    }

    fn get_param_u16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<u16, PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("get_param_u16({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn set_param_u16(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        _value: u16,
    ) -> Result<(), PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("set_param_u16({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn get_param_i16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<i16, PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("get_param_i16({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn set_param_i16(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        _value: i16,
    ) -> Result<(), PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("set_param_i16({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn get_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
    ) -> Result<PxRegion, PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("get_param_region({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn set_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        _value: PxRegion,
    ) -> Result<(), PvcamError> {
        let inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(handle) {
            return Err(PvcamError::CameraNotOpen { camera: *handle });
        }
        // TODO: Implement with pvcam-sys when feature is enabled
        log::warn!("set_param_region({:?}) not implemented for real SDK", param);
        Err(PvcamError::ParamNotSupported(param))
    }

    fn start_acquisition(
        self: Arc<Self>,
        handle: CameraHandle,
    ) -> Result<(mpsc::Receiver<Frame>, AcquisitionGuard), PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }
        if !inner.open_handles.contains_key(&handle) {
            return Err(PvcamError::CameraNotOpen { camera: handle });
        }
        if inner.active_acquisitions.contains_key(&handle) {
            return Err(PvcamError::AcquisitionInProgress(handle));
        }

        let (tx, rx) = mpsc::channel(16); // Buffer size of 16 frames
        let tx_box = Box::new(tx);
        let context = Box::into_raw(tx_box) as *mut c_void;

        #[cfg(feature = "pvcam-sdk")]
        {
            // TODO: Replace with actual SDK calls
            // unsafe {
            //     pl_exp_setup_cont(...);
            //     pl_exp_register_callback_ex3(handle.0, pvcam_sys::PL_CALLBACK_EVENT_EOF, Some(pvcam_frame_callback), context);
            //     pl_exp_start_cont(...);
            // }
            log::info!("Real PVCAM acquisition started (simulation)");
        }

        inner.active_acquisitions.insert(handle, SendPtr(context));

        let guard = AcquisitionGuard {
            sdk: self.clone(),
            handle,
        };
        Ok((rx, guard))
    }

    fn stop_acquisition(&self, handle: CameraHandle) -> Result<(), PvcamError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.is_initialized {
            return Err(PvcamError::NotInitialized);
        }

        if let Some(SendPtr(context)) = inner.active_acquisitions.remove(&handle) {
            #[cfg(feature = "pvcam-sdk")]
            {
                // TODO: Replace with actual SDK calls
                // unsafe {
                //     pl_exp_stop_cont(handle.0, pvcam_sys::CCS_HALT);
                //     pl_exp_finish_seq(handle.0, std::ptr::null_mut(), 0);
                // }
                log::info!("Real PVCAM acquisition stopped (simulation)");
            }

            // Reconstitute the Box<Sender> from the raw pointer and drop it.
            // This closes the channel and deallocates the sender.
            let tx_box = unsafe { Box::from_raw(context as *mut mpsc::Sender<Frame>) };
            drop(tx_box);
        } else if !inner.open_handles.contains_key(&handle) {
            return Err(PvcamError::CameraNotOpen { camera: handle });
        }

        Ok(())
    }
}

/// Mock implementation of `PvcamSdk` for testing and simulation.
pub struct MockPvcamSdk {
    is_initialized: Arc<Mutex<bool>>,
    next_init_fails: Arc<Mutex<bool>>,
    next_open_fails_with_error: Arc<Mutex<Option<PvcamError>>>,
    next_handle_id: Arc<Mutex<i16>>,
    open_cameras: Arc<Mutex<HashMap<CameraHandle, MockCameraState>>>,
    available_cameras: Vec<String>,
    
    // Error injection and simulation
    simulate_dropped_frames: Arc<Mutex<bool>>,
    drop_frame_probability: Arc<Mutex<f64>>, // 0.0 to 1.0
}

struct MockCameraState {
    name: String,
    parameters: HashMap<PvcamParam, Box<dyn Any + Send + Sync>>,
    acquisition_task: Option<JoinHandle<()>>,
    stop_acq_tx: Option<oneshot::Sender<()>>,
}

impl MockPvcamSdk {
    pub fn new() -> Self {
        MockPvcamSdk {
            is_initialized: Arc::new(Mutex::new(false)),
            next_init_fails: Arc::new(Mutex::new(false)),
            next_open_fails_with_error: Arc::new(Mutex::new(None)),
            next_handle_id: Arc::new(Mutex::new(1)),
            open_cameras: Arc::new(Mutex::new(HashMap::new())),
            available_cameras: vec![
                "PrimeBSI".to_string(),
                "MockCamera1".to_string(),
                "MockCamera2".to_string(),
            ],
            simulate_dropped_frames: Arc::new(Mutex::new(false)),
            drop_frame_probability: Arc::new(Mutex::new(0.0)),
        }
    }

    /// Configures the next `init()` call to fail.
    pub fn set_next_init_fails(&self, fails: bool) {
        *self.next_init_fails.lock().unwrap() = fails;
    }

    /// Configures the next `open_camera()` call to fail with a specific error.
    pub fn set_next_open_fails_with_error(&self, error: Option<PvcamError>) {
        *self.next_open_fails_with_error.lock().unwrap() = error;
    }

    /// Sets the list of camera names returned by `enumerate_cameras`.
    pub fn set_available_cameras(&mut self, names: Vec<String>) {
        self.available_cameras = names;
    }
    
    /// Enable or disable dropped frame simulation
    pub fn set_simulate_dropped_frames(&self, enable: bool) {
        *self.simulate_dropped_frames.lock().unwrap() = enable;
    }
    
    /// Set the probability of dropping frames (0.0 to 1.0)
    /// Only effective when simulate_dropped_frames is enabled
    pub fn set_drop_frame_probability(&self, probability: f64) {
        let prob = probability.clamp(0.0, 1.0);
        *self.drop_frame_probability.lock().unwrap() = prob;
    }
}

impl Default for MockPvcamSdk {
    fn default() -> Self {
        Self::new()
    }
}

impl PvcamSdk for MockPvcamSdk {
    fn init(&self) -> Result<(), PvcamError> {
        let mut initialized = self.is_initialized.lock().unwrap();
        if *initialized {
            return Err(PvcamError::AlreadyInitialized);
        }
        let mut should_fail = self.next_init_fails.lock().unwrap();
        if *should_fail {
            *should_fail = false;
            return Err(PvcamError::MockError(
                "Mock init failed as configured".to_string(),
            ));
        }
        *initialized = true;
        Ok(())
    }

    fn uninit(&self) -> Result<(), PvcamError> {
        let mut initialized = self.is_initialized.lock().unwrap();
        if !*initialized {
            return Err(PvcamError::NotInitialized);
        }
        self.open_cameras.lock().unwrap().clear();
        *initialized = false;
        Ok(())
    }

    fn enumerate_cameras(&self) -> Result<Vec<String>, PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        Ok(self.available_cameras.clone())
    }

    fn open_camera(&self, name: &str) -> Result<CameraHandle, PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        let mut open_fail_config = self.next_open_fails_with_error.lock().unwrap();
        if let Some(error) = open_fail_config.take() {
            return Err(error);
        }

        if !self.available_cameras.contains(&name.to_string()) {
            return Err(PvcamError::CameraNotFound(name.to_string()));
        }

        let mut handle_id = self.next_handle_id.lock().unwrap();
        let handle = CameraHandle(*handle_id);
        *handle_id += 1;

        let mut params: HashMap<PvcamParam, Box<dyn Any + Send + Sync>> = HashMap::new();
        params.insert(PvcamParam::Exposure, Box::new(100u16));
        params.insert(PvcamParam::Gain, Box::new(1u16));
        params.insert(PvcamParam::SensorTemperature, Box::new(25i16));
        params.insert(PvcamParam::PixelSize, Box::new(10u16));
        params.insert(
            PvcamParam::Roi,
            Box::new(PxRegion {
                s1: 0,
                s2: 2047,
                sbin: 1,
                p1: 0,
                p2: 2047,
                pbin: 1,
            }),
        );
        // Initialize trigger mode parameters
        params.insert(PvcamParam::ExposureMode, Box::new(TriggerMode::Timed.as_u16()));
        params.insert(PvcamParam::EdgeTrigger, Box::new(0u16)); // 0 = rising edge

        let mut cameras = self.open_cameras.lock().unwrap();
        if cameras.values().any(|cam| cam.name == name) {
            return Err(PvcamError::MockError(format!(
                "Camera {} already open",
                name
            )));
        }

        cameras.insert(
            handle,
            MockCameraState {
                name: name.to_string(),
                parameters: params,
                acquisition_task: None,
                stop_acq_tx: None,
            },
        );
        Ok(handle)
    }

    fn close_camera(&self, handle: CameraHandle) -> Result<(), PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        let mut cameras = self.open_cameras.lock().unwrap();
        if let Some(mut camera_state) = cameras.remove(&handle) {
            if let Some(tx) = camera_state.stop_acq_tx.take() {
                let _ = tx.send(());
            }
        } else {
            return Err(PvcamError::CameraNotOpen { camera: handle });
        }
        Ok(())
    }

    fn get_param_u16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<u16, PvcamError> {
        self.get_param_internal(handle, param)
    }

    fn set_param_u16(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        value: u16,
    ) -> Result<(), PvcamError> {
        self.set_param_internal(handle, param, value)
    }

    fn get_param_i16(&self, handle: &CameraHandle, param: PvcamParam) -> Result<i16, PvcamError> {
        self.get_param_internal(handle, param)
    }

    fn set_param_i16(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        value: i16,
    ) -> Result<(), PvcamError> {
        self.set_param_internal(handle, param, value)
    }

    fn get_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
    ) -> Result<PxRegion, PvcamError> {
        self.get_param_internal(handle, param)
    }

    fn set_param_region(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        value: PxRegion,
    ) -> Result<(), PvcamError> {
        self.set_param_internal(handle, param, value)
    }

    fn start_acquisition(
        self: Arc<Self>,
        handle: CameraHandle,
    ) -> Result<(mpsc::Receiver<Frame>, AcquisitionGuard), PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }

        let mut cameras = self.open_cameras.lock().unwrap();
        let camera_state = cameras
            .get_mut(&handle)
            .ok_or(PvcamError::CameraNotOpen { camera: handle })?;

        if camera_state.acquisition_task.is_some() {
            return Err(PvcamError::AcquisitionInProgress(handle));
        }

        let exposure_ms = *camera_state
            .parameters
            .get(&PvcamParam::Exposure)
            .unwrap()
            .downcast_ref::<u16>()
            .unwrap() as u64;

        let roi = *camera_state
            .parameters
            .get(&PvcamParam::Roi)
            .unwrap()
            .downcast_ref::<PxRegion>()
            .unwrap();
        
        let width = (roi.s2 - roi.s1 + 1) / roi.sbin;
        let height = (roi.p2 - roi.p1 + 1) / roi.pbin;

        let (tx, rx) = mpsc::channel(16);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        
        // Clone error injection flags for async task
        let simulate_drops = self.simulate_dropped_frames.clone();
        let drop_probability = self.drop_frame_probability.clone();

        let task = tokio::spawn(async move {
            let mut frame_count = 0u32;
            let mut last_frame_time = std::time::Instant::now();
            let start_time = chrono::Utc::now();
            
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(exposure_ms)) => {
                        // Check if we should simulate a dropped frame
                        let should_drop = {
                            let drops_enabled = *simulate_drops.lock().unwrap();
                            let prob = *drop_probability.lock().unwrap();
                            drops_enabled && rand::random::<f64>() < prob
                        };
                        
                        if should_drop {
                            // Skip this frame (simulate drop) but still increment counter
                            frame_count += 1;
                            log::debug!("Mock: Simulating dropped frame {}", frame_count);
                            continue;
                        }
                        
                        let width_usize = width as usize;
                        let height_usize = height as usize;
                        let mut frame_data = vec![0u16; width_usize * height_usize];
                        let frame_offset = (frame_count % 256) as u32;
                        for y in 0..height {
                            for x in 0..width {
                                let pixel_val = ((x as u32 + y as u32 + frame_offset) % 256) as u16;
                                let value = pixel_val.saturating_mul(100);
                                let idx = (y as usize) * width_usize + (x as usize);
                                frame_data[idx] = value;
                            }
                        }

                        // Calculate realistic metadata
                        let now = std::time::Instant::now();
                        let _readout_duration = now.duration_since(last_frame_time);
                        last_frame_time = now;

                        // Simulate hardware timestamp (microseconds since start)
                        // Use saturating arithmetic to prevent overflow
                        let frame_offset_us = (frame_count as i64).saturating_mul(exposure_ms as i64).saturating_mul(1000);
                        let hardware_timestamp_us = start_time.timestamp_micros().saturating_add(frame_offset_us);
                        
                        // Simulate sensor temperature variation (-10°C to 5°C with drift)
                        let temp_base = -5.0;
                        let temp_variation = (frame_count as f64 * 0.01).sin() * 2.0;
                        let sensor_temp = temp_base + temp_variation;
                        
                        // Simulate readout time (5-10ms with variation)
                        let readout_ms = 7.5 + (frame_count as f64 * 0.1).sin() * 2.5;

                        let frame = Frame {
                            data: frame_data,
                            frame_number: frame_count,
                            hardware_timestamp: Some(hardware_timestamp_us),
                            software_timestamp: chrono::Utc::now(),
                            exposure_time_ms: exposure_ms as f64,
                            readout_time_ms: Some(readout_ms),
                            sensor_temperature_c: Some(sensor_temp),
                            roi: (roi.s1, roi.p1, width, height),
                        };

                        if tx.send(frame).await.is_err() {
                            log::info!("Mock acquisition channel closed, stopping.");
                            break;
                        }
                        frame_count += 1;
                    }
                    _ = &mut stop_rx => {
                        log::info!("Mock acquisition stopped via signal.");
                        break;
                    }
                }
            }
        });

        camera_state.acquisition_task = Some(task);
        camera_state.stop_acq_tx = Some(stop_tx);

        let guard = AcquisitionGuard {
            sdk: self.clone(),
            handle,
        };

        Ok((rx, guard))
    }

    fn stop_acquisition(&self, handle: CameraHandle) -> Result<(), PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        let mut cameras = self.open_cameras.lock().unwrap();
        if let Some(camera_state) = cameras.get_mut(&handle) {
            if let Some(tx) = camera_state.stop_acq_tx.take() {
                let _ = tx.send(());
                log::info!("Stopped mock acquisition for handle {:?}", handle);
            }
            // The task will be awaited and removed when the owning instrument is shut down.
            // Forcing an await here would block, which we want to avoid in a drop impl.
            camera_state.acquisition_task = None;
        }
        Ok(())
    }
}

impl MockPvcamSdk {
    /// Internal generic get parameter method (not part of trait)
    fn get_param_internal<T: 'static + Copy + Send + Sync>(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
    ) -> Result<T, PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        let cameras = self.open_cameras.lock().unwrap();
        let camera_state = cameras
            .get(handle)
            .ok_or(PvcamError::CameraNotOpen { camera: *handle })?;

        let expected_type_id = param.expected_type_id();
        if TypeId::of::<T>() != expected_type_id {
            return Err(PvcamError::TypeMismatch {
                param,
                expected: param.expected_type_name().to_string(),
                actual_type_id: TypeId::of::<T>(),
            });
        }

        let value_any = camera_state
            .parameters
            .get(&param)
            .ok_or(PvcamError::ParamNotSupported(param))?;

        value_any
            .downcast_ref::<T>()
            .copied()
            .ok_or_else(|| PvcamError::TypeMismatch {
                param,
                expected: param.expected_type_name().to_string(),
                actual_type_id: value_any.type_id(),
            })
    }

    /// Internal generic set parameter method (not part of trait)
    fn set_param_internal<T: 'static + Copy + Send + Sync>(
        &self,
        handle: &CameraHandle,
        param: PvcamParam,
        value: T,
    ) -> Result<(), PvcamError> {
        if !*self.is_initialized.lock().unwrap() {
            return Err(PvcamError::NotInitialized);
        }
        let mut cameras = self.open_cameras.lock().unwrap();
        let camera_state = cameras
            .get_mut(handle)
            .ok_or(PvcamError::CameraNotOpen { camera: *handle })?;

        let expected_type_id = param.expected_type_id();
        if TypeId::of::<T>() != expected_type_id {
            return Err(PvcamError::TypeMismatch {
                param,
                expected: param.expected_type_name().to_string(),
                actual_type_id: TypeId::of::<T>(),
            });
        }

        if camera_state.parameters.contains_key(&param) {
            camera_state.parameters.insert(param, Box::new(value));
            Ok(())
        } else {
            Err(PvcamError::ParamNotSupported(param))
        }
    }
}