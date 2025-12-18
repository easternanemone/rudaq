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

use anyhow::{anyhow, bail, Result};
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicI16;
#[cfg(feature = "pvcam_hardware")]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use daq_core::data::Frame;
use daq_core::parameter::Parameter;
use crate::components::connection::PvcamConnection;
use daq_core::core::Roi;
use std::time::Duration;
use tokio::time::timeout;

#[cfg(feature = "pvcam_hardware")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_hardware")]
use tokio::task::JoinHandle;

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
        let result = self.condvar.wait_timeout_while(guard, timeout_duration, |notified| {
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
        let _ = self.pending_frames.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n > 0 { Some(n - 1) } else { None }
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

    /// Frame loss detection counters (bd-ek9n.3).
    /// Total number of frames lost due to buffer overflows or processing delays.
    pub lost_frames: Arc<AtomicU64>,
    /// Number of discontinuity events (gaps in frame sequence).
    pub discontinuity_events: Arc<AtomicU64>,
    /// Last hardware frame number for gap detection (-1 = uninitialized).
    #[cfg(feature = "pvcam_hardware")]
    last_hardware_frame_nr: Arc<AtomicI32>,

    /// Shutdown signal for the poll loop (bd-z8q8).
    /// Set to true in Drop to signal the poll thread to exit.
    #[cfg(feature = "pvcam_hardware")]
    shutdown: Arc<AtomicBool>,
    #[cfg(feature = "pvcam_hardware")]
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Circular buffer passed to PVCAM - MUST remain allocated for acquisition lifetime.
    /// Using Vec<u8> to match the byte pointer passed to pl_exp_start_cont.
    #[cfg(feature = "pvcam_hardware")]
    circ_buffer: Arc<Mutex<Option<Vec<u8>>>>,
    #[cfg(feature = "pvcam_hardware")]
    trigger_frame: Arc<Mutex<Option<Vec<u16>>>>,
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
}

impl PvcamAcquisition {
    pub fn new(streaming: Parameter<bool>) -> Self {
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            streaming,
            frame_count: Arc::new(AtomicU64::new(0)),
            frame_tx,
            reliable_tx: Arc::new(Mutex::new(None)),
            #[cfg(feature = "arrow_tap")]
            arrow_tap: Arc::new(Mutex::new(None)),

            // Frame loss detection counters (bd-ek9n.3)
            lost_frames: Arc::new(AtomicU64::new(0)),
            discontinuity_events: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "pvcam_hardware")]
            last_hardware_frame_nr: Arc::new(AtomicI32::new(-1)), // -1 = uninitialized

            #[cfg(feature = "pvcam_hardware")]
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "pvcam_hardware")]
            poll_handle: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            circ_buffer: Arc::new(Mutex::new(None)),
            #[cfg(feature = "pvcam_hardware")]
            trigger_frame: Arc::new(Mutex::new(None)),
            // Pinned callback context for EOF notifications (bd-ek9n.2)
            #[cfg(feature = "pvcam_hardware")]
            callback_context: Arc::new(Box::pin(CallbackContext::new())),
            // Camera handle and callback state for Drop cleanup
            // -1 is sentinel for "no active handle"
            #[cfg(feature = "pvcam_hardware")]
            active_hcam: Arc::new(AtomicI16::new(-1)),
            #[cfg(feature = "pvcam_hardware")]
            callback_registered: Arc::new(AtomicBool::new(false)),
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
        const MIN_BUFFER_FRAMES: usize = 16;
        const MAX_BUFFER_FRAMES: usize = 256;
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
    pub async fn set_arrow_tap(&self, tx: tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>) {
        let mut guard = self.arrow_tap.lock().await;
        *guard = Some(tx);
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
        exposure_ms: f64
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

            // PVCAM Best Practices: Use actual frame_bytes from pl_exp_setup_cont
            // rather than assuming pixels * 2 - metadata/alignment can change frame size.
            let mut frame_bytes: uns32 = 0;
            unsafe {
                // SAFETY: h is a valid camera handle; region points to initialized rgn_type; frame_bytes is writable.
                if pl_exp_setup_cont(
                    h,
                    1,
                    &region as *const _,
                    TIMED_MODE,
                    exposure_ms as uns32,
                    &mut frame_bytes,
                    CIRC_NO_OVERWRITE,
                ) == 0 {
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!("Failed to setup continuous acquisition"));
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
            tracing::info!(
                "PVCAM circular buffer: {} frames ({:.2} MB)",
                buffer_count,
                (actual_frame_bytes * buffer_count) as f64 / (1024.0 * 1024.0)
            );

            // PVCAM Best Practices (bd-ek9n.2): Register EOF callback before starting acquisition
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

            let mut circ_buf: Vec<u8> = vec![0u8; circ_buf_size];
            let circ_ptr = circ_buf.as_mut_ptr();

            unsafe {
                // SAFETY: circ_ptr points to contiguous buffer sized circ_size_bytes; SDK expects byte size.
                if pl_exp_start_cont(h, circ_ptr as *mut _, circ_size_bytes) == 0 {
                    // Deregister callback on failure
                    if use_callback {
                        pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                        self.callback_registered.store(false, Ordering::Release);
                    }
                    self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
                    let _ = self.streaming.set(false).await;
                    return Err(anyhow!("Failed to start continuous acquisition"));
                }
            }

            // CRITICAL: Store the ORIGINAL buffer passed to pl_exp_start_cont.
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

            let poll_handle = tokio::task::spawn_blocking(move || {
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
                    actual_frame_bytes,
                    expected_frame_bytes,
                    width,
                    height,
                );
            });

            *self.poll_handle.lock().await = Some(poll_handle);
            return Ok(());
        }

        // Mock path (or no handle)
        #[cfg(not(feature = "pvcam_hardware"))]
        {
            tracing::warn!("start_stream: pvcam_hardware NOT compiled - using mock stream");
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx).await?;
        }

        // Handle case where hardware feature enabled but handle missing (mock fallback logic)
        #[cfg(feature = "pvcam_hardware")]
        if conn.handle().is_none() {
            tracing::warn!("start_stream: pvcam_hardware compiled but handle is None - falling back to mock stream");
            self.start_mock_stream(roi, binning, exposure_ms, reliable_tx).await?;
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
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>
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
                if !streaming.get() { break; }

                let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                let mut pixels = vec![0u16; frame_size];
                for y in 0..binned_height {
                    for x in 0..binned_width {
                        let value = (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                        pixels[(y * binned_width + x) as usize] = value;
                    }
                }

                let frame = Arc::new(Frame::from_u16(binned_width, binned_height, &pixels));
                
                if let Some(ref tx) = reliable_tx {
                    let _ = tx.send(frame.clone()).await;
                }
                let _ = frame_tx.send(frame);
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

            if let Some(handle) = self.poll_handle.lock().await.take() {
                let _ = handle.await;
            }
            if let Some(h) = conn.handle() {
                unsafe {
                    // SAFETY: h is an open camera handle; stopping acquisition after poll loop exit.
                    pl_exp_stop_cont(h, CCS_HALT);
                    // Deregister EOF callback if registered (bd-ek9n.2)
                    if self.callback_registered.load(Ordering::Acquire) {
                        pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
                        self.callback_registered.store(false, Ordering::Release);
                    }
                }
            }
            // Clear stored state after cleanup
            self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
            *self.circ_buffer.lock().await = None;
        }
        Ok(())
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
    #[cfg(feature = "pvcam_hardware")]
    #[allow(clippy::too_many_arguments)]
    fn frame_loop_hardware(
        hcam: i16,
        streaming: Parameter<bool>,
        shutdown: Arc<AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        #[cfg(feature = "arrow_tap")]
        arrow_tap: Option<tokio::sync::mpsc::Sender<Arc<arrow::array::UInt16Array>>>,
        frame_count: Arc<AtomicU64>,
        lost_frames: Arc<AtomicU64>,
        discontinuity_events: Arc<AtomicU64>,
        last_hw_frame_nr: Arc<AtomicI32>,
        callback_ctx: Arc<std::pin::Pin<Box<CallbackContext>>>,
        use_callback: bool,
        frame_bytes: usize,
        expected_frame_bytes: usize,
        width: u32,
        height: u32,
    ) {
        let mut status: i16 = 0;
        let mut bytes_arrived: uns32 = 0;
        let mut buffer_cnt: uns32 = 0;
        let mut consecutive_timeouts: u32 = 0;
        const MAX_CONSECUTIVE_TIMEOUTS: u32 = 5000;
        const CALLBACK_WAIT_TIMEOUT_MS: u64 = 100; // Short timeout to check shutdown

        if use_callback {
            tracing::debug!("Using EOF callback mode for frame acquisition");
        } else {
            tracing::debug!("Using polling mode for frame acquisition");
        }

        // Check both streaming flag and shutdown signal (bd-z8q8).
        // Shutdown is set in Drop to ensure the loop exits before SDK uninit.
        // Use Acquire ordering to synchronize with Release store in Drop (bd-nfk6).
        while streaming.get() && !shutdown.load(Ordering::Acquire) {
            // Wait for frame notification (callback mode) or poll (fallback mode)
            let has_frames = if use_callback {
                // Callback mode (bd-ek9n.2): Wait on condvar with timeout
                // Returns number of pending frames (0 on timeout/shutdown)
                callback_ctx.wait_for_frames(CALLBACK_WAIT_TIMEOUT_MS) > 0
            } else {
                // Polling mode fallback: Check status with 1ms delay
                unsafe {
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) == 0 {
                        break;
                    }
                    status == READOUT_COMPLETE || status == EXPOSURE_IN_PROGRESS
                }
            };

            if !has_frames {
                if !use_callback {
                    // Polling mode: sleep between checks
                    std::thread::sleep(Duration::from_millis(1));
                }
                consecutive_timeouts += 1;
                if consecutive_timeouts >= MAX_CONSECUTIVE_TIMEOUTS {
                    tracing::warn!("Frame loop: max consecutive timeouts reached");
                    break;
                }
                continue;
            }
            consecutive_timeouts = 0;

            // Drain loop: process all available frames to avoid losing events
            // when multiple callbacks fire while we're processing
            let mut frames_processed_in_drain: u32 = 0;

            // Stack-allocated FRAME_INFO for pl_exp_get_oldest_frame_ex (bd-ek9n.3)
            // Using zeroed struct as PVCAM will fill in the fields on frame retrieval.
            let mut frame_info: FRAME_INFO = unsafe { std::mem::zeroed() };

            loop {
                // Check shutdown between frames
                if !streaming.get() || shutdown.load(Ordering::Acquire) {
                    break;
                }

                // Check acquisition status
                unsafe {
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) == 0 {
                        break;
                    }

                    if status == READOUT_FAILED {
                        tracing::error!("PVCAM readout failed");
                        break;
                    }

                    // Fetch oldest frame with FRAME_INFO for loss detection (bd-ek9n.3)
                    let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                    if pl_exp_get_oldest_frame_ex(hcam, &mut frame_ptr, &mut frame_info) == 0 || frame_ptr.is_null() {
                        // No more frames available - exit drain loop
                        break;
                    }
                    frames_processed_in_drain += 1;

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
                            tracing::warn!(
                                "Frame loss detected: expected FrameNr {}, got {} ({} frames lost)",
                                expected_frame_nr,
                                current_frame_nr,
                                frames_lost
                            );
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

                    // Memory optimization (bd-ek9n.5): Single copy from SDK buffer.
                    // Trim to expected_frame_bytes to exclude metadata/padding.
                    let copy_bytes = frame_bytes.min(expected_frame_bytes);
                    let sdk_bytes = std::slice::from_raw_parts(
                        frame_ptr as *const u8,
                        copy_bytes,
                    );
                    let pixel_data = sdk_bytes.to_vec();

                    // Unlock ASAP to free SDK buffer for next frame
                    pl_exp_unlock_oldest_frame(hcam);

                    // Decrement pending frame counter (callback mode)
                    if use_callback {
                        callback_ctx.consume_one();
                    }

                    // Create Frame with ownership transfer - no additional copy (bd-ek9n.5)
                    let frame = Frame::from_bytes(width, height, 16, pixel_data);
                    frame_count.fetch_add(1, Ordering::Relaxed);
                    let frame_arc = Arc::new(frame);

                    // Deliver to channels
                    if let Some(ref tx) = reliable_tx {
                        let _ = tx.blocking_send(frame_arc.clone());
                    }
                    let _ = frame_tx.send(frame_arc.clone());

                    // Arrow tap optimization (bd-ek9n.5)
                    #[cfg(feature = "arrow_tap")]
                    if let Some(ref tap) = arrow_tap {
                        use arrow::array::{PrimitiveArray, UInt16Type};
                        use arrow::buffer::Buffer;
                        let buffer = Buffer::from(frame_arc.data().to_vec());
                        let arr = Arc::new(PrimitiveArray::<UInt16Type>::new(Arc::new(buffer), None));
                        let _ = tap.blocking_send(arr);
                    }
                }
            }

            // Fix for pending_frames getting stuck (medium priority issue):
            // If pending_frames counter is out of sync with actual frames available,
            // reset it to 0 after drain loop to avoid getting stuck in infinite wait.
            // This can happen if PVCAM drops frames internally without calling callback.
            if use_callback {
                let remaining = callback_ctx.pending_frames.load(Ordering::Acquire);
                if remaining > 0 && frames_processed_in_drain == 0 {
                    // Callback said frames were ready, but we couldn't retrieve any.
                    // This indicates a desync - reset counter to avoid getting stuck.
                    tracing::warn!(
                        "pending_frames desync: {} pending but 0 retrieved, resetting counter",
                        remaining
                    );
                    callback_ctx.pending_frames.store(0, Ordering::Release);
                }
            }
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

            // Also abort the handle to clean up the JoinHandle.
            // Note: For spawn_blocking, abort() doesn't kill the thread - it just
            // marks the task as cancelled. The thread will exit on its next
            // check of the shutdown flag or callback wait timeout.
            if let Ok(mut guard) = self.poll_handle.try_lock() {
                if let Some(handle) = guard.take() {
                    handle.abort();
                    tracing::debug!("Aborted PVCAM frame loop handle in Drop");
                }
            } else {
                tracing::warn!("Could not acquire poll_handle lock in Drop - frame loop may outlive connection");
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
