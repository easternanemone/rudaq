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

pub use buffer::*;
#[cfg(feature = "pvcam_sdk")]
pub use callback_context::*;

#[cfg(feature = "pvcam_sdk")]
use crate::components::connection::get_pvcam_error;
use crate::components::connection::PvcamConnection;
#[cfg(feature = "pvcam_sdk")]
use crate::components::features::PvcamFeatures;
use crate::components::taps::TapRegistry;
use anyhow::{anyhow, bail, Result};
#[cfg(feature = "pvcam_sdk")]
use bytes::Bytes;
use common::core::Roi;
use common::data::Frame;
use common::parameter::Parameter;
#[cfg(feature = "pvcam_sdk")]
use pool::buffer_pool::BufferPool;
// bd-5oss: Frame pool for mock mode primary_tx delivery
use pool::{FrameData, Pool};
#[cfg(feature = "pvcam_sdk")]
use std::alloc::{alloc_zeroed, dealloc, Layout};
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicI16;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicI32;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;
use tokio::time::timeout;

#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;
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
    pub frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    pub reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,

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

    /// Reset frame loss metrics at the start of a new acquisition.
    pub fn reset_frame_loss_metrics(&self) {
        self.lost_frames.store(0, Ordering::SeqCst);
        self.discontinuity_events.store(0, Ordering::SeqCst);
        self.dropped_frames.store(0, Ordering::SeqCst);
        #[cfg(feature = "pvcam_sdk")]
        {
            self.last_hardware_frame_nr.store(-1, Ordering::SeqCst);
            // Reset callback context state (bd-ek9n.2)
            self.callback_context.reset();
        }
    }

    /// Get the current frame loss statistics.
    ///
    /// Returns a tuple of (lost_frames, discontinuity_events, dropped_frames).
    pub fn frame_loss_stats(&self) -> (u64, u64, u64) {
        (
            self.lost_frames.load(Ordering::Relaxed),
            self.discontinuity_events.load(Ordering::Relaxed),
            self.dropped_frames.load(Ordering::Relaxed),
        )
    }

    /// Get the number of frames dropped due to pool exhaustion (bd-dmbl).
    ///
    /// This counter is incremented when the buffer pool is exhausted and
    /// a frame must be dropped to maintain real-time performance.
    pub fn dropped_frame_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
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

    /// Register the primary output channel for zero-allocation frame delivery (bd-0dax.5).
    ///
    /// Only ONE primary consumer is allowed - subsequent calls replace the previous consumer.
    /// Call BEFORE `start_stream()` to ensure frames are delivered from the start.
    ///
    /// # Arguments
    /// * `tx` - Channel sender that will receive `LoanedFrame` ownership
    pub async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<common::capabilities::LoanedFrame>,
    ) -> anyhow::Result<()> {
        let mut primary = self.primary_tx.lock().await;
        *primary = Some(tx);
        tracing::debug!(target: "pvcam", "Primary output channel registered");
        Ok(())
    }

    /// Calculate optimal circular buffer frame count (bd-ek9n.4)
    ///
    /// Uses PARAM_FRAME_BUFFER_SIZE when available, with heuristic fallback:
    /// - Minimum 32 frames for reliability
    /// - At least 1 second of buffer at current frame rate
    /// - Capped at 255 frames (matches PVCAM example defaults)
    ///
    /// # Arguments
    ///
    /// * `hcam` - Open camera handle
    /// * `frame_bytes` - Size of one frame in bytes
    /// * `exposure_ms` - Exposure time in milliseconds (for frame rate calculation)
    #[cfg(feature = "pvcam_sdk")]
    fn calculate_buffer_count(hcam: i16, frame_bytes: usize, exposure_ms: f64) -> usize {
        // PVCAM examples default to 255-frame circular buffers for full-frame streaming.
        // We align with that default but still clamp to a sane upper bound to avoid
        // excessive host memory use on large frames.
        const MIN_BUFFER_FRAMES: usize = 32;
        const MAX_BUFFER_FRAMES: usize = 255;
        const ONE_SECOND_MS: f64 = 1000.0;

        // Try to query PARAM_FRAME_BUFFER_SIZE from SDK
        // This returns recommended buffer size in bytes for current acquisition settings
        // SAFETY: `hcam` is a valid camera handle. All output parameters
        // (`avail`, `recommended_bytes`) are stack-allocated with correct
        // types. These are read-only parameter queries with no side effects.
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
                    tracing::debug!("PARAM_FRAME_BUFFER_SIZE is not available on this camera");
                    None
                }
            } else {
                tracing::debug!("PARAM_FRAME_BUFFER_SIZE is not available, using heuristics");
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
        // then clamp to reasonable bounds. Default to SDK guidance when available
        // (typical Prime BSI recommendation is 255 frames at full frame).
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

    /// Get the number of ROIs supported by the camera (bd-vcbd)
    ///
    /// Returns the maximum number of regions of interest (ROIs) that can be
    /// configured for acquisition. Useful for multi-region readout modes.
    ///
    /// # SDK Pattern (bd-vcbd)
    /// Checks PARAM_ROI_COUNT availability before access.
    #[cfg(feature = "pvcam_sdk")]
    pub fn get_roi_count(conn: &PvcamConnection) -> Result<u16> {
        if let Some(h) = conn.handle() {
            // SDK Pattern: Check availability before access
            let mut avail: rs_bool = 0;
            // SAFETY: `h` is a valid camera handle. `avail` is a stack-allocated
            // rs_bool output parameter. Read-only availability query.
            unsafe {
                if pl_get_param(
                    h,
                    PARAM_ROI_COUNT,
                    ATTR_AVAIL as i16,
                    &mut avail as *mut _ as *mut _,
                ) == 0
                {
                    // Failed to query availability
                    return Err(anyhow!(
                        "Failed to query PARAM_ROI_COUNT availability: {}",
                        get_pvcam_error()
                    ));
                }

                if avail == 0 {
                    return Err(anyhow!("PARAM_ROI_COUNT is not available on this camera"));
                }

                let mut count: uns16 = 0;
                // SAFETY: h is valid handle; count is writable uns16 on stack.
                if pl_get_param(
                    h,
                    PARAM_ROI_COUNT,
                    ATTR_CURRENT as i16,
                    &mut count as *mut _ as *mut _,
                ) == 0
                {
                    return Err(anyhow!("Failed to get ROI count: {}", get_pvcam_error()));
                }
                return Ok(count);
            }
        }
        Err(anyhow!("Camera not connected"))
    }

    /// Get the number of ROIs supported by the camera (mock mode) (bd-vcbd)
    ///
    /// Mock version that returns a default value when hardware is not available.
    #[cfg(not(feature = "pvcam_sdk"))]
    pub fn get_roi_count(_conn: &PvcamConnection) -> Result<u16> {
        // Mock mode default: 1 ROI (single region)
        Ok(1)
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
    #[cfg(feature = "pvcam_sdk")]
    pub async fn enable_metadata(&self, tx: tokio::sync::mpsc::Sender<FrameMetadata>) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = Some(tx);
        self.metadata_enabled.store(true, Ordering::Release);
        tracing::info!("Metadata decoding enabled for acquisition");
    }

    /// Disable metadata decoding (Gemini SDK review).
    #[cfg(feature = "pvcam_sdk")]
    pub async fn disable_metadata(&self) {
        let mut guard = self.metadata_tx.lock().await;
        *guard = None;
        self.metadata_enabled.store(false, Ordering::Release);
    }

    /// Toggle metadata decoding without changing the channel (bd-32f4).
    ///
    /// Called by the driver's `metadata_enabled` parameter write callback to
    /// sync the acquisition's decoding flag with the SDK parameter. When enabled
    /// without a channel, frames are still decoded (useful for data integrity)
    /// but decoded metadata is silently dropped.
    #[cfg(feature = "pvcam_sdk")]
    pub fn set_metadata_decoding(&self, enabled: bool) {
        self.metadata_enabled.store(enabled, Ordering::Release);
        tracing::debug!("Metadata decoding flag set to {}", enabled);
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
        buffer_mode: String,
    ) -> Result<()> {
        tracing::info!(
            "start_stream: roi=({},{} {}x{}), binning=({},{}), exposure={:.1}ms, mode={}",
            roi.x,
            roi.y,
            roi.width,
            roi.height,
            binning.0,
            binning.1,
            exposure_ms,
            buffer_mode
        );

        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        let _ = buffer_mode;
        if self.streaming.get() {
            tracing::warn!("start_stream: already streaming");
            bail!("Already streaming");
        }

        tracing::debug!("Setting streaming=true, resetting frame counters");
        self.streaming.set_from_hardware(true).await?;
        self.frame_count.store(0, Ordering::SeqCst);
        // Reset frame loss metrics for this acquisition (bd-ek9n.3)
        self.reset_frame_loss_metrics();

        let reliable_tx = self.reliable_tx.lock().await.clone();
        tracing::debug!(
            "reliable_tx channel: {}",
            if reliable_tx.is_some() {
                "present"
            } else {
                "none"
            }
        );

        #[cfg(feature = "pvcam_sdk")]
        if let Some(h) = conn.handle() {
            tracing::info!("Hardware path: hcam={}", h);
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
            let start_span = tracing::info_span!(
                "pvcam_start_stream",
                roi_x = roi.x,
                roi_y = roi.y,
                width = roi.width,
                height = roi.height,
                bin_x = x_bin,
                bin_y = y_bin,
                exposure_ms
            );
            let _enter = start_span.enter();

            // PVCAM Best Practices: for reliable frame delivery (especially high FPS/high throughput),
            // prefer an EOF callback acquisition model over polling loops (bd-ek9n.2).
            // Setup region
            tracing::debug!(
                roi_x = roi.x,
                roi_y = roi.y,
                roi_width = roi.width,
                roi_height = roi.height,
                x_bin,
                y_bin,
                "Creating PVCAM region (rgn_type)"
            );
            // Validate ROI dimensions before casting to uns16 (bd-8zcu)
            if roi.width == 0 || roi.height == 0 {
                return Err(anyhow!(
                    "Invalid ROI: width and height must be > 0 (got {}x{})",
                    roi.width,
                    roi.height
                ));
            }
            if x_bin == 0 || y_bin == 0 {
                return Err(anyhow!(
                    "Invalid binning: x_bin and y_bin must be > 0 (got {}x{})",
                    x_bin,
                    y_bin
                ));
            }
            // s2 = roi.x + roi.width - 1 and p2 = roi.y + roi.height - 1 must fit in uns16
            let s2 = roi
                .x
                .checked_add(roi.width)
                .and_then(|v| v.checked_sub(1))
                .ok_or_else(|| {
                    anyhow!(
                        "ROI serial range overflow: x={} + width={} exceeds bounds",
                        roi.x,
                        roi.width
                    )
                })?;
            let p2 = roi
                .y
                .checked_add(roi.height)
                .and_then(|v| v.checked_sub(1))
                .ok_or_else(|| {
                    anyhow!(
                        "ROI parallel range overflow: y={} + height={} exceeds bounds",
                        roi.y,
                        roi.height
                    )
                })?;
            if s2 > u16::MAX as u32 {
                return Err(anyhow!(
                    "ROI serial endpoint {} exceeds uns16 max ({})",
                    s2,
                    u16::MAX
                ));
            }
            if p2 > u16::MAX as u32 {
                return Err(anyhow!(
                    "ROI parallel endpoint {} exceeds uns16 max ({})",
                    p2,
                    u16::MAX
                ));
            }
            if roi.width % x_bin as u32 != 0 {
                tracing::warn!(
                    "ROI width {} not evenly divisible by x_bin {}, PVCAM may round",
                    roi.width,
                    x_bin
                );
            }
            if roi.height % y_bin as u32 != 0 {
                tracing::warn!(
                    "ROI height {} not evenly divisible by y_bin {}, PVCAM may round",
                    roi.height,
                    y_bin
                );
            }
            // SAFETY: rgn_type is a plain-old-data (POD) C struct from the PVCAM SDK
            // containing only primitive integer fields (uns16). Zero-initialization
            // followed by explicit assignment of all fields is safe because:
            // 1. The struct has no pointers, references, padding requirements, or drop semantics
            // 2. All fields are primitive integers that accept any bit pattern
            // 3. Every field is explicitly set before the struct is passed to PVCAM
            // 4. All values validated above to fit within uns16 range (bd-8zcu)
            let region = unsafe {
                let mut rgn: rgn_type = std::mem::zeroed();
                rgn.s1 = roi.x as uns16;
                rgn.s2 = s2 as uns16;
                rgn.sbin = x_bin;
                rgn.p1 = roi.y as uns16;
                rgn.p2 = p2 as uns16;
                rgn.pbin = y_bin;
                tracing::debug!(
                    s1 = rgn.s1,
                    s2 = rgn.s2,
                    sbin = rgn.sbin,
                    p1 = rgn.p1,
                    p2 = rgn.p2,
                    pbin = rgn.pbin,
                    "PVCAM rgn_type configured"
                );
                rgn
            };

            // bd-9pel: Use sequence mode when buffer_mode is "Sequence" (runtime configurable).
            // Sequence mode uses pl_exp_setup_seq/start_seq for batch-based non-circular
            // acquisition. Useful for single-frame capture workflows or diagnostics.
            if buffer_mode == "Sequence" {
                return self
                    .start_stream_sequence_impl(
                        h,
                        region,
                        exposure_ms,
                        binning,
                        roi,
                        reliable_tx,
                        use_metadata,
                    )
                    .await;
            }

            // PVCAM Best Practices: Use actual frame_bytes from pl_exp_setup_cont
            // rather than assuming pixels * 2 - metadata/alignment can change frame size.
            let mut frame_bytes: uns32 = 0;
            // bd-g3ap P0 FIX: Force CIRC_NO_OVERWRITE to prevent data corruption.
            //
            // The current frame loop unlocks frames BEFORE copying pixel data and
            // decoding metadata (see bd-unlock-before-copy-2026-01-12 comment below).
            // In CIRC_OVERWRITE mode, the SDK can reuse the DMA buffer immediately
            // after unlock, causing a data race on frame_ptr. Until the frame loop
            // is restructured to copy-then-unlock, CIRC_OVERWRITE is unsafe.
            //
            // TODO(bd-g3ap): Restructure frame loop to copy data before unlock,
            // then re-enable CIRC_OVERWRITE for better throughput.
            let mut circ_overwrite = false;
            if matches!(buffer_mode.as_str(), "Overwrite") {
                tracing::warn!(
                    "CIRC_OVERWRITE requested but disabled (bd-g3ap): \
                     unlock-before-copy pattern is unsafe in overwrite mode"
                );
            }
            // Smoke tests on hardware have historically required CIRC_NO_OVERWRITE (bd-ek9n).
            if std::env::var("PVCAM_SMOKE_TEST")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
            {
                circ_overwrite = false;
                tracing::info!("PVCAM smoke test: forcing CIRC_NO_OVERWRITE");
            }
            let mut selected_buffer_mode = if circ_overwrite {
                CIRC_OVERWRITE
            } else {
                CIRC_NO_OVERWRITE
            };

            // Probe PARAM_CIRC_BUFFER for visibility only; do not override user choice unless setup fails.
            if circ_overwrite {
                // SAFETY: `h` is a valid camera handle from conn.handle().
                // All output parameters are stack-allocated with correct types
                // (rs_bool for ATTR_AVAIL, uns32 for ATTR_MIN/ATTR_MAX).
                // These are read-only queries with no side effects.
                unsafe {
                    let mut circ_avail: rs_bool = 0;
                    if pl_get_param(
                        h,
                        PARAM_CIRC_BUFFER,
                        ATTR_AVAIL as i16,
                        &mut circ_avail as *mut _ as *mut std::ffi::c_void,
                    ) == 0
                    {
                        tracing::warn!(
                            "PARAM_CIRC_BUFFER ATTR_AVAIL query failed: {}",
                            get_pvcam_error()
                        );
                    } else if circ_avail == 0 {
                        tracing::info!("CIRC_OVERWRITE requested but not advertised as available");
                    } else {
                        let mut circ_min: uns32 = 0;
                        let mut circ_max: uns32 = 0;
                        if pl_get_param(
                            h,
                            PARAM_CIRC_BUFFER,
                            ATTR_MIN as i16,
                            &mut circ_min as *mut _ as *mut std::ffi::c_void,
                        ) != 0
                            && pl_get_param(
                                h,
                                PARAM_CIRC_BUFFER,
                                ATTR_MAX as i16,
                                &mut circ_max as *mut _ as *mut std::ffi::c_void,
                            ) != 0
                        {
                            tracing::info!(
                                "PARAM_CIRC_BUFFER min={}, max={} (overwrite advertised)",
                                circ_min,
                                circ_max
                            );
                        }
                    }
                }
            }
            let exp_mode = TIMED_MODE; // EXT_TRIG_* are encoded as PL_EXPOSURE_MODES (see pvcam.h)

            // bd-ffi-sdk-match: Register EOF callback BEFORE pl_exp_setup_cont (SDK pattern).
            // The LiveImage.cpp example shows: 1) register callback, 2) setup_cont, 3) start_cont.
            // Registering after setup causes callbacks to never fire on some cameras.
            self.callback_context.set_hcam(h);

            // Scope the raw pointer usage to avoid holding it across await points.
            // Raw pointers aren't Send, so they can't exist in async functions across awaits.
            let use_callback = {
                // Get raw pointer to pinned CallbackContext for FFI
                // Deref Arc -> Pin<Box<T>> -> T, then take address
                let callback_ctx_ptr = &**self.callback_context as *const CallbackContext;
                tracing::info!(
                    hcam = h,
                    callback_type = PL_CALLBACK_EOF,
                    callback_fn = ?(pvcam_eof_callback as *mut std::ffi::c_void),
                    callback_ctx_ptr = ?callback_ctx_ptr,
                    "PVCAM registering EOF callback before pl_exp_setup_cont"
                );

                // bd-static-ctx-2026-01-12: Set global context BEFORE registering callback
                // The SDK p_context parameter stops working after ~19 frames on Prime BSI.
                // Using a static global pointer like the minimal test fixes this.
                set_global_callback_ctx(callback_ctx_ptr);

                // SAFETY: `h` is a valid camera handle. `pvcam_eof_callback` is
                // an extern "system" fn matching the SDK's expected callback
                // signature. `callback_ctx_ptr` points to a Pin<Box<CallbackContext>>
                // kept alive by self.callback_context for the duration of
                // acquisition. The callback uses catch_unwind to prevent panics
                // from unwinding across the FFI boundary.
                unsafe {
                    // Use bindgen-generated function, cast callback to *mut c_void
                    let result = pl_cam_register_callback_ex3(
                        h,
                        PL_CALLBACK_EOF,
                        pvcam_eof_callback as *mut std::ffi::c_void,
                        callback_ctx_ptr as *mut std::ffi::c_void, // Still passed for SDK, but callback ignores it
                    );
                    if result == 0 {
                        tracing::warn!(
                            hcam = h,
                            callback_type = PL_CALLBACK_EOF,
                            callback_ctx_ptr = ?callback_ctx_ptr,
                            "Failed to register EOF callback ({}), falling back to polling mode",
                            get_pvcam_error()
                        );
                        clear_global_callback_ctx(); // Clear on failure
                        false
                    } else {
                        tracing::info!(
                            hcam = h,
                            callback_type = PL_CALLBACK_EOF,
                            callback_ctx_ptr = ?callback_ctx_ptr,
                            "PVCAM EOF callback registered successfully (before setup)"
                        );
                        // Store callback state for Drop cleanup
                        self.callback_registered.store(true, Ordering::Release);
                        true
                    }
                }
            };

            // If PARAM_CIRC_BUFFER check already determined no overwrite, update callback context (bd-nzcq)
            if use_callback && !circ_overwrite {
                let callback_ctx = self.callback_context.as_ref();
                callback_ctx.set_circ_overwrite(false);
            }

            tracing::debug!(
                hcam = h,
                exp_mode = TIMED_MODE,
                exposure_ms = exposure_ms as uns32,
                buffer_mode = if selected_buffer_mode == CIRC_OVERWRITE {
                    "CIRC_OVERWRITE"
                } else {
                    "CIRC_NO_OVERWRITE"
                },
                "Calling pl_exp_setup_cont"
            );

            // SAFETY: `h` is a valid camera handle. `region` is a stack-allocated
            // rgn_type. `frame_bytes` is a stack-allocated uns32 output parameter.
            // `selected_buffer_mode` is a valid CIRC_* constant. No acquisition
            // is active (we are in setup). On failure, we retry with NO_OVERWRITE.
            unsafe {
                // Try overwrite first
                if pl_exp_setup_cont(
                    h,
                    1,
                    &region as *const _,
                    exp_mode,
                    exposure_ms as uns32,
                    &mut frame_bytes,
                    selected_buffer_mode,
                ) == 0
                {
                    let err_msg_overwrite = get_pvcam_error();
                    tracing::warn!(
                        "CIRC_OVERWRITE setup failed ({}), retrying with CIRC_NO_OVERWRITE",
                        err_msg_overwrite
                    );
                    // Retry with no-overwrite
                    selected_buffer_mode = CIRC_NO_OVERWRITE;
                    circ_overwrite = false;
                    // Update callback context so callback knows NOT to call get_latest_frame (bd-nzcq)
                    if use_callback {
                        let callback_ctx = self.callback_context.as_ref();
                        callback_ctx.set_circ_overwrite(false);
                    }
                    frame_bytes = 0;
                    if pl_exp_setup_cont(
                        h,
                        1,
                        &region as *const _,
                        exp_mode,
                        exposure_ms as uns32,
                        &mut frame_bytes,
                        selected_buffer_mode,
                    ) == 0
                    {
                        let err_msg = get_pvcam_error();
                        let _ = self.streaming.set_from_hardware(false).await;
                        return Err(anyhow!(
                            "Failed to setup continuous acquisition (both modes): {}",
                            err_msg
                        ));
                    }
                }
            }

            tracing::info!(
                "PVCAM continuous mode using {}",
                if circ_overwrite {
                    "CIRC_OVERWRITE"
                } else {
                    "CIRC_NO_OVERWRITE"
                }
            );

            // Report the current buffer mode the camera accepted.
            // SAFETY: `h` is a valid camera handle. `circ_current` is a
            // stack-allocated uns32 output parameter. Read-only query.
            unsafe {
                let mut circ_current: uns32 = 0;
                if pl_get_param(
                    h,
                    PARAM_CIRC_BUFFER,
                    ATTR_CURRENT as i16,
                    &mut circ_current as *mut _ as *mut std::ffi::c_void,
                ) == 0
                {
                    tracing::warn!(
                        "PARAM_CIRC_BUFFER ATTR_CURRENT query failed: {}",
                        get_pvcam_error()
                    );
                } else {
                    tracing::info!("PVCAM PARAM_CIRC_BUFFER current mode: {}", circ_current);
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
            let mut buffer_count = Self::calculate_buffer_count(h, actual_frame_bytes, exposure_ms);
            if std::env::var("PVCAM_SMOKE_TEST")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
            {
                let forced = 21usize;
                eprintln!(
                    "[PVCAM DIAG] Buffer count: calculated={}, using={} (forced by PVCAM_SMOKE_TEST)",
                    buffer_count, forced
                );
                buffer_count = forced;
            }
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

            // bd-0dax.4: Create buffer pool for TRUE zero-allocation frame handling.
            // Uses bytes::Bytes with custom drop to return buffers to pool when all
            // consumers are done with a frame. No allocations during steady-state streaming.
            // Pool size = SDK buffer count + 50% headroom for consumer latency.
            let pool_size = (buffer_count as f64 * 1.5).ceil() as usize;
            let buffer_pool = BufferPool::new(pool_size, actual_frame_bytes);
            *self.frame_pool.lock().await = Some(buffer_pool.clone());
            tracing::info!(
                pool_size,
                frame_capacity_mb = actual_frame_bytes as f64 / (1024.0 * 1024.0),
                total_pool_mb = (pool_size * actual_frame_bytes) as f64 / (1024.0 * 1024.0),
                "Buffer pool created for zero-allocation frames (bd-0dax.4)"
            );

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
            let mut circ_buf = PageAlignedBuffer::new(circ_buf_size)?;
            let circ_ptr = circ_buf.as_mut_ptr();
            // bd-3gnv: Convert raw pointer to usize BEFORE any await points.
            // Raw pointers are not Send, but usize is. Convert early to avoid
            // "future cannot be sent between threads" errors from holding raw
            // pointers across await boundaries.
            let circ_ptr_usize = circ_ptr as usize;
            // Note: Use circ_ptr_usize for logging to avoid holding raw pointer across await points
            tracing::debug!(
                "Allocated {}KB page-aligned circular buffer at 0x{:x}",
                circ_buf_size / 1024,
                circ_ptr_usize
            );

            tracing::debug!(
                hcam = h,
                circ_ptr_addr = circ_ptr_usize,
                circ_size_bytes,
                "Calling pl_exp_start_cont"
            );

            // SAFETY: `h` is a valid camera handle. `circ_ptr` points to a
            // page-aligned contiguous buffer allocated above. `circ_size_bytes`
            // matches the buffer's actual size. All inner fallback calls
            // (pl_exp_setup_cont, pl_exp_start_cont, pl_cam_deregister_callback,
            // pl_cam_register_callback_ex3) use the same valid `h` and
            // stack-allocated parameters. On failure paths, callbacks are
            // deregistered and state is cleaned up before returning.
            unsafe {
                if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                    // bd-3gnv: Log SDK error with full message for diagnostics
                    let err_msg = get_pvcam_error();

                    // bd-circ-start-fallback: Prime BSI cameras accept CIRC_OVERWRITE at setup
                    // but fail at start with error 185 (Invalid Configuration). When this happens,
                    // re-setup and re-start with CIRC_NO_OVERWRITE.
                    if circ_overwrite {
                        tracing::warn!(
                            "pl_exp_start_cont failed with CIRC_OVERWRITE ({}), retrying with CIRC_NO_OVERWRITE",
                            err_msg
                        );

                        // Re-setup with NO_OVERWRITE
                        let mut retry_frame_bytes: uns32 = 0;
                        if pl_exp_setup_cont(
                            h,
                            1,
                            &region as *const _,
                            exp_mode,
                            exposure_ms as uns32,
                            &mut retry_frame_bytes,
                            CIRC_NO_OVERWRITE,
                        ) == 0
                        {
                            let setup_err = get_pvcam_error();
                            // Deregister callback on failure
                            if use_callback {
                                pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                                clear_global_callback_ctx(); // bd-static-ctx-2026-01-12
                                self.callback_registered.store(false, Ordering::Release);
                            }
                            self.active_hcam.store(-1, Ordering::Release);
                            let _ = self.streaming.set_from_hardware(false).await;
                            return Err(anyhow!(
                                "Fallback setup with CIRC_NO_OVERWRITE also failed: {}",
                                setup_err
                            ));
                        }

                        // CRITICAL: Update circ_overwrite flag for frame loop FIFO drain path
                        circ_overwrite = false;

                        // CRITICAL (bd-nzcq): Update callback context so callback knows NOT to call
                        // get_latest_frame. In CIRC_NO_OVERWRITE mode, main loop must use
                        // get_oldest_frame for proper FIFO order.
                        if use_callback {
                            let callback_ctx = self.callback_context.as_ref();
                            callback_ctx.set_circ_overwrite(false);
                        }

                        // CRITICAL FIX (bd-nzcq-callback-rereg): Deregister callback before re-registering.
                        // Re-registering without deregistering first causes PVCAM internal state corruption
                        // that manifests as callbacks stopping after ~5 frames. The SDK examples only
                        // register callbacks ONCE and never re-register during a session.
                        if use_callback {
                            tracing::info!(
                                hcam = h,
                                callback_type = PL_CALLBACK_EOF,
                                "PVCAM deregistering EOF callback before fallback re-registration"
                            );
                            pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                            tracing::info!(
                                hcam = h,
                                callback_type = PL_CALLBACK_EOF,
                                "Deregistered EOF callback before fallback re-registration"
                            );
                        }

                        // Re-register callback after fallback setup (setup may invalidate callback)
                        // This matches the SDK pattern: callback registration before each setup
                        if use_callback {
                            // Recreate raw pointer (needed because original was scoped to avoid holding across await)
                            let callback_ctx_ptr =
                                &**self.callback_context as *const CallbackContext;
                            tracing::info!(
                                hcam = h,
                                callback_type = PL_CALLBACK_EOF,
                                callback_fn = ?(pvcam_eof_callback as *mut std::ffi::c_void),
                                callback_ctx_ptr = ?callback_ctx_ptr,
                                "PVCAM re-registering EOF callback after fallback setup"
                            );
                            let result = pl_cam_register_callback_ex3(
                                h,
                                PL_CALLBACK_EOF,
                                pvcam_eof_callback as *mut std::ffi::c_void,
                                callback_ctx_ptr as *mut std::ffi::c_void,
                            );
                            if result == 0 {
                                tracing::warn!(
                                    hcam = h,
                                    callback_type = PL_CALLBACK_EOF,
                                    callback_ctx_ptr = ?callback_ctx_ptr,
                                    "Failed to re-register EOF callback after fallback: {}",
                                    get_pvcam_error()
                                );
                            } else {
                                tracing::info!(
                                    hcam = h,
                                    callback_type = PL_CALLBACK_EOF,
                                    callback_ctx_ptr = ?callback_ctx_ptr,
                                    "EOF callback re-registered after fallback setup"
                                );
                            }
                        }

                        // Retry start with NO_OVERWRITE
                        if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                            let start_err = get_pvcam_error();
                            // Deregister callback on failure
                            if use_callback {
                                pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                                clear_global_callback_ctx(); // bd-static-ctx-2026-01-12
                                self.callback_registered.store(false, Ordering::Release);
                            }
                            self.active_hcam.store(-1, Ordering::Release);
                            let _ = self.streaming.set_from_hardware(false).await;
                            return Err(anyhow!(
                                "Fallback start with CIRC_NO_OVERWRITE also failed: {}",
                                start_err
                            ));
                        }

                        tracing::info!("Successfully fell back to CIRC_NO_OVERWRITE mode at start");
                    } else {
                        // Already using NO_OVERWRITE, no fallback available
                        // Deregister callback on failure
                        if use_callback {
                            pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                            clear_global_callback_ctx(); // bd-static-ctx-2026-01-12
                            self.callback_registered.store(false, Ordering::Release);
                        }
                        self.active_hcam.store(-1, Ordering::Release);
                        let _ = self.streaming.set_from_hardware(false).await;
                        return Err(anyhow!(
                            "Failed to start continuous acquisition: {}",
                            err_msg
                        ));
                    }
                }
            }

            // Capture initial streaming status/bytes immediately after start for diagnostics.
            if let Ok((st, bytes, buf_cnt)) = ffi_safe::check_cont_status(h) {
                tracing::info!(
                    "PVCAM start status: status={}, bytes_arrived={}, buffer_cnt={}",
                    st,
                    bytes,
                    buf_cnt
                );
            } else {
                tracing::warn!(
                    "PVCAM start status check failed right after pl_exp_start_cont: {}",
                    get_pvcam_error()
                );
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
            let dropped_frames = self.dropped_frames.clone();
            let last_hw_frame_nr = self.last_hardware_frame_nr.clone();
            let callback_ctx = self.callback_context.clone();
            let width = binned_width;
            let height = binned_height;

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

            // bd-0dax.4: Clone tap registry for frame observers
            let tap_registry = self.tap_registry.clone();

            // bd-r8ux: Capture primary_tx for LoanedFrame delivery in hardware path
            let primary_tx = self.primary_tx.lock().await.clone();

            // bd-r8ux: Create Pool<FrameData> if primary_tx is registered
            let primary_frame_pool: Option<Arc<Pool<FrameData>>> = if primary_tx.is_some() {
                let pool = Pool::new_with_reset(
                    pool_size,
                    {
                        let frame_cap = actual_frame_bytes;
                        move || FrameData::with_capacity(frame_cap)
                    },
                    FrameData::reset,
                );
                tracing::info!(
                    pool_size,
                    frame_bytes = actual_frame_bytes,
                    "Created Pool<FrameData> for primary_tx delivery (bd-r8ux)"
                );
                Some(pool)
            } else {
                None
            };

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
                    frame_count,
                    lost_frames,
                    discontinuity_events,
                    dropped_frames,
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
                    circ_overwrite,
                    buffer_pool,        // bd-0dax.4: Buffer pool for true zero-allocation
                    tap_registry,       // bd-0dax.4: For synchronous tap observers
                    primary_tx,         // bd-r8ux: Primary output for LoanedFrame delivery
                    primary_frame_pool, // bd-r8ux: Pool<FrameData> for primary_tx
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
                    if let Err(e) = streaming_for_watcher.set_from_hardware(false).await {
                        tracing::error!("Failed to update streaming state after error: {}", e);
                    }
                }
                // Channel closed (frame loop ended) - task completes naturally
            });

            // bd-diag-2026-01-17: Spawn streaming state change watcher to catch ALL changes
            // This will log whenever streaming changes from true to false (or vice versa),
            // regardless of which code path causes the change.
            let mut streaming_rx = self.streaming.subscribe();
            tokio::spawn(async move {
                while streaming_rx.changed().await.is_ok() {
                    let new_value = *streaming_rx.borrow();
                    tracing::debug!(streaming = new_value, "Streaming state changed");
                    if !new_value {
                        tracing::debug!("Streaming stopped - watcher task exiting");
                        break;
                    }
                }
            });
        }

        // Mock path (or no handle)
        #[cfg(not(feature = "pvcam_sdk"))]
        {
            tracing::warn!("start_stream: pvcam_sdk NOT compiled - using mock stream");
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx)
                .await?;
        }

        // Handle case where hardware feature enabled but handle missing (mock fallback logic)
        #[cfg(feature = "pvcam_sdk")]
        if conn.handle().is_none() {
            tracing::warn!(
                "start_stream: pvcam_sdk compiled but handle is None - falling back to mock stream"
            );
            // Clone reliable_tx again since the original may have been moved into hardware path
            let reliable_tx_mock = self.reliable_tx.lock().await.clone();
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx_mock)
                .await?;
        }

        Ok(())
    }

    /// Acquire a single frame by starting the stream, grabbing one frame, then stopping.
    pub async fn acquire_single_frame(
        &self,
        conn: &MutexGuard<'_, PvcamConnection>,
        roi: Roi,
        binning: (u16, u16),
        exposure_ms: f64,
    ) -> Result<Frame> {
        let mut rx = self.frame_tx.subscribe();
        self.start_stream(conn, roi, binning, exposure_ms, self.buffer_mode.get())
            .await?;

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
        let tap_registry = self.tap_registry.clone(); // bd-0dax.4: For tap observers
        let (x_bin, y_bin) = binning;

        // bd-5oss: Capture primary_tx for LoanedFrame delivery
        let primary_tx = self.primary_tx.lock().await.clone();

        // bd-5oss: Create frame pool if primary_tx is registered
        let frame_pool: Option<Arc<Pool<FrameData>>> = if primary_tx.is_some() {
            let binned_width = roi.width / x_bin as u32;
            let binned_height = roi.height / y_bin as u32;
            let frame_bytes = (binned_width * binned_height * 2) as usize; // 16-bit
            let pool_size = 16; // Reasonable default for mock
            let pool = Pool::new_with_reset(
                pool_size,
                move || FrameData::with_capacity(frame_bytes),
                FrameData::reset,
            );
            tracing::info!(
                pool_size,
                frame_bytes,
                "PVCAM mock: Created frame pool for primary_tx (bd-5oss)"
            );
            Some(pool)
        } else {
            None
        };

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

                // bd-5oss: Send through primary_tx if registered (pooled path)
                if let (Some(ref p_tx), Some(ref pool)) = (&primary_tx, &frame_pool) {
                    if let Some(mut loaned_frame) = pool.try_acquire() {
                        let frame_data = loaned_frame.get_mut();
                        frame_data.width = binned_width;
                        frame_data.height = binned_height;
                        frame_data.bit_depth = 16;
                        frame_data.frame_number = frame_num;
                        frame_data.exposure_ms = exposure_ms;
                        frame_data.roi_x = roi.x;
                        frame_data.roi_y = roi.y;
                        frame_data.binning = Some(binning);
                        frame_data.timestamp_ns = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos() as u64)
                            .unwrap_or(0);

                        // Copy pixel data (u16 -> u8 bytes)
                        let byte_len = pixels.len() * 2;
                        if byte_len <= frame_data.pixels.capacity() {
                            let src_ptr = pixels.as_ptr().cast::<u8>();
                            // SAFETY: copy_nonoverlapping is safe because:
                            // 1. src_ptr points to valid pixel data (Vec<u16> on stack)
                            // 2. frame_data.pixels has sufficient capacity (checked above)
                            // 3. byte_len is exactly pixels.len() * 2, matching u16 -> u8 conversion
                            // 4. Source and destination don't overlap (stack vs heap allocation)
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    src_ptr,
                                    frame_data.pixels.as_mut_ptr(),
                                    byte_len,
                                );
                            }
                            frame_data.actual_len = byte_len;
                        }

                        // Send LoanedFrame - non-blocking
                        if p_tx.try_send(loaned_frame).is_err() && frame_num.is_multiple_of(100) {
                            tracing::warn!(
                                "PVCAM mock: primary channel full at frame {}",
                                frame_num
                            );
                        }
                    } else if frame_num.is_multiple_of(100) {
                        tracing::warn!("PVCAM mock: frame pool exhausted at frame {}", frame_num);
                    }
                }

                // Legacy paths: Arc<Frame> for broadcast and reliable channels
                // Populate frame metadata using builder pattern (bd-183h)
                let ext_metadata = common::data::FrameMetadata {
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

                // bd-0dax.4: Run taps SYNCHRONOUSLY before broadcast (observers get &Frame)
                tap_registry.apply_frame_with_pixels(&frame);

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
        tracing::debug!("PVCAM stop_stream called");
        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        if !self.streaming.get() {
            tracing::debug!("PVCAM stop_stream: not streaming, returning early");
            return Ok(());
        }
        #[cfg(feature = "pvcam_sdk")]
        tracing::info!(
            active_hcam = self.active_hcam.load(Ordering::Acquire),
            callback_registered = self.callback_registered.load(Ordering::Acquire),
            "PVCAM stop_stream requested"
        );
        #[cfg(not(feature = "pvcam_sdk"))]
        tracing::info!("PVCAM stop_stream requested");
        tracing::debug!("PVCAM stop_stream: setting streaming=false");
        self.streaming.set_from_hardware(false).await?;

        #[cfg(feature = "pvcam_sdk")]
        {
            // Signal callback context to shutdown (bd-ek9n.2)
            // This wakes any waiting thread in the frame loop
            tracing::debug!("PVCAM stop_stream: signaling callback context shutdown");
            self.callback_context.signal_shutdown();

            // bd-hehw: Take handle under lock, then drop lock before awaiting
            // This prevents holding the mutex guard across the .await point
            tracing::debug!("PVCAM stop_stream: waiting for poll thread to complete");
            let handle = { self.poll_handle.lock().await.take() };
            if let Some(handle) = handle {
                tracing::debug!("PVCAM stop_stream: awaiting poll handle");
                let _ = handle.await;
                tracing::debug!("PVCAM stop_stream: poll handle completed");
            } else {
                tracing::debug!("PVCAM stop_stream: no poll handle to wait for");
            }
            if let Some(h) = conn.handle() {
                tracing::info!(hcam = h, "PVCAM stop_stream: issuing pl_exp_stop_cont");
                // bd-g9gq: Use FFI safe wrappers with explicit safety contracts
                ffi_safe::stop_acquisition(h, CCS_HALT);
                // Deregister EOF callback if registered (bd-ek9n.2)
                if self.callback_registered.load(Ordering::Acquire) {
                    let callback_ctx_ptr = &**self.callback_context as *const CallbackContext;
                    tracing::info!(
                        hcam = h,
                        callback_type = PL_CALLBACK_EOF,
                        callback_ctx_ptr = ?callback_ctx_ptr,
                        "PVCAM stop_stream: deregistering EOF callback"
                    );
                    ffi_safe::deregister_callback(h, PL_CALLBACK_EOF);
                    self.callback_registered.store(false, Ordering::Release);
                    clear_global_callback_ctx();
                    tracing::debug!(
                        hcam = h,
                        callback_type = PL_CALLBACK_EOF,
                        "PVCAM stop_stream: EOF callback deregistered, global ctx cleared"
                    );
                }
            } else {
                tracing::debug!("PVCAM stop_stream: no camera handle, skipping SDK cleanup");
            }
            // Clear stored state after cleanup
            tracing::debug!("PVCAM stop_stream: clearing stored state");
            self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
            *self.circ_buffer.lock().await = None;
            // bd-g6pr: Clear completion channel so Drop doesn't try to wait again
            if let Ok(mut guard) = self.poll_thread_done_rx.lock() {
                *guard = None;
            }
            if let Ok(mut guard) = self.poll_thread_done_tx.lock() {
                *guard = None;
            }
            tracing::debug!("PVCAM stop_stream: cleanup complete");
        }
        tracing::info!("PVCAM stop_stream completed successfully");
        Ok(())
    }

    /// bd-3gnv: Sequence mode streaming implementation.
    ///
    /// Uses `pl_exp_setup_seq` + `pl_exp_start_seq` for reliable frame acquisition
    /// when circular buffer mode fails (error 185) or stalls.
    ///
    /// Works in batches of SEQUENCE_BATCH_SIZE frames, polling for completion,
    /// then restarting for continuous streaming.
    #[cfg(feature = "pvcam_sdk")]
    #[allow(clippy::too_many_arguments)]
    async fn start_stream_sequence_impl(
        &self,
        hcam: i16,
        region: rgn_type,
        exposure_ms: f64,
        binning: (u16, u16),
        roi: Roi,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
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
        // SAFETY: `hcam` is a valid camera handle. `region` is a stack-allocated
        // rgn_type. `buffer_bytes` is a stack-allocated uns32 output parameter.
        // SEQUENCE_BATCH_SIZE and 1 (region count) are valid constants.
        // No acquisition is active (we are in setup).
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
            let _ = self.streaming.set_from_hardware(false).await;
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
        let tap_registry = self.tap_registry.clone(); // bd-0dax.4: For tap observers
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

        // Spawn blocking task for sequence acquisition loop.
        // NOTE: frame_loop_sequence uses std::thread::sleep + blocking PVCAM FFI calls,
        // so it must run on the tokio blocking pool (not runtime worker threads).
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
                tap_registry, // bd-0dax.4: For tap observers
            );
        });

        *self.poll_handle.lock().await = Some(poll_handle);
        Ok(())
    }

    /// bd-3gnv: Sequence mode frame loop (blocking).
    ///
    /// Repeatedly acquires batches of frames using pl_exp_setup_seq/start_seq,
    /// polls for completion, and sends frames to channels.
    #[cfg(feature = "pvcam_sdk")]
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
        tap_registry: Arc<TapRegistry>, // bd-0dax.4: For synchronous tap observers
    ) {
        // Main sequence loop
        let mut total_frames: u64 = 0;
        let mut batch_num: u64 = 0;

        while !shutdown.load(Ordering::SeqCst) && streaming.get() {
            batch_num += 1;

            // Setup sequence for batch
            // SAFETY: `hcam` is a valid camera handle. `region` is a valid
            // rgn_type. `buffer_bytes` is a stack-allocated output parameter.
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
            // SAFETY: `hcam` is a valid camera handle. `buffer` is a valid
            // mutable Vec<u8> with capacity = buffer_bytes from setup above.
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
                    // SAFETY: `hcam` is valid; CCS_HALT is a valid abort mode.
                    unsafe {
                        pl_exp_abort(hcam, CCS_HALT);
                    }
                    break;
                }

                // SAFETY: `hcam` is valid. `status` and `bytes_arrived` are
                // stack-allocated output parameters with correct types.
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
                        let ext_metadata = common::data::FrameMetadata {
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

                        // bd-0dax.4: Run taps SYNCHRONOUSLY before broadcast
                        tap_registry.apply_frame_with_pixels(&frame);

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
                    tracing::error!("Sequence readout failed: {}", get_pvcam_error());
                    break;
                }
                if status == READOUT_NOT_ACTIVE
                    && start_time.elapsed() > std::time::Duration::from_millis(100)
                {
                    tracing::warn!("Acquisition not active after 100ms: {}", get_pvcam_error());
                    break;
                }

                if start_time.elapsed() > timeout {
                    tracing::error!("Sequence batch {} timed out after {:?}", batch_num, timeout);
                    // SAFETY: `hcam` is valid; CCS_HALT is a valid abort mode.
                    unsafe {
                        pl_exp_abort(hcam, CCS_HALT);
                    }
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            // Finish sequence
            // SAFETY: `hcam` is valid. `buffer` is the same buffer passed to
            // pl_exp_start_seq. The 0 parameter resets the sequence state.
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
    /// * `frame_count` - Counter for acquired frames
    /// * `lost_frames` - Counter for lost frames (bd-ek9n.3)
    /// * `discontinuity_events` - Counter for gap events (bd-ek9n.3)
    /// * `dropped_frames` - Counter for frames dropped due to pool exhaustion (bd-dmbl)
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
    /// * `circ_overwrite` - Whether the acquisition was configured with CIRC_OVERWRITE
    /// * `buffer_pool` - Pre-allocated buffer pool for TRUE zero-allocation frame handling (bd-0dax.4).
    ///                  Uses bytes::Bytes with freeze() - no allocations during steady-state streaming.
    #[cfg(feature = "pvcam_sdk")]
    #[allow(clippy::too_many_arguments)]
    fn frame_loop_hardware(
        hcam: i16,
        streaming: Parameter<bool>,
        shutdown: Arc<AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        frame_count: Arc<AtomicU64>,
        lost_frames: Arc<AtomicU64>,
        discontinuity_events: Arc<AtomicU64>,
        dropped_frames: Arc<AtomicU64>,
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
        // unused in CIRC_OVERWRITE path but kept for signature
        _circ_ptr: *mut u8,
        _circ_size_bytes: u32,
        circ_overwrite: bool,
        buffer_pool: BufferPool, // bd-0dax.4: Buffer pool for true zero-allocation
        tap_registry: Arc<TapRegistry>, // bd-0dax.4: For synchronous tap observers
        primary_tx: Option<tokio::sync::mpsc::Sender<common::capabilities::LoanedFrame>>, // bd-r8ux
        primary_frame_pool: Option<Arc<Pool<FrameData>>>, // bd-r8ux
    ) {
        let loop_span = tracing::debug_span!(
            "pvcam_frame_loop",
            circ_overwrite,
            use_callback,
            exposure_ms,
            frame_bytes,
            expected_frame_bytes,
            width,
            height,
            roi_x,
            roi_y,
            bin_x = binning.0,
            bin_y = binning.1,
            metadata = use_metadata
        );
        let _enter = loop_span.enter();

        struct FrameLoopTrace {
            enabled: bool,
            log_every: u64,
        }

        impl FrameLoopTrace {
            fn new() -> Self {
                let enabled = std::env::var("PVCAM_TRACE")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                let log_every = std::env::var("PVCAM_TRACE_EVERY")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(50);
                Self { enabled, log_every }
            }

            fn log_frame(
                &self,
                monotonic: u64,
                hw_frame_nr: i64,
                pending: u32,
                buffer_cnt: u32,
                bytes_arrived: u32,
                lost: u64,
                discontinuities: u64,
                consecutive_timeouts: u32,
                circ_overwrite: bool,
            ) {
                if !self.enabled {
                    return;
                }
                if monotonic % self.log_every == 0 {
                    tracing::info!(
                        target: "pvcam_frame_trace",
                        frame = monotonic,
                        hw_frame_nr,
                        pending,
                        buffer_cnt,
                        bytes_arrived,
                        lost,
                        discontinuities,
                        consecutive_timeouts,
                        circ_overwrite,
                        "Frame loop status"
                    );
                }
            }

            fn log_timeout(
                &self,
                consecutive_timeouts: u32,
                status: i16,
                bytes_arrived: u32,
                buffer_cnt: u32,
                pending: u32,
            ) {
                if !self.enabled {
                    return;
                }
                if consecutive_timeouts % 10 == 0 {
                    tracing::warn!(
                        target: "pvcam_frame_trace",
                        consecutive_timeouts,
                        status,
                        bytes_arrived,
                        buffer_cnt,
                        pending,
                        "Frame loop timeout"
                    );
                }
            }
        }

        let frame_trace = FrameLoopTrace::new();
        if frame_trace.enabled {
            tracing::info!(
                target: "pvcam_frame_trace",
                log_every = frame_trace.log_every,
                "PVCAM frame trace enabled (PVCAM_TRACE=1)"
            );
        }

        // Backpressure monitoring (bd-izdj.1)
        let mut backpressure_paused = false;

        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;
        let mut consecutive_timeouts: u32 = 0;
        const CALLBACK_WAIT_TIMEOUT_MS: u64 = 2000; // 2 seconds (align with C++ 5s, but responsive enough)
                                                    // FORCE LONG TIMEOUT for debugging
        let max_consecutive_timeouts: u32 = 5; // 10 seconds total

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
        let md_frame_guard = if use_metadata {
            match ffi_safe::MdFrameGuard::new(MAX_ROIS) {
                Some(guard) => {
                    tracing::debug!(
                        "Created md_frame struct for {} ROIs for metadata decoding",
                        MAX_ROIS
                    );
                    guard
                }
                None => {
                    tracing::warn!("Failed to create md_frame struct, metadata decoding disabled");
                    ffi_safe::MdFrameGuard::null()
                }
            }
        } else {
            ffi_safe::MdFrameGuard::null()
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

            // TRACING: Loop iteration start with SDK status (bd-trace-2026-01-11)
            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                let (st, bytes, cnt) = match ffi_safe::check_cont_status(hcam) {
                    Ok(vals) => vals,
                    Err(_) => (-999, 0, 0),
                };
                let pending = callback_ctx.pending_frames.load(Ordering::Acquire);
                tracing::info!(
                    target: "pvcam_frame_trace",
                    iter = loop_iteration,
                    sdk_status = st,
                    sdk_bytes = bytes,
                    sdk_buffer_cnt = cnt,
                    callback_pending = pending,
                    "Frame loop iteration start"
                );
            }

            // Wait for frame notification (callback mode) or poll (fallback mode)
            // bd-g9gq: Use FFI safe wrapper with explicit safety contract
            let has_frames = if use_callback {
                // Callback mode (bd-ek9n.2): Wait on condvar with timeout
                // Returns number of pending frames (0 on timeout/shutdown)
                let wait_start = std::time::Instant::now();
                // bd-async-8hlm: Use async wait_for_frames inside spawn_blocking context.
                // Since we are in a spawn_blocking task, we can use block_on
                // for the notification. Notify is async-only, so we use
                // tokio::runtime::Handle::current().block_on to await the async function.
                let pending = tokio::runtime::Handle::current()
                    .block_on(callback_ctx.wait_for_frames(CALLBACK_WAIT_TIMEOUT_MS));
                let wait_elapsed_ms = wait_start.elapsed().as_millis();

                // TRACING: Wait result (bd-trace-2026-01-11)
                if pending == 0 || loop_iteration <= 10 {
                    tracing::info!(
                        target: "pvcam_frame_trace",
                        iter = loop_iteration,
                        pending_after_wait = pending,
                        wait_ms = wait_elapsed_ms,
                        timeout_ms = CALLBACK_WAIT_TIMEOUT_MS,
                        "Callback wait completed"
                    );
                }
                pending > 0
            } else {
                // Polling mode fallback: Check status with 1ms delay
                match ffi_safe::check_cont_status(hcam) {
                    Ok((_, _, cnt)) => {
                        buffer_cnt = cnt;
                        // Only treat as "has frames" when PVCAM reports filled buffers.
                        // Treating EXPOSURE_IN_PROGRESS as "has frames" causes a hot-spin when no frame is ready yet.
                        cnt > 0
                    }
                    Err(()) => {
                        // bd-diag-2026-01-17: Log before unlogged break to identify exit cause
                        eprintln!(
                            "[PVCAM DEBUG] Breaking due to check_cont_status error in polling mode (iter={})",
                            loop_iteration
                        );
                        break;
                    }
                }
            };

            if !has_frames {
                if !use_callback {
                    // Polling mode: sleep between checks
                    std::thread::sleep(Duration::from_millis(1));
                }
                consecutive_timeouts += 1;

                // DIAGNOSTIC PROBE: Print SDK status on EVERY timeout (bd-diag-2026-01-11)
                // Changed from % 10 to always print, since we exit after 5 timeouts
                if true {
                    let (st, bytes, cnt) = match ffi_safe::check_cont_status(hcam) {
                        Ok(vals) => vals,
                        Err(_) => (-999, 0, 0),
                    };
                    let pending = callback_ctx.pending_frames.load(Ordering::Acquire);
                    frame_trace.log_timeout(consecutive_timeouts, st, bytes, cnt, pending);
                    // bd-3gnv: Get SDK error code when status is READOUT_NOT_ACTIVE (0)
                    // SAFETY: pl_error_code() is a thread-local getter with no
                    // arguments and no side effects. Always safe to call.
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
                }

                /*
                // bd-3gnv: Detect stall (hardware errata) and auto-restart
                // DISABLED: C++ reproduction proved hardware does not stall.
                // This logic was causing false positives.
                if consecutive_timeouts >= 2 {
                    if let Ok((st, _, _)) = ffi_safe::check_cont_status(hcam) {
                        if st == 0 { // READOUT_NOT_ACTIVE
                            eprintln!(
                                "[PVCAM DEBUG] Detected stall (timeouts={}, status=0, frames={}) - attempting auto-restart",
                                consecutive_timeouts, frame_count.load(Ordering::Relaxed)
                            );
                            tracing::info!(
                                "PVCAM stall detected at {} frames - attempting auto-restart (bd-3gnv)",
                                frame_count.load(Ordering::Relaxed)
                            );

                            // ... (restart logic removed) ...
                        }
                    }
                }
                */

                if consecutive_timeouts >= max_consecutive_timeouts {
                    tracing::warn!("Frame loop: max consecutive timeouts reached");
                    eprintln!(
                        "[PVCAM DEBUG] Breaking due to max consecutive timeouts (iter={}, timeouts={})",
                        loop_iteration,
                        consecutive_timeouts
                    );
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

            // TRACING: Starting drain loop (bd-trace-2026-01-11)
            if loop_iteration <= 10 {
                tracing::info!(
                    target: "pvcam_frame_trace",
                    iter = loop_iteration,
                    "Starting frame drain loop"
                );
            }

            // bd-3gnv: Duplicate detection is handled by immediate exit on any duplicate.
            // The drain loop breaks as soon as a duplicate is detected, returning to
            // the outer loop to wait for the next callback signal.

            // Stack-allocated FRAME_INFO for pl_exp_get_oldest_frame_ex (bd-ek9n.3)
            // SAFETY: FRAME_INFO is a POD C struct with only primitive fields (i32, u32, etc.).
            // Zero-initialization is safe as all fields accept 0. The struct is immediately
            // passed to pl_exp_get_oldest_frame_ex which populates all fields before we read them.
            let mut frame_info: FRAME_INFO = unsafe { std::mem::zeroed() };

            // bd-flatten-2026-01-12: CRITICAL FIX - Remove inner drain loop entirely.
            // The minimal test that works for 200 frames has NO inner loop - just:
            //   wait → get_oldest_frame → unlock → continue
            // We were using an inner `loop {}` that breaks after 1 frame, but even that
            // structure seems to cause issues. Flatten to match minimal test exactly.

            // Check shutdown before attempting frame retrieval
            if !streaming.get() || shutdown.load(Ordering::Acquire) {
                // bd-diag-2026-01-17: Log before unlogged break to identify exit cause
                eprintln!(
                    "[PVCAM DEBUG] Breaking due to shutdown check (iter={}, streaming={}, shutdown={})",
                    loop_iteration,
                    streaming.get(),
                    shutdown.load(Ordering::Acquire)
                );
                break;
            }

            // FLAT STRUCTURE: ONE frame per wait, matching minimal test pattern exactly.
            // No inner loop - just try to get the frame and process it.

            // No inner loop - just try to get the frame and process it.
            let frame_ptr = match ffi_safe::get_oldest_frame(hcam, &mut frame_info) {
                Ok(ptr) => ptr,
                Err(()) => {
                    // No frame available despite callback - this is unusual
                    // TRACING: No frame available (bd-trace-2026-01-11)
                    if loop_iteration <= 10 {
                        tracing::info!(
                            target: "pvcam_frame_trace",
                            iter = loop_iteration,
                            "get_oldest_frame returned no frame despite callback"
                        );
                    }
                    // bd-spin-fix-2026-01-17: CRITICAL - Must consume pending count on failure!
                    // Without this, the fast-path in wait_for_frames sees pending_frames > 0,
                    // returns immediately, and we spin at 100% CPU trying to fetch a
                    // non-existent frame. Decrement counter and yield to break spin cycle.
                    if use_callback {
                        callback_ctx.consume_one();
                        std::thread::yield_now();
                    }
                    // Continue outer loop to wait for next callback
                    continue;
                }
            };

            frames_processed_in_drain += 1;

            // bd-unlock-before-copy-2026-01-12: CRITICAL FIX
            // The minimal test that works for 200 frames does: get_oldest_frame → UNLOCK → process
            // We MUST unlock BEFORE any processing to match the SDK's expected timing.
            //
            // SAFETY INVARIANT: This unlock-before-copy pattern is ONLY safe in
            // CIRC_NO_OVERWRITE mode. In this mode, the SDK won't reuse the buffer
            // slot until ALL buffer slots are filled, so frame_ptr remains valid
            // for the copy and metadata decode that follow.
            //
            // CIRC_OVERWRITE mode is disabled above (bd-g3ap P0) because the SDK
            // can reuse the DMA buffer immediately after unlock in that mode,
            // invalidating frame_ptr before the copy completes.

            // Step 1: UNLOCK IMMEDIATELY after get_oldest_frame - EXACTLY like minimal test
            // SAFETY: frame_info is a stack-allocated FRAME_INFO filled by
            // pl_exp_get_oldest_frame_ex above. FrameNr is a plain i32 field.
            let unlock_frame_nr = unsafe { frame_info.FrameNr };
            // bd-fix-2026-01-17: Use loop_iteration (global counter) instead of
            // frames_processed_in_drain (reset each loop) to limit debug logging.
            // Previous bug: logging fired every frame due to counter reset.
            if loop_iteration <= 25 || loop_iteration % 50 == 0 {
                eprintln!(
                    "[PVCAM DEBUG] Unlocking frame {} (before copy)",
                    unlock_frame_nr
                );
            }
            let unlock_result = ffi_safe::release_oldest_frame(hcam);
            if !unlock_result {
                unlock_failures += 1;
                eprintln!("[PVCAM ERROR] Unlock failed for frame {}", unlock_frame_nr);
            } else if loop_iteration <= 25 || loop_iteration % 50 == 0 {
                eprintln!(
                    "[PVCAM DEBUG] Frame {} unlocked successfully",
                    unlock_frame_nr
                );
            }

            // bd-diag-2026-01-12: REMOVED - calling check_cont_status after unlock
            // may cause SDK timing issues that stop callbacks at ~19 frames.
            // The minimal tests that work for 200 frames don't call check_cont_status
            // after unlock.

            // bd-diag-skip-processing-2026-01-12: DIAGNOSTIC MODE
            // When PVCAM_SKIP_PROCESSING=1 is set, skip ALL processing after unlock
            // to match minimal test behavior exactly (get → unlock → continue).
            // This isolates whether the issue is in processing vs SDK interaction.
            static SKIP_PROCESSING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let skip_processing = *SKIP_PROCESSING.get_or_init(|| {
                std::env::var("PVCAM_SKIP_PROCESSING")
                    .map(|v| v == "1")
                    .unwrap_or(false)
            });
            if skip_processing {
                // Exactly like minimal test: get → unlock → continue immediately
                frame_count.fetch_add(1, Ordering::Relaxed);
                if use_callback {
                    callback_ctx.consume_one();
                }
                loop_iteration += 1;
                continue;
            }

            // Backpressure handling (bd-izdj.1)
            // Pause frame processing when pool availability drops below 20%.
            if backpressure_paused {
                if buffer_pool.is_recovered() {
                    backpressure_paused = false;
                    tracing::warn!(
                        available = buffer_pool.available(),
                        capacity = buffer_pool.size(),
                        "Buffer pool recovered - resuming frame processing"
                    );
                } else {
                    dropped_frames.fetch_add(1, Ordering::Relaxed);
                    if use_callback {
                        callback_ctx.consume_one();
                    }
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
            } else if buffer_pool.is_under_pressure() {
                backpressure_paused = true;
                buffer_pool.record_exhaustion_event();
                tracing::warn!(
                    available = buffer_pool.available(),
                    capacity = buffer_pool.size(),
                    "Buffer pool under pressure - pausing frame processing"
                );
                dropped_frames.fetch_add(1, Ordering::Relaxed);
                if use_callback {
                    callback_ctx.consume_one();
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }

            // Step 2: Copy pixel data AFTER unlock
            // In CIRC_NO_OVERWRITE mode, the frame_ptr data is still valid because
            // the SDK won't reuse this buffer slot until all 20 slots are filled.
            let copy_bytes = frame_bytes.min(expected_frame_bytes);

            // Allocation tracking instrumentation (bd-0dax.1.1)
            // Track allocation latency and total bytes for frame buffer copies
            static ALLOC_TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);
            static ALLOC_TOTAL_TIME_NS: AtomicU64 = AtomicU64::new(0);
            static ALLOC_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
            static POOL_HITS: AtomicU64 = AtomicU64::new(0);
            static POOL_MISSES: AtomicU64 = AtomicU64::new(0);

            let alloc_start = std::time::Instant::now();

            // bd-0dax.4: TRUE zero-allocation path using BufferPool + freeze()
            // When consumers drop the Frame, buffer auto-returns to pool via Bytes::drop.
            // bd-dmbl: Drop frames with warning when pool is exhausted (Option A).
            let pixel_data: Bytes = match buffer_pool.try_acquire() {
                Some(mut buffer) => {
                    // Fast path: Copy SDK data into pre-allocated pool buffer
                    // SAFETY: copy_from_ptr is safe because:
                    // 1. frame_ptr is valid - returned by pl_exp_get_oldest_frame_ex
                    // 2. copy_bytes <= expected_frame_bytes, validated against SDK frame_bytes
                    // 3. In CIRC_NO_OVERWRITE mode, frame data remains valid after unlock
                    //    because SDK won't reuse the buffer until all slots are filled
                    // 4. buffer has capacity >= copy_bytes (created with actual_frame_bytes)
                    unsafe {
                        buffer.copy_from_ptr(frame_ptr as *const u8, copy_bytes);
                    }
                    // Zero-copy conversion to Bytes - buffer returns to pool when dropped
                    let data = buffer.freeze();
                    POOL_HITS.fetch_add(1, Ordering::Relaxed);
                    data
                }
                None => {
                    // bd-dmbl: Pool exhausted - drop frame with warning (Option A)
                    // This indicates backpressure (consumers too slow).
                    // Dropping frames maintains real-time performance at the cost of completeness.
                    POOL_MISSES.fetch_add(1, Ordering::Relaxed);
                    backpressure_paused = true;
                    buffer_pool.record_exhaustion_event();
                    let drop_count = dropped_frames.fetch_add(1, Ordering::Relaxed) + 1;
                    let misses = POOL_MISSES.load(Ordering::Relaxed);

                    // Log warning with rate limiting to avoid log spam
                    // SAFETY: frame_info.FrameNr is a plain i32 field in a
                    // stack-allocated FRAME_INFO filled earlier by the SDK.
                    if drop_count <= 10 || drop_count % 100 == 0 {
                        // eprintln for guaranteed console visibility during debugging
                        eprintln!(
                            "[PVCAM BACKPRESSURE] Frame {} dropped - pool exhausted ({}/{} available, {} total dropped)",
                            unsafe { frame_info.FrameNr },
                            buffer_pool.available(),
                            buffer_pool.size(),
                            drop_count
                        );
                        // SAFETY: Same frame_info.FrameNr access as above.
                        tracing::warn!(
                            target: "pvcam_pool",
                            frame_nr = unsafe { frame_info.FrameNr },
                            dropped_frames = drop_count,
                            pool_misses = misses,
                            pool_available = buffer_pool.available(),
                            pool_size = buffer_pool.size(),
                            "Buffer pool exhausted - dropping frame and pausing processing (bd-dmbl). \
                             Consumers may be too slow or pool size too small."
                        );
                    }

                    // Consume callback signal since we're not processing this frame
                    if use_callback {
                        callback_ctx.consume_one();
                    }

                    // Skip to next frame - don't process this one
                    continue;
                }
            };
            let alloc_duration = alloc_start.elapsed();

            // Update allocation metrics (Relaxed ordering for performance)
            ALLOC_TOTAL_BYTES.fetch_add(copy_bytes as u64, Ordering::Relaxed);
            ALLOC_TOTAL_TIME_NS.fetch_add(alloc_duration.as_nanos() as u64, Ordering::Relaxed);
            let alloc_frame_num = ALLOC_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

            // Log allocation metrics every 100 frames
            if alloc_frame_num % 100 == 0 {
                let total_bytes = ALLOC_TOTAL_BYTES.load(Ordering::Relaxed);
                let total_ns = ALLOC_TOTAL_TIME_NS.load(Ordering::Relaxed);
                let pool_hits = POOL_HITS.load(Ordering::Relaxed);
                let pool_misses = POOL_MISSES.load(Ordering::Relaxed);
                let total_dropped = dropped_frames.load(Ordering::Relaxed);
                let avg_alloc_us = if alloc_frame_num > 0 {
                    (total_ns / alloc_frame_num) / 1000
                } else {
                    0
                };
                let hit_rate_pct = if alloc_frame_num > 0 {
                    (pool_hits * 100) / alloc_frame_num
                } else {
                    0
                };
                tracing::info!(
                    target: "pvcam_alloc_trace",
                    frame = alloc_frame_num,
                    total_allocated_mb = total_bytes / 1_000_000,
                    avg_alloc_us = avg_alloc_us,
                    last_alloc_us = alloc_duration.as_micros(),
                    copy_bytes = copy_bytes,
                    pool_hit_rate_pct = hit_rate_pct,
                    pool_hits = pool_hits,
                    pool_misses = pool_misses,
                    dropped_frames = total_dropped,
                    "Allocation metrics (bd-0dax.3, bd-dmbl)"
                );
            }

            // Step 3: Decode metadata (frame_ptr data still valid in NO_OVERWRITE mode)
            // SAFETY: md_frame_guard wraps a valid md_frame pointer (non-null checked above).
            // frame_ptr and frame_bytes come from the SDK's get_oldest_frame call.
            // decode_frame_metadata is an FFI wrapper that reads the frame buffer.
            // The header dereference is safe because decode populates it on success.
            let frame_metadata = if !md_frame_guard.as_ptr().is_null() {
                unsafe {
                    if ffi_safe::decode_frame_metadata(
                        md_frame_guard.as_ptr(),
                        frame_ptr,
                        frame_bytes as uns32,
                    ) {
                        let hdr = &*(*md_frame_guard.as_ptr()).header;
                        let ts_res = hdr.timestampResNs as u64;
                        let exp_res = hdr.exposureTimeResNs as u64;
                        Some(FrameMetadata {
                            frame_nr: hdr.frameNr as i32,
                            timestamp_bof_ns: (hdr.timestampBOF as u64) * ts_res,
                            timestamp_eof_ns: (hdr.timestampEOF as u64) * ts_res,
                            exposure_time_ns: (hdr.exposureTime as u64) * exp_res,
                            bit_depth: hdr.bitDepth as u16,
                            roi_count: hdr.roiCount,
                        })
                    } else {
                        None
                    }
                }
            } else {
                None
            };

            // bd-0o6b: Extract per-ROI data when multi-ROI frame detected.
            // Only triggers when metadata reports roiCount > 1 and md_frame is valid.
            // This enables downstream consumers to access individual ROI pixel data.
            if let Some(ref md) = frame_metadata {
                if md.roi_count > 1 && !md_frame_guard.as_ptr().is_null() {
                    match ffi_safe::extract_roi_data(md_frame_guard.as_ptr()) {
                        Ok(roi_data) => {
                            // Log per-ROI dimensions on first multi-ROI frame or periodically
                            if loop_iteration <= 5 || loop_iteration % 500 == 0 {
                                for roi in &roi_data {
                                    tracing::info!(
                                        target: "pvcam_multi_roi",
                                        roi_nr = roi.roi_nr,
                                        width = roi.width,
                                        height = roi.height,
                                        s1 = roi.s1,
                                        s2 = roi.s2,
                                        p1 = roi.p1,
                                        p2 = roi.p2,
                                        data_bytes = roi.pixels.len(),
                                        "Multi-ROI frame: ROI data extracted (bd-0o6b)"
                                    );
                                }
                            }
                            // TODO(bd-0o6b): Pass roi_data to downstream consumers
                            // when multi-ROI Frame output format is defined.
                        }
                        Err(e) => {
                            tracing::warn!("Failed to extract multi-ROI data: {} (bd-0o6b)", e);
                        }
                    }
                }
            }

            // TRACING: Frame retrieved (bd-trace-2026-01-11)
            // bd-non-ex-2026-01-12: frame_info.FrameNr may be -1 if using non-_ex get_oldest_frame
            // bd-fix-2026-01-17: Use loop_iteration only (frames_processed_in_drain resets each loop)
            // SAFETY: frame_info is a stack-allocated FRAME_INFO struct filled
            // by pl_exp_get_oldest_frame_ex. All field accesses (FrameNr,
            // TimeStamp, TimeStampBOF, ReadoutTime) read plain integer fields.
            if loop_iteration <= 10 || loop_iteration % 100 == 0 {
                if unsafe { frame_info.FrameNr } >= 0 {
                    unsafe {
                        tracing::info!(
                            target: "pvcam_frame_trace",
                            iter = loop_iteration,
                            drain_frame = frames_processed_in_drain,
                            hw_frame_nr = frame_info.FrameNr,
                            timestamp = frame_info.TimeStamp,
                            timestamp_bof = frame_info.TimeStampBOF,
                            readout_time = frame_info.ReadoutTime,
                            "Frame retrieved from PVCAM"
                        );
                    }
                } else {
                    tracing::info!(
                        target: "pvcam_frame_trace",
                        iter = loop_iteration,
                        drain_frame = frames_processed_in_drain,
                        "Frame retrieved from PVCAM (no FRAME_INFO - using non-_ex API)"
                    );
                }
            }

            // Remaining frame processing uses our copies (pixel_data, frame_metadata, frame_info)
            // frame_ptr is NO LONGER VALID after unlock above
            // SAFETY: frame_info.FrameNr is a plain i32 field in the stack-
            // allocated FRAME_INFO. callback_ctx fields are atomic loads.
            // last_hw_frame_nr and discontinuity_events are AtomicI32/AtomicU64
            // with correct ordering. No raw pointer dereferences occur here.
            unsafe {
                // bd-non-ex-2026-01-12: Get frame number from callback context when using non-_ex API
                // The callback still receives FRAME_INFO from PVCAM even if get_oldest_frame doesn't fill it
                let current_frame_nr = if frame_info.FrameNr >= 0 {
                    frame_info.FrameNr
                } else {
                    // Using non-_ex API - get frame number from callback context
                    callback_ctx.latest_frame_nr.load(Ordering::Acquire)
                };

                // Frame loss detection (bd-ek9n.3): Check for gaps in FrameNr sequence
                // FrameNr is 1-based hardware counter from PVCAM
                // bd-non-ex-2026-01-12: Skip frame number tracking if we don't have valid data
                let prev_frame_nr = last_hw_frame_nr.load(Ordering::Acquire);

                if current_frame_nr >= 0 && prev_frame_nr >= 0 {
                    // Only do frame number checks if we have valid frame numbers
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

                        // Log the first duplicate in this drain with FRAME_INFO details for diagnosis.
                        if consecutive_duplicates == 1 {
                            tracing::warn!(
                                    "PVCAM duplicate frame detected: FrameNr={}, buffer_cnt={}, bytes_arrived={}",
                                    current_frame_nr,
                                    buffer_cnt,
                                    bytes_arrived
                                );
                        }

                        // bd-immediate-unlock-2026-01-12: Frame already unlocked at top of loop
                        // No need to unlock again here - just consume callback and exit
                        if use_callback {
                            callback_ctx.consume_one();
                        }

                        // bd-flatten-2026-01-12: On duplicate frame, skip processing and
                        // wait for next callback. (No inner loop anymore - just continue.)
                        continue; // Wait for next callback
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
                // Update last seen frame number (only if we have valid data)
                if current_frame_nr >= 0 {
                    last_hw_frame_nr.store(current_frame_nr, Ordering::Release);
                }
                // bd-3gnv: Reset duplicate counter on successful new frame
                consecutive_duplicates = 0;

                // bd-immediate-unlock-2026-01-12: pixel_data, frame_metadata, and unlock
                // are all handled at the top of the loop immediately after get_oldest_frame.
                // frame_ptr is no longer valid here - use only our copies.

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
                    // bd-immediate-unlock-2026-01-12: Frame already unlocked at top of loop
                    // Just consume callback and skip
                    if use_callback {
                        callback_ctx.consume_one();
                    }
                    continue; // Skip to next frame
                }

                // Decrement pending frame counter (callback mode)
                if use_callback {
                    callback_ctx.consume_one();
                }

                let monotonic_frame_count = frame_count.fetch_add(1, Ordering::Relaxed) + 1;

                let pending = callback_ctx.pending_frames.load(Ordering::Acquire);
                let hw_frame_nr = current_frame_nr as i64;
                frame_trace.log_frame(
                    monotonic_frame_count,
                    hw_frame_nr,
                    pending,
                    buffer_cnt,
                    bytes_arrived,
                    lost_frames.load(Ordering::Relaxed),
                    discontinuity_events.load(Ordering::Relaxed),
                    consecutive_timeouts,
                    circ_overwrite,
                );

                // Create Frame with ownership transfer - no additional copy (bd-ek9n.5)
                // Populate metadata using builder pattern (bd-183h)
                // bd-w5az: Use hardware bit_depth from metadata when available,
                // fall back to 16 (most PVCAM cameras are 16-bit).
                let frame_bit_depth: u32 = frame_metadata
                    .as_ref()
                    .map(|md| md.bit_depth as u32)
                    .unwrap_or(16);
                let mut frame = Frame::from_bytes(width, height, frame_bit_depth, pixel_data)
                    .with_frame_number(monotonic_frame_count)
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
                let ext_metadata = common::data::FrameMetadata {
                    binning: Some(binning),
                    ..Default::default()
                };
                frame = frame.with_metadata(ext_metadata);

                let frame_arc = Arc::new(frame);

                // Deliver to channels
                // CRITICAL: Send to broadcast FIRST before reliable path.
                // The reliable path uses blocking_send which can block if the
                // measurement pipeline is backpressured. Sending to broadcast
                // first ensures GUI streaming gets frames regardless.
                let receiver_count = frame_tx.receiver_count();

                // TRACING: Broadcast subscriber count (bd-trace-2026-01-11)
                // bd-fix-2026-01-17: Check BOTH broadcast subscribers AND tap observers
                // The gRPC streaming uses tap observers, not the broadcast channel, so we must
                // count observers to avoid stopping streaming when GUI is connected via gRPC.
                // bd-r8ux: Also count primary_tx as a consumer to prevent early exit.
                let has_observers = tap_registry.has_taps();
                let has_primary = primary_tx.is_some();
                let has_consumers = receiver_count > 0 || has_observers || has_primary;

                if monotonic_frame_count <= 10 || monotonic_frame_count % 30 == 1 {
                    tracing::info!(
                        target: "pvcam_frame_trace",
                        frame_nr = monotonic_frame_count,
                        hw_frame_nr = current_frame_nr,
                        receiver_count,
                        observer_count = tap_registry.tap_count(),
                        "Sending frame to broadcast channel"
                    );
                }

                if !has_consumers {
                    // Track when we lost all subscribers AND observers (bd-cckz, bd-fix-2026-01-17)
                    if no_subscribers_since.is_none() {
                        no_subscribers_since = Some(std::time::Instant::now());
                        tracing::info!(
                            "No consumers (broadcast={}, observers={}), starting {} second disconnect timer",
                            receiver_count,
                            tap_registry.tap_count(),
                            NO_SUBSCRIBER_TIMEOUT.as_secs()
                        );
                    } else if let Some(since) = no_subscribers_since {
                        if since.elapsed() >= NO_SUBSCRIBER_TIMEOUT {
                            tracing::info!(
                                "No consumers for {} seconds, stopping acquisition (bd-cckz)",
                                NO_SUBSCRIBER_TIMEOUT.as_secs()
                            );
                            eprintln!(
                                "[PVCAM DEBUG] Breaking due to no consumers for {} seconds (iter={}, receiver_count={}, observers={})",
                                NO_SUBSCRIBER_TIMEOUT.as_secs(),
                                loop_iteration,
                                receiver_count,
                                tap_registry.tap_count()
                            );
                            break;
                        }
                    }
                    tracing::warn!(
                        "Dropping frame {}: no active consumers (broadcast={}, observers={})",
                        current_frame_nr,
                        receiver_count,
                        tap_registry.tap_count()
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

                // bd-0dax.4: Run taps SYNCHRONOUSLY before broadcast (observers get &Frame)
                tap_registry.apply_frame_with_pixels(&*frame_arc);

                // bd-r8ux: Deliver through primary_tx if a consumer is registered.
                // Copies pixel data from the already-built frame_arc into a pooled
                // LoanedFrame for zero-allocation downstream consumption (HDF5, measurement).
                if let (Some(ref p_tx), Some(ref pool)) = (&primary_tx, &primary_frame_pool) {
                    if let Some(mut loaned) = pool.try_acquire() {
                        let fd = loaned.get_mut();
                        let src = frame_arc.data.as_ref();
                        let len = src.len();
                        if len > fd.pixels.capacity() {
                            // Frame size mismatch — pool was created with a different
                            // frame capacity than the SDK is now producing. Log and skip.
                            if monotonic_frame_count <= 5 || monotonic_frame_count % 100 == 0 {
                                tracing::warn!(
                                    frame_len = len,
                                    pool_capacity = fd.pixels.capacity(),
                                    "primary_tx: frame exceeds pool buffer capacity, skipping (bd-r8ux)"
                                );
                            }
                        } else {
                            fd.width = width;
                            fd.height = height;
                            fd.bit_depth = frame_bit_depth;
                            fd.frame_number = monotonic_frame_count;
                            fd.hw_frame_nr = current_frame_nr;
                            fd.roi_x = roi_x;
                            fd.roi_y = roi_y;
                            fd.binning = Some(binning);
                            if let Some(ref md) = frame_metadata {
                                fd.timestamp_ns = md.timestamp_bof_ns;
                                fd.exposure_ms = md.exposure_time_ns as f64 / 1_000_000.0;
                            } else {
                                fd.timestamp_ns = common::data::Frame::timestamp_now();
                                fd.exposure_ms = exposure_ms;
                            }
                            // SAFETY: src points to valid frame_arc.data bytes.
                            // fd.pixels has sufficient capacity (checked above).
                            // src and fd.pixels don't overlap (Bytes heap vs pool allocation).
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    src.as_ptr(),
                                    fd.pixels.as_mut_ptr(),
                                    len,
                                );
                            }
                            fd.actual_len = len;
                            if p_tx.try_send(loaned).is_err() && monotonic_frame_count % 100 == 0 {
                                tracing::warn!(
                                    "PVCAM primary channel full at frame {} (bd-r8ux)",
                                    monotonic_frame_count
                                );
                            }
                        }
                    } else if monotonic_frame_count % 100 == 0 {
                        tracing::warn!(
                            "PVCAM primary frame pool exhausted at frame {} (bd-r8ux)",
                            monotonic_frame_count
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

                // Gemini SDK review: Send metadata through channel if available
                // Use try_send to avoid blocking frame loop
                if let (Some(md), Some(ref tx)) = (frame_metadata, &metadata_tx) {
                    let _ = tx.try_send(md); // Non-blocking: drop if slow
                }
            }

            // bd-flatten-2026-01-12: No inner loop anymore - we process ONE frame per callback
            // and automatically continue to the outer loop to wait for the next callback.
            // This matches the minimal test pattern exactly.
            if loop_iteration <= 10 {
                tracing::info!(
                    target: "pvcam_frame_trace",
                    iter = loop_iteration,
                    "Flat frame processing: processed 1 frame, continuing to wait for next callback"
                );
            }

            // bd-3gnv: Critical warning if unlocks are failing - this causes buffer starvation
            if unlock_failures > 0 {
                tracing::error!(
                    "PVCAM unlock failures: {} in drain loop (bd-3gnv)",
                    unlock_failures
                );
            }

            // Gemini SDK review: Exit outer loop on fatal error to prevent zombie streaming
            if fatal_error {
                tracing::error!("Exiting frame loop due to fatal acquisition error");
                eprintln!(
                    "[PVCAM DEBUG] Breaking due to fatal_error (iter={})",
                    loop_iteration
                );
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
                    // SAFETY: `hcam` is a valid camera handle. `status`,
                    // `bytes_arrived`, and `buffer_cnt` are stack-allocated
                    // output parameters with correct types. This is a read-only
                    // SDK query with no side effects.
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
        } // end of outer while loop

        // bd-3gnv: Debug why we exited the outer loop
        eprintln!(
            "[PVCAM DEBUG] Frame loop exited: iter={}, streaming={}, shutdown={}",
            loop_iteration,
            streaming.get(),
            shutdown.load(Ordering::Acquire)
        );

        // Log acquisition summary with frame loss statistics (bd-ek9n.3, bd-dmbl)
        let total_frames = frame_count.load(Ordering::Relaxed);
        let total_lost = lost_frames.load(Ordering::Relaxed);
        let total_discontinuities = discontinuity_events.load(Ordering::Relaxed);
        let total_dropped = dropped_frames.load(Ordering::Relaxed);

        if total_lost > 0 || total_discontinuities > 0 || total_dropped > 0 {
            tracing::warn!(
                "PVCAM acquisition ended: {} frames captured, {} frames lost, {} discontinuities, {} dropped (pool exhaustion)",
                total_frames,
                total_lost,
                total_discontinuities,
                total_dropped
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
