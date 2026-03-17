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

#[cfg(feature = "pvcam_sdk")]
use crate::components::connection::get_pvcam_error;
use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_sdk")]
use crate::components::features::PvcamFeatures;
use crate::components::taps::TapRegistry;
use common::parameter::Parameter;
#[cfg(feature = "pvcam_sdk")]
use pool::buffer_pool::BufferPool;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
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
    shutdown: Arc<AtomicBool>,
    #[cfg(feature = "pvcam_sdk")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Page-aligned circular buffer for DMA performance (Gemini SDK review).
    /// PVCAM DMA requires 4KB alignment to avoid internal driver copies.
    #[cfg(feature = "pvcam_sdk")]
    circ_buffer: Arc<Mutex<Option<PageAlignedBuffer>>>,
    #[cfg(feature = "pvcam_sdk")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
    /// Error sender for signaling involuntary stops from frame loop (Gemini SDK review).
    /// Fatal errors (READOUT_FAILED, etc.) are sent here so the driver can update streaming state.
    /// Uses tokio::sync::mpsc::unbounded_channel for async-native error watching without polling.
    #[cfg(feature = "pvcam_sdk")]
    error_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<AcquisitionError>>>>,
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
    /// Completion signal for poll thread (bd-g6pr).
    /// Used in Drop to synchronously wait for the poll thread to exit before calling
    /// FFI cleanup functions. This prevents the race condition where pl_exp_stop_cont
    /// is called while pl_exp_get_oldest_frame_ex is still executing.
    #[cfg(feature = "pvcam_sdk")]
    poll_thread_done_rx: Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
    #[cfg(feature = "pvcam_sdk")]
    poll_thread_done_tx: Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>>,

    /// Frame pool for zero-allocation frame handling (bd-0dax.3).
    /// Created in start_stream() with size based on SDK buffer count.
    /// Pool is cleared in stop_stream() to release memory.
    #[cfg(feature = "pvcam_sdk")]
    frame_pool: Arc<Mutex<Option<BufferPool>>>,
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
            #[cfg(feature = "pvcam_sdk")]
            metadata_tx: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_sdk")]
            metadata_enabled: Arc::new(AtomicBool::new(false)),

            // Frame loss detection counters (bd-ek9n.3)
            lost_frames: Arc::new(AtomicU64::new(0)),
            discontinuity_events: Arc::new(AtomicU64::new(0)),
            // Pool exhaustion counter (bd-dmbl)
            dropped_frames: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "pvcam_sdk")]
            last_hardware_frame_nr: Arc::new(AtomicI32::new(-1)), // -1 = uninitialized

            // Error tracking (bd-g9po)
            last_error: Arc::new(std::sync::Mutex::new(None)),

            #[cfg(feature = "pvcam_sdk")]
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "pvcam_sdk")]
            poll_handle: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_sdk")]
            circ_buffer: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_sdk")]
            trigger_frame: Arc::new(Mutex::new(None)),
            // Error channel for signaling involuntary stop signaling (Gemini SDK review)
            #[cfg(feature = "pvcam_sdk")]
            error_tx: Arc::new(Mutex::new(None)),
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
            // Completion channel for poll thread synchronization (bd-g6pr)
            // Created fresh for each acquisition in start_stream
            #[cfg(feature = "pvcam_sdk")]
            poll_thread_done_rx: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(feature = "pvcam_sdk")]
            poll_thread_done_tx: Arc::new(std::sync::Mutex::new(None)),

            // Frame pool for zero-allocation (bd-0dax.3)
            // Created in start_stream() when frame size is known
            #[cfg(feature = "pvcam_sdk")]
            frame_pool: Arc::new(Mutex::new(None)),
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
                            tracing::debug!(
                                "PVCAM poll thread completion channel disconnected (thread already exited)"
                            );
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

            // Now safe to drop circ_buffer and callback_context (happens automatically)
            // The buffer and context will be freed when Arc refs drop to zero.
        }
    }
}
