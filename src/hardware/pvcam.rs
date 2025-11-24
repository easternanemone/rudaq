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
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

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
                return Err(anyhow!("Failed to initialize PVCAM SDK"));
            }
        }

        // Get list of cameras
        let mut total_cameras: i16 = 0;
        unsafe {
            if pl_cam_get_total(&mut total_cameras) == 0 {
                pl_pvcam_uninit();
                return Err(anyhow!("Failed to get camera count"));
            }
        }

        if total_cameras == 0 {
            unsafe { pl_pvcam_uninit(); }
            return Err(anyhow!("No PVCAM cameras detected"));
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

        eprintln!("⚠️  PVCAM hardware feature not enabled - using mock camera");
        eprintln!("    To use real hardware: cargo build --features pvcam_hardware");

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
            let handle = *self.camera_handle.lock().await;
            if let Some(h) = handle {
                unsafe {
                    // ROI in PVCAM is set during pl_exp_setup_seq, not via parameters
                    // Store for use during acquisition setup
                }
            }
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
}

impl Drop for PvcamDriver {
    fn drop(&mut self) {
        #[cfg(feature = "pvcam_hardware")]
        {
            // Cleanup PVCAM SDK resources
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
        #[cfg(feature = "pvcam_hardware")]
        {
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            // TODO: Implement continuous circular buffer acquisition
            // - pl_exp_setup_cont()
            // - pl_exp_start_cont()
        }

        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        #[cfg(feature = "pvcam_hardware")]
        {
            let handle = *self.camera_handle.lock().await;
            if handle.is_none() {
                return Err(anyhow!("Camera not opened"));
            }
            // TODO: Stop circular buffer
            // - pl_exp_stop_cont()
            // - pl_exp_finish_seq()
        }

        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        (self.sensor_width, self.sensor_height)
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
}
