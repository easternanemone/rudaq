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
                return Err(anyhow!("Failed to initialize PVCAM SDK: error {} - {}", err_code, err_str));
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
                return Err(anyhow!("Failed to get camera count: error {} - {}", err_code, err_str));
            }
        }

        if total_cameras == 0 {
            unsafe { pl_pvcam_uninit(); }
            return Err(anyhow!("No PVCAM cameras detected (is pvcam_usb daemon running?)"));
        }

        // Find camera by name or use first camera
        let mut camera_handle: i16 = 0;
        let camera_name_cstr = std::ffi::CString::new(camera_name)
            .context("Invalid camera name")?;

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
            if pl_get_param(camera_handle, PARAM_SER_SIZE, ATTR_CURRENT, &mut par_width as *mut _ as *mut _) != 0 {
                width = par_width as uns32;
            }
            if pl_get_param(camera_handle, PARAM_PAR_SIZE, ATTR_CURRENT, &mut par_height as *mut _ as *mut _) != 0 {
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

            if pl_exp_setup_seq(h, 1, 1, &region as *const _ as *const _, TIMED_MODE, exp_time_ms, &mut total_bytes) == 0 {
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
    pub async fn disarm(&self) -> Result<()> {
        *self.armed.lock().await = false;
        Ok(())
    }

    /// Wait for external trigger (for testing triggered mode)
    ///
    /// In mock mode, this just waits briefly. In hardware mode,
    /// this would wait for actual trigger signal.
    pub async fn wait_for_trigger(&self) -> Result<()> {
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            // Simulate trigger wait
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        #[cfg(feature = "pvcam_hardware")]
        {
            // TODO: Implement actual trigger waiting logic
            // This would involve checking camera status and waiting for trigger
            tokio::time::sleep(Duration::from_millis(100)).await;
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
                if pl_exp_check_cont_status(
                    hcam,
                    &mut status,
                    &mut bytes_arrived,
                    &mut buffer_cnt,
                ) == 0
                {
                    eprintln!("PVCAM: Failed to check continuous status");
                    break;
                }

                match status {
                    s if s == READOUT_COMPLETE || s == EXPOSURE_IN_PROGRESS => {
                        // Use get_oldest_frame for ordered delivery (FIFO)
                        let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

                        if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) != 0 && !frame_ptr.is_null()
                        {
                            // Copy frame data BEFORE unlock
                            let src = std::slice::from_raw_parts(frame_ptr as *const u16, frame_pixels);
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
                    eprintln!("PVCAM: Acquisition timeout - no frames for {} iterations", no_frame_count);
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
                                tokio::time::timeout(
                                    Duration::from_secs(2),
                                    handle,
                                ).await
                            });
                        }
                    }).join();
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
                Self::poll_loop_hardware(h, streaming, frame_tx, frame_count, frame_pixels, width, height);
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

            tokio::spawn(async move {
                while streaming.load(Ordering::SeqCst) {
                    // Simulate exposure time
                    tokio::time::sleep(Duration::from_millis(exposure_ms as u64)).await;

                    if !streaming.load(Ordering::SeqCst) {
                        break;
                    }

                    // Generate synthetic frame
                    let frame_size = (roi.width * roi.height) as usize;
                    let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                    let mut pixels = vec![0u16; frame_size];

                    // Create test pattern (gradient + frame number)
                    for y in 0..roi.height {
                        for x in 0..roi.width {
                            let value = (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                            pixels[(y * roi.width + x) as usize] = value;
                        }
                    }

                    let frame = Frame::new(roi.width, roi.height, pixels);

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
    async fn arm(&self) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        {
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            // TODO: Setup for triggered acquisition
            // - pl_exp_setup_seq() with TRIGGER_FIRST_MODE
        }

        *self.armed.lock().await = true;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        let is_armed = *self.armed.lock().await;
        if !is_armed {
            return Err(anyhow!("Camera must be armed before triggering"));
        }

        // Acquire a frame
        self.acquire_frame_internal().await?;

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
        let mut rx = camera.take_frame_receiver().await.expect("Should get receiver");

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
}
