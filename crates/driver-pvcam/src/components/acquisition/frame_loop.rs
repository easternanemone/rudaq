//! Frame acquisition loops for PVCAM hardware and sequence modes.

#[cfg(feature = "pvcam_sdk")]
use super::callback_context::SEQUENCE_BATCH_SIZE;
#[cfg(feature = "pvcam_sdk")]
use super::ffi_safe;
#[cfg(feature = "pvcam_sdk")]
use super::AcquisitionError;
use super::PvcamAcquisition;
#[cfg(feature = "pvcam_sdk")]
use super::TapRegistry;
#[cfg(feature = "pvcam_sdk")]
use super::{get_pvcam_error, CallbackContext, FrameMetadata};
#[cfg(feature = "pvcam_sdk")]
use crate::components::features::PvcamFeatures;
#[cfg(feature = "pvcam_sdk")]
use bytes::Bytes;
#[cfg(feature = "pvcam_sdk")]
use common::data::Frame;
#[cfg(feature = "pvcam_sdk")]
use common::parameter::Parameter;
#[cfg(feature = "pvcam_sdk")]
use pool::buffer_pool::BufferPool;
#[cfg(feature = "pvcam_sdk")]
use pool::{FrameData, Pool};
#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::AtomicU64;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::{self, Ordering};
#[cfg(feature = "pvcam_sdk")]
use std::sync::Arc;
#[cfg(feature = "pvcam_sdk")]
use std::time::Duration;

impl PvcamAcquisition {
    /// bd-3gnv: Sequence mode frame loop (blocking).
    ///
    /// Repeatedly acquires batches of frames using pl_exp_setup_seq/start_seq,
    /// polls for completion, and sends frames to channels.
    #[cfg(feature = "pvcam_sdk")]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn frame_loop_sequence(
        hcam: i16,
        region: rgn_type,
        exposure_ms: f64,
        frame_bytes: usize,
        streaming: Parameter<bool>,
        shutdown: Arc<atomic::AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        frame_count: Arc<atomic::AtomicU64>,
        _lost_frames: Arc<atomic::AtomicU64>,
        width: u32,
        height: u32,
        roi_x: u32,
        roi_y: u32,
        binning: (u16, u16),
        done_tx: std::sync::mpsc::Sender<()>,
        tap_registry: Arc<TapRegistry>, // bd-0dax.4: For synchronous tap observers
        host_summing_enabled: Parameter<bool>, // bd-oqo7.7
        host_summing_count: Parameter<u32>, // bd-oqo7.7
        smart_stream_count: usize,      // bd-oqo7.1: SMART Streaming exposure cycle length
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
                        // bd-oqo7.7: Include summing_count in frame metadata
                        let summing_count = if host_summing_enabled.get() {
                            let count = host_summing_count.get();
                            if count > 1 {
                                Some(count)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        let extra = if smart_stream_count > 0 {
                            let exposure_index = ((total_frames - 1) as usize) % smart_stream_count;
                            let mut m = std::collections::HashMap::with_capacity(2);
                            m.insert("smart_stream_index".into(), exposure_index.to_string());
                            m.insert("smart_stream_count".into(), smart_stream_count.to_string());
                            m
                        } else {
                            std::collections::HashMap::new()
                        };
                        let ext_metadata = common::data::FrameMetadata {
                            binning: Some(binning),
                            summing_count,
                            extra,
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
    /// * `smart_stream_count` - Number of SMART Streaming exposures in the cycle (0 = disabled, bd-oqo7.1).
    ///                         When > 0, each frame's extended metadata includes `smart_stream_index`
    ///                         and `smart_stream_count` for downstream HDR merging.
    #[cfg(feature = "pvcam_sdk")]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn frame_loop_hardware(
        hcam: i16,
        streaming: Parameter<bool>,
        shutdown: Arc<atomic::AtomicBool>,
        frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
        reliable_tx: Option<tokio::sync::mpsc::Sender<Arc<Frame>>>,
        frame_count: Arc<atomic::AtomicU64>,
        lost_frames: Arc<atomic::AtomicU64>,
        discontinuity_events: Arc<atomic::AtomicU64>,
        dropped_frames: Arc<atomic::AtomicU64>,
        last_hw_frame_nr: Arc<atomic::AtomicI32>,
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
        host_summing_enabled: Parameter<bool>, // bd-oqo7.7
        host_summing_count: Parameter<u32>, // bd-oqo7.7
        smart_stream_count: usize, // bd-oqo7.1: SMART Streaming exposure cycle length
        use_prime_locate: bool,  // bd-ldjy.4: PrimeLocate emits event records, not pixels
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
            metadata = use_metadata,
            prime_locate = use_prime_locate
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

        // bd-a9nr: Removed auto-stop-on-no-subscribers (was bd-cckz).
        // The caller controls acquisition lifetime via streaming Parameter and stop_stream().
        // Auto-stopping caused race conditions with gRPC tap observer registration.

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
                            // TODO(bd-wev5): Pass roi_data to downstream consumers
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
                // bd-oqo7.7: Include summing_count in frame metadata
                let summing_count = if host_summing_enabled.get() {
                    let count = host_summing_count.get();
                    if count > 1 {
                        Some(count)
                    } else {
                        None
                    }
                } else {
                    None
                };
                // bd-oqo7.1: Tag frames with SMART Streaming exposure index
                // bd-oqo7.2: Bridge hardware metadata fields to extra map
                let extra = {
                    let mut m = std::collections::HashMap::with_capacity(9);
                    // SMART Streaming exposure tagging (bd-oqo7.1)
                    if smart_stream_count > 0 {
                        let exposure_index =
                            ((monotonic_frame_count - 1) as usize) % smart_stream_count;
                        m.insert("smart_stream_index".into(), exposure_index.to_string());
                        m.insert("smart_stream_count".into(), smart_stream_count.to_string());
                    }
                    // Hardware metadata fields (bd-oqo7.2)
                    if let Some(ref md) = frame_metadata {
                        m.insert("hw_frame_nr".into(), md.frame_nr.to_string());
                        m.insert("timestamp_bof_ns".into(), md.timestamp_bof_ns.to_string());
                        m.insert("timestamp_eof_ns".into(), md.timestamp_eof_ns.to_string());
                        m.insert("exposure_time_ns".into(), md.exposure_time_ns.to_string());
                        m.insert("bit_depth".into(), md.bit_depth.to_string());
                        m.insert("roi_count".into(), md.roi_count.to_string());
                        // Derived: readout time = EOF - BOF - exposure (bd-oqo7.2)
                        if md.timestamp_eof_ns > md.timestamp_bof_ns + md.exposure_time_ns {
                            let readout_ns =
                                md.timestamp_eof_ns - md.timestamp_bof_ns - md.exposure_time_ns;
                            m.insert("readout_time_ns".into(), readout_ns.to_string());
                        }
                    }
                    if use_prime_locate {
                        m.insert("prime_locate_enabled".into(), "true".into());
                        let events = PvcamFeatures::parse_localization_events(&frame.data);
                        m.insert("localization_event_count".into(), events.len().to_string());
                        if !events.is_empty() {
                            if let Ok(json) = serde_json::to_string(&events) {
                                m.insert("localization_events_json".into(), json);
                            }
                        }
                    }
                    m
                };
                let ext_metadata = common::data::FrameMetadata {
                    binning: Some(binning),
                    summing_count,
                    extra,
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

                // bd-a9nr: Log consumer count periodically but never auto-stop.
                // Acquisition lifetime is controlled by streaming Parameter + stop_stream().
                if !has_consumers && monotonic_frame_count % 100 == 1 {
                    tracing::debug!(
                        "Frame {}: no active consumers (broadcast={}, observers={}) — streaming continues until stop_stream()",
                        current_frame_nr,
                        receiver_count,
                        tap_registry.tap_count()
                    );
                } else if has_consumers && current_frame_nr % 30 == 1 {
                    tracing::debug!(
                        "Sending frame {} to {} broadcast subscribers",
                        current_frame_nr,
                        receiver_count
                    );
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
