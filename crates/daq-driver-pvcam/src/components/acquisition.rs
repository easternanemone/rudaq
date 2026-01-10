//! PVCAM Acquisition Logic (bd-ek9n)
//!
//! Handles streaming, circular buffers, and frame acquisition with best-practices
//! frame loss detection, buffer management, and EOF callback signaling.
//!
//! # PVCAM Best Practices Implemented
//!
//! - **EOF Callback Acquisition (bd-ek9n.2)**: Uses `pl_cam_register_callback_ex3`
//!   with `PL_CALLBACK_EOF` to receive frame-ready notifications instead of polling.
//!   The callback signals a condvar, and the frame retrieval loop waits on the signal.
//!   This reduces CPU usage and latency compared to polling with sleep.
//!
//! - **Frame Loss Detection (bd-ek9n.3)**: Tracks `FRAME_INFO.FrameNr` discontinuities
//!   to detect and report dropped frames. Counters exposed via `lost_frames` and
//!   `discontinuity_events` for monitoring.
//!
//! - **Dynamic Buffer Sizing (bd-ek9n.4)**: Uses `PARAM_FRAME_BUFFER_SIZE` to
//!   calculate appropriate circular buffer size instead of fixed frame count.
//!
//! - **Frame Bytes Validation**: Uses actual `frame_bytes` from `pl_exp_setup_cont`
//!   rather than assuming `pixels * 2` to handle metadata/alignment correctly.
//!
//! # Acquisition Architecture (bd-ek9n.2)
//!
//! ```text
//! PVCAM SDK                    Rust Application
//! ┌─────────────────┐         ┌─────────────────────────────────┐
//! │ Camera Hardware │         │ CallbackContext                 │
//! │                 │         │ ├─ frame_ready: AtomicBool      │
//! │ EOF Interrupt ──┼────────►│ ├─ condvar: Condvar             │
//! │                 │ callback│ ├─ mutex: Mutex                 │
//! │                 │         │ └─ latest_frame_info: FRAME_INFO│
//! └─────────────────┘         └────────────┬────────────────────┘
//!                                          │ signal
//!                                          ▼
//!                             ┌─────────────────────────────────┐
//!                             │ Frame Retrieval Loop            │
//!                             │ ├─ wait on condvar              │
//!                             │ ├─ pl_exp_get_oldest_frame_ex   │
//!                             │ └─ broadcast Frame to channels  │
//!                             └─────────────────────────────────┘
//! ```
//!
//! # Frame Loss Detection
//!
//! The driver tracks hardware frame numbers via `FRAME_INFO.FrameNr` returned by
//! the EOF callback. When gaps are detected (current != prev + 1),
//! the `lost_frames` counter is incremented by the gap size and `discontinuity_events`
//! is incremented. This allows downstream consumers to know when data is missing.

#[cfg(feature = "pvcam_hardware")]
use crate::components::connection::get_pvcam_error;
use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_hardware")]
use crate::components::features::PvcamFeatures;
use anyhow::{anyhow, bail, Result};
use daq_core::core::Roi;
use daq_core::data::Frame;
use daq_core::parameter::Parameter;
#[cfg(feature = "pvcam_hardware")]
use std::alloc::{alloc_zeroed, dealloc, Layout};
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicI16;
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use tokio::task::JoinHandle;

/// bd-3gnv: Buffer mode selection for continuous streaming.
///
/// CIRC_OVERWRITE would avoid stalls by overwriting the oldest frame, but Prime BSI
/// returns error 185 (Invalid Configuration). We therefore run CIRC_NO_OVERWRITE and
/// drain the buffer strictly FIFO (get_oldest_frame + unlock_oldest_frame).
///
/// Reference: https://github.com/jbopp/dynexp/blob/main/src/DynExpManager/HardwareAdapters/HardwareAdapterPVCam.cpp
#[cfg(feature = "pvcam_hardware")]
const USE_CIRC_OVERWRITE_MODE: bool = false;

/// bd-3gnv: Prefer continuous FIFO streaming; keep sequence mode as a last-resort fallback.
///
/// When false: Use `pl_exp_setup_cont` + `pl_exp_start_cont` with CIRC_NO_OVERWRITE and the
/// FIFO drain path (get_oldest_frame + unlock_oldest_frame). This delivers higher sustained
/// throughput on Prime BSI when the buffer is drained correctly.
///
/// When true: Force sequence mode (`pl_exp_setup_seq` + `pl_exp_start_seq`) in batches. This
/// is slower but can be toggled for diagnostics if continuous mode regresses.
#[cfg(feature = "pvcam_hardware")]
const USE_SEQUENCE_MODE: bool = false;

/// Batch size for sequence mode streaming (bd-3gnv).
///
/// Smaller batches = lower latency, more restarts
/// Larger batches = higher throughput, less restart overhead
///
/// 10 frames at 10ms exposure = ~150ms batch time (balances latency + throughput)
#[cfg(feature = "pvcam_hardware")]
const SEQUENCE_BATCH_SIZE: u16 = 10;

/// Callback context for EOF notifications (bd-ek9n.2)
///
/// This structure is passed to the PVCAM callback and shared with the frame
/// retrieval loop. The callback increments `pending_frames` and notifies the condvar;
/// the loop waits on the condvar and drains all available frames.
///
/// Uses AtomicU32 counter instead of AtomicBool to avoid losing events when
/// multiple EOF callbacks fire while the loop is processing.
///
/// # Safety
///
/// This struct must remain valid for the lifetime of the acquisition.
/// It is pinned via `Box::pin` and passed to PVCAM as a raw pointer.
#[cfg(feature = "pvcam_hardware")]
pub struct CallbackContext {
    /// Count of pending frames (incremented by callback, decremented by consumer)
    pub pending_frames: std::sync::atomic::AtomicU32,
    /// Latest frame info from callback (informational, not for loss detection)
    pub latest_frame_nr: AtomicI32,
    /// Condvar for efficient waiting (reduces CPU vs polling)
    pub condvar: std::sync::Condvar,
    /// Mutex paired with condvar - MUST be locked when notifying to avoid missed wakeups
    pub mutex: std::sync::Mutex<bool>, // bool indicates "notified" state
    /// Shutdown signal to exit the wait loop
    pub shutdown: AtomicBool,
}

#[cfg(feature = "pvcam_hardware")]
impl CallbackContext {
    pub fn new() -> Self {
        Self {
            pending_frames: std::sync::atomic::AtomicU32::new(0),
            latest_frame_nr: AtomicI32::new(-1),
            condvar: std::sync::Condvar::new(),
            mutex: std::sync::Mutex::new(false),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Signal that a frame is ready (called from EOF callback)
    ///
    /// Increments the pending frame counter and notifies waiting threads.
    /// Must lock the mutex to avoid missed wakeups with condvar.
    #[inline]
    pub fn signal_frame_ready(&self, frame_nr: i32) {
        self.latest_frame_nr.store(frame_nr, Ordering::Release);
        self.pending_frames.fetch_add(1, Ordering::AcqRel);
        // Lock mutex to ensure condvar notification is seen
        if let Ok(mut guard) = self.mutex.lock() {
            *guard = true; // Set notified flag
            self.condvar.notify_one();
        }
    }

    /// Wait for frames to be available with timeout
    ///
    /// Returns the number of pending frames (0 on shutdown or timeout).
    /// Does NOT decrement the counter - caller should drain frames and call `consume_one()` for each.
    pub fn wait_for_frames(&self, timeout_ms: u64) -> u32 {
        // Check if shutdown requested
        if self.shutdown.load(Ordering::Acquire) {
            return 0;
        }

        // Check if frames already pending (fast path)
        let pending = self.pending_frames.load(Ordering::Acquire);
        if pending > 0 {
            return pending;
        }

        // Wait on condvar with timeout
        let guard = match self.mutex.lock() {
            Ok(g) => g,
            Err(_) => return 0, // Poisoned mutex
        };

        let timeout_duration = Duration::from_millis(timeout_ms);
        let result = self
            .condvar
            .wait_timeout_while(guard, timeout_duration, |notified| {
                // Wait while NOT notified AND no pending frames AND not shutdown
                !*notified
                    && self.pending_frames.load(Ordering::Acquire) == 0
                    && !self.shutdown.load(Ordering::Acquire)
            });

        match result {
            Ok((mut guard, _)) => {
                *guard = false; // Reset notified flag
                self.pending_frames.load(Ordering::Acquire)
            }
            Err(_) => 0, // Poisoned mutex
        }
    }

    /// Decrement the pending frames counter after successfully retrieving a frame
    #[inline]
    pub fn consume_one(&self) {
        // Saturating decrement to avoid underflow
        let _ = self
            .pending_frames
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                if n > 0 {
                    Some(n - 1)
                } else {
                    None
                }
            });
    }

    /// Signal shutdown to wake waiting threads
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut guard) = self.mutex.lock() {
            *guard = true;
            self.condvar.notify_all();
        }
    }

    /// Reset context state for new acquisition
    pub fn reset(&self) {
        self.pending_frames.store(0, Ordering::SeqCst);
        self.latest_frame_nr.store(-1, Ordering::SeqCst);
        self.shutdown.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.mutex.lock() {
            *guard = false;
        }
    }
}

/// FFI-safe EOF callback function (bd-ek9n.2)
///
/// This function is called by PVCAM when a frame is ready. It must:
/// 1. Be `extern "system"` for cross-platform ABI safety (stdcall on Windows, cdecl on Unix)
/// 2. Do minimal work (just signal, no heavy processing)
/// 3. Not block or perform I/O
///
/// Uses `extern "system"` instead of `extern "C"` for Windows compatibility:
/// - On Unix: both ABIs are equivalent (cdecl)
/// - On Windows: `extern "system"` uses __stdcall as PVCAM SDK expects
/// - On Windows x64: both ABIs are unified, so either works
///
/// The callback is cast to `*mut c_void` when registered with PVCAM.
///
/// # Safety
///
/// - `p_frame_info` must be a valid pointer to FRAME_INFO or null
/// - `p_context` must be a valid pointer to CallbackContext
#[cfg(feature = "pvcam_hardware")]
pub unsafe extern "system" fn pvcam_eof_callback(
    p_frame_info: *const FRAME_INFO,
    p_context: *mut std::ffi::c_void,
) {
    if p_context.is_null() {
        return;
    }

    let ctx = &*(p_context as *const CallbackContext);

    // Extract frame number for loss detection
    let frame_nr = if !p_frame_info.is_null() {
        (*p_frame_info).FrameNr
    } else {
        -1
    };

    // Signal frame ready - minimal work in callback
    ctx.signal_frame_ready(frame_nr);
}

/// Acquisition error type for signaling fatal errors from frame loop (Gemini SDK review).
/// Used to signal "involuntary stop" conditions back to the driver.
///
/// # Error Recovery (bd-g9po)
///
/// These errors indicate conditions that may require driver reinitialization:
/// - `ReadoutFailed`: Hardware error during frame readout (possible USB disconnect)
/// - `StatusCheckFailed`: SDK returned error from status query
/// - `Timeout`: No frames received for extended period
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionError {
    /// READOUT_FAILED status from pl_exp_check_cont_status
    ReadoutFailed,
    /// pl_exp_check_cont_status returned 0 (SDK error)
    StatusCheckFailed,
    /// Too many consecutive timeouts without frames
    Timeout,
}

/// Hardware frame metadata decoded from PVCAM embedded metadata (Gemini SDK review).
///
/// When `PARAM_METADATA_ENABLED` is true, PVCAM embeds timing information
/// directly in the frame buffer. This struct holds the decoded values
/// which provide microsecond-precision hardware timestamps from the FPGA.
///
/// # Timestamps
///
/// - `timestamp_bof_ns`: Beginning of frame timestamp in nanoseconds
/// - `timestamp_eof_ns`: End of frame timestamp in nanoseconds
/// - `exposure_time_ns`: Actual exposure time in nanoseconds
///
/// # Usage
///
/// Hardware timestamps are superior to software timestamps for:
/// - Correlating camera frames with other hardware events (stage movement, laser pulses)
/// - Precise inter-frame timing analysis
/// - Detecting timing jitter or irregularities
#[derive(Debug, Clone, Default)]
pub struct FrameMetadata {
    /// Hardware frame number (1-based, from FPGA)
    pub frame_nr: u32,
    /// Beginning of frame timestamp in nanoseconds (from FPGA clock)
    pub timestamp_bof_ns: u64,
    /// End of frame timestamp in nanoseconds (from FPGA clock)
    pub timestamp_eof_ns: u64,
    /// Actual exposure time in nanoseconds
    pub exposure_time_ns: u64,
    /// Bit depth of the image data
    pub bit_depth: u8,
    /// Number of ROIs in the frame
    pub roi_count: u16,
}

/// Page-aligned buffer for DMA performance (Gemini SDK review).
/// PVCAM DMA requires 4KB page alignment to avoid internal driver copies.
#[cfg(feature = "pvcam_hardware")]
pub struct PageAlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
    len: usize,
}

#[cfg(feature = "pvcam_hardware")]
impl PageAlignedBuffer {
    const PAGE_SIZE: usize = 4096;

    /// Allocate a page-aligned buffer of the given size.
    /// Panics if allocation fails (unlikely for reasonable sizes).
    pub fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, Self::PAGE_SIZE)
            .expect("Invalid layout for page-aligned buffer");
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            panic!("Failed to allocate page-aligned buffer of {} bytes", size);
        }
        Self {
            ptr,
            layout,
            len: size,
        }
    }

    /// Get a mutable pointer to the buffer for passing to PVCAM SDK.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Get the buffer length in bytes.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }
}

#[cfg(feature = "pvcam_hardware")]
impl Drop for PageAlignedBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                dealloc(self.ptr, self.layout);
            }
        }
    }
}

// SAFETY: The buffer is only accessed from the frame loop thread and
// PVCAM SDK (which operates on the same thread). The Arc<Mutex<>> wrapper
// ensures synchronized access.
// NOTE: Sync impl removed - Arc<Mutex<T>> only requires T: Send for Arc to be Sync
#[cfg(feature = "pvcam_hardware")]
unsafe impl Send for PageAlignedBuffer {}

// =============================================================================
// FFI Safety Wrappers (bd-g9gq)
// =============================================================================
//
// These functions isolate unsafe FFI calls into small helpers with explicit
// safety contracts. Each function documents its preconditions and invariants.
// =============================================================================

#[cfg(feature = "pvcam_hardware")]
mod ffi_safe {
    use super::*;

    /// Stop continuous acquisition on a camera.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - Acquisition must have been started with `pl_exp_start_cont`
    /// - Must be called before closing the camera
    pub fn stop_acquisition(hcam: i16, mode: i16) {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        // SAFETY: Caller guarantees hcam is valid and acquisition is active
        unsafe {
            pl_exp_stop_cont(hcam, mode);
        }
    }

    /// Restart continuous acquisition on a camera (bd-3gnv).
    ///
    /// Used for auto-restart workaround when camera stalls at 85 frames.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - `circ_ptr` must point to valid, page-aligned buffer
    /// - `circ_size_bytes` must match the allocated buffer size
    /// - Camera must be in stopped state (call stop_acquisition first)
    ///
    /// # Returns
    /// `Ok(())` on success, `Err(String)` with error message on failure
    pub fn restart_acquisition(
        hcam: i16,
        circ_ptr: *mut u8,
        circ_size_bytes: u32,
    ) -> Result<(), String> {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        debug_assert!(!circ_ptr.is_null(), "Circular buffer pointer is null");
        debug_assert!(circ_size_bytes > 0, "Circular buffer size must be > 0");

        // SAFETY: Caller guarantees hcam is valid, circ_ptr is valid page-aligned buffer
        let result = unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size_bytes) };
        if result == 0 {
            let err_msg = super::get_pvcam_error();
            Err(format!("pl_exp_start_cont failed: {}", err_msg))
        } else {
            Ok(())
        }
    }

    /// Full restart: setup + start continuous acquisition (bd-3gnv).
    ///
    /// Used when simple restart fails - camera may require full re-setup.
    /// This calls pl_exp_setup_cont followed by pl_exp_start_cont.
    ///
    /// # Parameters
    /// - `hcam`: Valid, open camera handle
    /// - `roi_x`, `roi_y`: ROI offset
    /// - `width`, `height`: ROI dimensions
    /// - `binning`: (x_bin, y_bin) factors
    /// - `exposure_ms`: Exposure time in milliseconds
    /// - `circ_ptr`: Page-aligned circular buffer
    /// - `circ_size_bytes`: Buffer size in bytes
    ///
    /// # Returns
    /// `Ok(frame_bytes)` on success, `Err(String)` on failure
    #[allow(clippy::too_many_arguments)]
    pub fn full_restart_acquisition(
        hcam: i16,
        roi_x: u32,
        roi_y: u32,
        width: u32,
        height: u32,
        binning: (u16, u16),
        exposure_ms: f64,
        circ_ptr: *mut u8,
        circ_size_bytes: u32,
    ) -> Result<uns32, String> {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        debug_assert!(!circ_ptr.is_null(), "Circular buffer pointer is null");
        debug_assert!(circ_size_bytes > 0, "Circular buffer size must be > 0");

        let (x_bin, y_bin) = binning;

        // Setup region (same as initial setup)
        let region = unsafe {
            let mut rgn: rgn_type = std::mem::zeroed();
            rgn.s1 = roi_x as uns16;
            rgn.s2 = (roi_x + width - 1) as uns16;
            rgn.sbin = x_bin;
            rgn.p1 = roi_y as uns16;
            rgn.p2 = (roi_y + height - 1) as uns16;
            rgn.pbin = y_bin;
            rgn
        };

        // Use same constants as initial setup
        let exp_mode = TIMED_MODE;
        let buffer_mode = CIRC_NO_OVERWRITE;
        let mut frame_bytes: uns32 = 0;

        // Step 1: pl_exp_setup_cont
        let setup_result = unsafe {
            pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                exp_mode,
                exposure_ms as uns32,
                &mut frame_bytes,
                buffer_mode,
            )
        };
        if setup_result == 0 {
            let err_msg = super::get_pvcam_error();
            return Err(format!("pl_exp_setup_cont failed: {}", err_msg));
        }

        // Step 2: pl_exp_start_cont
        let start_result = unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size_bytes) };
        if start_result == 0 {
            let err_msg = super::get_pvcam_error();
            return Err(format!("pl_exp_start_cont failed: {}", err_msg));
        }

        Ok(frame_bytes)
    }

    /// Deregister a callback from a camera.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - Callback must have been registered with `pl_cam_register_callback_ex3`
    pub fn deregister_callback(hcam: i16, callback_type: i32) {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        // SAFETY: Caller guarantees hcam is valid and callback was registered
        unsafe {
            pl_cam_deregister_callback(hcam, callback_type);
        }
    }

    /// Register EOF callback for frame notifications (bd-3gnv).
    ///
    /// Used to re-register callback after full restart.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - `callback_ctx_ptr` must point to a valid, pinned CallbackContext
    ///
    /// # Returns
    /// `true` if registration succeeded, `false` otherwise
    pub fn register_eof_callback(hcam: i16, callback_ctx_ptr: *const CallbackContext) -> bool {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        debug_assert!(
            !callback_ctx_ptr.is_null(),
            "Callback context pointer is null"
        );

        // SAFETY: Caller guarantees hcam is valid, callback_ctx_ptr points to valid pinned context
        let result = unsafe {
            pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                super::pvcam_eof_callback as *mut std::ffi::c_void,
                callback_ctx_ptr as *mut std::ffi::c_void,
            )
        };
        result != 0
    }

    /// Check continuous acquisition status.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - Acquisition must be active
    ///
    /// # Returns
    /// - `Ok((status, bytes_arrived, buffer_cnt))` on success
    /// - `Err(())` if the status check failed (camera error)
    pub fn check_cont_status(hcam: i16) -> Result<(i16, uns32, uns32), ()> {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;

        // SAFETY: All pointers are valid stack allocations
        let result = unsafe {
            pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt)
        };

        if result == 0 {
            Err(())
        } else {
            Ok((status, bytes_arrived, buffer_cnt))
        }
    }

    /// Get the oldest frame from the circular buffer with frame info.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - Acquisition must be active with frames available
    /// - `frame_info` must be a valid pointer to a FRAME_INFO struct
    ///
    /// # Returns
    /// - `Ok(frame_ptr)` - pointer to the frame data in the circular buffer
    /// - `Err(())` if no frame available or error
    pub fn get_oldest_frame(
        hcam: i16,
        frame_info: &mut FRAME_INFO,
    ) -> Result<*mut std::ffi::c_void, ()> {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

        // SAFETY: hcam is valid, frame_info and frame_ptr are valid stack allocations
        let result = unsafe { pl_exp_get_oldest_frame_ex(hcam, &mut frame_ptr, frame_info) };

        if result == 0 || frame_ptr.is_null() {
            // bd-3gnv: Log error code to diagnose why get_oldest_frame is failing
            let err_code = unsafe { pl_error_code() };
            eprintln!(
                "[PVCAM DEBUG] get_oldest_frame failed: result={}, err_code={}, frame_ptr_null={}",
                result,
                err_code,
                frame_ptr.is_null()
            );
            Err(())
        } else {
            Ok(frame_ptr)
        }
    }

    /// Release the oldest frame back to the circular buffer.
    ///
    /// # Safety Contract
    /// - `hcam` must be a valid, open camera handle
    /// - A frame must have been retrieved with `get_oldest_frame`
    ///
    /// # Returns
    /// true if unlock succeeded, false if it failed
    pub fn release_oldest_frame(hcam: i16) -> bool {
        debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
        // SAFETY: Caller guarantees hcam is valid and a frame was retrieved
        // bd-3gnv: Check return value - silent unlock failures would stall CIRC_NO_OVERWRITE mode
        let result = unsafe { pl_exp_unlock_oldest_frame(hcam) };
        if result == 0 {
            // Unlock failed - this is critical for continuous acquisition
            let err_msg = super::get_pvcam_error();
            tracing::error!("pl_exp_unlock_oldest_frame failed: {} (bd-3gnv)", err_msg);
            false
        } else {
            true
        }
    }

    /// Create a metadata frame struct for decoding.
    ///
    /// # Safety Contract
    /// - `roi_count` must be > 0
    ///
    /// # Returns
    /// - `Some(ptr)` - valid md_frame pointer (must be released with `release_md_frame`)
    /// - `None` if creation failed
    pub fn create_md_frame(roi_count: u16) -> Option<*mut md_frame> {
        debug_assert!(roi_count > 0, "ROI count must be positive");
        let mut ptr: *mut md_frame = std::ptr::null_mut();

        // SAFETY: ptr is a valid stack allocation, roi_count is validated
        let result = unsafe { pl_md_create_frame_struct_cont(&mut ptr, roi_count) };

        if result == 0 || ptr.is_null() {
            None
        } else {
            Some(ptr)
        }
    }

    /// Release a metadata frame struct.
    ///
    /// # Safety Contract
    /// - `ptr` must have been created with `create_md_frame`
    /// - Must not be called twice on the same pointer
    pub fn release_md_frame(ptr: *mut md_frame) {
        if !ptr.is_null() {
            // SAFETY: Caller guarantees ptr was created by create_md_frame
            unsafe {
                pl_md_release_frame_struct(ptr);
            }
        }
    }

    /// Decode metadata from a frame buffer.
    ///
    /// # Safety Contract
    /// - `md_frame_ptr` must be a valid md_frame struct
    /// - `frame_ptr` must point to valid frame data
    /// - `frame_size` must match the actual buffer size
    ///
    /// # Returns
    /// - `true` if decoding succeeded
    /// - `false` if decoding failed
    pub fn decode_frame_metadata(
        md_frame_ptr: *mut md_frame,
        frame_ptr: *const std::ffi::c_void,
        frame_size: u32,
    ) -> bool {
        debug_assert!(!md_frame_ptr.is_null(), "md_frame_ptr must not be null");
        debug_assert!(!frame_ptr.is_null(), "frame_ptr must not be null");
        debug_assert!(frame_size > 0, "frame_size must be positive");

        // SAFETY: All pointers are valid per caller contract, frame_size matches buffer
        let result = unsafe { pl_md_frame_decode(md_frame_ptr, frame_ptr as *mut _, frame_size) };

        result != 0
    }
}

/// PVCAM acquisition state and frame streaming.
///
/// Manages continuous acquisition with circular buffers and provides frame
/// delivery via broadcast and mpsc channels.
///
/// # Frame Loss Metrics (bd-ek9n.3)
///
/// - `lost_frames`: Total count of frames lost due to buffer overflows
/// - `discontinuity_events`: Number of gap events detected in frame sequence
/// - `last_hardware_frame_nr`: Last seen hardware frame number for gap detection
pub struct PvcamAcquisition {
    pub streaming: Parameter<bool>,
    pub frame_count: Arc<AtomicU64>,
    pub frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    pub reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,
    #[cfg(feature = "arrow_tap")]
    pub arrow_tap: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>>>>,

    /// Optional metadata channel for hardware timestamps (Gemini SDK review).
    /// When enabled, each frame's decoded metadata is sent here alongside the frame data.
    #[cfg(feature = "pvcam_hardware")]
    pub metadata_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<FrameMetadata>>>>,
    /// Whether metadata decoding is enabled for this acquisition.
    #[cfg(feature = "pvcam_hardware")]
    metadata_enabled: Arc<AtomicBool>,

    /// Frame loss detection counters (bd-ek9n.3).
    /// Total number of frames lost due to buffer overflows or processing delays.
    pub lost_frames: Arc<AtomicU64>,
    /// Number of discontinuity events (gaps in frame sequence).
    pub discontinuity_events: Arc<AtomicU64>,
    /// Last hardware frame number for gap detection (-1 = uninitialized).
    #[cfg(feature = "pvcam_hardware")]
    last_hardware_frame_nr: Arc<AtomicI32>,

    /// Last error that occurred during acquisition (bd-g9po).
    /// Set when a fatal error causes involuntary stop. Cleared by `clear_error()`.
    last_error: Arc<std::sync::Mutex<Option<AcquisitionError>>>,

    /// Shutdown signal for the poll loop (bd-z8q8).
    /// Set to true in Drop to signal the poll thread to exit.
    #[cfg(feature = "pvcam_hardware")]
    shutdown: Arc<AtomicBool>,
    #[cfg(feature = "pvcam_hardware")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Page-aligned circular buffer for DMA performance (Gemini SDK review).
    /// PVCAM DMA requires 4KB alignment to avoid internal driver copies.
    #[cfg(feature = "pvcam_hardware")]
    circ_buffer: Arc<Mutex<Option<PageAlignedBuffer>>>,
    #[cfg(feature = "pvcam_hardware")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
    /// Error sender for signaling involuntary stops from frame loop (Gemini SDK review).
    /// Fatal errors (READOUT_FAILED, etc.) are sent here so the driver can update streaming state.
    /// Uses tokio::sync::mpsc::unbounded_channel for async-native error watching without polling.
    #[cfg(feature = "pvcam_hardware")]
    error_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<AcquisitionError>>>>,
    /// Callback context for EOF notifications (bd-ek9n.2).
    /// Pinned to ensure stable address for FFI callback.
    #[cfg(feature = "pvcam_hardware")]
    callback_context: Arc<std::pin::Pin<Box<CallbackContext>>>,
    /// Camera handle for cleanup in Drop. Stored during start_stream, cleared in stop_stream.
    /// Uses AtomicI16 with sentinel -1 (invalid handle) for lock-free access in Drop.
    #[cfg(feature = "pvcam_hardware")]
    active_hcam: Arc<AtomicI16>,
    /// Whether EOF callback is registered (for cleanup in Drop)
    #[cfg(feature = "pvcam_hardware")]
    callback_registered: Arc<AtomicBool>,
    /// Completion signal for poll thread (bd-g6pr).
    /// Used in Drop to synchronously wait for the poll thread to exit before calling
    /// FFI cleanup functions. This prevents the race condition where pl_exp_stop_cont
    /// is called while pl_exp_get_oldest_frame_ex is still executing.
    #[cfg(feature = "pvcam_hardware")]
    poll_thread_done_rx: Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
    #[cfg(feature = "pvcam_hardware")]
    poll_thread_done_tx: Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>>,
}

impl PvcamAcquisition {
    pub fn new(streaming: Parameter<bool>) -> Self {
        // bd-3gnv: Increased from 32 to 256 frames to prevent stalls during sustained streaming.
        // At 100 FPS, 32 frames = 0.32s buffer (too small); 256 frames = 2.56s buffer (adequate).
        let (frame_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            streaming,
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            reliable_tx: Arc::new(Mutex::new(None)),
            #[cfg(feature = "arrow_tap")]
            arrow_tap: Arc::new(Mutex::new(None)),

            // Metadata channel and state (Gemini SDK review)
            #[cfg(feature = "pvcam_hardware")]
            metadata_tx: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            metadata_enabled: Arc::new(AtomicBool::new(false)),

            // Frame loss detection counters (bd-ek9n.3)
            lost_frames: Arc::new(AtomicU64::new(0)),
            discontinuity_events: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "pvcam_hardware")]
            last_hardware_frame_nr: Arc::new(AtomicI32::new(-1)), // -1 = uninitialized

            // Error tracking (bd-g9po)
            last_error: Arc::new(std::sync::Mutex::new(None)),

            #[cfg(feature = "pvcam_hardware")]
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "pvcam_hardware")]
            poll_handle: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            circ_buffer: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            trigger_frame: Arc::new(Mutex::new(None)),
            // Error channel for involuntary stop signaling (Gemini SDK review)
            #[cfg(feature = "pvcam_hardware")]
            error_tx: Arc::new(Mutex::new(None)),
            // Pinned callback context for EOF notifications (bd-ek9n.2)
            #[cfg(feature = "pvcam_hardware")]
            callback_context: Arc::new(Box::pin(CallbackContext::new())),
            // Camera handle and callback state for Drop cleanup
            // -1 is sentinel for "no active handle"
            #[cfg(feature = "pvcam_hardware")]
            active_hcam: Arc::new(AtomicI16::new(-1)),
            #[cfg(feature = "pvcam_hardware")]
            callback_registered: Arc::new(AtomicBool::new(false)),
            // Completion channel for poll thread synchronization (bd-g6pr)
            // Created fresh for each acquisition in start_stream
            #[cfg(feature = "pvcam_hardware")]
            poll_thread_done_rx: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            poll_thread_done_tx: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Reset frame loss metrics at the start of a new acquisition.
    pub fn reset_frame_loss_metrics(&self) {
        self.lost_frames.store(0, Ordering::SeqCst);
        self.discontinuity_events.store(0, Ordering::SeqCst);
        #[cfg(feature = "pvcam_hardware")]
        {
            self.last_hardware_frame_nr.store(-1, Ordering::SeqCst);
            // Reset callback context state (bd-ek9n.2)
            self.callback_context.reset();
        }
    }

    /// Get the current frame loss statistics.
    pub fn frame_loss_stats(&self) -> (u64, u64) {
        (
            self.lost_frames.load(Ordering::Relaxed),
            self.discontinuity_events.load(Ordering::Relaxed),
        )
    }

    /// Check if an error occurred during acquisition (bd-g9po).
    ///
    /// Returns true if the last acquisition ended due to an error rather than
    /// a normal stop. Use `last_error()` to get details.
    pub fn has_error(&self) -> bool {
        self.last_error
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Get the last acquisition error, if any (bd-g9po).
    ///
    /// Returns the error type from the last failed acquisition. Errors are
    /// set when the frame loop exits due to SDK failures or timeouts.
    pub fn last_error(&self) -> Option<AcquisitionError> {
        self.last_error.lock().ok().and_then(|guard| *guard)
    }

    /// Clear the error state (bd-g9po).
    ///
    /// Call this before retrying an operation after an error, or as part of
    /// driver reinitialization.
    pub fn clear_error(&self) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = None;
        }
    }

    /// Calculate optimal circular buffer frame count (bd-ek9n.4)
    ///
    /// Uses PARAM_FRAME_BUFFER_SIZE when available, with heuristic fallback:
    /// - Minimum 16 frames for reliability
    /// - At least 1 second of buffer at current frame rate
    /// - Capped at 256 frames to prevent excessive memory use
    ///
    /// # Arguments
    ///
    /// * `hcam` - Open camera handle
    /// * `frame_bytes` - Size of one frame in bytes
    /// * `exposure_ms` - Exposure time in milliseconds (for frame rate calculation)
    #[cfg(feature = "pvcam_hardware")]
    fn calculate_buffer_count(hcam: i16, frame_bytes: usize, exposure_ms: f64) -> usize {
        // bd-circ: Reduced buffer size to avoid DMA limit issues on Prime BSI.
        // Full sensor (2048x2048 @ 8MB/frame) stalls at ~85 frames (~780MB).
        // Limiting to 32 frames (256MB) forces earlier wrap-around within safe DMA limits.
        // Smaller ROIs (256x256 @ 128KB) can handle more frames (200+) successfully.
        const MIN_BUFFER_FRAMES: usize = 16;
        const MAX_BUFFER_FRAMES: usize = 32;
        const ONE_SECOND_MS: f64 = 1000.0;

        // Try to query PARAM_FRAME_BUFFER_SIZE from SDK
        // This returns recommended buffer size in bytes for current acquisition settings
        let sdk_recommended = unsafe {
            let mut avail: rs_bool = 0;
            // Check if parameter is available
            if pl_get_param(
                hcam,
                PARAM_FRAME_BUFFER_SIZE,
                ATTR_AVAIL as i16,
                &mut avail as *mut _ as *mut _,
            ) != 0
                && avail != 0
            {
                // Get the default (recommended) value
                let mut recommended_bytes: u64 = 0;
                if pl_get_param(
                    hcam,
                    PARAM_FRAME_BUFFER_SIZE,
                    ATTR_DEFAULT as i16,
                    &mut recommended_bytes as *mut _ as *mut _,
                ) != 0
                {
                    Some(recommended_bytes as usize)
                } else {
                    tracing::debug!("PARAM_FRAME_BUFFER_SIZE available but failed to read default");
                    None
                }
            } else {
                tracing::debug!("PARAM_FRAME_BUFFER_SIZE not available, using heuristics");
                None
            }
        };

        // Calculate frame count from SDK recommendation
        let sdk_frames = sdk_recommended
            .map(|bytes| bytes / frame_bytes.max(1))
            .unwrap_or(0);

        // Calculate frames needed for ~1 second of buffer based on exposure time
        // Frame period ~= exposure_ms (simplified; ignores readout time)
        let fps_estimate = if exposure_ms > 0.0 {
            ONE_SECOND_MS / exposure_ms
        } else {
            100.0 // Default assumption: 100 FPS
        };
        let one_second_frames = fps_estimate.ceil() as usize;

        // Choose the larger of SDK recommendation and 1-second heuristic,
        // then clamp to reasonable bounds
        let target = sdk_frames.max(one_second_frames).max(MIN_BUFFER_FRAMES);
        let clamped = target.min(MAX_BUFFER_FRAMES);

        tracing::debug!(
            "Buffer sizing: SDK={:?} frames, 1sec={} frames, target={}, clamped={}",
            sdk_recommended.map(|b| b / frame_bytes.max(1)),
            one_second_frames,
            target,
            clamped
        );

        clamped
    }

    #[cfg(feature = "arrow_tap")]
    pub async fn set_arrow_tap(
        &self,
        tx: tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>,
    ) {
        let mut guard = self.arrow_tap.lock().await;
        *guard = Some(tx);
    }

    /// Enable metadata decoding and set the metadata channel (Gemini SDK review).
    ///
    /// When enabled, PVCAM embeds hardware timestamps in frame buffers which are
    /// decoded using `pl_md_frame_decode`. This provides microsecond-precision
    /// timing from the FPGA for correlating frames with other hardware events.
    ///
    /// # Arguments
    ///
    /// * `tx` - Channel to receive `FrameMetadata` for each frame
    ///
    /// # Note
    ///
    /// Must be called before `start_stream()`. The metadata channel will receive
    /// one `FrameMetadata` per frame in sync with the frame delivery.
    #[cfg(feature = "pvcam_hardware")]
    pub async fn enable_metadata(&self, tx: tokio::sync::mpsc::Sender<FrameMetadata>) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = Some(tx);
        self.metadata_enabled.store(true, Ordering::Release);
        tracing::info!("Metadata decoding enabled for acquisition");
    }

    /// Disable metadata decoding (Gemini SDK review).
    #[cfg(feature = "pvcam_hardware")]
    pub async fn disable_metadata(&self) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = None;
        self.metadata_enabled.store(false, Ordering::Release);
    }

    /// Start streaming frames
    ///
    /// # Frame Loss Detection (bd-ek9n.3)
    ///
    /// Resets frame loss metrics at the start of each acquisition. During streaming,
    /// the poll loop tracks hardware frame numbers to detect and count dropped frames.
    pub async fn start_stream(
        &self,
        conn: &PvcamConnection,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64,
    ) -> Result<()> {
        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        if self.streaming.get() {
            bail!("Already streaming");
        }

        self.streaming.set(true).await?;
        self.frame_count.store(0, Ordering::SeqCst);
        // Reset frame loss metrics for this acquisition (bd-ek9n.3)
        self.reset_frame_loss_metrics();

        let reliable_tx = self.reliable_tx.lock().await.clone();
        #[cfg(feature = "arrow_tap")]
        let _arrow_tap = self.arrow_tap.lock().await.clone();

        #[cfg(feature = "pvcam_hardware")]
        if let Some(h) = conn.handle() {
            // Hardware path

            // Check if metadata decoding is enabled (via enable_metadata() call)
            let use_metadata = self.metadata_enabled.load(Ordering::Acquire);

            // Configure PVCAM metadata based on whether decoding is enabled (Gemini SDK review).
            // When metadata is enabled, frame buffers contain header data before pixels.
            // We only enable it when pl_md_frame_decode will be used to parse the data.
            let current_metadata = PvcamFeatures::is_metadata_enabled(conn).unwrap_or(false);
            if use_metadata && !current_metadata {
                tracing::info!("Enabling PVCAM metadata for hardware timestamp decoding");
                if let Err(e) = PvcamFeatures::set_metadata_enabled(conn, true) {
                    tracing::error!(
                        "Failed to enable metadata: {}. Falling back to no metadata",
                        e
                    );
                    self.metadata_enabled.store(false, Ordering::Release);
                }
            } else if !use_metadata && current_metadata {
                // Disable metadata to prevent data corruption when not decoding
                tracing::debug!("Disabling PVCAM metadata (no decoder configured)");
                if let Err(e) = PvcamFeatures::set_metadata_enabled(conn, false) {
                    tracing::warn!(
                        "Failed to disable metadata: {}. Data may include headers",
                        e
                    );
                }
            }

            let (x_bin, y_bin) = binning;

            // PVCAM Best Practices: for reliable frame delivery (especially high FPS/high throughput),
            // prefer an EOF callback acquisition model over polling loops (bd-ek9n.2).
            // Setup region
            let region = unsafe {
                // SAFETY: rgn_type is POD; zeroed then fully initialized before use.
                let mut rgn: rgn_type = std::mem::zeroed();
                rgn.s1 = roi.x as uns16;
                rgn.s2 = (roi.x + roi.width - 1) as uns16;
                rgn.sbin = x_bin;
                rgn.p1 = roi.y as uns16;
                rgn.p2 = (roi.y + roi.height - 1) as uns16;
                rgn.pbin = y_bin;
                rgn
            };

            // bd-3gnv: Use sequence mode if enabled (proven to work on Prime BSI)
            if USE_SEQUENCE_MODE {
                return self
                    .start_stream_sequence_impl(
                        h,
                        region,
                        exposure_ms,
                        binning,
                        roi,
                        reliable_tx,
                        #[cfg(feature = "arrow_tap")]
                        _arrow_tap,
                        use_metadata,
                    )
                    .await;
            }

            // PVCAM Best Practices: Use actual frame_bytes from pl_exp_setup_cont
            // rather than assuming pixels * 2 - metadata/alignment can change frame size.
            let mut frame_bytes: uns32 = 0;
            // bd-3gnv: Select buffer mode - CIRC_OVERWRITE for indefinite streaming (like DynExp)
            // CIRC_OVERWRITE: oldest frame overwritten when buffer full (no stall)
            // CIRC_NO_OVERWRITE: camera pauses when buffer fills until frames unlocked
            let buffer_mode = if USE_CIRC_OVERWRITE_MODE {
                CIRC_OVERWRITE
            } else {
                CIRC_NO_OVERWRITE
            };
            // bd-3gnv: Always use TIMED_MODE (like DynExp). EXT_TRIG_INTERNAL caused error 185.
            // DynExp reference: uses TIMED_MODE with CIRC_OVERWRITE successfully.
            let exp_mode = TIMED_MODE;

            unsafe {
                // SAFETY: h is a valid camera handle; region points to initialized rgn_type; frame_bytes is writable.
                if pl_exp_setup_cont(
                    h,
                    1,
                    &region as *const _,
                    exp_mode,
                    exposure_ms as uns32,
                    &mut frame_bytes,
                    buffer_mode,
                ) == 0
                {
                    // bd-3gnv: Log SDK error with full message for diagnostics
                    let err_msg = get_pvcam_error();
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!(
                        "Failed to setup continuous acquisition: {}",
                        err_msg
                    ));
                }
            }

            // Calculate dimensions for frame construction
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let expected_frame_pixels = (binned_width * binned_height) as usize;
            let expected_frame_bytes = expected_frame_pixels * std::mem::size_of::<u16>();

            // Validate frame_bytes matches expected (unless metadata enabled)
            // frame_bytes from SDK should be >= expected_frame_bytes
            if (frame_bytes as usize) < expected_frame_bytes {
                tracing::warn!(
                    "PVCAM frame_bytes ({}) < expected ({}), possible SDK issue",
                    frame_bytes,
                    expected_frame_bytes
                );
            }
            let actual_frame_bytes = frame_bytes as usize;

            // PVCAM Best Practices (bd-ek9n.4): Use SDK-recommended buffer size
            // Query PARAM_FRAME_BUFFER_SIZE for optimal sizing, with fallback to heuristics.
            let buffer_count = Self::calculate_buffer_count(h, actual_frame_bytes, exposure_ms);
            // bd-3gnv: Debug output to verify buffer count
            eprintln!(
                "[PVCAM DEBUG] Circular buffer: {} frames, {} bytes/frame, {:.2} MB total",
                buffer_count,
                actual_frame_bytes,
                (actual_frame_bytes * buffer_count) as f64 / (1024.0 * 1024.0)
            );
            tracing::info!(
                "PVCAM circular buffer: {} frames ({:.2} MB)",
                buffer_count,
                (actual_frame_bytes * buffer_count) as f64 / (1024.0 * 1024.0)
            );

            // bd-3gnv: Register EOF callback before starting acquisition.
            // PVCAM Best Practices (bd-ek9n.2): Register EOF callback to avoid polling overhead.
            // The callback signals frame readiness, eliminating polling overhead.
            // Get raw pointer to pinned CallbackContext for FFI
            // Deref Arc -> Pin<Box<T>> -> T, then take address
            let callback_ctx_ptr = &**self.callback_context as *const CallbackContext;
            let use_callback = unsafe {
                // Use bindgen-generated function, cast callback to *mut c_void
                let result = pl_cam_register_callback_ex3(
                    h,
                    PL_CALLBACK_EOF,
                    pvcam_eof_callback as *mut std::ffi::c_void,
                    callback_ctx_ptr as *mut std::ffi::c_void,
                );
                if result == 0 {
                    tracing::warn!("Failed to register EOF callback, falling back to polling mode");
                    false
                } else {
                    tracing::info!("PVCAM EOF callback registered successfully");
                    // Store callback state for Drop cleanup
                    self.callback_registered.store(true, Ordering::Release);
                    true
                }
            };

            // Store camera handle for Drop cleanup (critical: must happen before acquisition starts)
            // Uses atomic store for lock-free access in Drop
            self.active_hcam.store(h, Ordering::Release);

            // Allocate based on actual frame_bytes, not assumed pixel count
            let circ_buf_size = actual_frame_bytes * buffer_count;

            // CRITICAL: Validate buffer size doesn't exceed u32::MAX to prevent overflow
            // when passing to pl_exp_start_cont. SDK expects uns32 (u32).
            let circ_size_bytes: uns32 = circ_buf_size.try_into().map_err(|_| {
                anyhow!(
                    "Circular buffer size {} exceeds u32::MAX ({}). Reduce buffer_count or frame size.",
                    circ_buf_size,
                    u32::MAX
                )
            })?;

            // Gemini SDK review: Use page-aligned buffer for DMA performance.
            // Standard Vec<u8> is only 1-byte aligned; PVCAM DMA requires 4KB alignment
            // to avoid internal driver copies (double buffering).
            let mut circ_buf = PageAlignedBuffer::new(circ_buf_size);
            let circ_ptr = circ_buf.as_mut_ptr();
            // bd-3gnv: Convert raw pointer to usize BEFORE any await points.
            // Raw pointers are not Send, but usize is. Convert early to avoid
            // "future cannot be sent between threads" errors from holding raw
            // pointers across await boundaries.
            let circ_ptr_usize = circ_ptr as usize;
            tracing::debug!(
                "Allocated {}KB page-aligned circular buffer",
                circ_buf_size / 1024
            );

            unsafe {
                // SAFETY: circ_ptr points to page-aligned contiguous buffer; SDK expects byte size.
                if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                    // bd-3gnv: Log SDK error with full message for diagnostics
                    let err_msg = get_pvcam_error();
                    // Deregister callback on failure
                    if use_callback {
                        pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                        self.callback_registered.store(false, Ordering::Release);
                    }
                    self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!(
                        "Failed to start continuous acquisition: {}",
                        err_msg
                    ));
                }
            }

            // CRITICAL: Store the page-aligned buffer passed to pl_exp_start_cont.
            // The buffer MUST remain allocated for the entire acquisition lifetime.
            // DO NOT convert or transform - PVCAM holds a raw pointer to this memory.
            *self.circ_buffer.lock().await = Some(circ_buf);

            // Reset shutdown flag before starting (in case of restart after stop)
            self.shutdown.store(false, Ordering::SeqCst);

            let streaming = self.streaming.clone();
            let shutdown = self.shutdown.clone();
            let frame_tx = self.frame_tx.clone();
            let frame_count = self.frame_count.clone();
            let lost_frames = self.lost_frames.clone();
            let discontinuity_events = self.discontinuity_events.clone();
            let last_hw_frame_nr = self.last_hardware_frame_nr.clone();
            let callback_ctx = self.callback_context.clone();
            let width = binned_width;
            let height = binned_height;
            #[cfg(feature = "arrow_tap")]
            let arrow_tap = _arrow_tap.clone();

            // Gemini SDK review: Metadata channel for hardware timestamps
            let metadata_tx = self.metadata_tx.lock().await.clone();
            // Re-check use_metadata after potential error during enable
            let use_metadata = self.metadata_enabled.load(Ordering::Acquire);

            // Gemini SDK review: Create error channel for involuntary stop signaling.
            // Fatal errors (READOUT_FAILED, etc.) are sent from frame loop to update streaming state.
            // Uses tokio unbounded_channel: send() is non-blocking (safe from sync code),
            // recv() is async-native (no polling needed in watcher task).
            let (error_tx, mut error_rx) =
                tokio::sync::mpsc::unbounded_channel::<AcquisitionError>();
            *self.error_tx.lock().await = Some(error_tx.clone());

            // Clone streaming parameter for error watcher task
            let streaming_for_watcher = self.streaming.clone();

            // Clone last_error for error watcher task (bd-g9po)
            let last_error_for_watcher = self.last_error.clone();

            // Capture ROI and binning for frame metadata (bd-183h)
            let roi_x = roi.x;
            let roi_y = roi.y;

            // bd-g6pr: Create completion channel for poll thread synchronization.
            // Drop will wait on this receiver before calling FFI cleanup functions,
            // preventing the race where pl_exp_stop_cont is called while
            // pl_exp_get_oldest_frame_ex is still executing.
            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
            if let Ok(mut guard) = self.poll_thread_done_rx.lock() {
                *guard = Some(done_rx);
            }
            if let Ok(mut guard) = self.poll_thread_done_tx.lock() {
                *guard = Some(done_tx.clone());
            }

            // bd-3gnv: circ_ptr_usize was converted from raw pointer at line 1110,
            // BEFORE any await points. We use it here for cross-thread transfer.

            let poll_handle = tokio::task::spawn_blocking(move || {
                // bd-3gnv: Convert usize back to raw pointer inside the closure.
                let circ_ptr_restored = circ_ptr_usize as *mut u8;

                Self::frame_loop_hardware(
                    h,
                    streaming,
                    shutdown,
                    frame_tx,
                    reliable_tx,
                    #[cfg(feature = "arrow_tap")]
                    arrow_tap,
                    frame_count,
                    lost_frames,
                    discontinuity_events,
                    last_hw_frame_nr,
                    callback_ctx,
                    use_callback,
                    exposure_ms,
                    actual_frame_bytes,
                    expected_frame_bytes,
                    width,
                    height,
                    error_tx,
                    use_metadata,
                    roi_x,
                    roi_y,
                    binning,
                    metadata_tx,
                    done_tx,
                    circ_ptr_restored, // bd-3gnv: Pass buffer for auto-restart
                    circ_size_bytes,   // bd-3gnv: Pass size for auto-restart
                );
            });

            *self.poll_handle.lock().await = Some(poll_handle);

            // Gemini SDK review: Spawn error watcher to handle involuntary stops.
            // This prevents "zombie streaming" where fatal errors leave streaming=true.
            // Uses tokio::sync::mpsc::unbounded_channel for async-native recv() without polling.
            // bd-g9po: Also stores error in last_error for recovery detection.
            tokio::spawn(async move {
                // Async recv() suspends the task until a message arrives or channel closes.
                // No polling loop needed - tokio handles the wake-up efficiently.
                if let Some(err) = error_rx.recv().await {
                    tracing::error!("Acquisition error (involuntary stop): {:?}", err);

                    // bd-g9po: Store error for recovery detection
                    if let Ok(mut guard) = last_error_for_watcher.lock() {
                        *guard = Some(err);
                    }

                    // Update streaming state to reflect the involuntary stop
                    if let Err(e) = streaming_for_watcher.set(false).await {
                        tracing::error!("Failed to update streaming state after error: {}", e);
                    }
                }
                // Channel closed (frame loop ended) - task completes naturally
            });

            return Ok(());
        }

        // Mock path (or no handle)
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            tracing::warn!("start_stream: pvcam_hardware NOT compiled - using mock stream");
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx)
                .await?;
        }

        // Handle case where hardware feature enabled but handle missing (mock fallback logic)
        #[cfg(feature = "pvcam_hardware")]
        if conn.handle().is_none() {
            tracing::warn!(
                "start_stream: pvcam_hardware compiled but handle is None - falling back to mock stream"
            );
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx)
                .await?;
        }

        Ok(())
    }

    /// Acquire a single frame by starting the stream, grabbing one frame, then stopping.
    pub async fn acquire_single_frame(
        &self,
        conn: &PvcamConnection,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64,
    ) -> Result<Frame> {
        let mut rx = self.frame_tx.subscribe();
        self.start_stream(conn, roi, binning, exposure_ms).await?;

        let frame = timeout(Duration::from_secs(5), rx.recv())
            .await
            .map_err(|_| anyhow!("Timed out waiting for frame"))?
            .map_err(|e| anyhow!("Frame channel closed: {e}"))?;

        let _ = self.stop_stream(conn).await;
        Ok((*frame).clone())
    }

    async fn start_mock_stream(
        &self,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
    ) -> Result<()> {
        let streaming = self.streaming.clone();
        let frame_tx = self.frame_tx.clone();
        let frame_count = self.frame_count.clone();
        let (x_bin, y_bin) = binning;

        tokio::spawn(async move {
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_size = (binned_width * binned_height) as usize;

            while streaming.get() {
                tokio::time::sleep(Duration::from_millis(exposure_ms as u64)).await;
                if !streaming.get() {
                    break;
                }

                let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                let mut pixels = vec![0u16; frame_size];
                for y in 0..binned_height {
                    for x in 0..binned_width {
                        let value =
                            (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                        pixels[(y * binned_width + x) as usize] = value;
                    }
                }

                // Populate frame metadata using builder pattern (bd-183h)
                let ext_metadata = daq_core::data::FrameMetadata {
                    binning: Some(binning),
                    ..Default::default()
                };
                let frame = Arc::new(
                    Frame::from_u16(binned_width, binned_height, &pixels)
                        .with_frame_number(frame_num)
                        .with_timestamp(Frame::timestamp_now())
                        .with_exposure(exposure_ms)
                        .with_roi_offset(roi.x, roi.y)
                        .with_metadata(ext_metadata),
                );

                // CRITICAL: Broadcast first, then reliable (matches hardware path)
                // This ensures GUI streaming gets frames regardless of pipeline state
                let _ = frame_tx.send(frame.clone());
                if let Some(ref tx) = reliable_tx {
                    // Use try_send to avoid blocking mock stream loop
                    if tx.try_send(frame).is_err() && frame_num.is_multiple_of(100) {
                        tracing::warn!("Mock stream: reliable channel full at frame {}", frame_num);
                    }
                }
            }
        });
        Ok(())
    }

    pub async fn stop_stream(&self, conn: &PvcamConnection) -> Result<()> {
        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        if !self.streaming.get() {
            return Ok(());
        }
        self.streaming.set(false).await?;

        #[cfg(feature = "pvcam_hardware")]
        {
            // Signal callback context to shutdown (bd-ek9n.2)
            // This wakes any waiting thread in the frame loop
            self.callback_context.signal_shutdown();

            // bd-hehw: Take handle under lock, then drop lock before awaiting
            // This prevents holding the mutex guard across the .await point
            let handle = { self.poll_handle.lock().await.take() };
            if let Some(handle) = handle {
                let _ = handle.await;
            }
            if let Some(h) = conn.handle() {
                // bd-g9gq: Use FFI safe wrappers with explicit safety contracts
                ffi_safe::stop_acquisition(h, CCS_HALT);
                // Deregister EOF callback if registered (bd-ek9n.2)
                if self.callback_registered.load(Ordering::Acquire) {
                    ffi_safe::deregister_callback(h, PL_CALLBACK_EOF);
                    self.callback_registered.store(false, Ordering::Release);
                }
            }
            // Clear stored state after cleanup
            self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
            *self.circ_buffer.lock().await = None;
            // bd-g6pr: Clear completion channel so Drop doesn't try to wait again
            if let Ok(mut guard) = self.poll_thread_done_rx.lock() {
                *guard = None;
            }
            if let Ok(mut guard) = self.poll_thread_done_tx.lock() {
                *guard = None;
            }
        }
        Ok(())
    }

    /// bd-3gnv: Sequence mode streaming implementation.
    ///
    /// Uses `pl_exp_setup_seq` + `pl_exp_start_seq` for reliable frame acquisition
    /// when circular buffer mode fails (error 185) or stalls.
    ///
    /// Works in batches of SEQUENCE_BATCH_SIZE frames, polling for completion,
    /// then restarting for continuous streaming.
    #[cfg(feature = "pvcam_hardware")]
    #[allow(clippy::too_many_arguments)]
    async fn start_stream_sequence_impl(
        &self,
        hcam: i16,
        region: rgn_type,
        exposure_ms: f64,
        binning: (u16, u16),
        roi: Roi,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        #[cfg(feature = "arrow_tap")] _arrow_tap: Option<
            tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>,
        >,
        _use_metadata: bool,
    ) -> Result<()> {
        let (x_bin, y_bin) = binning;
        let binned_width = roi.width / x_bin as u32;
        let binned_height = roi.height / y_bin as u32;

        tracing::info!(
            "Starting sequence mode streaming: {}x{} frames, {}ms exposure, batch size {}",
            binned_width,
            binned_height,
            exposure_ms,
            SEQUENCE_BATCH_SIZE
        );

        // Query frame size using pl_exp_setup_seq
        let mut buffer_bytes: uns32 = 0;
        let setup_result = unsafe {
            pl_exp_setup_seq(
                hcam,
                SEQUENCE_BATCH_SIZE,
                1, // region count
                &region as *const _,
                TIMED_MODE,
                exposure_ms as uns32,
                &mut buffer_bytes,
            )
        };

        if setup_result == 0 {
            let err_msg = get_pvcam_error();
            let _ = self.streaming.set(false).await;
            return Err(anyhow!("pl_exp_setup_seq failed: {}", err_msg));
        }

        let frame_bytes = buffer_bytes as usize / SEQUENCE_BATCH_SIZE as usize;
        tracing::info!(
            "Sequence mode: buffer_bytes={}, frame_bytes={}",
            buffer_bytes,
            frame_bytes
        );

        // Store camera handle for Drop cleanup
        self.active_hcam.store(hcam, Ordering::Release);

        // Reset shutdown flag
        self.shutdown.store(false, Ordering::SeqCst);

        let streaming = self.streaming.clone();
        let shutdown = self.shutdown.clone();
        let frame_tx = self.frame_tx.clone();
        let frame_count = self.frame_count.clone();
        let lost_frames = self.lost_frames.clone();
        let width = binned_width;
        let height = binned_height;
        let roi_x = roi.x;
        let roi_y = roi.y;

        // Create completion channel for poll thread synchronization
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        if let Ok(mut guard) = self.poll_thread_done_rx.lock() {
            *guard = Some(done_rx);
        }
        if let Ok(mut guard) = self.poll_thread_done_tx.lock() {
            *guard = Some(done_tx.clone());
        }

        // Spawn blocking task for sequence acquisition loop
        let poll_handle = tokio::task::spawn_blocking(move || {
            Self::frame_loop_sequence(
                hcam,
                region,
                exposure_ms,
                frame_bytes,
                streaming,
                shutdown,
                frame_tx,
                reliable_tx,
                frame_count,
                lost_frames,
                width,
                height,
                roi_x,
                roi_y,
                binning,
                done_tx,
            );
        });

        *self.poll_handle.lock().await = Some(poll_handle);
        Ok(())
    }

    /// bd-3gnv: Sequence mode frame loop (blocking).
    ///
    /// Repeatedly acquires batches of frames using pl_exp_setup_seq/start_seq,
    /// polls for completion, and sends frames to channels.
    #[cfg(feature = "pvcam_hardware")]
    #[allow(clippy::too_many_arguments)]
    fn frame_loop_sequence(
        hcam: i16,
        region: rgn_type,
        exposure_ms: f64,
        frame_bytes: usize,
        streaming: Parameter<bool>,
        shutdown: Arc<AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        frame_count: Arc<AtomicU64>,
        _lost_frames: Arc<AtomicU64>,
        width: u32,
        height: u32,
        roi_x: u32,
        roi_y: u32,
        binning: (u16, u16),
        done_tx: std::sync::mpsc::Sender<()>,
    ) {
        // Main sequence loop
        let mut total_frames: u64 = 0;
        let mut batch_num: u64 = 0;

        while !shutdown.load(Ordering::SeqCst) && streaming.get() {
            batch_num += 1;

            // Setup sequence for batch
            let mut buffer_bytes: uns32 = 0;
            let setup_result = unsafe {
                pl_exp_setup_seq(
                    hcam,
                    SEQUENCE_BATCH_SIZE,
                    1,
                    &region as *const _,
                    TIMED_MODE,
                    exposure_ms as uns32,
                    &mut buffer_bytes,
                )
            };

            if setup_result == 0 {
                tracing::error!("pl_exp_setup_seq failed in loop: {}", get_pvcam_error());
                break;
            }

            // Allocate buffer for batch
            let mut buffer = vec![0u8; buffer_bytes as usize];

            // Start sequence acquisition
            let start_result =
                unsafe { pl_exp_start_seq(hcam, buffer.as_mut_ptr() as *mut std::ffi::c_void) };

            if start_result == 0 {
                tracing::error!("pl_exp_start_seq failed: {}", get_pvcam_error());
                break;
            }

            // Poll for completion
            let mut status: i16 = 0;
            let mut bytes_arrived: uns32 = 0;
            let timeout = std::time::Duration::from_secs(
                ((exposure_ms * SEQUENCE_BATCH_SIZE as f64 / 1000.0) + 5.0) as u64,
            );
            let start_time = std::time::Instant::now();

            loop {
                if shutdown.load(Ordering::SeqCst) || !streaming.get() {
                    unsafe {
                        pl_exp_abort(hcam, CCS_HALT);
                    }
                    break;
                }

                unsafe {
                    pl_exp_check_status(hcam, &mut status, &mut bytes_arrived);
                }

                if status == READOUT_COMPLETE {
                    // Extract frames from buffer
                    for frame_idx in 0..SEQUENCE_BATCH_SIZE {
                        let offset = frame_idx as usize * frame_bytes;
                        if offset + frame_bytes > buffer.len() {
                            break;
                        }

                        // Convert bytes to u16 pixels
                        let pixel_data: Vec<u16> = buffer[offset..offset + frame_bytes]
                            .chunks_exact(2)
                            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                            .collect();

                        total_frames += 1;
                        frame_count.store(total_frames, Ordering::SeqCst);

                        // Build frame (matching mock and hardware path patterns)
                        let ext_metadata = daq_core::data::FrameMetadata {
                            binning: Some(binning),
                            ..Default::default()
                        };
                        let frame = Arc::new(
                            Frame::from_u16(width, height, &pixel_data)
                                .with_frame_number(total_frames)
                                .with_timestamp(Frame::timestamp_now())
                                .with_exposure(exposure_ms)
                                .with_roi_offset(roi_x, roi_y)
                                .with_metadata(ext_metadata),
                        );

                        // Send to channels
                        let _ = frame_tx.send(frame.clone());
                        if let Some(ref tx) = reliable_tx {
                            let _ = tx.blocking_send(frame);
                        }
                    }

                    if batch_num % 10 == 0 {
                        tracing::debug!(
                            "Sequence mode batch {} complete, total frames: {}",
                            batch_num,
                            total_frames
                        );
                    }
                    break;
                }

                if status == READOUT_FAILED {
                    tracing::error!("Sequence readout failed");
                    break;
                }
                if status == READOUT_NOT_ACTIVE
                    && start_time.elapsed() > std::time::Duration::from_millis(100)
                {
                    tracing::warn!("Acquisition not active after 100ms");
                    break;
                }

                if start_time.elapsed() > timeout {
                    tracing::error!("Sequence batch {} timed out after {:?}", batch_num, timeout);
                    unsafe {
                        pl_exp_abort(hcam, CCS_HALT);
                    }
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            // Finish sequence
            unsafe {
                pl_exp_finish_seq(hcam, buffer.as_mut_ptr() as *mut std::ffi::c_void, 0);
            }
        }

        tracing::info!(
            "Sequence mode loop ended: {} total frames in {} batches",
            total_frames,
            batch_num
        );

        // Signal completion
        let _ = done_tx.send(());
    }

    /// Hardware frame retrieval loop with callback support (bd-ek9n.2, bd-ek9n.3)
    ///
    /// When `use_callback` is true, waits on the callback context's condvar for
    /// EOF notifications instead of polling. This reduces CPU usage and latency.
    /// Falls back to polling with 1ms sleep when callbacks aren't available.
    ///
    /// Drains all available frames on each wake to avoid losing events when
    /// multiple callbacks fire while processing.
    ///
    /// # Arguments
    ///
    /// * `hcam` - Open camera handle
    /// * `streaming` - Streaming state parameter
    /// * `shutdown` - Shutdown signal for graceful termination
    /// * `frame_tx` - Broadcast channel for frame delivery
    /// * `reliable_tx` - Optional mpsc channel for reliable delivery
    /// * `arrow_tap` - Optional Arrow array channel (feature-gated)
    /// * `frame_count` - Counter for acquired frames
    /// * `lost_frames` - Counter for lost frames (bd-ek9n.3)
    /// * `discontinuity_events` - Counter for gap events (bd-ek9n.3)
    /// * `last_hw_frame_nr` - Last hardware frame number for gap detection
    /// * `callback_ctx` - Callback context for EOF notifications (bd-ek9n.2)
    /// * `use_callback` - Whether EOF callback is registered
    /// * `frame_bytes` - Actual frame size in bytes from SDK (may include metadata)
    /// * `expected_frame_bytes` - Expected pixel data size (without metadata)
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    /// * `error_tx` - Tokio unbounded channel to signal fatal errors for involuntary stop handling.
    ///                UnboundedSender::send() is non-blocking and safe to call from sync code.
    /// * `use_metadata` - Whether metadata decoding is enabled (Gemini SDK review)
    /// * `metadata_tx` - Optional channel for decoded hardware timestamps
    /// * `roi_x` - ROI X offset in sensor coordinates (bd-183h)
    /// * `roi_y` - ROI Y offset in sensor coordinates (bd-183h)
    /// * `binning` - Binning factors (x, y) for extended metadata (bd-183h)
    /// * `done_tx` - Completion signal sender (bd-g6pr). Sent when the loop exits to signal
    ///               that all SDK calls are complete and Drop can safely call FFI cleanup.
    /// * `circ_ptr` - Pointer to circular buffer (for auto-restart on stall, bd-3gnv)
    /// * `circ_size_bytes` - Size of circular buffer in bytes (for auto-restart)
    #[cfg(feature = "pvcam_hardware")]
    #[allow(clippy::too_many_arguments)]
    fn frame_loop_hardware(
        hcam: i16,
        streaming: Parameter<bool>,
        shutdown: Arc<AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        #[cfg(feature = "arrow_tap")] arrow_tap: Option<
            tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>,
        >,
        frame_count: Arc<AtomicU64>,
        lost_frames: Arc<AtomicU64>,
        discontinuity_events: Arc<AtomicU64>,
        last_hw_frame_nr: Arc<AtomicI32>,
        callback_ctx: Arc<std::pin::Pin<Box<CallbackContext>>>,
        use_callback: bool,
        exposure_ms: f64,
        frame_bytes: usize,
        expected_frame_bytes: usize,
        width: u32,
        height: u32,
        error_tx: tokio::sync::mpsc::UnboundedSender<AcquisitionError>,
        use_metadata: bool,
        roi_x: u32,
        roi_y: u32,
        binning: (u16, u16),
        metadata_tx: Option<tokio::sync::mpsc::Sender<FrameMetadata>>,
        done_tx: std::sync::mpsc::Sender<()>,
        circ_ptr: *mut u8,
        circ_size_bytes: u32,
    ) {
        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;
        let mut consecutive_timeouts: u32 = 0;
        const CALLBACK_WAIT_TIMEOUT_MS: u64 = 100; // Short timeout to check shutdown
                                                   // FORCE LONG TIMEOUT for debugging
        let max_consecutive_timeouts: u32 = 1000; // 100s

        if use_callback {
            tracing::debug!("Using EOF callback mode for frame acquisition");
        } else {
            tracing::debug!("Using polling mode for frame acquisition");
        }

        // ... (existing md_frame logic) ... check file content

        // Inside loop:

        // Gemini SDK review: Create md_frame struct for metadata decoding
        // This struct holds pointers into the frame buffer for extracting timestamps.
        // Must be created before the loop and released after.
        // bd-g9gq: Use FFI safe wrapper with explicit safety contract
        //
        // bd-2q8j: Allocate space for 16 ROIs (PVCAM maximum) to prevent buffer overflow.
        // The camera can return multiple ROIs in centroids mode or with multi-ROI acquisition.
        // If we allocate for only 1 ROI but pl_md_frame_decode finds more, it writes past
        // the allocated buffer causing heap corruption and silent crashes (~35 frames in).
        // 16 is the PVCAM SDK maximum for multi-ROI acquisition.
        const MAX_ROIS: u16 = 16;
        let md_frame_ptr: *mut md_frame = if use_metadata {
            match ffi_safe::create_md_frame(MAX_ROIS) {
                Some(ptr) => {
                    tracing::debug!(
                        "Created md_frame struct for {} ROIs for metadata decoding",
                        MAX_ROIS
                    );
                    ptr
                }
                None => {
                    tracing::warn!("Failed to create md_frame struct, metadata decoding disabled");
                    std::ptr::null_mut()
                }
            }
        } else {
            std::ptr::null_mut()
        };

        // Track when receiver count became zero for graceful disconnect (bd-cckz)
        // Auto-stop acquisition after 5 seconds of no subscribers
        let mut no_subscribers_since: Option<std::time::Instant> = None;
        const NO_SUBSCRIBER_TIMEOUT: Duration = Duration::from_secs(5);

        // Check both streaming flag and shutdown signal (bd-z8q8).
        // Shutdown is set in Drop to ensure the loop exits before SDK uninit.
        // Use Acquire ordering to synchronize with Release store in Drop (bd-nfk6).
        let mut loop_iteration: u64 = 0;

        while streaming.get() && !shutdown.load(Ordering::Acquire) {
            loop_iteration += 1;
            // Wait for frame notification (callback mode) or poll (fallback mode)
            // bd-g9gq: Use FFI safe wrapper with explicit safety contract
            let has_frames = if use_callback {
                // Callback mode (bd-ek9n.2): Wait on condvar with timeout
                // Returns number of pending frames (0 on timeout/shutdown)
                let pending = callback_ctx.wait_for_frames(CALLBACK_WAIT_TIMEOUT_MS);
                if pending > 0 {
                    true
                } else {
                    // Fallback: if callbacks are missed, avoid deadlock by occasionally checking status.
                    match ffi_safe::check_cont_status(hcam) {
                        Ok((_, _, cnt)) => cnt > 0,
                        Err(()) => false,
                    }
                }
            } else {
                // Polling mode fallback: Check status with 1ms delay
                match ffi_safe::check_cont_status(hcam) {
                    Ok((_, _, cnt)) => {
                        buffer_cnt = cnt;
                        // Only treat as "has frames" when PVCAM reports filled buffers.
                        // Treating EXPOSURE_IN_PROGRESS as "has frames" causes a hot-spin when no frame is ready yet.
                        cnt > 0
                    }
                    Err(()) => break,
                }
            };

            if !has_frames {
                if !use_callback {
                    // Polling mode: sleep between checks
                    std::thread::sleep(Duration::from_millis(1));
                }
                consecutive_timeouts += 1;

                // DIAGNOSTIC PROBE: Warn every 1 second (bd-3gnv debug)
                if consecutive_timeouts % 10 == 0 {
                    let (st, bytes, cnt) = match ffi_safe::check_cont_status(hcam) {
                        Ok(vals) => vals,
                        Err(_) => (-999, 0, 0),
                    };
                    // bd-3gnv: Get SDK error code when status is READOUT_NOT_ACTIVE (0)
                    let err_code = if st == 0 {
                        unsafe { pl_error_code() }
                    } else {
                        0
                    };
                    // bd-3gnv: Use eprintln for guaranteed output during debugging
                    eprintln!(
                        "[PVCAM DEBUG] Timeouts: {}, Status: {}, Bytes: {}, BufferCnt: {}, streaming: {}, callback_pending: {}, err_code: {}",
                        consecutive_timeouts,
                        st,
                        bytes,
                        cnt,
                        streaming.get(),
                        callback_ctx.pending_frames.load(Ordering::Acquire),
                        err_code
                    );
                    tracing::warn!(
                        "DIAGNOSTIC: Timeouts: {}, Status: {}, Bytes: {}, BufferCnt: {}, err_code: {}",
                        consecutive_timeouts,
                        st,
                        bytes,
                        cnt,
                        err_code
                    );

                    // bd-3gnv: Detect 85-frame stall and auto-restart
                    // Signature: 10+ timeouts (1s), status is READOUT_NOT_ACTIVE (0), 80+ frames received
                    if consecutive_timeouts >= 10
                        && st == 0
                        && frame_count.load(Ordering::Relaxed) >= 80
                    {
                        eprintln!(
                            "[PVCAM DEBUG] Detected 85-frame stall (timeouts={}, status={}, frames={}) - attempting auto-restart",
                            consecutive_timeouts, st, frame_count.load(Ordering::Relaxed)
                        );
                        tracing::info!(
                            "PVCAM stall detected at {} frames - attempting auto-restart (bd-3gnv)",
                            frame_count.load(Ordering::Relaxed)
                        );

                        // Step 1: Stop acquisition with CCS_CLEAR to fully reset camera state
                        // CCS_CLEAR may reset more internal state than CCS_HALT
                        ffi_safe::stop_acquisition(hcam, CCS_CLEAR);

                        // Step 2: Extended delay for camera to fully reset
                        // The camera may need more time after CCS_CLEAR
                        std::thread::sleep(std::time::Duration::from_millis(200));

                        // Step 3: Full restart with setup + start (camera may need re-setup)
                        match ffi_safe::full_restart_acquisition(
                            hcam,
                            roi_x,
                            roi_y,
                            width,
                            height,
                            binning,
                            exposure_ms,
                            circ_ptr,
                            circ_size_bytes,
                        ) {
                            Ok(_new_frame_bytes) => {
                                // Step 4: Re-register EOF callback (setup may invalidate it)
                                if use_callback {
                                    let callback_ctx_ptr =
                                        &**callback_ctx as *const CallbackContext;
                                    if !ffi_safe::register_eof_callback(hcam, callback_ctx_ptr) {
                                        tracing::warn!("Failed to re-register EOF callback after restart (bd-3gnv)");
                                    }
                                }

                                tracing::info!("PVCAM full auto-restart succeeded (bd-3gnv)");

                                // Reset state for new acquisition cycle
                                callback_ctx.pending_frames.store(0, Ordering::Release);
                                last_hw_frame_nr.store(-1, Ordering::Release);
                                consecutive_timeouts = 0;
                                continue; // Resume waiting for frames
                            }
                            Err(err_msg) => {
                                tracing::error!(
                                    "PVCAM full auto-restart failed: {} (bd-3gnv)",
                                    err_msg
                                );
                                // Continue to max_consecutive_timeouts check - will eventually timeout
                            }
                        }
                    }
                }

                if consecutive_timeouts >= max_consecutive_timeouts {
                    tracing::warn!("Frame loop: max consecutive timeouts reached");
                    // Gemini SDK review: Signal involuntary stop on timeout
                    let _ = error_tx.send(AcquisitionError::Timeout);
                    break;
                }
                continue;
            }
            consecutive_timeouts = 0;

            // Drain loop: process all available frames to avoid losing events
            // when multiple callbacks fire while we're processing
            let mut frames_processed_in_drain: u32 = 0;
            let mut consecutive_duplicates: u32 = 0;
            let mut fatal_error = false;
            let mut unlock_failures: u32 = 0; // bd-3gnv: Track unlock failures
            let mut stall_detected = false; // bd-3gnv: Flag for auto-restart

            // bd-3gnv: Duplicate detection is handled by immediate exit on any duplicate.
            // The drain loop breaks as soon as a duplicate is detected, returning to
            // the outer loop to wait for the next callback signal.

            // Stack-allocated FRAME_INFO for pl_exp_get_oldest_frame_ex (bd-ek9n.3)
            // Using zeroed struct as PVCAM will fill in the fields on frame retrieval.
            let mut frame_info: FRAME_INFO = unsafe { std::mem::zeroed() };

            // bd-3gnv: Now checking buffer_cnt on every iteration to prevent stale frame loop.
            // This is necessary because the SDK returns stale frames if we don't verify availability first.

            loop {
                // Check shutdown between frames
                if !streaming.get() || shutdown.load(Ordering::Acquire) {
                    break;
                }

                // bd-3gnv FIX: Check if there are actually frames available BEFORE calling get_oldest_frame.
                // The SDK can return stale frames if we don't check properly.
                // This prevents the 60-million-iteration busy loop caused by the SDK returning
                // the same frame repeatedly after the camera stops producing new frames.
                //
                // Strategy: Use callback pending_frames as primary indicator (more reliable),
                // but also check buffer_cnt as a secondary signal. Exit drain loop when BOTH
                // the callback says no frames pending AND the SDK says buffer is empty.
                let pending = callback_ctx.pending_frames.load(Ordering::Acquire);
                let (check_status, _, buf_cnt) = match ffi_safe::check_cont_status(hcam) {
                    Ok(result) => result,
                    Err(()) => {
                        tracing::error!("PVCAM status check failed");
                        let _ = error_tx.send(AcquisitionError::StatusCheckFailed);
                        fatal_error = true;
                        break;
                    }
                };
                status = check_status;
                buffer_cnt = buf_cnt;

                // Check for fatal errors
                if status == READOUT_FAILED {
                    tracing::error!("PVCAM readout failed");
                    let _ = error_tx.send(AcquisitionError::ReadoutFailed);
                    fatal_error = true;
                    break;
                }

                // bd-3gnv: Detect unexpected READOUT_NOT_ACTIVE condition
                // This indicates the SDK internally stopped acquisition without an error.
                // This can happen if the buffer fills up and the SDK times out waiting
                // for buffers to be unlocked (even though we are unlocking them).
                if status == READOUT_NOT_ACTIVE && frames_processed_in_drain > 0 {
                    eprintln!(
                        "[PVCAM DEBUG] WARNING: READOUT_NOT_ACTIVE detected mid-drain (frames_processed={}, buffer_cnt={}) - will attempt restart",
                        frames_processed_in_drain, buffer_cnt
                    );
                    tracing::warn!(
                        "PVCAM acquisition unexpectedly inactive after {} frames (buffer_cnt={}) - attempting restart (bd-3gnv)",
                        frame_count.load(Ordering::Relaxed),
                        buffer_cnt
                    );
                    // Set flag for auto-restart after drain loop
                    stall_detected = true;
                    break;
                }

                // bd-3gnv FIX: Only attempt to get a frame if either:
                // 1. Callback says frames are pending, OR
                // 2. SDK buffer has frames available
                // Exit if BOTH say no frames, to prevent stale frame spin.
                if pending == 0 && buffer_cnt == 0 {
                    // No frames available from either source - exit drain loop
                    break;
                }

                // bd-circ: FIFO-only strategy using get_oldest_frame + unlock_oldest_frame.
                // This ensures every frame is drained in order and prevents buffer leaks.
                let frame_ptr = match ffi_safe::get_oldest_frame(hcam, &mut frame_info) {
                    Ok(ptr) => ptr,
                    Err(()) => {
                        // No more frames available - exit drain loop normally
                        break;
                    }
                };
                frames_processed_in_drain += 1;

                // Remaining frame processing is in an unsafe block for pointer operations
                unsafe {
                    // Frame loss detection (bd-ek9n.3): Check for gaps in FrameNr sequence
                    // FrameNr is 1-based hardware counter from PVCAM
                    let current_frame_nr = frame_info.FrameNr;
                    let prev_frame_nr = last_hw_frame_nr.load(Ordering::Acquire);

                    if prev_frame_nr >= 0 {
                        // Not the first frame - check for gaps
                        let expected_frame_nr = prev_frame_nr + 1;
                        if current_frame_nr > expected_frame_nr {
                            // Gap detected: frames were lost between prev and current
                            let frames_lost = (current_frame_nr - expected_frame_nr) as u64;
                            lost_frames.fetch_add(frames_lost, Ordering::Relaxed);
                            discontinuity_events.fetch_add(1, Ordering::Relaxed);
                            tracing::debug!(
                                "Frame skip detected: expected {}, got {} ({} frames skipped)",
                                expected_frame_nr,
                                current_frame_nr,
                                frames_lost
                            );
                        } else if current_frame_nr == prev_frame_nr {
                            // Duplicate frame detected (bd-ha3w): same FrameNr as previous
                            // This happens when the SDK returns the same buffer before new data arrives.
                            // bd-3gnv FIX: Exit drain loop IMMEDIATELY on duplicate.
                            // Continuing would just get the same stale frame again.
                            // Return to outer loop to wait for next callback signal.
                            discontinuity_events.fetch_add(1, Ordering::Relaxed);
                            consecutive_duplicates += 1;

                            // bd-circ: In CIRC_NO_OVERWRITE mode, ALWAYS release frames
                            // to drain the buffer, regardless of which get function was used.
                            if !USE_CIRC_OVERWRITE_MODE {
                                if !ffi_safe::release_oldest_frame(hcam) {
                                    unlock_failures += 1;
                                }
                            }
                            if use_callback {
                                callback_ctx.consume_one();
                            }

                            // bd-3gnv: Exit drain loop immediately on ANY duplicate.
                            // The SDK is returning stale data because no new frame is ready.
                            // Waiting in outer loop for next callback is more efficient than spinning.
                            break; // Exit drain loop, wait for next callback
                        } else if current_frame_nr < expected_frame_nr && current_frame_nr != 1 {
                            // Frame number went backwards (not due to wrap to 1)
                            // This is unexpected but log it as discontinuity
                            discontinuity_events.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                "Frame number discontinuity: expected {}, got {} (possible SDK reset)",
                                expected_frame_nr,
                                current_frame_nr
                            );
                        }
                    }
                    // Update last seen frame number
                    last_hw_frame_nr.store(current_frame_nr, Ordering::Release);
                    // bd-3gnv: Reset duplicate counter on successful new frame
                    consecutive_duplicates = 0;

                    // Gemini SDK review: Decode frame metadata for hardware timestamps
                    // This must happen before copying pixel data as metadata parsing identifies
                    // the pixel data offset within the buffer.
                    // bd-g9gq: Use FFI safe wrapper with explicit safety contract
                    let frame_metadata = if !md_frame_ptr.is_null() {
                        // Decode the metadata-enabled frame buffer
                        if ffi_safe::decode_frame_metadata(
                            md_frame_ptr,
                            frame_ptr,
                            frame_bytes as uns32,
                        ) {
                            // Successfully decoded - extract timestamps
                            let hdr = &*(*md_frame_ptr).header;
                            let ts_res = hdr.timestampResNs as u64;
                            let exp_res = hdr.exposureTimeResNs as u64;
                            Some(FrameMetadata {
                                frame_nr: hdr.frameNr,
                                timestamp_bof_ns: (hdr.timestampBOF as u64) * ts_res,
                                timestamp_eof_ns: (hdr.timestampEOF as u64) * ts_res,
                                exposure_time_ns: (hdr.exposureTime as u64) * exp_res,
                                bit_depth: hdr.bitDepth,
                                roi_count: hdr.roiCount,
                            })
                        } else {
                            tracing::warn!(
                                "pl_md_frame_decode failed for frame {}",
                                current_frame_nr
                            );
                            None
                        }
                    } else {
                        None
                    };

                    // Memory optimization (bd-ek9n.5): Single copy from SDK buffer.
                    // When metadata is enabled, the pixel data starts after the header.
                    // For simplicity, we copy expected_frame_bytes from the buffer start
                    // (metadata header is at the END of the buffer per PVCAM design).
                    let copy_bytes = frame_bytes.min(expected_frame_bytes);
                    let sdk_bytes = std::slice::from_raw_parts(frame_ptr as *const u8, copy_bytes);
                    let pixel_data = sdk_bytes.to_vec();

                    // Zero-frame detection (bd-ha3w): Check if frame contains valid data
                    // Sample several positions to detect all-zero frames which indicate
                    // either buffer corruption or reading before SDK finished writing.
                    // Real camera data typically has noise even in dark frames.
                    let sample_positions = [
                        copy_bytes / 4,
                        copy_bytes / 2,
                        copy_bytes * 3 / 4,
                        copy_bytes - 1,
                    ];
                    let has_nonzero = sample_positions
                        .iter()
                        .any(|&pos| pos < pixel_data.len() && pixel_data[pos] != 0);
                    if !has_nonzero && copy_bytes > 1000 {
                        // Frame appears to be all zeros - likely corrupted or race condition
                        discontinuity_events.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            "Zero-frame detected for FrameNr {}: buffer appears uninitialized, skipping (bd-ha3w)",
                            current_frame_nr
                        );
                        // bd-circ: In CIRC_NO_OVERWRITE mode, ALWAYS release frames to drain buffer
                        if !USE_CIRC_OVERWRITE_MODE {
                            if !ffi_safe::release_oldest_frame(hcam) {
                                unlock_failures += 1;
                            }
                        }
                        if use_callback {
                            callback_ctx.consume_one();
                        }
                        continue; // Skip to next frame
                    }

                    // bd-circ: In CIRC_NO_OVERWRITE mode, ALWAYS release frames to drain
                    // the buffer. unlock_oldest_frame removes the oldest frame; without it
                    // the buffer fills and acquisition stalls (~85 frames).
                    if !USE_CIRC_OVERWRITE_MODE {
                        if !ffi_safe::release_oldest_frame(hcam) {
                            unlock_failures += 1;
                        }
                    }

                    // Decrement pending frame counter (callback mode)
                    if use_callback {
                        callback_ctx.consume_one();
                    }

                    // Create Frame with ownership transfer - no additional copy (bd-ek9n.5)
                    // Populate metadata using builder pattern (bd-183h)
                    let mut frame = Frame::from_bytes(width, height, 16, pixel_data)
                        .with_frame_number(current_frame_nr as u64)
                        .with_roi_offset(roi_x, roi_y);

                    // Use hardware timestamps/exposure when available, fall back to software values
                    if let Some(ref md) = frame_metadata {
                        frame = frame
                            .with_timestamp(md.timestamp_bof_ns)
                            .with_exposure(md.exposure_time_ns as f64 / 1_000_000.0);
                    } else {
                        // Software fallback: use system time and configured exposure
                        frame = frame
                            .with_timestamp(Frame::timestamp_now())
                            .with_exposure(exposure_ms);
                    }

                    // Add extended metadata (bd-183h)
                    let ext_metadata = daq_core::data::FrameMetadata {
                        binning: Some(binning),
                        ..Default::default()
                    };
                    frame = frame.with_metadata(ext_metadata);

                    frame_count.fetch_add(1, Ordering::Relaxed);
                    let frame_arc = Arc::new(frame);

                    // Deliver to channels
                    // CRITICAL: Send to broadcast FIRST before reliable path.
                    // The reliable path uses blocking_send which can block if the
                    // measurement pipeline is backpressured. Sending to broadcast
                    // first ensures GUI streaming gets frames regardless.
                    let receiver_count = frame_tx.receiver_count();
                    if receiver_count == 0 {
                        // Track when we lost all subscribers (bd-cckz)
                        if no_subscribers_since.is_none() {
                            no_subscribers_since = Some(std::time::Instant::now());
                            tracing::info!(
                                "No broadcast subscribers, starting {} second disconnect timer",
                                NO_SUBSCRIBER_TIMEOUT.as_secs()
                            );
                        } else if let Some(since) = no_subscribers_since {
                            if since.elapsed() >= NO_SUBSCRIBER_TIMEOUT {
                                tracing::info!(
                                    "No subscribers for {} seconds, stopping acquisition (bd-cckz)",
                                    NO_SUBSCRIBER_TIMEOUT.as_secs()
                                );
                                break;
                            }
                        }
                        tracing::warn!(
                            "Dropping frame {}: no active broadcast subscribers",
                            current_frame_nr
                        );
                    } else {
                        // Reset timer when subscribers reconnect
                        if no_subscribers_since.is_some() {
                            tracing::info!("Subscriber reconnected, canceling disconnect timer");
                            no_subscribers_since = None;
                        }
                        if current_frame_nr % 30 == 1 {
                            tracing::debug!(
                                "Sending frame {} to {} broadcast subscribers",
                                current_frame_nr,
                                receiver_count
                            );
                        }
                    }
                    let _ = frame_tx.send(frame_arc.clone());

                    // Reliable path: use try_send to avoid blocking the frame loop
                    // If measurement pipeline is slow, frames will be dropped here
                    // rather than blocking broadcast delivery
                    if let Some(ref tx) = reliable_tx {
                        if tx.try_send(frame_arc.clone()).is_err() && current_frame_nr % 100 == 0 {
                            // Rate-limit warnings to avoid log spam at high FPS
                            tracing::warn!(
                                "Reliable channel full, dropping frames around {} for measurement pipeline",
                                current_frame_nr
                            );
                        }
                    }

                    // Arrow tap optimization (bd-ek9n.5)
                    // Use try_send to avoid blocking frame loop
                    #[cfg(feature = "arrow_tap")]
                    if let Some(ref tap) = arrow_tap {
                        use arrow::array::{PrimitiveArray, UInt16Type};
                        use arrow::buffer::Buffer;
                        // Frame.data is a public Vec<u8> field, not a method
                        let buffer = Buffer::from(frame_arc.data.clone());
                        let arr =
                            Arc::new(PrimitiveArray::<UInt16Type>::new(Arc::new(buffer), None));
                        let _ = tap.try_send(arr); // Non-blocking: drop if slow
                    }

                    // Gemini SDK review: Send metadata through channel if available
                    // Use try_send to avoid blocking frame loop
                    if let (Some(md), Some(ref tx)) = (frame_metadata, &metadata_tx) {
                        let _ = tx.try_send(md); // Non-blocking: drop if slow
                    }
                }
            }

            // bd-3gnv: Critical warning if unlocks are failing - this causes buffer starvation
            if unlock_failures > 0 {
                tracing::error!(
                    "PVCAM unlock failures: {} in drain loop (bd-3gnv)",
                    unlock_failures
                );
            }

            // bd-3gnv: Auto-restart acquisition if stall detected (workaround for 85-frame limit)
            // The Prime BSI camera stops producing frames after exactly 85 frames regardless of
            // buffer size, exposure time, or unlock behavior. Restarting acquisition works around this.
            if stall_detected {
                tracing::info!(
                    "PVCAM auto-restart: stopping acquisition to recover from stall (bd-3gnv)"
                );

                // Step 1: Stop acquisition
                ffi_safe::stop_acquisition(hcam, CCS_HALT);

                // Step 2: Brief delay for camera to settle
                std::thread::sleep(std::time::Duration::from_millis(10));

                // Step 3: Restart acquisition with same buffer
                match ffi_safe::restart_acquisition(hcam, circ_ptr, circ_size_bytes) {
                    Ok(()) => {
                        tracing::info!(
                            "PVCAM auto-restart succeeded after {} frames (bd-3gnv)",
                            frame_count.load(Ordering::Relaxed)
                        );

                        // Step 4: Reset callback context (clear pending frames)
                        callback_ctx.pending_frames.store(0, Ordering::Release);

                        // Step 5: Reset frame number tracking to avoid false gap detection
                        // Set to -1 so next frame (whatever its FrameNr) is treated as "first"
                        last_hw_frame_nr.store(-1, Ordering::Release);

                        // Continue to next iteration - acquisition should resume
                        continue;
                    }
                    Err(err_msg) => {
                        tracing::error!("PVCAM auto-restart failed: {} (bd-3gnv)", err_msg);
                        // Don't set fatal_error - let outer loop continue with timeout handling
                        // The camera may recover on its own, or user can stop/restart manually
                    }
                }
            }

            // Gemini SDK review: Exit outer loop on fatal error to prevent zombie streaming
            if fatal_error {
                tracing::error!("Exiting frame loop due to fatal acquisition error");
                break;
            }

            // Fix for pending_frames getting stuck (medium priority issue):
            // If pending_frames counter is out of sync with actual frames available,
            // avoid a busy-loop where pending_frames>0 prevents waiting, but no frame can be retrieved.
            //
            // Do NOT assume the callback implies the oldest frame is immediately retrievable.
            // If we couldn't retrieve any frames, clear pending_frames and rely on the callback timeout
            // fallback status check above to avoid deadlock if the callback was early/missed.
            if use_callback {
                let remaining = callback_ctx.pending_frames.load(Ordering::Acquire);
                if remaining > 0 && frames_processed_in_drain == 0 {
                    // Callback said frames were ready, but we couldn't retrieve any.
                    // Confirm there's really no data available and then clear pending_frames to avoid spin.
                    let mut has_buffered_frames = false;
                    unsafe {
                        if pl_exp_check_cont_status(
                            hcam,
                            &mut status,
                            &mut bytes_arrived,
                            &mut buffer_cnt,
                        ) != 0
                        {
                            has_buffered_frames = buffer_cnt > 0;
                        }
                    }

                    if !has_buffered_frames {
                        tracing::warn!(
                            "pending_frames desync: {} pending but 0 retrieved; clearing pending counter and continuing",
                            remaining
                        );
                        callback_ctx.pending_frames.store(0, Ordering::Release);
                        // bd-3gnv: Use yield_now() instead of sleep(1ms) to reduce latency
                        // while still preventing tight busy-loop during pending_frames desync.
                        std::thread::yield_now();
                    }
                }
            }
        }

        // bd-3gnv: Debug why we exited the outer loop
        eprintln!(
            "[PVCAM DEBUG] Frame loop exited: iter={}, streaming={}, shutdown={}",
            loop_iteration,
            streaming.get(),
            shutdown.load(Ordering::Acquire)
        );

        // Gemini SDK review: Release md_frame struct if it was allocated
        // bd-g9gq: Use FFI safe wrapper with explicit safety contract
        if !md_frame_ptr.is_null() {
            ffi_safe::release_md_frame(md_frame_ptr);
            tracing::debug!("Released md_frame struct");
        }

        // Log acquisition summary with frame loss statistics (bd-ek9n.3)
        let total_frames = frame_count.load(Ordering::Relaxed);
        let total_lost = lost_frames.load(Ordering::Relaxed);
        let total_discontinuities = discontinuity_events.load(Ordering::Relaxed);

        if total_lost > 0 || total_discontinuities > 0 {
            tracing::warn!(
                "PVCAM acquisition ended: {} frames captured, {} frames lost, {} discontinuity events",
                total_frames,
                total_lost,
                total_discontinuities
            );
        } else {
            tracing::info!(
                "PVCAM acquisition ended: {} frames captured (no frame loss detected)",
                total_frames
            );
        }

        // NOTE: We do NOT call pl_exp_stop_cont here - that's done in stop_stream()
        // after the poll handle is awaited. Calling it here would race with
        // stop_stream() and could cause issues. The frame loop exits gracefully
        // via the shutdown flag, then stop_stream() does cleanup.

        // bd-g6pr: Signal completion to Drop so it knows all SDK calls are done.
        // This MUST be the last thing we do before returning, ensuring no SDK
        // calls can happen after this signal is sent.
        let _ = done_tx.send(());
        tracing::debug!("PVCAM frame loop signaled completion");
    }
}

/// Drop implementation ensures frame loop is stopped and PVCAM is cleaned up (bd-z8q8).
///
/// CRITICAL SAFETY FIX: Must stop camera and deregister callback BEFORE freeing buffers.
/// Without this, dropping PvcamDriver without calling stop_stream() would:
/// 1. Allow the frame loop to continue calling PVCAM SDK functions while SDK is uninitialized
/// 2. Leave PVCAM holding a dangling pointer to the freed callback context
/// 3. Cause use-after-free when PVCAM tries to invoke the callback
impl Drop for PvcamAcquisition {
    fn drop(&mut self) {
        #[cfg(feature = "pvcam_hardware")]
        {
            // Signal the frame loop to stop via the shutdown flag.
            // The frame loop checks this flag on each iteration and will exit promptly.
            // Use Release ordering to synchronize with Acquire load in frame loop (bd-nfk6).
            self.shutdown.store(true, Ordering::Release);

            // Signal callback context shutdown to wake any waiting threads (bd-ek9n.2)
            self.callback_context.signal_shutdown();
            tracing::debug!("Set PVCAM shutdown flag and signaled callback context in Drop");

            // bd-g6pr: Wait for poll thread to fully exit before calling FFI cleanup.
            // This fixes the race condition where pl_exp_stop_cont was called while
            // pl_exp_get_oldest_frame_ex was still executing in the poll thread.
            //
            // CRITICAL: spawn_blocking tasks cannot be cancelled with abort() - they
            // continue running until completion. We MUST wait for the thread to exit
            // naturally (via the shutdown flag) before calling any FFI cleanup.
            //
            // Use recv_timeout to avoid hanging forever if something goes wrong.
            const POLL_THREAD_TIMEOUT: Duration = Duration::from_secs(5);
            let poll_thread_exited = if let Ok(guard) = self.poll_thread_done_rx.lock() {
                if let Some(ref rx) = *guard {
                    match rx.recv_timeout(POLL_THREAD_TIMEOUT) {
                        Ok(()) => {
                            tracing::debug!("PVCAM poll thread exited cleanly in Drop");
                            true
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            tracing::error!(
                                "PVCAM poll thread did not exit within {:?} - proceeding with cleanup anyway (may cause UB)",
                                POLL_THREAD_TIMEOUT
                            );
                            false
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            // Sender was dropped, which means the poll thread exited
                            // (possibly before we could receive the signal)
                            tracing::debug!("PVCAM poll thread completion channel disconnected (thread already exited)");
                            true
                        }
                    }
                } else {
                    // No receiver = no active poll thread (stream was never started or already stopped)
                    tracing::debug!("No active PVCAM poll thread to wait for");
                    true
                }
            } else {
                // Lock poisoned - unusual but try to proceed
                tracing::warn!("Could not acquire poll_thread_done_rx lock in Drop");
                false
            };

            // Clean up the JoinHandle (optional - it will be dropped anyway, but this
            // prevents any "task not awaited" warnings and clears the Option)
            if let Ok(mut guard) = self.poll_handle.try_lock() {
                // Don't abort - just drop the handle. The thread has already exited
                // (or we timed out and are proceeding anyway).
                let _ = guard.take();
            }

            // CRITICAL SAFETY: Stop camera and deregister callback BEFORE buffer/context are freed.
            // This prevents use-after-free where PVCAM might try to:
            // 1. Write to the circular buffer after it's deallocated
            // 2. Invoke the EOF callback after the context is freed
            //
            // Uses atomic load for lock-free access - no risk of deadlock or UAF from lock contention.
            // If stop_stream() was called properly, active_hcam will be -1 and this is a no-op.
            let hcam = self.active_hcam.swap(-1, Ordering::AcqRel);
            if hcam >= 0 {
                if !poll_thread_exited {
                    // Log extra warning - we're calling FFI while thread may still be running
                    tracing::error!(
                        "Calling pl_exp_stop_cont while poll thread may still be active - risk of SDK race condition"
                    );
                }

                unsafe {
                    // Stop continuous acquisition first (halts camera operation)
                    let stop_result = pl_exp_stop_cont(hcam, CCS_HALT);
                    if stop_result == 0 {
                        tracing::warn!("pl_exp_stop_cont failed in Drop (may already be stopped)");
                    } else {
                        tracing::debug!("Stopped PVCAM acquisition in Drop");
                    }

                    // Deregister callback to prevent use-after-free
                    if self.callback_registered.swap(false, Ordering::AcqRel) {
                        let dereg_result = pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
                        if dereg_result == 0 {
                            tracing::warn!("pl_cam_deregister_callback failed in Drop");
                        } else {
                            tracing::debug!("Deregistered PVCAM EOF callback in Drop");
                        }
                    }
                }
            }

            // Now safe to drop circ_buffer and callback_context (happens automatically)
            // The buffer and context will be freed when Arc refs drop to zero.
        }
    }
}
