//! PVCAM Connection Management
//!
//! Handles SDK initialization, camera opening/closing, and resource cleanup.

use anyhow::{anyhow, Context, Result};
use std::ffi::CString;
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

/// Helper to get PVCAM error string
#[cfg(feature = "pvcam_hardware")]
pub(crate) fn get_pvcam_error() -> String {
    unsafe {
        let err_code = pl_error_code();
        let mut err_msg = vec![0i8; 256];
        pl_error_message(err_code, err_msg.as_mut_ptr());
        let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
        format!("error {} - {}", err_code, err_str)
    }
}

/// Manages the connection to the PVCAM SDK and a specific camera.
pub struct PvcamConnection {
    /// Camera handle from PVCAM SDK
    #[cfg(feature = "pvcam_hardware")]
    handle: Option<i16>,
    /// Whether SDK is initialized
    #[cfg(feature = "pvcam_hardware")]
    sdk_initialized: bool,
}

impl PvcamConnection {
    /// Create a new, unconnected connection manager.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "pvcam_hardware")]
            handle: None,
            #[cfg(feature = "pvcam_hardware")]
            sdk_initialized: false,
        }
    }

    /// Initialize the PVCAM SDK.
    ///
    /// This must be called before opening a camera.
    #[cfg(feature = "pvcam_hardware")]
    pub fn initialize(&mut self) -> Result<()> {
        if self.sdk_initialized {
            return Ok(());
        }

        unsafe {
            if pl_pvcam_init() == 0 {
                return Err(anyhow!("Failed to initialize PVCAM SDK: {}", get_pvcam_error()));
            }
        }
        self.sdk_initialized = true;
        Ok(())
    }

    /// Open a camera by name.
    ///
    /// If name is not found, tries to open the first available camera.
    #[cfg(feature = "pvcam_hardware")]
    pub fn open(&mut self, camera_name: &str) -> Result<()> {
        if !self.sdk_initialized {
            return Err(anyhow!("SDK not initialized"));
        }
        if self.handle.is_some() {
            return Ok(()); // Already open
        }

        // Get camera count
        let mut total_cameras: i16 = 0;
        unsafe {
            if pl_cam_get_total(&mut total_cameras) == 0 {
                return Err(anyhow!("Failed to get camera count: {}", get_pvcam_error()));
            }
        }

        if total_cameras == 0 {
            return Err(anyhow!("No PVCAM cameras detected"));
        }

        let camera_name_cstr = CString::new(camera_name).context("Invalid camera name")?;
        let mut hcam: i16 = 0;

        unsafe {
            if pl_cam_open(camera_name_cstr.as_ptr() as *mut i8, &mut hcam, 0) == 0 {
                // Try first available camera
                let mut name_buffer = vec![0i8; 256];
                if pl_cam_get_name(0, name_buffer.as_mut_ptr()) != 0 {
                    if pl_cam_open(name_buffer.as_mut_ptr(), &mut hcam, 0) == 0 {
                        return Err(anyhow!("Failed to open any camera"));
                    }
                } else {
                    return Err(anyhow!("Failed to open camera: {}", camera_name));
                }
            }
        }

        self.handle = Some(hcam);
        Ok(())
    }

    /// Close the camera if open.
    #[cfg(feature = "pvcam_hardware")]
    pub fn close(&mut self) {
        if let Some(h) = self.handle.take() {
            unsafe {
                pl_cam_close(h);
            }
        }
    }

    /// Uninitialize the SDK.
    #[cfg(feature = "pvcam_hardware")]
    pub fn uninitialize(&mut self) {
        self.close(); // Ensure camera closed first
        if self.sdk_initialized {
            unsafe {
                pl_pvcam_uninit();
            }
            self.sdk_initialized = false;
        }
    }

    /// Get the raw camera handle.
    #[cfg(feature = "pvcam_hardware")]
    pub fn handle(&self) -> Option<i16> {
        self.handle
    }
}

#[cfg(feature = "pvcam_hardware")]
impl Drop for PvcamConnection {
    fn drop(&mut self) {
        self.uninitialize();
    }
}
