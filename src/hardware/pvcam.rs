//! Photometrics PVCAM Camera Driver
//!
//! Reference: PVCAM SDK Documentation
//!
//! Protocol Overview:
//! - Uses PVCAM SDK C library via FFI
//! - Supports Prime BSI, Prime 95B, and other Photometrics cameras
//! - Circular buffer acquisition for high-speed imaging
//!
//! # Example Usage
//!
//! ```no_run
//! use rust_daq::hardware::pvcam::PvcamDriver;
//! use rust_daq::hardware::capabilities::{FrameProducer, ExposureControl};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let camera = PvcamDriver::new("PrimeBSI")?;
//!
//!     // Set exposure
//!     camera.set_exposure_ms(100.0).await?;
//!
//!     // Acquire frame
//!     let frame = camera.acquire_frame().await?;
//!     println!("Frame: {}x{} pixels", frame.width, frame.height);
//!
//!     Ok(())
//! }
//! ```

use crate::hardware::capabilities::{ExposureControl, FrameProducer, Parameterized, Triggerable};
use crate::hardware::{Frame, Roi};
use crate::observable::ParameterSet;
use crate::parameter::Parameter;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use tokio::task::JoinHandle;

// =============================================================================
// Data Structures for Camera Information
// =============================================================================

/// Comprehensive camera information
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Chip/sensor name (e.g., "sCMOS", "EMCCD")
    pub chip_name: String,
    /// Current sensor temperature in degrees Celsius
    pub temperature_c: f64,
    /// ADC bit depth (e.g., 12, 16)
    pub bit_depth: u16,
    /// Frame readout time in microseconds
    pub readout_time_us: f64,
    /// Pixel size in nanometers (width, height)
    pub pixel_size_nm: (u32, u32),
    /// Sensor dimensions in pixels (width, height)
    pub sensor_size: (u32, u32),
    /// Current gain mode name
    pub gain_name: String,
    /// Current speed mode name
    pub speed_name: String,
}

/// Gain mode information
#[derive(Debug, Clone)]
pub struct GainMode {
    /// Index for setting this gain mode
    pub index: u16,
    /// Human-readable name
    pub name: String,
}

/// Speed/readout mode information
#[derive(Debug, Clone)]
pub struct SpeedMode {
    /// Index for setting this speed mode
    pub index: u16,
    /// Human-readable name (e.g., "100 MHz", "200 MHz")
    pub name: String,
}

/// Fan speed setting
///
/// Maps to PVCAM's PL_FAN_SPEEDS enum values:
/// - FAN_SPEED_HIGH = 0 (default for most cameras)
/// - FAN_SPEED_MEDIUM = 1
/// - FAN_SPEED_LOW = 2
/// - FAN_SPEED_OFF = 3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanSpeed {
    /// Full fan speed (default for most cameras)
    High,
    /// Medium fan speed
    Medium,
    /// Low fan speed
    Low,
    /// Fan is turned off
    Off,
}

impl FanSpeed {
    /// Convert from PVCAM enum value (PL_FAN_SPEEDS)
    #[cfg(feature = "pvcam_hardware")]
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => FanSpeed::High,
            1 => FanSpeed::Medium,
            2 => FanSpeed::Low,
            3 => FanSpeed::Off,
            _ => FanSpeed::High, // Default to high for unknown values
        }
    }

    /// Convert to PVCAM enum value (PL_FAN_SPEEDS)
    #[cfg(feature = "pvcam_hardware")]
    pub fn to_pvcam(self) -> i32 {
        match self {
            FanSpeed::High => 0,
            FanSpeed::Medium => 1,
            FanSpeed::Low => 2,
            FanSpeed::Off => 3,
        }
    }
}

/// Post-processing feature information
#[derive(Debug, Clone)]
pub struct PPFeature {
    /// Feature index
    pub index: u16,
    /// Feature ID (for setting parameters)
    pub id: u16,
    /// Human-readable feature name
    pub name: String,
}

/// Post-processing parameter information
#[derive(Debug, Clone)]
pub struct PPParam {
    /// Parameter index within feature
    pub index: u16,
    /// Parameter ID (for get/set)
    pub id: u16,
    /// Human-readable parameter name
    pub name: String,
    /// Current value
    pub value: u32,
}

/// Centroids detection mode
///
/// Maps to PVCAM's PL_CENTROIDS_MODES enum:
/// - PL_CENTROIDS_MODE_LOCATE = 0 (PrimeLocate)
/// - PL_CENTROIDS_MODE_TRACK = 1 (Particle Tracking)
/// - PL_CENTROIDS_MODE_BLOB = 2 (Blob Detection)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CentroidsMode {
    /// Locate mode (PrimeLocate) - find particle positions
    Locate,
    /// Particle Tracking mode - track particles across frames
    Track,
    /// Blob Detection mode - detect larger objects
    Blob,
}

impl CentroidsMode {
    /// Convert from PVCAM enum value
    #[cfg(feature = "pvcam_hardware")]
    pub fn from_pvcam(value: i32) -> Self {
        match value {
            0 => CentroidsMode::Locate,
            1 => CentroidsMode::Track,
            2 => CentroidsMode::Blob,
            _ => CentroidsMode::Locate, // Default
        }
    }

    /// Convert to PVCAM enum value
    #[cfg(feature = "pvcam_hardware")]
    pub fn to_pvcam(self) -> i32 {
        match self {
            CentroidsMode::Locate => 0,
            CentroidsMode::Track => 1,
            CentroidsMode::Blob => 2,
        }
    }
}

/// Centroids configuration and status
#[derive(Debug, Clone)]
pub struct CentroidsConfig {
    /// Detection mode
    pub mode: CentroidsMode,
    /// Search radius in pixels
    pub radius: u16,
    /// Maximum number of particles to detect
    pub max_count: u16,
    /// Detection threshold
    pub threshold: u32,
}

/// Driver for Photometrics PVCAM cameras
///
/// Implements FrameProducer, ExposureControl, and Triggerable capability traits.
/// Uses PVCAM SDK for hardware communication when `pvcam_hardware` feature is enabled.
pub struct PvcamDriver {
    /// Camera name (e.g., "PrimeBSI", "Prime95B")
    camera_name: String,
    /// Camera handle from PVCAM SDK (only with hardware feature)
    #[cfg(feature = "pvcam_hardware")]
    camera_handle: Arc<Mutex<Option<i16>>>,
    /// Current exposure time in milliseconds
    exposure_ms: Parameter<f64>,
    /// Current ROI setting
    roi: Arc<Mutex<Roi>>,
    /// Binning factors (x, y)
    binning: Arc<Mutex<(u16, u16)>>,
    /// Frame buffer (for mock mode or temporary storage)
    frame_buffer: Arc<Mutex<Vec<u16>>>,
    /// Sensor dimensions
    sensor_width: u32,
    sensor_height: u32,
    /// Whether the camera is armed for triggering
    armed: Arc<Mutex<bool>>,
    /// Whether PVCAM SDK is initialized
    #[cfg(feature = "pvcam_hardware")]
    sdk_initialized: Arc<Mutex<bool>>,
    /// Whether continuous streaming is active
    streaming: Arc<AtomicBool>,
    /// Frame counter for streaming
    frame_count: Arc<AtomicU64>,
    /// Broadcast sender for streaming frames (supports multiple subscribers)
    frame_tx: tokio::sync::broadcast::Sender<std::sync::Arc<Frame>>,
    /// Handle to the streaming poll task
    #[cfg(feature = "pvcam_hardware")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Circular buffer for continuous acquisition (hardware only)
    #[cfg(feature = "pvcam_hardware")]
    circ_buffer: Arc<Mutex<Option<Vec<u16>>>>,
    /// Trigger frame buffer - holds the frame during triggered acquisition
    #[cfg(feature = "pvcam_hardware")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
    /// Parameter registry for observable parameters
    params: ParameterSet,
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Get the last PVCAM error as a formatted string
#[cfg(feature = "pvcam_hardware")]
fn get_pvcam_error() -> String {
    // SAFETY: pl_error_code and pl_error_message are called to retrieve error information.
    // err_msg buffer is properly allocated (256 bytes) and pl_error_message writes a
    // null-terminated string. CStr::from_ptr is safe because PVCAM guarantees null-termination.
    unsafe {
        let err_code = pl_error_code();
        let mut err_msg = vec![0i8; 256];
        pl_error_message(err_code, err_msg.as_mut_ptr());
        let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
        format!("error {} - {}", err_code, err_str)
    }
}

impl PvcamDriver {
    /// Create a new PVCAM driver instance (async-safe version)
    ///
    /// # Arguments
    /// * `camera_name` - Name of camera (e.g., "PrimeBSI", "PMCam")
    ///
    /// # Errors
    /// Returns error if camera cannot be opened
    ///
    /// # Hardware Feature
    /// With `pvcam_hardware` feature enabled, this will:
    /// - Call pl_pvcam_init() to initialize PVCAM SDK (in spawn_blocking)
    /// - Call pl_cam_open() to open the camera (in spawn_blocking)
    /// - Query actual sensor size from hardware
    ///
    /// Without feature, uses mock data with known dimensions.
    ///
    /// # Performance
    /// This method runs blocking C-API calls in `tokio::task::spawn_blocking()` to avoid
    /// blocking the async runtime. This is critical during application startup when
    /// multiple drivers may be initializing concurrently.
    pub async fn new_async(camera_name: String) -> Result<Self> {
        #[cfg(feature = "pvcam_hardware")]
        {
            Self::new_with_hardware_async(camera_name).await
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            Self::new_mock(&camera_name)
        }
    }

    /// Create a new PVCAM driver instance (synchronous, deprecated)
    ///
    /// # Arguments
    /// * `camera_name` - Name of camera (e.g., "PrimeBSI", "PMCam")
    ///
    /// # Errors
    /// Returns error if camera cannot be opened
    ///
    /// # Hardware Feature
    /// With `pvcam_hardware` feature enabled, this will:
    /// - Call pl_pvcam_init() to initialize PVCAM SDK
    /// - Call pl_cam_open() to open the camera
    /// - Query actual sensor size from hardware
    ///
    /// Without feature, uses mock data with known dimensions.
    #[deprecated(
        since = "0.5.0",
        note = "Use new_async() instead to avoid blocking the async runtime during PVCAM initialization"
    )]
    pub fn new(camera_name: &str) -> Result<Self> {
        #[cfg(feature = "pvcam_hardware")]
        {
            Self::new_with_hardware(camera_name)
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            Self::new_mock(camera_name)
        }
    }

    #[cfg(feature = "pvcam_hardware")]
    async fn new_with_hardware_async(camera_name: String) -> Result<Self> {
        // Run all blocking PVCAM C-API calls in spawn_blocking to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || Self::new_with_hardware(&camera_name))
            .await
            .context("PVCAM initialization task panicked")?
    }

    #[cfg(feature = "pvcam_hardware")]
    fn new_with_hardware(camera_name: &str) -> Result<Self> {
        // SAFETY: pl_pvcam_init initializes the PVCAM library. Safe to call before any other
        // PVCAM operations. Returns 0 on failure, non-zero on success per PVCAM spec.
        // Error handling uses properly allocated buffers for error messages.
        unsafe {
            if pl_pvcam_init() == 0 {
                let err_code = pl_error_code();
                let mut err_msg = vec![0i8; 256];
                pl_error_message(err_code, err_msg.as_mut_ptr());
                let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
                return Err(anyhow!(
                    "Failed to initialize PVCAM SDK: error {} - {}",
                    err_code,
                    err_str
                ));
            }
        }

        // Get list of cameras
        let mut total_cameras: i16 = 0;
        // SAFETY: pl_cam_get_total retrieves the number of available cameras. The total_cameras
        // pointer is valid and properly aligned. PVCAM SDK is initialized at this point.
        unsafe {
            if pl_cam_get_total(&mut total_cameras) == 0 {
                let err_code = pl_error_code();
                let mut err_msg = vec![0i8; 256];
                pl_error_message(err_code, err_msg.as_mut_ptr());
                let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
                pl_pvcam_uninit();
                return Err(anyhow!(
                    "Failed to get camera count: error {} - {}",
                    err_code,
                    err_str
                ));
            }
        }

        if total_cameras == 0 {
            // SAFETY: pl_pvcam_uninit safely cleans up PVCAM SDK resources initialized above.
            // Safe to call after successful pl_pvcam_init.
            unsafe {
                pl_pvcam_uninit();
            }
            return Err(anyhow!(
                "No PVCAM cameras detected (is pvcam_usb daemon running?)"
            ));
        }

        // Find camera by name or use first camera
        let mut camera_handle: i16 = 0;
        let camera_name_cstr =
            std::ffi::CString::new(camera_name).context("Invalid camera name")?;

        // SAFETY: pl_cam_open opens a camera by name. The camera_name_cstr pointer is valid
        // and null-terminated (guaranteed by CString). camera_handle pointer is valid.
        // pl_cam_get_name retrieves camera name into a properly sized buffer (256 bytes).
        // pl_pvcam_uninit is safe to call on error paths.
        // SAFETY: Fallback path when named camera fails: pl_cam_get_name retrieves the first
        // available camera name into name_buffer (256 bytes). If successful, pl_cam_open opens
        // that camera. pl_pvcam_uninit cleanup is safe on all error paths.
        unsafe {
            if pl_cam_open(camera_name_cstr.as_ptr() as *mut i8, &mut camera_handle, 0) == 0 {
                // If named camera not found, try first camera
                let mut name_buffer = vec![0i8; 256];
                if pl_cam_get_name(0, name_buffer.as_mut_ptr()) != 0 {
                    if pl_cam_open(name_buffer.as_mut_ptr(), &mut camera_handle, 0) == 0 {
                        pl_pvcam_uninit();
                        return Err(anyhow!("Failed to open any camera"));
                    }
                } else {
                    pl_pvcam_uninit();
                    return Err(anyhow!("Failed to open camera: {}", camera_name));
                }
            }
        }

        // Query sensor size
        let mut width: uns32 = 0;
        let mut height: uns32 = 0;

        // SAFETY: pl_get_param retrieves camera parameters. camera_handle is valid from pl_cam_open.
        // The parameter pointers (par_width, par_height) are properly aligned and valid for writes.
        // PARAM_SER_SIZE and PARAM_PAR_SIZE are valid PVCAM parameter IDs.
        unsafe {
            let mut par_width: uns16 = 0;
            let mut par_height: uns16 = 0;

            // Get sensor dimensions via PARAM_SER_SIZE
            if pl_get_param(
                camera_handle,
                PARAM_SER_SIZE,
                ATTR_CURRENT,
                &mut par_width as *mut _ as *mut _,
            ) != 0
            {
                width = par_width as uns32;
            }
            if pl_get_param(
                camera_handle,
                PARAM_PAR_SIZE,
                ATTR_CURRENT,
                &mut par_height as *mut _ as *mut _,
            ) != 0
            {
                height = par_height as uns32;
            }
        }

        if width == 0 || height == 0 {
            // Fallback to known dimensions
            (width, height) = match camera_name {
                "PrimeBSI" => (2048, 2048),
                "Prime95B" => (1200, 1200),
                _ => (2048, 2048),
            };
        }

        // Create broadcast channel for streaming frames (supports multiple subscribers)
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);

        // Create exposure parameter with validation and metadata
        let mut params = ParameterSet::new();
        let exposure_ms = Parameter::new("exposure_ms", 100.0)
            .with_description("Camera exposure time")
            .with_unit("ms")
            .with_range(0.1, 60000.0); // 0.1ms to 60s range

        params.register(exposure_ms.inner().clone());

        Ok(Self {
            camera_name: camera_name.to_string(),
            camera_handle: Arc::new(Mutex::new(Some(camera_handle))),
            exposure_ms,
            roi: Arc::new(Mutex::new(Roi {
                x: 0,
                y: 0,
                width,
                height,
            })),
            binning: Arc::new(Mutex::new((1, 1))),
            frame_buffer: Arc::new(Mutex::new(vec![0u16; (width * height) as usize])),
            sensor_width: width,
            sensor_height: height,
            armed: Arc::new(Mutex::new(false)),
            sdk_initialized: Arc::new(Mutex::new(true)),
            streaming: Arc::new(AtomicBool::new(false)),
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            poll_handle: Arc::new(Mutex::new(None)),
            circ_buffer: Arc::new(Mutex::new(None)),
            trigger_frame: Arc::new(Mutex::new(None)),
            params,
        })
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    fn new_mock(camera_name: &str) -> Result<Self> {
        // Mock implementation with known camera dimensions
        let (width, height) = match camera_name {
            "PrimeBSI" => (2048, 2048),
            "Prime95B" => (1200, 1200),
            _ => (2048, 2048), // Default
        };

        eprintln!("PVCAM hardware feature not enabled - using mock camera");
        eprintln!("    To use real hardware: cargo build --features pvcam_hardware");

        // Create broadcast channel for streaming frames (supports multiple subscribers)
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);

        // Create exposure parameter with validation and metadata
        let mut params = ParameterSet::new();
        let exposure_ms = Parameter::new("exposure_ms", 100.0)
            .with_description("Camera exposure time")
            .with_unit("ms")
            .with_range(0.1, 60000.0); // 0.1ms to 60s range

        params.register(exposure_ms.inner().clone());

        Ok(Self {
            camera_name: camera_name.to_string(),
            exposure_ms,
            roi: Arc::new(Mutex::new(Roi {
                x: 0,
                y: 0,
                width,
                height,
            })),
            binning: Arc::new(Mutex::new((1, 1))),
            frame_buffer: Arc::new(Mutex::new(vec![0u16; (width * height) as usize])),
            sensor_width: width,
            sensor_height: height,
            armed: Arc::new(Mutex::new(false)),
            streaming: Arc::new(AtomicBool::new(false)),
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            params,
        })
    }

    /// Set binning factors
    ///
    /// # Arguments
    /// * `x_bin` - Horizontal binning (1, 2, 4, 8)
    /// * `y_bin` - Vertical binning (1, 2, 4, 8)
    pub async fn set_binning(&self, x_bin: u16, y_bin: u16) -> Result<()> {
        if ![1, 2, 4, 8].contains(&x_bin) || ![1, 2, 4, 8].contains(&y_bin) {
            return Err(anyhow!("Binning must be 1, 2, 4, or 8"));
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            // Note: PVCAM binning is set via the rgn_type structure during acquisition,
            // not as camera parameters. The binning values are stored and used when
            // calling pl_exp_setup_seq. See acquire_frame_hardware() for implementation.
        }

        *self.binning.lock().await = (x_bin, y_bin);
        Ok(())
    }

    /// Get current binning
    pub async fn binning(&self) -> (u16, u16) {
        *self.binning.lock().await
    }

    /// Set Region of Interest
    pub async fn set_roi(&self, roi: Roi) -> Result<()> {
        if !roi.is_valid_for(self.sensor_width, self.sensor_height) {
            return Err(anyhow!("ROI exceeds sensor dimensions"));
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            let _handle = *self.camera_handle.lock().await;
            // Note: ROI in PVCAM is set during pl_exp_setup_seq, not via parameters
            // Store for use during acquisition setup
        }

        *self.roi.lock().await = roi;
        Ok(())
    }

    /// Get current ROI
    pub async fn roi(&self) -> Roi {
        *self.roi.lock().await
    }

    /// Acquire a single frame (internal implementation)
    ///
    /// With hardware: Uses pl_exp_setup_seq/start_seq/get_latest_frame
    /// Without hardware: Generates synthetic test pattern
    async fn acquire_frame_internal(&self) -> Result<Vec<u16>> {
        #[cfg(feature = "pvcam_hardware")]
        {
            self.acquire_frame_hardware().await
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            self.acquire_frame_mock().await
        }
    }

    #[cfg(feature = "pvcam_hardware")]
    async fn acquire_frame_hardware(&self) -> Result<Vec<u16>> {
        let h = (*self.camera_handle.lock().await).ok_or_else(|| anyhow!("Camera not opened"))?;

        let exposure = self.exposure_ms.get();
        let roi = *self.roi.lock().await;
        let (x_bin, y_bin) = *self.binning.lock().await;

        // Setup region for acquisition
        // SAFETY: zeroed() creates a zero-initialized rgn_type struct. This is safe because
        // rgn_type is a C struct with no drop semantics. All fields are then properly initialized
        // with valid ROI and binning values before use in pl_exp_setup_seq.
        let region = unsafe {
            let mut rgn: rgn_type = std::mem::zeroed();
            rgn.s1 = roi.x as uns16;
            rgn.s2 = (roi.x + roi.width - 1) as uns16;
            rgn.sbin = x_bin;
            rgn.p1 = roi.y as uns16;
            rgn.p2 = (roi.y + roi.height - 1) as uns16;
            rgn.pbin = y_bin;
            rgn
        };

        // Calculate frame size (region dimensions are in unbinned coordinates,
        // but the actual frame will have binned dimensions)
        let binned_width = roi.width / x_bin as u32;
        let binned_height = roi.height / y_bin as u32;
        let frame_size: uns32 = (binned_width * binned_height) as uns32;
        let mut frame = vec![0u16; frame_size as usize];

        // SAFETY: All PVCAM acquisition functions receive valid handles and pointers.
        // camera_handle h is valid from pl_cam_open. frame buffer is properly allocated
        // with correct size. region pointer is valid and points to initialized rgn_type.
        // pl_exp_check_status receives valid mutable pointers for status output.
        // pl_exp_finish_seq is called to cleanup resources on all exit paths.
        // SAFETY: Acquisition sequence functions operate on valid camera handle h. frame buffer
        // is properly allocated with correct size. region pointer is valid. All status pointers
        // are valid mutable references. pl_exp_finish_seq cleanup is safe on all paths.
        unsafe {
            // Setup exposure sequence
            // PVCAM expects exposure time in milliseconds for TIMED_MODE
            let exp_time_ms = exposure as uns32;
            let mut total_bytes: uns32 = 0;

            if pl_exp_setup_seq(
                h,
                1,
                1,
                &region as *const _ as *const _,
                TIMED_MODE,
                exp_time_ms,
                &mut total_bytes,
            ) == 0
            {
                return Err(anyhow!("Failed to setup acquisition sequence"));
            }

            // Start acquisition
            if pl_exp_start_seq(h, frame.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to start acquisition"));
            }

            // Wait for completion
            let mut status: i16 = 0;
            let mut bytes_arrived: uns32 = 0;

            let timeout = Duration::from_millis((exposure + 1000.0) as u64);
            let start = std::time::Instant::now();

            loop {
                // SAFETY: h is a valid camera handle. pl_exp_check_status receives valid mutable
                // pointers for status and bytes_arrived output parameters.
                unsafe {
                    if pl_exp_check_status(h, &mut status, &mut bytes_arrived) == 0 {
                        pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                        return Err(anyhow!("Failed to check acquisition status"));
                    }
                }

                if status == READOUT_COMPLETE || status == READOUT_FAILED {
                    break;
                }

                if start.elapsed() > timeout {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                    return Err(anyhow!("Acquisition timeout"));
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
                tokio::task::yield_now().await;
            }

            if status == READOUT_FAILED {
                pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                return Err(anyhow!("Acquisition failed"));
            }

            // Finish sequence
            if pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0) == 0 {
                return Err(anyhow!("Failed to finish acquisition sequence"));
            }
        }

        Ok(frame)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    async fn acquire_frame_mock(&self) -> Result<Vec<u16>> {
        let exposure = self.exposure_ms.get();
        let roi = *self.roi.lock().await;

        // Simulate exposure delay
        tokio::time::sleep(Duration::from_millis(exposure as u64)).await;

        // Generate synthetic frame data (for testing without real camera)
        let frame_size = (roi.width * roi.height) as usize;
        let mut frame = vec![0u16; frame_size];

        // Create test pattern (gradient)
        for y in 0..roi.height {
            for x in 0..roi.width {
                let value = ((x + y) % 4096) as u16;
                frame[(y * roi.width + x) as usize] = value;
            }
        }

        // Store in frame buffer
        *self.frame_buffer.lock().await = frame.clone();

        Ok(frame)
    }

    /// Acquire a single frame from the camera
    ///
    /// This is the public API for frame acquisition. Returns a Frame struct
    /// containing the pixel data and dimensions.
    ///
    /// # Example
    /// ```no_run
    /// use rust_daq::hardware::pvcam::PvcamDriver;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let camera = PvcamDriver::new("PrimeBSI")?;
    /// camera.set_exposure_ms(100.0).await?;
    ///
    /// let frame = camera.acquire_frame().await?;
    /// println!("Acquired {}x{} frame", frame.width, frame.height);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn acquire_frame(&self) -> Result<Frame> {
        let buffer = self.acquire_frame_internal().await?;
        let roi = self.roi().await;
        let (x_bin, y_bin) = *self.binning.lock().await;

        // Calculate actual frame dimensions (binned)
        // ROI coordinates are in unbinned pixels, but the frame size is binned
        let frame_width = roi.width / x_bin as u32;
        let frame_height = roi.height / y_bin as u32;

        Ok(Frame::new(frame_width, frame_height, buffer))
    }

    /// Set exposure time in milliseconds (convenience method)
    ///
    /// This is a helper that wraps the ExposureControl trait method
    /// and works in milliseconds instead of seconds.
    pub async fn set_exposure_ms(&self, exposure_ms: f64) -> Result<()> {
        self.set_exposure(exposure_ms / 1000.0).await
    }

    /// Get exposure time in milliseconds (convenience method)
    pub async fn get_exposure_ms(&self) -> Result<f64> {
        Ok(self.get_exposure().await? * 1000.0)
    }

    /// Disarm the camera after triggering
    ///
    /// # Hardware Implementation
    /// Stops any ongoing acquisition and cleans up resources.
    ///
    /// # Mock Implementation
    /// Simply marks the camera as unarmed.
    pub async fn disarm(&self) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        {
            let handle = *self.camera_handle.lock().await;
            if let Some(h) = handle {
                // SAFETY: h is a valid camera handle from pl_cam_open. pl_exp_stop_cont safely
                // halts continuous acquisition. trigger_frame buffer is owned and valid if present.
                // pl_exp_finish_seq cleans up the acquisition sequence resources.
                unsafe {
                    // Stop any ongoing continuous acquisition
                    pl_exp_stop_cont(h, CCS_HALT);

                    // Finish any pending triggered acquisition and cleanup buffer
                    if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                        pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                    }
                }
            }
        }

        *self.armed.lock().await = false;
        Ok(())
    }

    /// Wait for external trigger (for testing triggered mode)
    ///
    /// # Hardware Implementation
    /// Checks the camera status repeatedly until a trigger is received or timeout occurs.
    /// This is a polling-based approach that works with the current single-frame
    /// acquisition model.
    ///
    /// Note: Full external hardware trigger support (e.g., TTL input) requires
    /// trigger mode constants (TRIGGER_FIRST_MODE) that are not yet exposed in the
    /// PVCAM bindings. This method provides status-based trigger detection as a workaround.
    ///
    /// # Mock Implementation
    /// Simulates a brief wait period for testing without hardware.
    ///
    /// # Errors
    /// Returns error if:
    /// - Camera is not opened
    /// - Camera is not armed
    /// - Acquisition fails
    /// - Timeout (30 seconds) is exceeded
    pub async fn wait_for_trigger(&self) -> Result<()> {
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Simulate trigger wait
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            let h =
                (*self.camera_handle.lock().await).ok_or_else(|| anyhow!("Camera not opened"))?;

            let is_armed = *self.armed.lock().await;
            if !is_armed {
                return Err(anyhow!("Camera must be armed before waiting for trigger"));
            }

            // Wait for trigger/frame with timeout
            let timeout = Duration::from_secs(30);
            let start = std::time::Instant::now();
            let mut status: i16 = 0;
            let mut bytes_arrived: uns32 = 0;

            let result = loop {
                // SAFETY: h is a valid camera handle. pl_exp_check_status receives valid mutable
                // pointers for status and bytes_arrived output parameters.
                unsafe {
                    if pl_exp_check_status(h, &mut status, &mut bytes_arrived) == 0 {
                        break Err(anyhow!("Failed to check acquisition status"));
                    }
                }

                // Frame ready or readout complete
                if status == READOUT_COMPLETE || bytes_arrived > 0 {
                    break Ok(());
                }

                if status == READOUT_FAILED {
                    break Err(anyhow!("Acquisition failed"));
                }

                if start.elapsed() > timeout {
                    break Err(anyhow!("Trigger wait timeout after 30 seconds"));
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
                tokio::task::yield_now().await;
            };

            // Always finish the sequence and cleanup on exit
            // SAFETY: h is valid camera handle. trigger_frame buffer is owned by this struct
            // and valid if present. pl_exp_finish_seq safely cleans up acquisition resources.
            unsafe {
                if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                }
            }

            // Clear armed flag and increment frame count on success
            if result.is_ok() {
                self.frame_count.fetch_add(1, Ordering::SeqCst);
            }
            *self.armed.lock().await = false;

            return result;
        }

        Ok(())
    }

    /// Subscribe to the frame broadcast stream
    ///
    /// Returns a new receiver that will receive all frames broadcast after subscription.
    /// Multiple subscribers can receive the same frames simultaneously without cloning.
    pub fn subscribe_frames(&self) -> tokio::sync::broadcast::Receiver<std::sync::Arc<Frame>> {
        self.frame_tx.subscribe()
    }

    /// Get the frame receiver for consuming streamed frames
    ///
    /// DEPRECATED: Use `subscribe_frames()` instead, which supports multiple subscribers.
    #[deprecated(
        since = "0.2.0",
        note = "Use subscribe_frames() for multi-subscriber support"
    )]
    pub async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<Frame>> {
        // No longer supported - use subscribe_frames() instead
        None
    }

    /// Check if streaming is active
    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::SeqCst)
    }

    /// Get current frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::SeqCst)
    }

    // =========================================================================
    // Camera Information Query Methods
    // =========================================================================

    /// Get current sensor temperature in degrees Celsius
    ///
    /// Returns the measured sensor temperature. PVCAM reports this as
    /// hundredths of degrees Celsius internally.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_temperature(&self) -> Result<f64> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut temp_raw: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_TEMP retrieves temperature.
        // temp_raw pointer is valid and properly aligned for i16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_TEMP,
                ATTR_CURRENT,
                &mut temp_raw as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get temperature: {}", get_pvcam_error()));
            }
        }
        // PVCAM returns temperature in hundredths of degrees C
        Ok(temp_raw as f64 / 100.0)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_temperature(&self) -> Result<f64> {
        Ok(-40.0) // Mock: typical cooled sensor temperature
    }

    /// Get the camera chip/sensor name
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_chip_name(&self) -> Result<String> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut buf = vec![0i8; 256];
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_CHIP_NAME retrieves
        // the chip name string. buf is properly allocated (256 bytes) and valid for writes.
        // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
        unsafe {
            if pl_get_param(h, PARAM_CHIP_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get chip name: {}", get_pvcam_error()));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned();
            Ok(name)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_chip_name(&self) -> Result<String> {
        Ok("MockSensor".to_string())
    }

    /// Get the ADC bit depth (e.g., 12, 16)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_bit_depth(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut depth: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_BIT_DEPTH retrieves
        // the ADC bit depth. depth pointer is valid and properly aligned for i16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_BIT_DEPTH,
                ATTR_CURRENT,
                &mut depth as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get bit depth: {}", get_pvcam_error()));
            }
        }
        Ok(depth as u16)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_bit_depth(&self) -> Result<u16> {
        Ok(16) // Mock: typical 16-bit depth
    }

    /// Get frame readout time in microseconds
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_readout_time_us(&self) -> Result<f64> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut time_us: f64 = 0.0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_READOUT_TIME retrieves
        // the frame readout time. time_us pointer is valid and properly aligned for f64 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_READOUT_TIME,
                ATTR_CURRENT,
                &mut time_us as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get readout time: {}", get_pvcam_error()));
            }
        }
        Ok(time_us)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_readout_time_us(&self) -> Result<f64> {
        Ok(10000.0) // Mock: 10ms readout
    }

    /// Get pixel size in nanometers (width, height)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_pixel_size_nm(&self) -> Result<(u32, u32)> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut pix_ser: uns16 = 0;
        let mut pix_par: uns16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_PIX_SER_SIZE and
        // PARAM_PIX_PAR_SIZE retrieves pixel dimensions. Both pointers are valid and properly
        // aligned for uns16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_PIX_SER_SIZE,
                ATTR_CURRENT,
                &mut pix_ser as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get serial pixel size: {}",
                    get_pvcam_error()
                ));
            }
            if pl_get_param(
                h,
                PARAM_PIX_PAR_SIZE,
                ATTR_CURRENT,
                &mut pix_par as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get parallel pixel size: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok((pix_ser as u32, pix_par as u32))
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_pixel_size_nm(&self) -> Result<(u32, u32)> {
        Ok((6500, 6500)) // Mock: 6.5um pixels
    }

    /// Get the current gain mode name
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_gain_name(&self) -> Result<String> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut buf = vec![0i8; 256];
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_GAIN_NAME retrieves
        // the chip name string. buf is properly allocated (256 bytes) and valid for writes.
        // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
        unsafe {
            if pl_get_param(h, PARAM_GAIN_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get gain name: {}", get_pvcam_error()));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned();
            Ok(name)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_gain_name(&self) -> Result<String> {
        Ok("HDR".to_string())
    }

    /// Get the current speed table name
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_speed_name(&self) -> Result<String> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut buf = vec![0i8; 256];
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_SPDTAB_NAME retrieves
        // the chip name string. buf is properly allocated (256 bytes) and valid for writes.
        // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
        unsafe {
            if pl_get_param(
                h,
                PARAM_SPDTAB_NAME,
                ATTR_CURRENT,
                buf.as_mut_ptr() as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get speed table name: {}",
                    get_pvcam_error()
                ));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned();
            Ok(name)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_speed_name(&self) -> Result<String> {
        Ok("100 MHz".to_string())
    }

    /// Get the current gain index
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_gain_index(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut idx: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_GAIN_INDEX retrieves
        // the gain index. idx pointer is valid and properly aligned for i16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_GAIN_INDEX,
                ATTR_CURRENT,
                &mut idx as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get gain index: {}", get_pvcam_error()));
            }
        }
        Ok(idx as u16)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_gain_index(&self) -> Result<u16> {
        Ok(0)
    }

    /// Get the current speed table index
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_speed_index(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut idx: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_SPDTAB_INDEX retrieves
        // the speed table index. idx pointer is valid and properly aligned for i16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_SPDTAB_INDEX,
                ATTR_CURRENT,
                &mut idx as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get speed table index: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(idx as u16)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_speed_index(&self) -> Result<u16> {
        Ok(0)
    }

    /// Get comprehensive camera information
    ///
    /// Returns a struct with all available camera status information including
    /// sensor name, temperature, bit depth, readout time, pixel size, and
    /// current gain/speed mode names.
    pub async fn get_camera_info(&self) -> Result<CameraInfo> {
        Ok(CameraInfo {
            chip_name: self
                .get_chip_name()
                .await
                .unwrap_or_else(|_| "Unknown".to_string()),
            temperature_c: self.get_temperature().await.unwrap_or(f64::NAN),
            bit_depth: self.get_bit_depth().await.unwrap_or(0),
            readout_time_us: self.get_readout_time_us().await.unwrap_or(0.0),
            pixel_size_nm: self.get_pixel_size_nm().await.unwrap_or((0, 0)),
            sensor_size: (self.sensor_width, self.sensor_height),
            gain_name: self
                .get_gain_name()
                .await
                .unwrap_or_else(|_| "Unknown".to_string()),
            speed_name: self
                .get_speed_name()
                .await
                .unwrap_or_else(|_| "Unknown".to_string()),
        })
    }

    // =========================================================================
    // Gain and Speed Table Selection Methods
    // =========================================================================

    /// List all available gain modes for this camera
    ///
    /// Returns a vector of GainMode structs, each containing the index and name
    /// of an available gain setting. The index can be passed to `set_gain_index()`
    /// to select that gain mode.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn list_gain_modes(&self) -> Result<Vec<GainMode>> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // Get max gain index
        let mut max_idx: i32 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_GAIN_INDEX and ATTR_MAX
        // retrieves the maximum gain index. max_idx pointer is valid and properly aligned.
        unsafe {
            if pl_get_param(
                h,
                PARAM_GAIN_INDEX,
                ATTR_MAX,
                &mut max_idx as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get max gain index: {}",
                    get_pvcam_error()
                ));
            }
        }

        // Save current gain index so we can restore it
        let mut current_idx: i16 = 0;
        // SAFETY: h is valid. Retrieving current gain index to restore later. Ignoring return
        // value as this is a save operation - if it fails we'll just not restore the index.
        unsafe {
            pl_get_param(
                h,
                PARAM_GAIN_INDEX,
                ATTR_CURRENT,
                &mut current_idx as *mut _ as *mut _,
            );
        }

        let mut modes = Vec::new();
        for idx in 0..=max_idx {
            // Set gain index temporarily to read its name
            // SAFETY: h is valid. pl_set_param and pl_get_param operate on the camera handle.
            // idx_i16 pointer is valid. buf is properly allocated (256 bytes) for string output.
            // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
            unsafe {
                let idx_i16 = idx as i16;
                if pl_set_param(h, PARAM_GAIN_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                    continue; // Skip if setting fails
                }

                // Read the gain name for this index
                let mut buf = vec![0i8; 256];
                if pl_get_param(h, PARAM_GAIN_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) != 0 {
                    let name = std::ffi::CStr::from_ptr(buf.as_ptr())
                        .to_string_lossy()
                        .into_owned();
                    modes.push(GainMode {
                        index: idx as u16,
                        name,
                    });
                }
            }
        }

        // Restore original gain index
        // SAFETY: h is valid. Restoring the original gain index saved earlier.
        // current_idx contains the value retrieved at the start of this function.
        unsafe {
            pl_set_param(h, PARAM_GAIN_INDEX, &current_idx as *const _ as *mut _);
        }

        Ok(modes)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn list_gain_modes(&self) -> Result<Vec<GainMode>> {
        Ok(vec![
            GainMode {
                index: 0,
                name: "HDR".to_string(),
            },
            GainMode {
                index: 1,
                name: "High Sensitivity".to_string(),
            },
            GainMode {
                index: 2,
                name: "Full Well".to_string(),
            },
        ])
    }

    /// Set the gain mode by index
    ///
    /// Use `list_gain_modes()` to get available indices and their names.
    /// Changes take effect on the next acquisition.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_gain_index(&self, index: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let idx_i16 = index as i16;
        // SAFETY: h is a valid camera handle. pl_set_param with PARAM_GAIN_INDEX sets the
        // gain mode. idx_i16 pointer is valid and contains a valid index value.
        unsafe {
            if pl_set_param(h, PARAM_GAIN_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to set gain index: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_gain_index(&self, _index: u16) -> Result<()> {
        Ok(())
    }

    /// Get current gain mode information
    ///
    /// Returns a GainMode struct with the current gain index and name.
    pub async fn get_gain(&self) -> Result<GainMode> {
        let index = self.get_gain_index().await?;
        let name = self
            .get_gain_name()
            .await
            .unwrap_or_else(|_| "Unknown".to_string());
        Ok(GainMode { index, name })
    }

    /// List all available speed/readout modes for this camera
    ///
    /// Returns a vector of SpeedMode structs, each containing the index and name
    /// of an available speed setting. The index can be passed to `set_speed_index()`
    /// to select that speed mode.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn list_speed_modes(&self) -> Result<Vec<SpeedMode>> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // Get max speed index
        let mut max_idx: i32 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_SPDTAB_INDEX and ATTR_MAX
        // retrieves the maximum speed table index. max_idx pointer is valid and properly aligned.
        unsafe {
            if pl_get_param(
                h,
                PARAM_SPDTAB_INDEX,
                ATTR_MAX,
                &mut max_idx as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get max speed index: {}",
                    get_pvcam_error()
                ));
            }
        }

        // Save current speed index so we can restore it
        let mut current_idx: i16 = 0;
        // SAFETY: h is valid. Retrieving current speed index to restore later. Ignoring return
        // value as this is a save operation - if it fails we'll just not restore the index.
        unsafe {
            pl_get_param(
                h,
                PARAM_SPDTAB_INDEX,
                ATTR_CURRENT,
                &mut current_idx as *mut _ as *mut _,
            );
        }

        let mut modes = Vec::new();
        for idx in 0..=max_idx {
            // Set speed index temporarily to read its name
            // SAFETY: h is valid. pl_set_param and pl_get_param operate on the camera handle.
            // idx_i16 and pix_time pointers are valid. buf is properly allocated (256 bytes).
            // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
            unsafe {
                let idx_i16 = idx as i16;
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                    continue; // Skip if setting fails
                }

                // Try to read speed name (may not be available on all cameras)
                let mut buf = vec![0i8; 256];
                let name = if pl_get_param(
                    h,
                    PARAM_SPDTAB_NAME,
                    ATTR_CURRENT,
                    buf.as_mut_ptr() as *mut _,
                ) != 0
                {
                    std::ffi::CStr::from_ptr(buf.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                } else {
                    // Speed name not available, try to get pixel time instead
                    let mut pix_time: uns16 = 0;
                    if pl_get_param(
                        h,
                        PARAM_PIX_TIME,
                        ATTR_CURRENT,
                        &mut pix_time as *mut _ as *mut _,
                    ) != 0
                    {
                        format!("{} ns/pixel", pix_time)
                    } else {
                        format!("Speed {}", idx)
                    }
                };

                modes.push(SpeedMode {
                    index: idx as u16,
                    name,
                });
            }
        }

        // Restore original speed index
        // SAFETY: h is valid. Restoring the original speed index saved earlier.
        // current_idx contains the value retrieved at the start of this function.
        unsafe {
            pl_set_param(h, PARAM_SPDTAB_INDEX, &current_idx as *const _ as *mut _);
        }

        Ok(modes)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn list_speed_modes(&self) -> Result<Vec<SpeedMode>> {
        Ok(vec![
            SpeedMode {
                index: 0,
                name: "100 MHz".to_string(),
            },
            SpeedMode {
                index: 1,
                name: "200 MHz".to_string(),
            },
        ])
    }

    /// Set the speed/readout mode by index
    ///
    /// Use `list_speed_modes()` to get available indices and their names.
    /// Changes take effect on the next acquisition.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_speed_index(&self, index: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let idx_i16 = index as i16;
        // SAFETY: h is a valid camera handle. pl_set_param with PARAM_SPDTAB_INDEX sets the
        // speed/readout mode. idx_i16 pointer is valid and contains a valid index value.
        unsafe {
            if pl_set_param(h, PARAM_SPDTAB_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to set speed index: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_speed_index(&self, _index: u16) -> Result<()> {
        Ok(())
    }

    /// Get current speed/readout mode information
    ///
    /// Returns a SpeedMode struct with the current speed index and name.
    pub async fn get_speed(&self) -> Result<SpeedMode> {
        let index = self.get_speed_index().await?;
        let name = self
            .get_speed_name()
            .await
            .unwrap_or_else(|_| "Unknown".to_string());
        Ok(SpeedMode { index, name })
    }

    // =========================================================================
    // Temperature Control Methods
    // =========================================================================

    /// Get the temperature setpoint in degrees Celsius
    ///
    /// Returns the target temperature that the camera is trying to reach.
    /// PVCAM stores this as hundredths of degrees Celsius.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_temperature_setpoint(&self) -> Result<f64> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut temp_raw: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_TEMP_SETPOINT retrieves
        // the temperature setpoint. temp_raw pointer is valid and properly aligned for i16 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_TEMP_SETPOINT,
                ATTR_CURRENT,
                &mut temp_raw as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get temperature setpoint: {}",
                    get_pvcam_error()
                ));
            }
        }
        // PVCAM returns temperature in hundredths of degrees C
        Ok(temp_raw as f64 / 100.0)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_temperature_setpoint(&self) -> Result<f64> {
        Ok(-40.0) // Mock: typical cooled sensor setpoint
    }

    /// Set the temperature setpoint in degrees Celsius
    ///
    /// Sets the target temperature for the camera's cooling system.
    /// The actual temperature may take time to reach the setpoint.
    /// Typical range is -40°C to +25°C depending on camera model.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_temperature_setpoint(&self, celsius: f64) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // PVCAM expects temperature in hundredths of degrees
        let temp_raw = (celsius * 100.0) as i16;
        // SAFETY: h is a valid camera handle. pl_set_param with PARAM_TEMP_SETPOINT sets the
        // temperature setpoint. temp_raw pointer is valid and contains a valid temperature value.
        unsafe {
            if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set temperature setpoint: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_temperature_setpoint(&self, _celsius: f64) -> Result<()> {
        Ok(())
    }

    /// Get the current fan speed setting
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_fan_speed(&self) -> Result<FanSpeed> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;
        let mut speed: i32 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_FAN_SPEED_SETPOINT retrieves
        // the fan speed setting. speed pointer is valid and properly aligned for i32 writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_FAN_SPEED_SETPOINT,
                ATTR_CURRENT,
                &mut speed as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get fan speed: {}", get_pvcam_error()));
            }
        }
        Ok(FanSpeed::from_pvcam(speed))
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_fan_speed(&self) -> Result<FanSpeed> {
        Ok(FanSpeed::High) // Mock: default to high
    }

    /// Set the fan speed
    ///
    /// Controls the camera's cooling fan. Higher speeds provide better cooling
    /// but may introduce vibration. Lower speeds or off may be needed for
    /// vibration-sensitive applications but may limit cooling performance.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_fan_speed(&self, speed: FanSpeed) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let speed_val = speed.to_pvcam();
        // SAFETY: h is a valid camera handle. pl_set_param with PARAM_FAN_SPEED_SETPOINT sets
        // the fan speed. speed_val pointer is valid and contains a valid FanSpeed enum value.
        unsafe {
            if pl_set_param(
                h,
                PARAM_FAN_SPEED_SETPOINT,
                &speed_val as *const _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to set fan speed: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_fan_speed(&self, _speed: FanSpeed) -> Result<()> {
        Ok(())
    }

    // =========================================================================
    // Post-Processing Feature Methods
    // =========================================================================

    /// List all available post-processing features
    ///
    /// Returns information about each PP feature including its index, ID, and name.
    /// Common features include defect correction, background subtraction, etc.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn list_pp_features(&self) -> Result<Vec<PPFeature>> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // Get count of PP features
        let mut count: i16 = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_PP_INDEX and ATTR_COUNT
        // retrieves the number of post-processing features. count pointer is valid and aligned.
        // PP features may not be available on all cameras - we return empty vec if unavailable.
        unsafe {
            if pl_get_param(
                h,
                PARAM_PP_INDEX,
                ATTR_COUNT,
                &mut count as *mut _ as *mut _,
            ) == 0
            {
                // PP features may not be available on all cameras
                return Ok(Vec::new());
            }
        }

        let mut features = Vec::new();
        for idx in 0..count {
            // Set PP index to select this feature
            // SAFETY: h is valid. pl_set_param and pl_get_param access PP feature parameters.
            // idx_i16, feat_id pointers are valid. name_buf is properly allocated (256 bytes).
            // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
            unsafe {
                let idx_i16 = idx as i16;
                if pl_set_param(h, PARAM_PP_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                    continue; // Skip this feature if we can't select it
                }

                // Get feature ID
                let mut feat_id: u16 = 0;
                if pl_get_param(
                    h,
                    PARAM_PP_FEAT_ID,
                    ATTR_CURRENT,
                    &mut feat_id as *mut _ as *mut _,
                ) == 0
                {
                    continue;
                }

                // Get feature name
                let mut name_buf = vec![0i8; 256];
                if pl_get_param(
                    h,
                    PARAM_PP_FEAT_NAME,
                    ATTR_CURRENT,
                    name_buf.as_mut_ptr() as *mut _,
                ) == 0
                {
                    continue;
                }
                let name = std::ffi::CStr::from_ptr(name_buf.as_ptr())
                    .to_string_lossy()
                    .into_owned();

                features.push(PPFeature {
                    index: idx as u16,
                    id: feat_id,
                    name,
                });
            }
        }

        Ok(features)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn list_pp_features(&self) -> Result<Vec<PPFeature>> {
        // Mock: return some typical PP features
        Ok(vec![
            PPFeature {
                index: 0,
                id: 1,
                name: "Defect Correction".to_string(),
            },
            PPFeature {
                index: 1,
                id: 2,
                name: "Background Subtraction".to_string(),
            },
        ])
    }

    /// Get all parameters for a post-processing feature
    ///
    /// # Arguments
    /// * `feature_index` - Index of the PP feature (from list_pp_features)
    ///
    /// Returns information about each parameter including its index, ID, name, and current value.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_pp_params(&self, feature_index: u16) -> Result<Vec<PPParam>> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // Select the PP feature
        let idx = feature_index as i16;
        // SAFETY: h is a valid camera handle. All pl_set_param and pl_get_param calls operate
        // on the camera handle with valid parameter pointers. idx, param_idx, param_id, and value
        // pointers are all valid and properly aligned. name_buf is allocated (256 bytes).
        // CStr::from_ptr is safe because PVCAM guarantees null-terminated strings.
        unsafe {
            if pl_set_param(h, PARAM_PP_INDEX, &idx as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to select PP feature {}: {}",
                    feature_index,
                    get_pvcam_error()
                ));
            }

            // Get count of parameters for this feature
            let mut count: i16 = 0;
            if pl_get_param(
                h,
                PARAM_PP_PARAM_INDEX,
                ATTR_COUNT,
                &mut count as *mut _ as *mut _,
            ) == 0
            {
                return Ok(Vec::new()); // No parameters for this feature
            }

            let mut params = Vec::new();
            for param_idx in 0..count {
                // Select this parameter
                if pl_set_param(h, PARAM_PP_PARAM_INDEX, &param_idx as *const _ as *mut _) == 0 {
                    continue;
                }

                // Get parameter ID
                let mut param_id: u16 = 0;
                if pl_get_param(
                    h,
                    PARAM_PP_PARAM_ID,
                    ATTR_CURRENT,
                    &mut param_id as *mut _ as *mut _,
                ) == 0
                {
                    continue;
                }

                // Get parameter name
                let mut name_buf = vec![0i8; 256];
                if pl_get_param(
                    h,
                    PARAM_PP_PARAM_NAME,
                    ATTR_CURRENT,
                    name_buf.as_mut_ptr() as *mut _,
                ) == 0
                {
                    continue;
                }
                let name = std::ffi::CStr::from_ptr(name_buf.as_ptr())
                    .to_string_lossy()
                    .into_owned();

                // Get current value
                let mut value: u32 = 0;
                if pl_get_param(
                    h,
                    PARAM_PP_PARAM,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut _,
                ) == 0
                {
                    continue;
                }

                params.push(PPParam {
                    index: param_idx as u16,
                    id: param_id,
                    name,
                    value,
                });
            }

            Ok(params)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_pp_params(&self, _feature_index: u16) -> Result<Vec<PPParam>> {
        // Mock: return typical parameters
        Ok(vec![
            PPParam {
                index: 0,
                id: 1,
                name: "Enabled".to_string(),
                value: 1,
            },
            PPParam {
                index: 1,
                id: 2,
                name: "Threshold".to_string(),
                value: 100,
            },
        ])
    }

    /// Get a specific post-processing parameter value
    ///
    /// # Arguments
    /// * `feature_index` - Index of the PP feature
    /// * `param_index` - Index of the parameter within the feature
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_pp_param(&self, feature_index: u16, param_index: u16) -> Result<u32> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let feat_idx = feature_index as i16;
        let par_idx = param_index as i16;

        // SAFETY: h is a valid camera handle. pl_set_param selects the PP feature and parameter.
        // pl_get_param retrieves the parameter value. All pointers are valid and properly aligned.
        unsafe {
            // Select the PP feature
            if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to select PP feature {}: {}",
                    feature_index,
                    get_pvcam_error()
                ));
            }

            // Select the parameter
            if pl_set_param(h, PARAM_PP_PARAM_INDEX, &par_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to select PP param {}: {}",
                    param_index,
                    get_pvcam_error()
                ));
            }

            // Get the value
            let mut value: u32 = 0;
            if pl_get_param(
                h,
                PARAM_PP_PARAM,
                ATTR_CURRENT,
                &mut value as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get PP param value: {}",
                    get_pvcam_error()
                ));
            }

            Ok(value)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_pp_param(&self, _feature_index: u16, _param_index: u16) -> Result<u32> {
        Ok(0) // Mock value
    }

    /// Set a post-processing parameter value
    ///
    /// # Arguments
    /// * `feature_index` - Index of the PP feature
    /// * `param_index` - Index of the parameter within the feature
    /// * `value` - New value to set
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_pp_param(
        &self,
        feature_index: u16,
        param_index: u16,
        value: u32,
    ) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let feat_idx = feature_index as i16;
        let par_idx = param_index as i16;

        // SAFETY: h is a valid camera handle. pl_set_param selects the PP feature/parameter
        // and sets the parameter value. All pointers are valid and properly aligned.
        unsafe {
            // Select the PP feature
            if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to select PP feature {}: {}",
                    feature_index,
                    get_pvcam_error()
                ));
            }

            // Select the parameter
            if pl_set_param(h, PARAM_PP_PARAM_INDEX, &par_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to select PP param {}: {}",
                    param_index,
                    get_pvcam_error()
                ));
            }

            // Set the value
            if pl_set_param(h, PARAM_PP_PARAM, &value as *const _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set PP param value: {}",
                    get_pvcam_error()
                ));
            }

            Ok(())
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_pp_param(
        &self,
        _feature_index: u16,
        _param_index: u16,
        _value: u32,
    ) -> Result<()> {
        Ok(())
    }

    /// Reset all post-processing features to default values
    #[cfg(feature = "pvcam_hardware")]
    pub async fn reset_pp_features(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: h is a valid camera handle. pl_pp_reset resets all post-processing features
        // to their default values. This is a safe operation with no pointer arguments.
        unsafe {
            if pl_pp_reset(h) == 0 {
                return Err(anyhow!(
                    "Failed to reset PP features: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn reset_pp_features(&self) -> Result<()> {
        Ok(())
    }

    // =========================================================================
    // Smart Streaming Methods (Variable Exposure Sequences)
    // =========================================================================

    /// Check if Smart Streaming is available on this camera
    ///
    /// Smart Streaming allows specifying different exposure times for each
    /// frame in a sequence, useful for HDR acquisition.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_smart_streaming_available(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let mut available: rs_bool = 0;
        // SAFETY: h is a valid camera handle. pl_get_param with PARAM_SMART_STREAM_MODE_ENABLED
        // and ATTR_AVAIL checks feature availability. available pointer is valid and aligned.
        unsafe {
            // Check if PARAM_SMART_STREAM_MODE_ENABLED is available
            if pl_get_param(
                h,
                PARAM_SMART_STREAM_MODE_ENABLED,
                ATTR_AVAIL,
                &mut available as *mut _ as *mut _,
            ) == 0
            {
                return Ok(false);
            }
        }
        Ok(available != 0)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_smart_streaming_available(&self) -> Result<bool> {
        Ok(true) // Mock: assume available
    }

    /// Check if Smart Streaming is currently enabled
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_smart_streaming_enabled(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let mut enabled: rs_bool = 0;
        // SAFETY: h is a valid camera handle. pl_get_param retrieves the Smart Streaming
        // enabled state. enabled pointer is valid and properly aligned for rs_bool writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_SMART_STREAM_MODE_ENABLED,
                ATTR_CURRENT,
                &mut enabled as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get Smart Streaming status: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(enabled != 0)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_smart_streaming_enabled(&self) -> Result<bool> {
        Ok(false) // Mock: disabled by default
    }

    /// Enable Smart Streaming mode
    ///
    /// Must be called before setting exposure sequences.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn enable_smart_streaming(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let enabled: rs_bool = 1;
        // SAFETY: h is a valid camera handle. pl_set_param enables Smart Streaming mode.
        // enabled pointer is valid and contains the enable value (1).
        unsafe {
            if pl_set_param(
                h,
                PARAM_SMART_STREAM_MODE_ENABLED,
                &enabled as *const _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to enable Smart Streaming: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn enable_smart_streaming(&self) -> Result<()> {
        Ok(())
    }

    /// Disable Smart Streaming mode
    #[cfg(feature = "pvcam_hardware")]
    pub async fn disable_smart_streaming(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let enabled: rs_bool = 0;
        // SAFETY: h is a valid camera handle. pl_set_param disables Smart Streaming mode.
        // enabled pointer is valid and contains the disable value (0).
        unsafe {
            if pl_set_param(
                h,
                PARAM_SMART_STREAM_MODE_ENABLED,
                &enabled as *const _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to disable Smart Streaming: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn disable_smart_streaming(&self) -> Result<()> {
        Ok(())
    }

    /// Get the maximum number of Smart Streaming entries supported
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_smart_stream_max_entries(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let mut max_entries: u16 = 0;
        // SAFETY: pl_get_param retrieves PARAM_SMART_STREAM_MODE max value. Camera handle h is
        // valid from pl_cam_open. max_entries pointer is valid and properly aligned for writes.
        unsafe {
            if pl_get_param(
                h,
                PARAM_SMART_STREAM_MODE,
                ATTR_MAX,
                &mut max_entries as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get Smart Streaming max entries: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(max_entries)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_smart_stream_max_entries(&self) -> Result<u16> {
        Ok(128) // Mock: typical max
    }

    /// Set Smart Streaming exposure sequence
    ///
    /// # Arguments
    /// * `exposures_ms` - Array of exposure times in milliseconds
    ///
    /// Smart Streaming must be enabled before calling this method.
    /// The camera will cycle through these exposure times during acquisition.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_smart_stream_exposures(&self, exposures_ms: &[f64]) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        if exposures_ms.is_empty() {
            return Err(anyhow!("At least one exposure time required"));
        }

        // SAFETY: pl_create_smart_stream_struct allocates a smart_stream_type structure. The returned
        // pointer is checked for null. Dereferencing ss_ptr and writing to ss.params are safe because
        // PVCAM allocates sufficient space for the requested number of entries. pl_release_smart_stream_struct
        // and pl_set_param properly handle the structure. Pointer arithmetic with .add(i) is safe within bounds.
        unsafe {
            // Create smart_stream_type structure
            let mut ss_ptr: *mut smart_stream_type = std::ptr::null_mut();
            if pl_create_smart_stream_struct(&mut ss_ptr, exposures_ms.len() as u16) == 0 {
                return Err(anyhow!(
                    "Failed to create Smart Stream struct: {}",
                    get_pvcam_error()
                ));
            }

            // Fill in exposure values (convert ms to microseconds)
            let ss = &mut *ss_ptr;
            for (i, &exp_ms) in exposures_ms.iter().enumerate() {
                let exp_us = (exp_ms * 1000.0) as u32;
                *(ss.params.add(i)) = exp_us;
            }

            // Set the exposure parameters
            let result = pl_set_param(h, PARAM_SMART_STREAM_EXP_PARAMS, ss_ptr as *mut _);

            // Clean up
            pl_release_smart_stream_struct(&mut ss_ptr);

            if result == 0 {
                return Err(anyhow!(
                    "Failed to set Smart Stream exposures: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_smart_stream_exposures(&self, _exposures_ms: &[f64]) -> Result<()> {
        Ok(())
    }

    /// Get current Smart Streaming exposure sequence count
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_smart_stream_exposure_count(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves smart_stream_type pointer. Camera handle h is valid.
        // ss_ptr is checked for null before dereferencing. Reading (*ss_ptr).entries is safe
        // as PVCAM maintains the structure validity.
        unsafe {
            let mut ss_ptr: *mut smart_stream_type = std::ptr::null_mut();

            // Get current exposure params
            if pl_get_param(
                h,
                PARAM_SMART_STREAM_EXP_PARAMS,
                ATTR_CURRENT,
                &mut ss_ptr as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get Smart Stream exposures: {}",
                    get_pvcam_error()
                ));
            }

            if ss_ptr.is_null() {
                return Ok(0);
            }

            let entries = (*ss_ptr).entries;
            Ok(entries)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_smart_stream_exposure_count(&self) -> Result<u16> {
        Ok(0) // Mock: no entries
    }

    // =========================================================================
    // Centroids Mode (PrimeLocate / Particle Tracking)
    // =========================================================================

    /// Check if centroids feature is available on this camera
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_centroids_available(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param checks parameter availability. Camera handle h is valid.
        // avail pointer is valid and properly aligned for writes.
        unsafe {
            let mut avail: rs_bool = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_ENABLED,
                ATTR_AVAIL,
                &mut avail as *mut _ as *mut _,
            ) == 0
            {
                // Parameter not supported
                return Ok(false);
            }
            Ok(avail != 0)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_centroids_available(&self) -> Result<bool> {
        Ok(true) // Mock: always available
    }

    /// Check if centroids mode is currently enabled
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_centroids_enabled(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves centroids enabled state. Camera handle h is valid.
        // enabled pointer is valid and properly aligned for writes.
        unsafe {
            let mut enabled: rs_bool = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_ENABLED,
                ATTR_CURRENT,
                &mut enabled as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get centroids enabled state: {}",
                    get_pvcam_error()
                ));
            }
            Ok(enabled != 0)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_centroids_enabled(&self) -> Result<bool> {
        Ok(false) // Mock: disabled by default
    }

    /// Enable centroids mode
    #[cfg(feature = "pvcam_hardware")]
    pub async fn enable_centroids(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param sets centroids enabled parameter. Camera handle h is valid.
        // enabled pointer is valid and properly aligned for reads.
        unsafe {
            let mut enabled: rs_bool = 1;
            if pl_set_param(h, PARAM_CENTROIDS_ENABLED, &mut enabled as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to enable centroids: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn enable_centroids(&self) -> Result<()> {
        Ok(())
    }

    /// Disable centroids mode
    #[cfg(feature = "pvcam_hardware")]
    pub async fn disable_centroids(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param disables centroids parameter. Camera handle h is valid.
        // enabled pointer is valid and properly aligned for reads.
        unsafe {
            let mut enabled: rs_bool = 0;
            if pl_set_param(h, PARAM_CENTROIDS_ENABLED, &mut enabled as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to disable centroids: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn disable_centroids(&self) -> Result<()> {
        Ok(())
    }

    /// Get current centroids mode (Locate, Track, or Blob)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_centroids_mode(&self) -> Result<CentroidsMode> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves centroids mode value. Camera handle h is valid.
        // mode pointer is valid and properly aligned for writes.
        unsafe {
            let mut mode: i32 = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_MODE,
                ATTR_CURRENT,
                &mut mode as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get centroids mode: {}",
                    get_pvcam_error()
                ));
            }
            Ok(CentroidsMode::from_pvcam(mode))
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_centroids_mode(&self) -> Result<CentroidsMode> {
        Ok(CentroidsMode::Locate) // Mock: default to Locate
    }

    /// Set centroids mode (Locate, Track, or Blob)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_centroids_mode(&self, mode: CentroidsMode) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param sets centroids mode parameter. Camera handle h is valid.
        // mode_val pointer is valid and properly aligned for reads.
        unsafe {
            let mut mode_val: i32 = mode.to_pvcam();
            if pl_set_param(h, PARAM_CENTROIDS_MODE, &mut mode_val as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set centroids mode: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_centroids_mode(&self, _mode: CentroidsMode) -> Result<()> {
        Ok(())
    }

    /// Get centroids search radius in pixels
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_centroids_radius(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves centroids radius value. Camera handle h is valid.
        // radius pointer is valid and properly aligned for writes.
        unsafe {
            let mut radius: uns16 = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_RADIUS,
                ATTR_CURRENT,
                &mut radius as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get centroids radius: {}",
                    get_pvcam_error()
                ));
            }
            Ok(radius)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_centroids_radius(&self) -> Result<u16> {
        Ok(5) // Mock: default radius
    }

    /// Set centroids search radius in pixels
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_centroids_radius(&self, radius: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param sets centroids radius parameter. Camera handle h is valid.
        // r pointer is valid and properly aligned for reads.
        unsafe {
            let mut r: uns16 = radius;
            if pl_set_param(h, PARAM_CENTROIDS_RADIUS, &mut r as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set centroids radius: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_centroids_radius(&self, _radius: u16) -> Result<()> {
        Ok(())
    }

    /// Get maximum number of centroids to detect
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_centroids_count(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves centroids count value. Camera handle h is valid.
        // count pointer is valid and properly aligned for writes.
        unsafe {
            let mut count: uns16 = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_COUNT,
                ATTR_CURRENT,
                &mut count as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get centroids count: {}",
                    get_pvcam_error()
                ));
            }
            Ok(count)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_centroids_count(&self) -> Result<u16> {
        Ok(100) // Mock: default max count
    }

    /// Set maximum number of centroids to detect
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_centroids_count(&self, count: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param sets centroids count parameter. Camera handle h is valid.
        // c pointer is valid and properly aligned for reads.
        unsafe {
            let mut c: uns16 = count;
            if pl_set_param(h, PARAM_CENTROIDS_COUNT, &mut c as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set centroids count: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_centroids_count(&self, _count: u16) -> Result<()> {
        Ok(())
    }

    /// Get centroids detection threshold
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_centroids_threshold(&self) -> Result<u32> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves centroids threshold value. Camera handle h is valid.
        // thresh pointer is valid and properly aligned for writes.
        unsafe {
            let mut thresh: uns32 = 0;
            if pl_get_param(
                h,
                PARAM_CENTROIDS_THRESHOLD,
                ATTR_CURRENT,
                &mut thresh as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get centroids threshold: {}",
                    get_pvcam_error()
                ));
            }
            Ok(thresh)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_centroids_threshold(&self) -> Result<u32> {
        Ok(1000) // Mock: default threshold
    }

    /// Set centroids detection threshold
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_centroids_threshold(&self, threshold: u32) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_set_param sets centroids threshold parameter. Camera handle h is valid.
        // t pointer is valid and properly aligned for reads.
        unsafe {
            let mut t: uns32 = threshold;
            if pl_set_param(h, PARAM_CENTROIDS_THRESHOLD, &mut t as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set centroids threshold: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_centroids_threshold(&self, _threshold: u32) -> Result<()> {
        Ok(())
    }

    /// Get full centroids configuration
    pub async fn get_centroids_config(&self) -> Result<CentroidsConfig> {
        Ok(CentroidsConfig {
            mode: self.get_centroids_mode().await?,
            radius: self.get_centroids_radius().await?,
            max_count: self.get_centroids_count().await?,
            threshold: self.get_centroids_threshold().await?,
        })
    }

    /// Set full centroids configuration
    pub async fn set_centroids_config(&self, config: &CentroidsConfig) -> Result<()> {
        self.set_centroids_mode(config.mode).await?;
        self.set_centroids_radius(config.radius).await?;
        self.set_centroids_count(config.max_count).await?;
        self.set_centroids_threshold(config.threshold).await?;
        Ok(())
    }

    // =========================================================================
    // PrimeEnhance Convenience API (wraps DENOISING PP feature)
    // =========================================================================

    /// Find the index of the DENOISING (PrimeEnhance) PP feature
    async fn find_prime_enhance_index(&self) -> Result<Option<u16>> {
        let features = self.list_pp_features().await?;
        for (idx, feature) in features.iter().enumerate() {
            // DENOISING feature ID is 14
            if feature.id == 14 {
                return Ok(Some(idx as u16));
            }
        }
        Ok(None)
    }

    /// Check if PrimeEnhance is available on this camera
    pub async fn is_prime_enhance_available(&self) -> Result<bool> {
        Ok(self.find_prime_enhance_index().await?.is_some())
    }

    /// Check if PrimeEnhance is currently enabled
    pub async fn is_prime_enhance_enabled(&self) -> Result<bool> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        // Parameter 0 is always ENABLED
        let value = self.get_pp_param(idx, 0).await?;
        Ok(value != 0)
    }

    /// Enable PrimeEnhance noise reduction
    pub async fn enable_prime_enhance(&self) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 0, 1).await
    }

    /// Disable PrimeEnhance noise reduction
    pub async fn disable_prime_enhance(&self) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 0, 0).await
    }

    /// Get PrimeEnhance number of iterations
    pub async fn get_prime_enhance_iterations(&self) -> Result<u32> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        // Parameter 1 is NO OF ITERATIONS
        self.get_pp_param(idx, 1).await
    }

    /// Set PrimeEnhance number of iterations
    pub async fn set_prime_enhance_iterations(&self, iterations: u32) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 1, iterations).await
    }

    /// Get PrimeEnhance gain parameter
    pub async fn get_prime_enhance_gain(&self) -> Result<u32> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        // Parameter 2 is GAIN
        self.get_pp_param(idx, 2).await
    }

    /// Set PrimeEnhance gain parameter
    pub async fn set_prime_enhance_gain(&self, gain: u32) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 2, gain).await
    }

    /// Get PrimeEnhance offset parameter
    pub async fn get_prime_enhance_offset(&self) -> Result<u32> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        // Parameter 3 is OFFSET
        self.get_pp_param(idx, 3).await
    }

    /// Set PrimeEnhance offset parameter
    pub async fn set_prime_enhance_offset(&self, offset: u32) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 3, offset).await
    }

    /// Get PrimeEnhance lambda parameter (noise reduction strength)
    pub async fn get_prime_enhance_lambda(&self) -> Result<u32> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        // Parameter 4 is LAMBDA
        self.get_pp_param(idx, 4).await
    }

    /// Set PrimeEnhance lambda parameter (noise reduction strength)
    pub async fn set_prime_enhance_lambda(&self, lambda: u32) -> Result<()> {
        let idx = self
            .find_prime_enhance_index()
            .await?
            .ok_or_else(|| anyhow!("PrimeEnhance not available on this camera"))?;
        self.set_pp_param(idx, 4, lambda).await
    }

    // =========================================================================
    // Frame Rotation and Flip
    // =========================================================================

    /// Check if frame rotation is available
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_frame_rotation_available(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param checks frame rotation parameter availability. Camera handle h
        // is valid. avail pointer is valid and properly aligned for writes.
        unsafe {
            let mut avail: rs_bool = 0;
            if pl_get_param(
                h,
                PARAM_FRAME_ROTATE,
                ATTR_AVAIL,
                &mut avail as *mut _ as *mut _,
            ) == 0
            {
                return Ok(false);
            }
            Ok(avail != 0)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_frame_rotation_available(&self) -> Result<bool> {
        Ok(true)
    }

    /// Get current frame rotation (0, 90, 180, or 270 degrees)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_frame_rotation(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves frame rotation value. Camera handle h is valid.
        // rotation pointer is valid and properly aligned for writes.
        unsafe {
            let mut rotation: i32 = 0;
            if pl_get_param(
                h,
                PARAM_FRAME_ROTATE,
                ATTR_CURRENT,
                &mut rotation as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!(
                    "Failed to get frame rotation: {}",
                    get_pvcam_error()
                ));
            }
            // Convert enum value to degrees: 0=0, 1=90, 2=180, 3=270
            Ok((rotation * 90) as u16)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_frame_rotation(&self) -> Result<u16> {
        Ok(0)
    }

    /// Set frame rotation (0, 90, 180, or 270 degrees)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_frame_rotation(&self, degrees: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // Convert degrees to enum: 0=0, 90=1, 180=2, 270=3
        let rotation = match degrees {
            0 => 0i32,
            90 => 1i32,
            180 => 2i32,
            270 => 3i32,
            _ => return Err(anyhow!("Invalid rotation: must be 0, 90, 180, or 270")),
        };

        // SAFETY: pl_set_param sets frame rotation parameter. Camera handle h is valid.
        // r pointer is valid and properly aligned for reads.
        unsafe {
            let mut r = rotation;
            if pl_set_param(h, PARAM_FRAME_ROTATE, &mut r as *mut _ as *mut _) == 0 {
                return Err(anyhow!(
                    "Failed to set frame rotation: {}",
                    get_pvcam_error()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_frame_rotation(&self, _degrees: u16) -> Result<()> {
        Ok(())
    }

    /// Check if frame flip is available
    #[cfg(feature = "pvcam_hardware")]
    pub async fn is_frame_flip_available(&self) -> Result<bool> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param checks frame flip parameter availability. Camera handle h
        // is valid. avail pointer is valid and properly aligned for writes.
        unsafe {
            let mut avail: rs_bool = 0;
            if pl_get_param(
                h,
                PARAM_FRAME_FLIP,
                ATTR_AVAIL,
                &mut avail as *mut _ as *mut _,
            ) == 0
            {
                return Ok(false);
            }
            Ok(avail != 0)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn is_frame_flip_available(&self) -> Result<bool> {
        Ok(true)
    }

    /// Get current frame flip mode (0=none, 1=horizontal, 2=vertical, 3=both)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn get_frame_flip(&self) -> Result<u16> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        // SAFETY: pl_get_param retrieves frame flip value. Camera handle h is valid.
        // flip pointer is valid and properly aligned for writes.
        unsafe {
            let mut flip: i32 = 0;
            if pl_get_param(
                h,
                PARAM_FRAME_FLIP,
                ATTR_CURRENT,
                &mut flip as *mut _ as *mut _,
            ) == 0
            {
                return Err(anyhow!("Failed to get frame flip: {}", get_pvcam_error()));
            }
            Ok(flip as u16)
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn get_frame_flip(&self) -> Result<u16> {
        Ok(0)
    }

    /// Set frame flip mode (0=none, 1=horizontal, 2=vertical, 3=both)
    #[cfg(feature = "pvcam_hardware")]
    pub async fn set_frame_flip(&self, mode: u16) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        if mode > 3 {
            return Err(anyhow!("Invalid flip mode: must be 0-3"));
        }

        // SAFETY: pl_set_param sets frame flip parameter. Camera handle h is valid.
        // m pointer is valid and properly aligned for reads.
        unsafe {
            let mut m = mode as i32;
            if pl_set_param(h, PARAM_FRAME_FLIP, &mut m as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to set frame flip: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_frame_flip(&self, _mode: u16) -> Result<()> {
        Ok(())
    }

    /// Convenience method: flip frame horizontally
    pub async fn flip_horizontal(&self) -> Result<()> {
        let current = self.get_frame_flip().await?;
        // Toggle bit 0 (horizontal)
        self.set_frame_flip(current ^ 1).await
    }

    /// Convenience method: flip frame vertically
    pub async fn flip_vertical(&self) -> Result<()> {
        let current = self.get_frame_flip().await?;
        // Toggle bit 1 (vertical)
        self.set_frame_flip(current ^ 2).await
    }

    /// Hardware polling loop for continuous acquisition
    ///
    /// This runs in a blocking thread and polls the PVCAM SDK for new frames.
    /// Uses get_oldest_frame + unlock_oldest_frame pattern for ordered, lossless delivery.
    #[cfg(feature = "pvcam_hardware")]
    fn poll_loop_hardware(
        hcam: i16,
        streaming: Arc<AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<std::sync::Arc<Frame>>,
        frame_count: Arc<AtomicU64>,
        frame_pixels: usize,
        width: u32,
        height: u32,
    ) {
        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;
        let mut no_frame_count: u32 = 0;
        const MAX_NO_FRAME_ITERATIONS: u32 = 5000; // ~5 seconds at 1ms sleep

        while streaming.load(Ordering::SeqCst) {
            // SAFETY: Poll loop handles continuous acquisition with valid camera handle hcam.
            // All status pointers are valid and properly aligned. pl_exp_get_oldest_frame returns
            // a frame pointer that is checked for null before dereferencing. Frame data is copied
            // before calling pl_exp_unlock_oldest_frame which invalidates the pointer.
            unsafe {
                // Check continuous acquisition status
                if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt)
                    == 0
                {
                    eprintln!("PVCAM: Failed to check continuous status");
                    break;
                }

                match status {
                    s if s == READOUT_COMPLETE || s == EXPOSURE_IN_PROGRESS => {
                        // Use get_oldest_frame for ordered delivery (FIFO)
                        let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

                        if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) != 0
                            && !frame_ptr.is_null()
                        {
                            // Copy frame data BEFORE unlock
                            let src =
                                std::slice::from_raw_parts(frame_ptr as *const u16, frame_pixels);
                            let pixels = src.to_vec();

                            // CRITICAL: Unlock buffer slot so SDK can reuse it
                            // Must be called after copy, pointer invalid after unlock
                            pl_exp_unlock_oldest_frame(hcam);

                            let frame = Frame::new(width, height, pixels);
                            frame_count.fetch_add(1, Ordering::SeqCst);

                            // Send frame wrapped in Arc (non-blocking, log if no subscribers)
                            if frame_tx.send(std::sync::Arc::new(frame)).is_err() {
                                // No subscribers - this is OK, frames are dropped silently
                            }

                            // Reset timeout counter on successful frame
                            no_frame_count = 0;
                        } else {
                            // No frame available yet, wait briefly
                            std::thread::sleep(Duration::from_millis(1));
                            no_frame_count += 1;
                        }
                    }
                    s if s == READOUT_FAILED => {
                        eprintln!("PVCAM: Readout failed");
                        break;
                    }
                    _ => {
                        // READOUT_NOT_ACTIVE or READOUT_IN_PROGRESS - wait a bit
                        std::thread::sleep(Duration::from_millis(1));
                        no_frame_count += 1;
                    }
                }

                // Timeout watchdog: break if no frames for too long
                if no_frame_count >= MAX_NO_FRAME_ITERATIONS {
                    eprintln!(
                        "PVCAM: Acquisition timeout - no frames for {} iterations",
                        no_frame_count
                    );
                    break;
                }
            }
        }

        // Cleanup: stop acquisition (poll task owns this now)
        // SAFETY: pl_exp_stop_cont stops continuous acquisition. Camera handle hcam is valid.
        unsafe {
            pl_exp_stop_cont(hcam, CCS_HALT);
        }
    }
}

impl Drop for PvcamDriver {
    fn drop(&mut self) {
        // Signal streaming to stop first
        self.streaming.store(false, Ordering::SeqCst);

        #[cfg(feature = "pvcam_hardware")]
        {
            // CRITICAL: Wait for poll task to finish BEFORE closing camera
            // The poll task will call pl_exp_stop_cont when it exits
            if let Ok(mut poll_guard) = self.poll_handle.try_lock() {
                if let Some(handle) = poll_guard.take() {
                    // Block on the poll task with a timeout
                    // Use std blocking since we're in Drop (can't be async)
                    let _ = std::thread::spawn(move || {
                        // This runs in a separate thread to avoid blocking Drop indefinitely
                        let rt = tokio::runtime::Handle::try_current();
                        if let Ok(rt) = rt {
                            let _ = rt.block_on(async {
                                tokio::time::timeout(Duration::from_secs(2), handle).await
                            });
                        }
                    })
                    .join();
                }
            }

            // Now safe to close camera - poll task has exited
            if let Ok(handle) = self.camera_handle.try_lock() {
                if let Some(h) = *handle {
                    // SAFETY: pl_cam_close closes the camera. Handle h is valid and poll task
                    // has exited so no other threads are using this camera handle.
                    unsafe {
                        pl_cam_close(h);
                    }
                }
            }

            if let Ok(initialized) = self.sdk_initialized.try_lock() {
                if *initialized {
                    // SAFETY: pl_pvcam_uninit uninitializes the PVCAM library. Called only
                    // after camera is closed and all operations have ceased.
                    unsafe {
                        pl_pvcam_uninit();
                    }
                }
            }
        }
    }
}

#[async_trait]
impl FrameProducer for PvcamDriver {
    async fn start_stream(&self) -> Result<()> {
        // Check if already streaming
        if self.streaming.swap(true, Ordering::SeqCst) {
            bail!("Already streaming");
        }

        // Reset frame counter
        self.frame_count.store(0, Ordering::SeqCst);

        #[cfg(feature = "pvcam_hardware")]
        {
            let handle_guard = self.camera_handle.lock().await;
            let h = handle_guard.ok_or_else(|| anyhow!("Camera not open"))?;

            // Get current settings
            let roi = *self.roi.lock().await;
            let (x_bin, y_bin) = *self.binning.lock().await;
            let exp_ms = self.exposure_ms.get() as uns32;

            // Setup region
            // SAFETY: mem::zeroed creates a zero-initialized rgn_type structure which is valid
            // for PVCAM. All fields are primitive integers where zero is a valid bit pattern.
            let region = unsafe {
                let mut rgn: rgn_type = std::mem::zeroed();
                rgn.s1 = roi.x as uns16;
                rgn.s2 = (roi.x + roi.width - 1) as uns16;
                rgn.sbin = x_bin;
                rgn.p1 = roi.y as uns16;
                rgn.p2 = (roi.y + roi.height - 1) as uns16;
                rgn.pbin = y_bin;
                rgn
            };

            // Calculate frame size and setup continuous acquisition
            let mut frame_bytes: uns32 = 0;
            // SAFETY: pl_exp_setup_cont sets up continuous acquisition. Camera handle h is valid.
            // region pointer is valid and points to initialized rgn_type. frame_bytes pointer is valid.
            unsafe {
                if pl_exp_setup_cont(
                    h,
                    1,
                    &region as *const _,
                    TIMED_MODE,
                    exp_ms,
                    &mut frame_bytes,
                    CIRC_NO_OVERWRITE,
                ) == 0
                {
                    self.streaming.store(false, Ordering::SeqCst);
                    return Err(anyhow!("Failed to setup continuous acquisition"));
                }
            }

            // Calculate frame dimensions and allocate circular buffer
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_pixels = (binned_width * binned_height) as usize;
            let buffer_count = 8; // Use 8 frame buffers
            let mut circ_buf = vec![0u16; frame_pixels * buffer_count];

            // Start continuous acquisition
            let circ_ptr = circ_buf.as_mut_ptr();
            let circ_size_bytes = (circ_buf.len() * 2) as uns32;

            // SAFETY: pl_exp_start_cont starts continuous acquisition. Camera handle h is valid.
            // circ_ptr points to properly allocated circular buffer of correct size. The buffer
            // is stored in self.circ_buffer to keep it alive for the duration of acquisition.
            unsafe {
                if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                    self.streaming.store(false, Ordering::SeqCst);
                    return Err(anyhow!("Failed to start continuous acquisition"));
                }
            }

            // Store circular buffer to keep it alive
            *self.circ_buffer.lock().await = Some(circ_buf);

            // Spawn polling task
            let streaming = self.streaming.clone();
            let frame_tx = self.frame_tx.clone();
            let frame_count = self.frame_count.clone();
            let width = binned_width;
            let height = binned_height;

            let poll_handle = tokio::task::spawn_blocking(move || {
                Self::poll_loop_hardware(
                    h,
                    streaming,
                    frame_tx,
                    frame_count,
                    frame_pixels,
                    width,
                    height,
                );
            });

            *self.poll_handle.lock().await = Some(poll_handle);
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Mock streaming - spawn a task that generates synthetic frames
            let streaming = self.streaming.clone();
            let frame_tx = self.frame_tx.clone();
            let frame_count = self.frame_count.clone();
            let roi = *self.roi.lock().await;
            let exposure_ms = self.exposure_ms.get();
            let (x_bin, y_bin) = *self.binning.lock().await;

            tokio::spawn(async move {
                // Calculate binned dimensions (same as acquire_frame)
                let binned_width = roi.width / x_bin as u32;
                let binned_height = roi.height / y_bin as u32;
                let frame_size = (binned_width * binned_height) as usize;

                while streaming.load(Ordering::SeqCst) {
                    // Simulate exposure time
                    tokio::time::sleep(Duration::from_millis(exposure_ms as u64)).await;

                    if !streaming.load(Ordering::SeqCst) {
                        break;
                    }

                    // Generate synthetic frame with binned dimensions
                    let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                    let mut pixels = vec![0u16; frame_size];

                    // Create test pattern (gradient + frame number) in binned coordinates
                    for y in 0..binned_height {
                        for x in 0..binned_width {
                            let value =
                                (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                            pixels[(y * binned_width + x) as usize] = value;
                        }
                    }

                    let frame = Frame::new(binned_width, binned_height, pixels);

                    // Send frame wrapped in Arc (non-blocking, drop if no subscribers)
                    let _ = frame_tx.send(std::sync::Arc::new(frame));
                }
            });
        }

        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        // Signal streaming to stop
        if !self.streaming.swap(false, Ordering::SeqCst) {
            // Wasn't streaming
            return Ok(());
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            // Wait for poll task to finish
            if let Some(handle) = self.poll_handle.lock().await.take() {
                let _ = handle.await;
            }

            // Stop continuous acquisition
            let handle_guard = self.camera_handle.lock().await;
            if let Some(h) = *handle_guard {
                // SAFETY: pl_exp_stop_cont stops continuous acquisition. Camera handle h is valid
                // and poll task has already exited so no concurrent access.
                unsafe {
                    pl_exp_stop_cont(h, CCS_HALT);
                }
            }

            // Release circular buffer
            *self.circ_buffer.lock().await = None;
        }

        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (self.sensor_width, self.sensor_height)
    }

    #[expect(
        deprecated,
        reason = "take_frame_receiver implements deprecated FrameProducer trait method"
    )]
    async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<Frame>> {
        // Deprecated: use subscribe_frames() instead
        None
    }

    async fn subscribe_frames(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<Frame>>> {
        Some(self.frame_tx.subscribe())
    }

    async fn is_streaming(&self) -> Result<bool> {
        Ok(self.streaming.load(Ordering::SeqCst))
    }

    fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ExposureControl for PvcamDriver {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        let exposure_ms = seconds * 1000.0;

        if exposure_ms <= 0.0 || exposure_ms > 60000.0 {
            return Err(anyhow!("Exposure must be between 0 and 60000 ms"));
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            // Exposure is set during pl_exp_setup_seq, not via parameters
            // Store for use during acquisition
        }

        self.exposure_ms.set(exposure_ms).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.exposure_ms.get() / 1000.0) // Convert ms to seconds
    }
}

impl Parameterized for PvcamDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Triggerable for PvcamDriver {
    /// Arm the camera for triggered acquisition
    ///
    /// # Hardware Implementation
    /// Prepares the camera for triggered frame capture. Currently implements software-based
    /// triggering via the arm/trigger pattern. Full hardware external trigger support
    /// (e.g., TTL pulse input) requires trigger mode constants not yet exposed in PVCAM bindings.
    ///
    /// # Software Trigger Workflow
    /// 1. `arm()` - prepares camera (sets armed flag)
    /// 2. `wait_for_trigger()` or `trigger()` - initiates frame capture
    /// 3. Frame is acquired and can be read via `acquire_frame()`
    /// 4. `disarm()` - cleanup
    ///
    /// # Returns
    /// - Ok(()) if armed successfully
    /// - Err if camera is not opened
    ///
    /// # Future Enhancement
    /// To implement external hardware triggers (TRIGGER_FIRST_MODE):
    /// 1. Add trigger mode constants to pvcam-sys (build.rs allowlist)
    /// 2. Call pl_exp_setup_seq() with trigger mode in this method
    /// 3. Use wait_for_trigger() to poll for external trigger signal
    async fn arm(&self) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        {
            let h =
                (*self.camera_handle.lock().await).ok_or_else(|| anyhow!("Camera not opened"))?;

            let exposure = self.exposure_ms.get();
            let roi = *self.roi.lock().await;
            let (x_bin, y_bin) = *self.binning.lock().await;

            // Setup region for acquisition
            // SAFETY: mem::zeroed creates a zero-initialized rgn_type structure which is valid
            // for PVCAM. All fields are primitive integers where zero is a valid bit pattern.
            let region = unsafe {
                let mut rgn: rgn_type = std::mem::zeroed();
                rgn.s1 = roi.x as uns16;
                rgn.s2 = (roi.x + roi.width - 1) as uns16;
                rgn.sbin = x_bin;
                rgn.p1 = roi.y as uns16;
                rgn.p2 = (roi.y + roi.height - 1) as uns16;
                rgn.pbin = y_bin;
                rgn
            };

            // Calculate frame size with binning
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_size: uns32 = (binned_width * binned_height) as uns32;

            // Create frame buffer for triggered acquisition
            let mut frame = vec![0u16; frame_size as usize];

            // SAFETY: pl_exp_setup_seq and pl_exp_start_seq arm the camera for triggered acquisition.
            // Camera handle h is valid. region pointer is valid. frame buffer is properly allocated.
            // All parameter pointers are valid and properly aligned.
            unsafe {
                // Setup exposure sequence
                let exp_time_ms = exposure as uns32;
                let mut total_bytes: uns32 = 0;

                if pl_exp_setup_seq(
                    h,
                    1,
                    1,
                    &region as *const _ as *const _,
                    TIMED_MODE,
                    exp_time_ms,
                    &mut total_bytes,
                ) == 0
                {
                    return Err(anyhow!("Failed to setup acquisition sequence for trigger"));
                }

                // Start acquisition - camera will begin exposure immediately
                if pl_exp_start_seq(h, frame.as_mut_ptr() as *mut _) == 0 {
                    return Err(anyhow!("Failed to start acquisition for trigger"));
                }
            }

            // Store the frame buffer for retrieval after wait_for_trigger/trigger
            *self.trigger_frame.lock().await = Some(frame);
        }

        *self.armed.lock().await = true;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        let is_armed = *self.armed.lock().await;
        if !is_armed {
            return Err(anyhow!("Camera must be armed before triggering"));
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            let h =
                (*self.camera_handle.lock().await).ok_or_else(|| anyhow!("Camera not opened"))?;

            let exposure = self.exposure_ms.get();

            // Wait for acquisition to complete
            let mut status: i16 = 0;
            let mut bytes_arrived: uns32 = 0;

            let timeout = Duration::from_millis((exposure + 5000.0) as u64);
            let start = std::time::Instant::now();

            loop {
                // SAFETY: pl_exp_check_status polls acquisition status. Camera handle h is valid.
                // status and bytes_arrived pointers are valid. pl_exp_finish_seq cleanup is safe
                // with valid camera handle and frame buffer pointer.
                unsafe {
                    if pl_exp_check_status(h, &mut status, &mut bytes_arrived) == 0 {
                        // Cleanup on error
                        if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                            pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                        }
                        *self.armed.lock().await = false;
                        return Err(anyhow!("Failed to check acquisition status"));
                    }
                }

                if status == READOUT_COMPLETE || bytes_arrived > 0 {
                    break;
                }

                if status == READOUT_FAILED {
                    // Cleanup on failure
                    // SAFETY: pl_exp_finish_seq cleans up sequence. Camera handle h and frame pointer are valid.
                    unsafe {
                        if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                            pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                        }
                    }
                    *self.armed.lock().await = false;
                    return Err(anyhow!("Acquisition failed"));
                }

                if start.elapsed() > timeout {
                    // Cleanup on timeout
                    // SAFETY: pl_exp_finish_seq cleans up sequence. Camera handle h and frame pointer are valid.
                    unsafe {
                        if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                            pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                        }
                    }
                    *self.armed.lock().await = false;
                    return Err(anyhow!("Trigger timeout"));
                }

                tokio::time::sleep(Duration::from_millis(1)).await;
                tokio::task::yield_now().await;
            }

            // Cleanup the sequence on success
            // SAFETY: pl_exp_finish_seq completes sequence after successful acquisition.
            // Camera handle h and frame pointer are valid.
            unsafe {
                if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                }
            }
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Mock: just simulate the acquisition time
            let exposure = self.exposure_ms.get();
            tokio::time::sleep(Duration::from_millis(exposure as u64)).await;
        }

        // Increment frame count and disarm
        self.frame_count.fetch_add(1, Ordering::SeqCst);
        *self.armed.lock().await = false;

        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(*self.armed.lock().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exposure_setting() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Set exposure to 0.05 seconds (50 ms)
        camera.set_exposure(0.05).await.unwrap();
        assert_eq!(camera.get_exposure().await.unwrap(), 0.05);
    }

    #[tokio::test]
    async fn test_binning() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        camera.set_binning(2, 2).await.unwrap();
        assert_eq!(camera.binning().await, (2, 2));

        // Invalid binning
        assert!(camera.set_binning(3, 3).await.is_err());
    }

    #[tokio::test]
    async fn test_roi() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        let roi = Roi {
            x: 100,
            y: 100,
            width: 512,
            height: 512,
        };

        camera.set_roi(roi).await.unwrap();
        assert_eq!(camera.roi().await, roi);
    }

    #[tokio::test]
    async fn test_triggerable_arm() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Camera should not be armed initially
        assert!(!*camera.armed.lock().await);

        // Arm the camera
        camera.arm().await.unwrap();
        assert!(*camera.armed.lock().await);
    }

    #[tokio::test]
    async fn test_triggerable_trigger_without_arm() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Triggering without arming should fail
        assert!(camera.trigger().await.is_err());
    }

    #[tokio::test]
    async fn test_triggerable_trigger_with_arm() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Arm the camera
        camera.arm().await.unwrap();

        // Now triggering should succeed
        assert!(camera.trigger().await.is_ok());
    }

    #[tokio::test]
    async fn test_frame_producer_traits() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Test resolution
        let (width, height) = camera.resolution();
        assert_eq!((width, height), (2048, 2048));

        // Test streaming
        assert!(camera.start_stream().await.is_ok());
        assert!(camera.stop_stream().await.is_ok());
    }

    #[tokio::test]
    async fn test_streaming_double_start() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Start streaming
        camera.start_stream().await.unwrap();

        // Second start should fail
        assert!(camera.start_stream().await.is_err());

        // Stop streaming
        camera.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn test_streaming_stop_without_start() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();

        // Stop without start should be OK (no-op)
        assert!(camera.stop_stream().await.is_ok());
    }

    #[tokio::test]
    #[cfg(not(feature = "pvcam_hardware"))]
    async fn test_mock_frame_acquisition() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();
        camera.set_exposure(0.01).await.unwrap(); // 10ms for fast test

        let frame = camera.acquire_frame_mock().await.unwrap();
        assert_eq!(frame.len(), 2048 * 2048);

        // Verify test pattern
        assert_eq!(frame[0], 0);
        assert_eq!(frame[1], 1);
    }

    #[tokio::test]
    #[cfg(not(feature = "pvcam_hardware"))]
    async fn test_mock_streaming() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();
        camera.set_exposure(0.01).await.unwrap(); // 10ms for fast test

        // Subscribe to the frame broadcast before starting stream
        let mut rx = camera.subscribe_frames();

        // Verify not streaming initially
        assert!(!camera.is_streaming());

        // Start streaming
        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming());

        // Wait for a few frames
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should have received some frames (broadcast recv returns Result<Arc<Frame>, _>)
        let mut frame_count = 0;
        while let Ok(frame) = rx.try_recv() {
            frame_count += 1;
            assert_eq!(frame.width, 2048);
            assert_eq!(frame.height, 2048);
            if frame_count >= 3 {
                break;
            }
        }

        // Stop streaming
        camera.stop_stream().await.unwrap();
        assert!(!camera.is_streaming());

        // Verify we got frames
        assert!(frame_count > 0, "Should have received at least one frame");
    }

    #[tokio::test]
    async fn test_streaming_with_binning() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();
        camera.set_exposure(0.01).await.unwrap(); // 10ms for fast test

        // Set binning to 2x2
        camera.set_binning(2, 2).await.unwrap();

        // Subscribe to the frame broadcast before starting stream
        let mut rx = camera.subscribe_frames();

        // Start streaming with binning
        camera.start_stream().await.unwrap();

        // Wait for a frame
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Receive frame and verify binned dimensions (broadcast recv returns Result)
        let frame = rx.recv().await.expect("Should receive frame");

        // Full sensor is 2048x2048, with 2x2 binning should be 1024x1024
        assert_eq!(
            frame.width, 1024,
            "Frame width should be binned (2048 / 2 = 1024)"
        );
        assert_eq!(
            frame.height, 1024,
            "Frame height should be binned (2048 / 2 = 1024)"
        );

        // Verify pixel data is generated in binned coordinates
        assert_eq!(
            frame.buffer.len(),
            1024 * 1024,
            "Pixel count should match binned dimensions"
        );

        // Stop streaming
        camera.stop_stream().await.unwrap();
    }
}
