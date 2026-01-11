//! PVCAM Connection Management
//!
//! Handles SDK initialization, camera opening/closing, and resource cleanup.
//!
//! ## SDK Reference Counting (bd-9ou9)
//!
//! The PVCAM SDK uses global state: `pl_pvcam_init()` and `pl_pvcam_uninit()` affect
//! the entire process. To support multiple `PvcamDriver` instances, we use a global
//! reference counter. The SDK is only uninitialized when the last connection closes.

// Common imports for all configurations
use anyhow::Result;

#[cfg(feature = "pvcam_hardware")]
use anyhow::{anyhow, Context};
#[cfg(feature = "pvcam_hardware")]
use std::ffi::CString;
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(feature = "pvcam_hardware")]
use std::sync::Mutex;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;

/// Global reference counter for PVCAM SDK initialization (bd-9ou9).
///
/// The SDK uses global state, so we track how many connections exist.
/// - When count goes 0 → 1: call pl_pvcam_init()
/// - When count goes 1 → 0: call pl_pvcam_uninit()
#[cfg(feature = "pvcam_hardware")]
static SDK_REF_COUNT: AtomicU32 = AtomicU32::new(0);

/// Mutex to ensure atomic increment + init and decrement + uninit.
#[cfg(feature = "pvcam_hardware")]
static SDK_INIT_MUTEX: Mutex<()> = Mutex::new(());

/// Helper to get PVCAM error string
#[cfg(feature = "pvcam_hardware")]
pub(crate) fn get_pvcam_error() -> String {
    unsafe {
        // SAFETY: PVCAM docs state error query functions are thread-safe after initialization.
        let err_code = pl_error_code();
        let mut err_msg = vec![0i8; 256];
        // SAFETY: Buffer is valid and sized per SDK requirement (256 bytes).
        pl_error_message(err_code, err_msg.as_mut_ptr());
        let err_str = std::ffi::CStr::from_ptr(err_msg.as_ptr()).to_string_lossy();
        format!("error {} - {}", err_code, err_str)
    }
}

/// Manages the connection to the PVCAM SDK and a specific camera.
#[derive(Default)]
pub struct PvcamConnection {
    /// Camera handle from PVCAM SDK
    #[cfg(feature = "pvcam_hardware")]
    handle: Option<i16>,
    /// Whether SDK is initialized
    #[cfg(feature = "pvcam_hardware")]
    sdk_initialized: bool,

    /// Mock state for testing without hardware
    #[cfg(not(feature = "pvcam_hardware"))]
    pub mock_state: std::sync::Mutex<MockCameraState>,
}

#[cfg(not(feature = "pvcam_hardware"))]
#[derive(Debug, Clone)]
pub struct MockCameraState {
    pub temperature_c: f64,
    pub temperature_setpoint_c: f64,
    pub fan_speed: i32, // Store as raw int to match simpler mocking, or use clean types if easy
    pub exposure_mode: i32,
    pub clear_mode: i32,
    pub expose_out_mode: i32,
    pub shutter_mode: i32,
    pub shutter_open_delay_us: u32,
    pub shutter_close_delay_us: u32,
    pub smart_stream_enabled: bool,
    pub smart_stream_mode: i32,
    pub readout_port_index: u16,
    pub speed_index: u16,
    pub gain_index: u16,
}

#[cfg(not(feature = "pvcam_hardware"))]
impl Default for MockCameraState {
    fn default() -> Self {
        Self {
            temperature_c: 25.0,
            temperature_setpoint_c: -10.0,
            fan_speed: 0,       // High
            exposure_mode: 0,   // Timed
            clear_mode: 1,      // PreExposure
            expose_out_mode: 0, // FirstRow
            shutter_mode: 0,    // Normal
            shutter_open_delay_us: 10,
            shutter_close_delay_us: 10,
            smart_stream_enabled: false,
            smart_stream_mode: 0, // Exposures
            readout_port_index: 0,
            speed_index: 0,
            gain_index: 0,
        }
    }
}

impl PvcamConnection {
    /// Create a new, unconnected connection manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize the PVCAM SDK.
    ///
    /// This must be called before opening a camera.
    ///
    /// Uses global reference counting (bd-9ou9) to ensure the SDK is only
    /// initialized once, even with multiple PvcamDriver instances.
    #[cfg(feature = "pvcam_hardware")]
    pub fn initialize(&mut self) -> Result<()> {
        if self.sdk_initialized {
            return Ok(());
        }

        // Lock to ensure atomic check-and-init.
        // Recover from poison since we need to manage ref count consistently (bd-vw80).
        let _guard = match SDK_INIT_MUTEX.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("SDK init mutex poisoned during initialize - recovering (bd-vw80)");
                poisoned.into_inner()
            }
        };

        // Increment ref count first, then init if we're the first
        let prev_count = SDK_REF_COUNT.fetch_add(1, Ordering::SeqCst);

        if prev_count == 0 {
            // We're the first connection - initialize the SDK
            unsafe {
                // SAFETY: Global PVCAM init; protected by SDK_INIT_MUTEX.
                if pl_pvcam_init() == 0 {
                    // Rollback ref count on failure
                    SDK_REF_COUNT.fetch_sub(1, Ordering::SeqCst);
                    return Err(anyhow!(
                        "Failed to initialize PVCAM SDK: {}",
                        get_pvcam_error()
                    ));
                }
            }
            tracing::info!("PVCAM SDK initialized (ref count: 1)");
        } else {
            tracing::debug!(
                "PVCAM SDK already initialized (ref count: {})",
                prev_count + 1
            );
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
            // SAFETY: total_cameras is a valid out pointer; SDK already initialized.
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
            // SAFETY: camera_name_cstr is a valid C string; hcam is a valid out pointer.
            if pl_cam_open(camera_name_cstr.as_ptr() as *mut i8, &mut hcam, 0) == 0 {
                // Try first available camera
                let mut name_buffer = vec![0i8; 256];
                // SAFETY: name_buffer is writable and sized per SDK requirement.
                if pl_cam_get_name(0, name_buffer.as_mut_ptr()) != 0 {
                    if pl_cam_open(name_buffer.as_mut_ptr(), &mut hcam, 0) == 0 {
                        return Err(anyhow!("Failed to open any camera: {}", get_pvcam_error()));
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
                // SAFETY: h was returned by pl_cam_open and is still owned by this connection.
                pl_cam_close(h);
            }
        }
    }

    /// Uninitialize the SDK.
    ///
    /// Uses global reference counting (bd-9ou9) to ensure the SDK is only
    /// uninitialized when the last connection closes.
    ///
    /// Recovers from mutex poisoning to ensure ref count is always decremented (bd-vw80).
    #[cfg(feature = "pvcam_hardware")]
    pub fn uninitialize(&mut self) {
        self.close(); // Ensure camera closed first

        if !self.sdk_initialized {
            return;
        }
        self.sdk_initialized = false;

        // Lock to ensure atomic check-and-uninit.
        // Use into_inner() to recover from poison - we MUST decrement ref count (bd-vw80).
        let _guard = match SDK_INIT_MUTEX.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!(
                    "SDK init mutex poisoned during uninitialize - recovering (bd-vw80)"
                );
                poisoned.into_inner()
            }
        };

        // Decrement ref count, then uninit if we're the last
        let prev_count = SDK_REF_COUNT.fetch_sub(1, Ordering::SeqCst);

        if prev_count == 1 {
            // We were the last connection - uninitialize the SDK
            unsafe {
                // SAFETY: Global PVCAM uninit; protected by SDK_INIT_MUTEX and ref count.
                pl_pvcam_uninit();
            }
            tracing::info!("PVCAM SDK uninitialized (last connection closed)");
        } else if prev_count == 0 {
            // This shouldn't happen - we decremented below zero
            tracing::error!("PVCAM SDK ref count underflow - this indicates a bug");
            SDK_REF_COUNT.store(0, Ordering::SeqCst);
        } else {
            tracing::debug!("PVCAM SDK still in use (ref count: {})", prev_count - 1);
        }
    }

    /// Get the raw camera handle.
    #[cfg(feature = "pvcam_hardware")]
    pub fn handle(&self) -> Option<i16> {
        self.handle
    }

    /// List all available PVCAM cameras connected to the system.
    ///
    /// Returns a vector of camera names that can be used to open connections.
    /// The SDK must be initialized before calling this function.
    ///
    /// # Example
    /// ```ignore
    /// let conn = PvcamConnection::new();
    /// conn.initialize()?;
    /// let cameras = PvcamConnection::list_available_cameras()?;
    /// for name in cameras {
    ///     println!("Found camera: {}", name);
    /// }
    /// ```
    #[cfg(feature = "pvcam_hardware")]
    pub fn list_available_cameras() -> Result<Vec<String>> {
        // Note: SDK must be initialized before calling this
        // We check if ref count > 0 to verify SDK is ready
        let ref_count = SDK_REF_COUNT.load(Ordering::SeqCst);
        if ref_count == 0 {
            return Err(anyhow!("PVCAM SDK not initialized. Call initialize() first."));
        }

        let mut total_cameras: i16 = 0;
        unsafe {
            // SAFETY: total_cameras is a valid out pointer; SDK already initialized.
            if pl_cam_get_total(&mut total_cameras) == 0 {
                return Err(anyhow!("Failed to get camera count: {}", get_pvcam_error()));
            }
        }

        let mut cameras = Vec::with_capacity(total_cameras as usize);

        for i in 0..total_cameras {
            let mut name_buffer = vec![0i8; 256];
            unsafe {
                // SAFETY: name_buffer is writable and sized per SDK requirement (256 bytes).
                if pl_cam_get_name(i, name_buffer.as_mut_ptr()) != 0 {
                    let name = std::ffi::CStr::from_ptr(name_buffer.as_ptr())
                        .to_string_lossy()
                        .into_owned();
                    cameras.push(name);
                } else {
                    tracing::warn!("Failed to get name for camera {}: {}", i, get_pvcam_error());
                }
            }
        }

        Ok(cameras)
    }

    /// List all available PVCAM cameras (mock mode).
    #[cfg(not(feature = "pvcam_hardware"))]
    pub fn list_available_cameras() -> Result<Vec<String>> {
        Ok(vec![
            "MockCamera".to_string(),
            "PrimeBSI".to_string(),
        ])
    }
}

#[cfg(feature = "pvcam_hardware")]
impl Drop for PvcamConnection {
    fn drop(&mut self) {
        self.uninitialize();
    }
}
