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

use crate::hardware::capabilities::{ExposureControl, FrameProducer, Triggerable};
use crate::hardware::{Frame, Roi};
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
    exposure_ms: Arc<Mutex<f64>>,
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
    /// Channel sender for streaming frames
    frame_tx: tokio::sync::mpsc::Sender<Frame>,
    /// Channel receiver for streaming frames (stored for consumer access)
    frame_rx: Arc<Mutex<Option<tokio::sync::mpsc::Receiver<Frame>>>>,
    /// Handle to the streaming poll task
    #[cfg(feature = "pvcam_hardware")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Circular buffer for continuous acquisition (hardware only)
    #[cfg(feature = "pvcam_hardware")]
    circ_buffer: Arc<Mutex<Option<Vec<u16>>>>,
    /// Trigger frame buffer - holds the frame during triggered acquisition
    #[cfg(feature = "pvcam_hardware")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Get the last PVCAM error as a formatted string
#[cfg(feature = "pvcam_hardware")]
fn get_pvcam_error() -> String {
    unsafe {
        let err_code = pl_error_code();
        let mut err_msg = vec![0i8; 256];
        pl_error_message(err_code, err_msg.as_mut_ptr());
        let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
        format!("error {} - {}", err_code, err_str)
    }
}

impl PvcamDriver {
    /// Create a new PVCAM driver instance
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
    fn new_with_hardware(camera_name: &str) -> Result<Self> {
        // Initialize PVCAM SDK
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

        // Create channel for streaming frames (buffer 16 frames)
        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel(16);

        Ok(Self {
            camera_name: camera_name.to_string(),
            camera_handle: Arc::new(Mutex::new(Some(camera_handle))),
            exposure_ms: Arc::new(Mutex::new(100.0)),
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
            frame_rx: Arc::new(Mutex::new(Some(frame_rx))),
            poll_handle: Arc::new(Mutex::new(None)),
            circ_buffer: Arc::new(Mutex::new(None)),
            trigger_frame: Arc::new(Mutex::new(None)),
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

        // Create channel for streaming frames (buffer 16 frames)
        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel(16);

        Ok(Self {
            camera_name: camera_name.to_string(),
            exposure_ms: Arc::new(Mutex::new(100.0)),
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
            frame_rx: Arc::new(Mutex::new(Some(frame_rx))),
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
        let handle = *self.camera_handle.lock().await;
        if handle.is_none() {
            return Err(anyhow!("Camera not opened"));
        }
        let h = handle.unwrap();

        let exposure = *self.exposure_ms.lock().await;
        let roi = *self.roi.lock().await;
        let (x_bin, y_bin) = *self.binning.lock().await;

        // Setup region for acquisition
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
                if pl_exp_check_status(h, &mut status, &mut bytes_arrived) == 0 {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                    return Err(anyhow!("Failed to check acquisition status"));
                }

                if status == READOUT_COMPLETE || status == READOUT_FAILED {
                    break;
                }

                if start.elapsed() > timeout {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                    return Err(anyhow!("Acquisition timeout"));
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
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
        let exposure = *self.exposure_ms.lock().await;
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
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            let h = handle.unwrap();

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
            };

            // Always finish the sequence and cleanup on exit
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

    /// Get the frame receiver for consuming streamed frames
    ///
    /// Returns the receiver, which can only be taken once. Subsequent calls return None.
    pub async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<Frame>> {
        self.frame_rx.lock().await.take()
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
        unsafe {
            if pl_get_param(h, PARAM_TEMP, ATTR_CURRENT, &mut temp_raw as *mut _ as *mut _) == 0 {
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
        unsafe {
            if pl_get_param(h, PARAM_CHIP_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get chip name: {}", get_pvcam_error()));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
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
        unsafe {
            if pl_get_param(h, PARAM_BIT_DEPTH, ATTR_CURRENT, &mut depth as *mut _ as *mut _) == 0 {
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
        unsafe {
            if pl_get_param(h, PARAM_READOUT_TIME, ATTR_CURRENT, &mut time_us as *mut _ as *mut _) == 0 {
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
        unsafe {
            if pl_get_param(h, PARAM_PIX_SER_SIZE, ATTR_CURRENT, &mut pix_ser as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get serial pixel size: {}", get_pvcam_error()));
            }
            if pl_get_param(h, PARAM_PIX_PAR_SIZE, ATTR_CURRENT, &mut pix_par as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get parallel pixel size: {}", get_pvcam_error()));
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
        unsafe {
            if pl_get_param(h, PARAM_GAIN_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get gain name: {}", get_pvcam_error()));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
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
        unsafe {
            if pl_get_param(h, PARAM_SPDTAB_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) == 0 {
                return Err(anyhow!("Failed to get speed table name: {}", get_pvcam_error()));
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
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
        unsafe {
            if pl_get_param(h, PARAM_GAIN_INDEX, ATTR_CURRENT, &mut idx as *mut _ as *mut _) == 0 {
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
        unsafe {
            if pl_get_param(h, PARAM_SPDTAB_INDEX, ATTR_CURRENT, &mut idx as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get speed table index: {}", get_pvcam_error()));
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
            chip_name: self.get_chip_name().await.unwrap_or_else(|_| "Unknown".to_string()),
            temperature_c: self.get_temperature().await.unwrap_or(f64::NAN),
            bit_depth: self.get_bit_depth().await.unwrap_or(0),
            readout_time_us: self.get_readout_time_us().await.unwrap_or(0.0),
            pixel_size_nm: self.get_pixel_size_nm().await.unwrap_or((0, 0)),
            sensor_size: (self.sensor_width, self.sensor_height),
            gain_name: self.get_gain_name().await.unwrap_or_else(|_| "Unknown".to_string()),
            speed_name: self.get_speed_name().await.unwrap_or_else(|_| "Unknown".to_string()),
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
        unsafe {
            if pl_get_param(h, PARAM_GAIN_INDEX, ATTR_MAX, &mut max_idx as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get max gain index: {}", get_pvcam_error()));
            }
        }

        // Save current gain index so we can restore it
        let mut current_idx: i16 = 0;
        unsafe {
            pl_get_param(h, PARAM_GAIN_INDEX, ATTR_CURRENT, &mut current_idx as *mut _ as *mut _);
        }

        let mut modes = Vec::new();
        for idx in 0..=max_idx {
            // Set gain index temporarily to read its name
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
        unsafe {
            pl_set_param(h, PARAM_GAIN_INDEX, &current_idx as *const _ as *mut _);
        }

        Ok(modes)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn list_gain_modes(&self) -> Result<Vec<GainMode>> {
        Ok(vec![
            GainMode { index: 0, name: "HDR".to_string() },
            GainMode { index: 1, name: "High Sensitivity".to_string() },
            GainMode { index: 2, name: "Full Well".to_string() },
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
        let name = self.get_gain_name().await.unwrap_or_else(|_| "Unknown".to_string());
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
        unsafe {
            if pl_get_param(h, PARAM_SPDTAB_INDEX, ATTR_MAX, &mut max_idx as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get max speed index: {}", get_pvcam_error()));
            }
        }

        // Save current speed index so we can restore it
        let mut current_idx: i16 = 0;
        unsafe {
            pl_get_param(h, PARAM_SPDTAB_INDEX, ATTR_CURRENT, &mut current_idx as *mut _ as *mut _);
        }

        let mut modes = Vec::new();
        for idx in 0..=max_idx {
            // Set speed index temporarily to read its name
            unsafe {
                let idx_i16 = idx as i16;
                if pl_set_param(h, PARAM_SPDTAB_INDEX, &idx_i16 as *const _ as *mut _) == 0 {
                    continue; // Skip if setting fails
                }

                // Try to read speed name (may not be available on all cameras)
                let mut buf = vec![0i8; 256];
                let name = if pl_get_param(h, PARAM_SPDTAB_NAME, ATTR_CURRENT, buf.as_mut_ptr() as *mut _) != 0 {
                    std::ffi::CStr::from_ptr(buf.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                } else {
                    // Speed name not available, try to get pixel time instead
                    let mut pix_time: uns16 = 0;
                    if pl_get_param(h, PARAM_PIX_TIME, ATTR_CURRENT, &mut pix_time as *mut _ as *mut _) != 0 {
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
        unsafe {
            pl_set_param(h, PARAM_SPDTAB_INDEX, &current_idx as *const _ as *mut _);
        }

        Ok(modes)
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn list_speed_modes(&self) -> Result<Vec<SpeedMode>> {
        Ok(vec![
            SpeedMode { index: 0, name: "100 MHz".to_string() },
            SpeedMode { index: 1, name: "200 MHz".to_string() },
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
        let name = self.get_speed_name().await.unwrap_or_else(|_| "Unknown".to_string());
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
        unsafe {
            if pl_get_param(h, PARAM_TEMP_SETPOINT, ATTR_CURRENT, &mut temp_raw as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get temperature setpoint: {}", get_pvcam_error()));
            }
        }
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
        unsafe {
            if pl_set_param(h, PARAM_TEMP_SETPOINT, &temp_raw as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to set temperature setpoint: {}", get_pvcam_error()));
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
        unsafe {
            if pl_get_param(h, PARAM_FAN_SPEED_SETPOINT, ATTR_CURRENT, &mut speed as *mut _ as *mut _) == 0 {
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
        unsafe {
            if pl_set_param(h, PARAM_FAN_SPEED_SETPOINT, &speed_val as *const _ as *mut _) == 0 {
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
        unsafe {
            if pl_get_param(h, PARAM_PP_INDEX, ATTR_COUNT, &mut count as *mut _ as *mut _) == 0 {
                // PP features may not be available on all cameras
                return Ok(Vec::new());
            }
        }

        let mut features = Vec::new();
        for idx in 0..count {
            // Set PP index to select this feature
            unsafe {
                if pl_set_param(h, PARAM_PP_INDEX, &idx as *const _ as *mut _) == 0 {
                    continue; // Skip this feature if we can't select it
                }

                // Get feature ID
                let mut feat_id: u16 = 0;
                if pl_get_param(h, PARAM_PP_FEAT_ID, ATTR_CURRENT, &mut feat_id as *mut _ as *mut _) == 0 {
                    continue;
                }

                // Get feature name
                let mut name_buf = vec![0i8; 256];
                if pl_get_param(h, PARAM_PP_FEAT_NAME, ATTR_CURRENT, name_buf.as_mut_ptr() as *mut _) == 0 {
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
            PPFeature { index: 0, id: 1, name: "Defect Correction".to_string() },
            PPFeature { index: 1, id: 2, name: "Background Subtraction".to_string() },
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
        unsafe {
            if pl_set_param(h, PARAM_PP_INDEX, &idx as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to select PP feature {}: {}", feature_index, get_pvcam_error()));
            }

            // Get count of parameters for this feature
            let mut count: i16 = 0;
            if pl_get_param(h, PARAM_PP_PARAM_INDEX, ATTR_COUNT, &mut count as *mut _ as *mut _) == 0 {
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
                if pl_get_param(h, PARAM_PP_PARAM_ID, ATTR_CURRENT, &mut param_id as *mut _ as *mut _) == 0 {
                    continue;
                }

                // Get parameter name
                let mut name_buf = vec![0i8; 256];
                if pl_get_param(h, PARAM_PP_PARAM_NAME, ATTR_CURRENT, name_buf.as_mut_ptr() as *mut _) == 0 {
                    continue;
                }
                let name = std::ffi::CStr::from_ptr(name_buf.as_ptr())
                    .to_string_lossy()
                    .into_owned();

                // Get current value
                let mut value: u32 = 0;
                if pl_get_param(h, PARAM_PP_PARAM, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
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
            PPParam { index: 0, id: 1, name: "Enabled".to_string(), value: 1 },
            PPParam { index: 1, id: 2, name: "Threshold".to_string(), value: 100 },
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

        unsafe {
            // Select the PP feature
            if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to select PP feature {}: {}", feature_index, get_pvcam_error()));
            }

            // Select the parameter
            if pl_set_param(h, PARAM_PP_PARAM_INDEX, &par_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to select PP param {}: {}", param_index, get_pvcam_error()));
            }

            // Get the value
            let mut value: u32 = 0;
            if pl_get_param(h, PARAM_PP_PARAM, ATTR_CURRENT, &mut value as *mut _ as *mut _) == 0 {
                return Err(anyhow!("Failed to get PP param value: {}", get_pvcam_error()));
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
    pub async fn set_pp_param(&self, feature_index: u16, param_index: u16, value: u32) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        let feat_idx = feature_index as i16;
        let par_idx = param_index as i16;

        unsafe {
            // Select the PP feature
            if pl_set_param(h, PARAM_PP_INDEX, &feat_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to select PP feature {}: {}", feature_index, get_pvcam_error()));
            }

            // Select the parameter
            if pl_set_param(h, PARAM_PP_PARAM_INDEX, &par_idx as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to select PP param {}: {}", param_index, get_pvcam_error()));
            }

            // Set the value
            if pl_set_param(h, PARAM_PP_PARAM, &value as *const _ as *mut _) == 0 {
                return Err(anyhow!("Failed to set PP param value: {}", get_pvcam_error()));
            }

            Ok(())
        }
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn set_pp_param(&self, _feature_index: u16, _param_index: u16, _value: u32) -> Result<()> {
        Ok(())
    }

    /// Reset all post-processing features to default values
    #[cfg(feature = "pvcam_hardware")]
    pub async fn reset_pp_features(&self) -> Result<()> {
        let guard = self.camera_handle.lock().await;
        let h = guard.ok_or_else(|| anyhow!("Camera not open"))?;

        unsafe {
            if pl_pp_reset(h) == 0 {
                return Err(anyhow!("Failed to reset PP features: {}", get_pvcam_error()));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "pvcam_hardware"))]
    pub async fn reset_pp_features(&self) -> Result<()> {
        Ok(())
    }

    /// Hardware polling loop for continuous acquisition
    ///
    /// This runs in a blocking thread and polls the PVCAM SDK for new frames.
    /// Uses get_oldest_frame + unlock_oldest_frame pattern for ordered, lossless delivery.
    #[cfg(feature = "pvcam_hardware")]
    fn poll_loop_hardware(
        hcam: i16,
        streaming: Arc<AtomicBool>,
        frame_tx: tokio::sync::mpsc::Sender<Frame>,
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

                            // Send frame (non-blocking, log if channel full)
                            if frame_tx.try_send(frame).is_err() {
                                eprintln!("PVCAM: Frame channel full, frame dropped");
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

        // Cleanup: stop acquisition (poll loop owns this now)
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
                    unsafe {
                        pl_cam_close(h);
                    }
                }
            }

            if let Ok(initialized) = self.sdk_initialized.try_lock() {
                if *initialized {
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
            let h = handle_guard.ok_or_else(|| anyhow!("Camera not opened"))?;

            // Get current settings
            let roi = *self.roi.lock().await;
            let (x_bin, y_bin) = *self.binning.lock().await;
            let exp_ms = *self.exposure_ms.lock().await as uns32;

            // Setup region
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
            let exposure_ms = *self.exposure_ms.lock().await;
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

                    // Send frame (non-blocking, drop if channel full)
                    let _ = frame_tx.try_send(frame);
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

    async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<Frame>> {
        self.frame_rx.lock().await.take()
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

        *self.exposure_ms.lock().await = exposure_ms;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(*self.exposure_ms.lock().await / 1000.0) // Convert ms to seconds
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
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            let h = handle.unwrap();

            let exposure = *self.exposure_ms.lock().await;
            let roi = *self.roi.lock().await;
            let (x_bin, y_bin) = *self.binning.lock().await;

            // Setup region for acquisition
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
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            let h = handle.unwrap();

            let exposure = *self.exposure_ms.lock().await;

            // Wait for acquisition to complete
            let mut status: i16 = 0;
            let mut bytes_arrived: uns32 = 0;

            let timeout = Duration::from_millis((exposure + 5000.0) as u64);
            let start = std::time::Instant::now();

            loop {
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
                    unsafe {
                        if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                            pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                        }
                    }
                    *self.armed.lock().await = false;
                    return Err(anyhow!("Trigger timeout"));
                }

                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            // Cleanup the sequence on success
            unsafe {
                if let Some(mut frame) = self.trigger_frame.lock().await.take() {
                    pl_exp_finish_seq(h, frame.as_mut_ptr() as *mut _, 0);
                }
            }
        }

        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Mock: just simulate the acquisition time
            let exposure = *self.exposure_ms.lock().await;
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
    async fn test_combined_traits() {
        let camera = PvcamDriver::new("Prime95B").unwrap();

        // Verify resolution
        assert_eq!(camera.resolution(), (1200, 1200));

        // Set up exposure
        camera.set_exposure(0.1).await.unwrap();
        assert_eq!(camera.get_exposure().await.unwrap(), 0.1);

        // Arm and trigger
        camera.arm().await.unwrap();
        camera.trigger().await.unwrap();

        // Stream control
        camera.start_stream().await.unwrap();
        camera.stop_stream().await.unwrap();
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

        // Take the receiver before starting stream
        let mut rx = camera
            .take_frame_receiver()
            .await
            .expect("Should get receiver");

        // Verify not streaming initially
        assert!(!camera.is_streaming());

        // Start streaming
        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming());

        // Wait for a few frames
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should have received some frames
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
    async fn test_mock_streaming_with_binning() {
        let camera = PvcamDriver::new("PrimeBSI").unwrap();
        camera.set_exposure(0.01).await.unwrap(); // 10ms for fast test

        // Set binning to 2x2
        camera.set_binning(2, 2).await.unwrap();

        // Take the receiver before starting stream
        let mut rx = camera
            .take_frame_receiver()
            .await
            .expect("Should get receiver");

        // Start streaming with binning
        camera.start_stream().await.unwrap();

        // Wait for a frame
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Receive frame and verify binned dimensions
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
