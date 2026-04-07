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

mod buffer;
mod callback_context;
#[cfg(feature = "pvcam_sdk")]
mod ffi_safe;
mod frame_loop;
mod metadata;
mod metrics;
mod output;
mod streaming;

pub use buffer::*;
#[cfg(feature = "pvcam_sdk")]
pub use callback_context::*;

/// Streaming configuration parameters bundled into a single struct (bd-rh4k).
///
/// Replaces the 8+ positional parameters that were passed through
/// `start_stream()` → `start_stream_sequence_impl()` → frame loops.
/// `PvcamConnection` and SDK-level handles remain separate since they
/// are runtime state, not configuration.
#[derive(Clone)]
pub struct StreamConfig {
    pub roi: common::core::Roi,
    pub binning: (u16, u16),
    pub exposure_ms: f64,
    pub buffer_mode: crate::components::features::BufferMode,
    pub host_summing_enabled: common::parameter::Parameter<bool>,
    pub host_summing_count: common::parameter::Parameter<u32>,
    pub smart_stream_enabled: common::parameter::Parameter<bool>,
    pub smart_stream_exposures: common::parameter::Parameter<String>,
    pub prime_locate_enabled: common::parameter::Parameter<bool>,
    pub prime_enhance_enabled: common::parameter::Parameter<bool>,
    /// Multi-ROI configuration (bd-oqo7.4). Empty vec = single ROI mode.
    pub multi_roi_regions: Vec<RoiRegion>,
}

/// A single ROI region for Multi-ROI acquisition (bd-oqo7.4).
///
/// Prime BSI supports up to 16 overlapping ROIs. Each ROI is defined by
/// sensor-coordinate origin (x, y) and dimensions (w, h). The camera reads
/// only the specified regions, dramatically increasing frame rate for
/// sparse readout patterns (e.g., echelle spectroscopy with ~74 active orders).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoiRegion {
    /// X offset in sensor coordinates (pixels).
    pub x: u16,
    /// Y offset in sensor coordinates (pixels).
    pub y: u16,
    /// Width in pixels.
    pub w: u16,
    /// Height in pixels.
    pub h: u16,
}

/// Maximum number of ROIs supported by PVCAM (Prime BSI FPGA limit).
pub const MAX_ROI_COUNT: usize = 16;

impl RoiRegion {
    /// Parse a JSON array of ROI regions with validation.
    ///
    /// Returns an error if:
    /// - JSON is malformed
    /// - More than 16 ROIs specified
    /// - Any ROI has zero width or height
    /// - Any ROI extends beyond the sensor bounds
    pub fn parse_json(
        json: &str,
        sensor_width: u16,
        sensor_height: u16,
    ) -> anyhow::Result<Vec<Self>> {
        let regions: Vec<Self> = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("Invalid Multi-ROI JSON: {e}"))?;

        if regions.len() > MAX_ROI_COUNT {
            anyhow::bail!("Too many ROIs: {} (max {})", regions.len(), MAX_ROI_COUNT);
        }

        for (i, roi) in regions.iter().enumerate() {
            if roi.w == 0 || roi.h == 0 {
                anyhow::bail!("ROI {i} has zero dimension: {}x{}", roi.w, roi.h);
            }
            let end_x = roi
                .x
                .checked_add(roi.w)
                .ok_or_else(|| anyhow::anyhow!("ROI {i} x+w overflows u16"))?;
            if end_x > sensor_width {
                anyhow::bail!(
                    "ROI {i} exceeds sensor width: x={} + w={} > {}",
                    roi.x,
                    roi.w,
                    sensor_width
                );
            }
            let end_y = roi
                .y
                .checked_add(roi.h)
                .ok_or_else(|| anyhow::anyhow!("ROI {i} y+h overflows u16"))?;
            if end_y > sensor_height {
                anyhow::bail!(
                    "ROI {i} exceeds sensor height: y={} + h={} > {}",
                    roi.y,
                    roi.h,
                    sensor_height
                );
            }
        }

        Ok(regions)
    }

    /// Total pixel count across all ROIs (for buffer sizing).
    pub fn total_pixels(regions: &[Self]) -> usize {
        regions.iter().map(|r| r.w as usize * r.h as usize).sum()
    }
}

use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_sdk")]
use crate::components::connection::get_pvcam_error;
#[cfg(feature = "pvcam_sdk")]
use crate::components::features::PvcamFeatures;
use crate::components::taps::TapRegistry;
use common::parameter::Parameter;
#[cfg(feature = "pvcam_sdk")]
use pool::buffer_pool::BufferPool;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::Mutex;

#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicI16;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicI32;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::Ordering;
#[cfg(feature = "pvcam_sdk")]
use std::time::Duration;
#[cfg(feature = "pvcam_sdk")]
use tokio::task::JoinHandle;

/// SDK-level streaming state machine for PVCAM acquisition.
///
/// Replaces 7 separate `Arc<Mutex<Option<T>>>` fields with a single enum
/// that makes the Idle/Streaming lifecycle explicit. All fields that are
/// populated in `start_stream()` and cleared in `stop_stream()` live in
/// the `Streaming` variant.
///
/// Fields with a different lifecycle (registration channels set by external
/// callers) remain as separate fields on `PvcamAcquisition`.
#[cfg(feature = "pvcam_sdk")]
pub(super) enum SdkStreamingState {
    /// No active acquisition. All SDK resources are released.
    Idle,
    /// Active acquisition with all associated SDK resources.
    Streaming {
        /// Handle for the blocking frame loop task.
        poll_handle: JoinHandle<()>,
        /// Page-aligned circular buffer for DMA performance (Gemini SDK review).
        /// PVCAM DMA requires 4KB alignment to avoid internal driver copies.
        circ_buffer: PageAlignedBuffer,
        /// Error sender for signaling involuntary stops from frame loop.
        /// Fatal errors (READOUT_FAILED, etc.) are sent here so the driver can
        /// update streaming state.
        error_tx: tokio::sync::mpsc::UnboundedSender<AcquisitionError>,
        /// Pre-allocated buffer pool for zero-allocation frame handling (bd-0dax.3).
        /// Created in `start_stream()` with size based on SDK buffer count.
        frame_pool: BufferPool,
        /// Completion signal receiver for poll thread (bd-g6pr).
        /// Used in Drop to synchronously wait for the poll thread to exit before
        /// calling FFI cleanup functions.
        poll_thread_done_rx: std::sync::mpsc::Receiver<()>,
        /// Owned here for lifetime management; the frame loop holds a clone.
        #[expect(
            dead_code,
            reason = "sender is held here for lifetime management; frame loop holds a clone"
        )]
        poll_thread_done_tx: std::sync::mpsc::Sender<()>,
    },
}

#[cfg(feature = "pvcam_sdk")]
impl SdkStreamingState {
    /// Returns `true` if in the `Streaming` state.
    pub(super) fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming { .. })
    }
}

/// Acquisition error types for involuntary stop signaling (bd-g9po)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionError {
    /// Frame retrieval timed out
    Timeout,
    /// pl_exp_check_cont_status returned an error
    StatusCheckFailed,
    /// pl_exp_get_oldest_frame/pl_exp_get_latest_frame failed
    ReadoutFailed,
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
    pub buffer_mode: Parameter<String>,
    pub frame_count: Arc<AtomicU64>,
    pub frame_tx: tokio::sync::broadcast::Sender<Arc<common::data::Frame>>,

    // --- Registration channels (set by external callers, persist across start/stop) ---
    pub reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<common::data::Frame>>>>>,

    /// Primary output channel for zero-allocation frame delivery (bd-0dax.5).
    /// Single consumer receives LoanedFrame ownership for high-performance streaming.
    pub primary_tx:
        Arc<Mutex<Option<tokio::sync::mpsc::Sender<common::capabilities::LoanedFrame>>>>,

    /// Tap registry for synchronous frame observers (bd-0dax.4).
    /// Taps are called with borrowed frame references before broadcast.
    pub tap_registry: Arc<TapRegistry>,

    /// Optional metadata channel for hardware timestamps (Gemini SDK review).
    /// When enabled, each frame's decoded metadata is sent here alongside the frame data.
    #[cfg(feature = "pvcam_sdk")]
    pub metadata_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<FrameMetadata>>>>,
    /// Whether metadata decoding is enabled for this acquisition.
    #[cfg(feature = "pvcam_sdk")]
    metadata_enabled: Arc<AtomicBool>,

    // --- Counters and error tracking (always available, not tied to streaming state) ---
    /// Frame loss detection counters (bd-ek9n.3).
    /// Total number of frames lost due to buffer overflows or processing delays.
    pub lost_frames: Arc<AtomicU64>,
    /// Number of discontinuity events (gaps in frame sequence).
    pub discontinuity_events: Arc<AtomicU64>,
    /// Number of frames dropped due to pool exhaustion (bd-dmbl).
    /// When the buffer pool is exhausted, frames are dropped with a warning
    /// rather than falling back to heap allocation.
    pub dropped_frames: Arc<AtomicU64>,
    /// Last hardware frame number for gap detection (-1 = uninitialized).
    #[cfg(feature = "pvcam_sdk")]
    last_hardware_frame_nr: Arc<AtomicI32>,

    /// Last error that occurred during acquisition (bd-g9po).
    /// Set when a fatal error causes involuntary stop. Cleared by `clear_error()`.
    last_error: Arc<std::sync::Mutex<Option<AcquisitionError>>>,

    #[cfg(feature = "pvcam_sdk")]
    pub(super) sdk_state: Arc<std::sync::Mutex<SdkStreamingState>>,

    // --- SDK atomics (lock-free, accessed from Drop and frame loops) ---
    #[cfg(feature = "pvcam_sdk")]
    shutdown: Arc<AtomicBool>,
    /// Callback context for EOF notifications (bd-ek9n.2, bd-d9nw.2).
    ///
    /// SAFETY: Arc<Pin<Box<>>> provides critical lifetime guarantees for FFI callback:
    /// - Pin prevents moves, ensuring pointer passed to PVCAM remains valid
    /// - Arc ensures context outlives the acquisition (not dropped until all refs gone)
    /// - Box heap-allocates with stable address
    /// - Raw pointer stored in GLOBAL_CALLBACK_CTX for callback to dereference
    /// - Drop impl deregisters callback BEFORE Arc drops, preventing use-after-free
    #[cfg(feature = "pvcam_sdk")]
    callback_context: Arc<std::pin::Pin<Box<CallbackContext>>>,
    /// Camera handle for cleanup in Drop. Stored during start_stream, cleared in stop_stream.
    /// Uses AtomicI16 with sentinel -1 (invalid handle) for lock-free access in Drop.
    #[cfg(feature = "pvcam_sdk")]
    active_hcam: Arc<AtomicI16>,
    /// Whether EOF callback is registered (for cleanup in Drop)
    #[cfg(feature = "pvcam_sdk")]
    callback_registered: Arc<AtomicBool>,
}

impl PvcamAcquisition {
    pub fn new(streaming: Parameter<bool>, buffer_mode: Parameter<String>) -> Self {
        // bd-3gnv: Increased from 32 to 256 frames to prevent stalls during sustained streaming.
        // At 100 FPS, 32 frames = 0.32s buffer (too small); 256 frames = 2.56s buffer (adequate).
        let (frame_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            streaming,
            buffer_mode,
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            reliable_tx: Arc::new(Mutex::new(None)),

            // Primary output for zero-allocation frame delivery (bd-0dax.5)
            primary_tx: Arc::new(Mutex::new(None)),

            // Tap registry for synchronous frame observers (bd-0dax.4)
            tap_registry: Arc::new(TapRegistry::new()),

            // Metadata channel and state (Gemini SDK review)
            // Default to true: metadata decoding is always enabled (bd-oqo7.2)
            #[cfg(feature = "pvcam_sdk")]
            metadata_tx: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_sdk")]
            metadata_enabled: Arc::new(AtomicBool::new(true)),

            // Frame loss detection counters (bd-ek9n.3)
            lost_frames: Arc::new(AtomicU64::new(0)),
            discontinuity_events: Arc::new(AtomicU64::new(0)),
            // Pool exhaustion counter (bd-dmbl)
            dropped_frames: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "pvcam_sdk")]
            last_hardware_frame_nr: Arc::new(AtomicI32::new(-1)), // -1 = uninitialized

            // Error tracking (bd-g9po)
            last_error: Arc::new(std::sync::Mutex::new(None)),

            // SDK streaming state machine: starts Idle, transitions to Streaming
            // in start_stream(), back to Idle in stop_stream()
            #[cfg(feature = "pvcam_sdk")]
            sdk_state: Arc::new(std::sync::Mutex::new(SdkStreamingState::Idle)),

            #[cfg(feature = "pvcam_sdk")]
            shutdown: Arc::new(AtomicBool::new(false)),
            // Pinned callback context for EOF notifications (bd-ek9n.2, bd-ffi-sdk-match)
            // Initially created with -1 (invalid handle); hcam is updated before callback registration
            #[cfg(feature = "pvcam_sdk")]
            callback_context: Arc::new(Box::pin(CallbackContext::new(-1))),
            // Camera handle and callback state for Drop cleanup
            // -1 is sentinel for "no active handle"
            #[cfg(feature = "pvcam_sdk")]
            active_hcam: Arc::new(AtomicI16::new(-1)),
            #[cfg(feature = "pvcam_sdk")]
            callback_registered: Arc::new(AtomicBool::new(false)),
        }
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
        #[cfg(feature = "pvcam_sdk")]
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
            // Use blocking_lock() instead of try_lock() to ensure we reliably acquire
            // the mutex during shutdown. try_lock() can fail if another task holds it,
            // which would leak streaming resources and skip FFI cleanup.
            //
            // Use recv_timeout to avoid hanging forever if something goes wrong.
            const POLL_THREAD_TIMEOUT: Duration = Duration::from_secs(5);

            // Extract streaming resources into a local. CRITICAL SAFETY: The
            // circ_buffer must remain alive until AFTER pl_exp_stop_cont and
            // callback deregistration complete, because the PVCAM SDK holds raw
            // pointers into the buffer during acquisition.
            let mut guard = self.sdk_state.lock().expect("sdk_state poisoned");
            let old_state = std::mem::replace(&mut *guard, SdkStreamingState::Idle);
            // Release the lock immediately — we only needed it for the swap.
            drop(guard);

            let (poll_thread_exited, _circ_buffer_guard) = match old_state {
                SdkStreamingState::Streaming {
                    poll_thread_done_rx,
                    poll_handle,
                    circ_buffer,
                    ..
                } => {
                    // Drop the poll_handle (don't abort - thread exits via shutdown flag)
                    drop(poll_handle);
                    let exited = match poll_thread_done_rx.recv_timeout(POLL_THREAD_TIMEOUT) {
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
                            tracing::debug!(
                                "PVCAM poll thread completion channel disconnected (thread already exited)"
                            );
                            true
                        }
                    };
                    // Keep circ_buffer alive until after FFI cleanup below
                    (exited, Some(circ_buffer))
                }
                SdkStreamingState::Idle => {
                    // No active poll thread (stream was never started or already stopped)
                    tracing::debug!("No active PVCAM poll thread to wait for");
                    (true, None)
                }
            };

            // CRITICAL SAFETY: Stop camera and deregister callback BEFORE buffer/context are freed.
            // _circ_buffer_guard keeps the circular buffer alive through this entire block.
            // This prevents use-after-free where PVCAM might try to:
            // 1. Write to the circular buffer after it's deallocated
            // 2. Invoke the EOF callback after the context is freed (bd-d9nw.2)
            //
            // CallbackContext Lifetime Safety (bd-d9nw.2):
            // - self.callback_context is Arc<Pin<Box<CallbackContext>>>
            // - PVCAM holds raw pointer to context via GLOBAL_CALLBACK_CTX
            // - pl_cam_deregister_callback removes callback registration
            // - clear_global_callback_ctx() nulls the static pointer
            // - Only AFTER deregistration does the Arc drop, freeing the context
            // - If callback fires after deregistration, it sees null pointer and exits early
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

                // SAFETY: `hcam` was obtained from active_hcam.swap(-1), so we
                // have exclusive ownership of the handle for cleanup. The swap
                // ensures no concurrent stop_stream can use the same handle.
                // pl_exp_stop_cont halts the camera; pl_cam_deregister_callback
                // removes the EOF callback. Both are idempotent if acquisition
                // is already stopped. Callback deregistration MUST happen before
                // circ_buffer/callback_context are dropped (below) to prevent
                // use-after-free in the callback.
                unsafe {
                    // Stop continuous acquisition first (halts camera operation)
                    tracing::info!(
                        hcam,
                        callback_registered = self.callback_registered.load(Ordering::Acquire),
                        "PVCAM acquisition Drop cleanup: issuing pl_exp_stop_cont"
                    );
                    let stop_result = pl_exp_stop_cont(hcam, CCS_HALT);
                    if stop_result == 0 {
                        tracing::warn!(
                            "pl_exp_stop_cont failed in Drop (may already be stopped): {}",
                            get_pvcam_error()
                        );
                    } else {
                        tracing::debug!("Stopped PVCAM acquisition in Drop");
                    }

                    // Deregister callback to prevent use-after-free
                    if self.callback_registered.swap(false, Ordering::AcqRel) {
                        let callback_ctx_ptr = &**self.callback_context as *const CallbackContext;
                        tracing::info!(
                            hcam,
                            callback_type = PL_CALLBACK_EOF,
                            callback_ctx_ptr = ?callback_ctx_ptr,
                            "PVCAM acquisition Drop cleanup: deregistering EOF callback"
                        );
                        let dereg_result = pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
                        if dereg_result == 0 {
                            tracing::warn!(
                                "pl_cam_deregister_callback failed in Drop: {}",
                                get_pvcam_error()
                            );
                        } else {
                            tracing::debug!(
                                hcam,
                                callback_type = PL_CALLBACK_EOF,
                                "Deregistered PVCAM EOF callback in Drop"
                            );
                        }
                        clear_global_callback_ctx();
                    }
                }
            }

            // _circ_buffer_guard drops here — safe because FFI cleanup is complete.
            // The buffer and context will be freed when Arc refs drop to zero.
        }
    }
}
