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
use crate::hardware::Roi;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Driver for Photometrics PVCAM cameras
///
/// Implements FrameProducer, ExposureControl, and Triggerable capability traits.
/// Uses PVCAM SDK for hardware communication.
pub struct PvcamDriver {
    /// Camera name (e.g., "PrimeBSI", "Prime95B")
    #[allow(dead_code)]
    camera_name: String,
    /// Current exposure time in milliseconds
    exposure_ms: Arc<Mutex<f64>>,
    /// Current ROI setting
    roi: Arc<Mutex<Roi>>,
    /// Binning factors (x, y)
    binning: Arc<Mutex<(u16, u16)>>,
    /// Frame buffer (simulated for now, real impl would use PVCAM SDK)
    #[allow(dead_code)]
    frame_buffer: Arc<Mutex<Vec<u16>>>,
    /// Sensor dimensions
    sensor_width: u32,
    sensor_height: u32,
    /// Whether the camera is armed for triggering
    armed: Arc<Mutex<bool>>,
}

impl PvcamDriver {
    /// Create a new PVCAM driver instance
    ///
    /// # Arguments
    /// * `camera_name` - Name of camera (e.g., "PrimeBSI")
    ///
    /// # Errors
    /// Returns error if camera cannot be opened
    ///
    /// # Note
    /// This is currently a mock implementation. Real implementation would:
    /// - Call pl_cam_open() from PVCAM SDK
    /// - Query camera capabilities
    /// - Initialize circular buffer
    pub fn new(camera_name: &str) -> Result<Self> {
        // TODO: Real PVCAM SDK initialization
        // - pl_pvcam_init()
        // - pl_cam_open()
        // - Query sensor size
        //
        // For now, use known Prime BSI dimensions
        let (width, height) = match camera_name {
            "PrimeBSI" => (2048, 2048),
            "Prime95B" => (1200, 1200),
            _ => (2048, 2048), // Default
        };

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

        *self.roi.lock().await = roi;
        Ok(())
    }

    /// Get current ROI
    pub async fn roi(&self) -> Roi {
        *self.roi.lock().await
    }

    /// Simulate frame acquisition (replace with real PVCAM SDK calls)
    ///
    /// Real implementation would:
    /// - pl_exp_setup_seq() to configure acquisition
    /// - pl_exp_start_seq() to start capture
    /// - pl_exp_check_status() to poll completion
    /// - pl_exp_get_latest_frame() to retrieve data
    #[allow(dead_code)]
    async fn acquire_frame_internal(&self) -> Result<Vec<u16>> {
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
}

#[async_trait]
impl FrameProducer for PvcamDriver {
    async fn start_stream(&self) -> Result<()> {
        // TODO: Start circular buffer acquisition
        // pl_exp_setup_cont()
        // pl_exp_start_cont()
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        // TODO: Stop circular buffer
        // pl_exp_stop_cont()
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
        // TODO: Real PVCAM SDK call
        // - pl_exp_setup_seq() to configure acquisition
        // - Initialize trigger waiting mode
        *self.armed.lock().await = true;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        let is_armed = *self.armed.lock().await;
        if !is_armed {
            return Err(anyhow!("Camera must be armed before triggering"));
        }

        // TODO: Real PVCAM SDK call
        // - pl_exp_start_seq() to start capture
        // - pl_exp_check_status() to poll completion
        // - pl_exp_get_latest_frame() to retrieve data

        // Simulate exposure delay
        let exposure = *self.exposure_ms.lock().await;
        tokio::time::sleep(Duration::from_millis(exposure as u64)).await;

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
}
