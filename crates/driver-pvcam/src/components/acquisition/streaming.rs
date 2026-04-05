//! Stream start/stop and mock streaming for PVCAM acquisition.

#[cfg(feature = "pvcam_sdk")]
use super::AcquisitionError;
use super::PvcamAcquisition;
use super::PvcamConnection;
use super::StreamConfig;
#[cfg(feature = "pvcam_sdk")]
use super::callback_context::SEQUENCE_BATCH_SIZE;
#[cfg(feature = "pvcam_sdk")]
use super::{PvcamFeatures, get_pvcam_error};
use crate::components::features::BufferMode;
use anyhow::{Result, anyhow, bail};
use common::core::Roi;
use common::data::Frame;
use common::parameter::Parameter;
use pool::{FrameData, Pool};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::MutexGuard;

#[cfg(feature = "pvcam_sdk")]
use super::buffer::PageAlignedBuffer;
#[cfg(feature = "pvcam_sdk")]
use super::ffi_safe;
#[cfg(feature = "pvcam_sdk")]
use super::{
    CallbackContext, clear_global_callback_ctx, pvcam_eof_callback, set_global_callback_ctx,
};
#[cfg(feature = "pvcam_sdk")]
use pool::buffer_pool::BufferPool;
#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;

impl PvcamAcquisition {
    /// Start streaming frames
    ///
    /// # Frame Loss Detection (bd-ek9n.3)
    ///
    /// Resets frame loss metrics at the start of each acquisition. During streaming,
    /// the poll loop tracks hardware frame numbers to detect and count dropped frames.
    pub async fn start_stream(&self, conn: &PvcamConnection, config: StreamConfig) -> Result<()> {
        tracing::info!(
            "start_stream: roi=({},{} {}x{}), binning=({},{}), exposure={:.1}ms, mode={}",
            config.roi.x,
            config.roi.y,
            config.roi.width,
            config.roi.height,
            config.binning.0,
            config.binning.1,
            config.exposure_ms,
            config.buffer_mode
        );

        // Destructure config for local use
        let StreamConfig {
            roi,
            binning,
            exposure_ms,
            buffer_mode,
            host_summing_enabled,
            host_summing_count,
            smart_stream_enabled,
            smart_stream_exposures,
            prime_locate_enabled,
            prime_enhance_enabled,
            multi_roi_regions,
        } = config;

        // Avoid unused parameter warnings when hardware feature is disabled.
        let _ = conn;
        let _ = &buffer_mode;
        let _ = &host_summing_enabled;
        let _ = &host_summing_count;
        let _ = &smart_stream_enabled;
        let _ = &smart_stream_exposures;
        let _ = &prime_locate_enabled;
        let _ = &prime_enhance_enabled;
        let _ = &multi_roi_regions;
        if self.streaming.get() {
            tracing::warn!("start_stream: already streaming");
            bail!("Already streaming");
        }

        tracing::debug!("Setting streaming=true, resetting frame counters");
        self.streaming.set_from_hardware(true).await?;
        self.frame_count.store(0, Ordering::SeqCst);
        // Reset frame loss metrics for this acquisition (bd-ek9n.3)
        self.reset_frame_loss_metrics();
        // bd-9id0: Clear sticky error state so has_acquisition_error() is session-scoped.
        self.clear_error();

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

            // Auto-enable metadata for all acquisitions (bd-oqo7.2).
            // Hardware timestamps are essential for data provenance and timing.
            // The metadata_enabled atomic controls whether decoded metadata is sent
            // through the metadata channel, but SDK-level PARAM_METADATA_ENABLED
            // should always be true when the camera supports it.
            let current_metadata = PvcamFeatures::is_metadata_enabled(conn).unwrap_or(false);
            if !current_metadata {
                tracing::info!(
                    "Auto-enabling PVCAM metadata for hardware timestamp decoding (bd-oqo7.2)"
                );
                if let Err(e) = PvcamFeatures::set_metadata_enabled(conn, true) {
                    tracing::error!(
                        "Failed to enable metadata: {}. Hardware timestamps unavailable",
                        e
                    );
                }
            }
            // Always decode metadata in the frame loop for Frame.timestamp accuracy.
            // Note: the driver's `processing.metadata_enabled` Parameter defaults to true
            // and is synced at creation time. If a client sets it to false, the write
            // callback toggles this atomic, which is overridden here. This is intentional:
            // disabling metadata mid-acquisition risks data corruption.
            self.metadata_enabled.store(true, Ordering::Release);
            let use_metadata = true;

            // bd-oqo7.3: Re-apply PrimeEnhance state before acquisition setup.
            // The write callback sets PP state immediately on Parameter change,
            // but camera power-cycles reset PP features. Re-apply on each start_stream
            // to ensure the configured state is always active.
            let use_prime_enhance = prime_enhance_enabled.get();
            if use_prime_enhance {
                tracing::info!("Re-applying PrimeEnhance enabled state for acquisition");
                if let Err(e) = PvcamFeatures::set_prime_enhance(conn, true) {
                    tracing::warn!(
                        "Failed to re-apply PrimeEnhance: {e}. Continuing without denoising."
                    );
                }
            }

            // bd-oqo7.1: Configure SMART Streaming before acquisition setup.
            // SMART Streaming lets the FPGA cycle through pre-programmed exposures,
            // eliminating host communication latency between exposure changes.
            let use_smart_stream = smart_stream_enabled.get();
            let use_prime_locate = prime_locate_enabled.get();
            let smart_exposures: Vec<u32> = if use_smart_stream {
                let json_str = smart_stream_exposures.get();
                match serde_json::from_str(&json_str) {
                    Ok(exposures) => exposures,
                    Err(e) => {
                        tracing::error!(
                            "Failed to parse SMART stream exposures JSON '{}': {}",
                            json_str,
                            e
                        );
                        vec![]
                    }
                }
            } else {
                vec![]
            };
            let smart_stream_count: usize = if use_smart_stream && !smart_exposures.is_empty() {
                tracing::info!(
                    "SMART Streaming enabled: {} exposures {:?}ms",
                    smart_exposures.len(),
                    smart_exposures
                );
                PvcamFeatures::set_smart_stream_enabled(conn, true)?;
                if let Err(err) = PvcamFeatures::upload_smart_stream(conn, &smart_exposures) {
                    // Rollback: disable SMART streaming to keep camera in known state
                    if let Err(disable_err) = PvcamFeatures::set_smart_stream_enabled(conn, false) {
                        tracing::warn!(
                            "Failed to disable SMART Streaming after upload error: {}",
                            disable_err
                        );
                    }
                    return Err(err.into());
                }
                smart_exposures.len()
            } else if use_smart_stream {
                tracing::warn!(
                    "SMART streaming enabled but no exposures configured - using standard mode"
                );
                0
            } else {
                0
            };

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

            // bd-9pel: Use sequence mode when buffer_mode is Sequence (runtime configurable).
            // Sequence mode uses pl_exp_setup_seq/start_seq for batch-based non-circular
            // acquisition. Useful for single-frame capture workflows or diagnostics.
            if buffer_mode == BufferMode::Sequence {
                return self
                    .start_stream_sequence_impl(
                        h,
                        region,
                        exposure_ms,
                        binning,
                        roi,
                        reliable_tx,
                        use_metadata,
                        host_summing_enabled,
                        host_summing_count,
                        smart_stream_count, // bd-oqo7.1
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
            // The current unlock-before-copy pattern is safe ONLY in CIRC_NO_OVERWRITE
            // mode: the SDK won't reuse a buffer slot until all 20 slots are filled,
            // so frame_ptr remains valid for the copy that follows. Restructuring
            // to copy-then-unlock would allow re-enabling CIRC_OVERWRITE for ~10-15%
            // higher throughput, but risks subtle regressions in the frame pipeline.
            // Deferring until profiling shows CIRC_NO_OVERWRITE is the bottleneck.
            let mut circ_overwrite = false;
            if buffer_mode == BufferMode::Overwrite {
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

            // bd-oqo7.4: Build region array for multi-ROI when configured.
            // Uses validated RoiRegion → rgn_type conversion. Falls back to single ROI
            // when multi_roi_regions is empty.
            let use_multi_roi = !multi_roi_regions.is_empty();
            let multi_regions: Vec<rgn_type> = if use_multi_roi {
                tracing::info!(
                    roi_count = multi_roi_regions.len(),
                    "Multi-ROI acquisition: building {} regions",
                    multi_roi_regions.len()
                );
                let mut regions = Vec::with_capacity(multi_roi_regions.len());
                for roi_region in &multi_roi_regions {
                    let rgn = unsafe {
                        let mut r: rgn_type = std::mem::zeroed();
                        r.s1 = roi_region.x;
                        r.s2 = roi_region.x + roi_region.w - 1;
                        r.sbin = x_bin;
                        r.p1 = roi_region.y;
                        r.p2 = roi_region.y + roi_region.h - 1;
                        r.pbin = y_bin;
                        r
                    };
                    regions.push(rgn);
                }
                regions
            } else {
                vec![region]
            };

            // SAFETY: `h` is a valid camera handle. `region` is a stack-allocated
            // rgn_type. `frame_bytes` is a stack-allocated uns32 output parameter.
            // `selected_buffer_mode` is a valid CIRC_* constant. No acquisition
            // is active (we are in setup). On failure, we retry with NO_OVERWRITE.
            unsafe {
                let rgn_count = multi_regions.len() as u16;
                let rgn_ptr = multi_regions.as_ptr() as *const _;
                // Try overwrite first
                if pl_exp_setup_cont(
                    h,
                    rgn_count,
                    rgn_ptr,
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
                        rgn_count,
                        rgn_ptr,
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
                tracing::debug!(
                    calculated = buffer_count,
                    forced,
                    "Buffer count overridden by PVCAM_SMOKE_TEST"
                );
                buffer_count = forced;
            }
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

            // CRITICAL: circ_buf MUST remain allocated for the entire acquisition lifetime.
            // It will be stored in SdkStreamingState::Streaming below.
            // DO NOT convert or transform - PVCAM holds a raw pointer to this memory.

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

            // Gemini SDK review: Create error channel for involuntary stop signaling.
            // Fatal errors (READOUT_FAILED, etc.) are sent from frame loop to update streaming state.
            // Uses tokio unbounded_channel: send() is non-blocking (safe from sync code),
            // recv() is async-native (no polling needed in watcher task).
            let (error_tx, mut error_rx) =
                tokio::sync::mpsc::unbounded_channel::<AcquisitionError>();

            // Clone streaming parameter for error watcher task
            let streaming_for_watcher = self.streaming.clone();

            // Clone last_error for error watcher task (bd-g9po)
            let last_error_for_watcher = self.last_error.clone();

            // Clone tap_registry for error watcher (bd-9id0): clear observers on fatal error
            // so gRPC streaming tasks detect channel close and report failure to supervisor.
            let tap_registry_for_watcher = self.tap_registry.clone();

            // Capture ROI and binning for frame metadata (bd-183h)
            let roi_x = roi.x;
            let roi_y = roi.y;

            // bd-0dax.4: Clone tap registry for frame observers
            let tap_registry = self.tap_registry.clone();

            // bd-oqo7.7: Clone summing parameters for frame loop
            let host_summing_enabled = host_summing_enabled.clone();
            let host_summing_count = host_summing_count.clone();

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
            // Stored in SdkStreamingState::Streaming below.
            let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

            // bd-3gnv: circ_ptr_usize was converted from raw pointer at line 1110,
            // BEFORE any await points. We use it here for cross-thread transfer.

            // Clone senders and pool BEFORE the closure captures (moves) the originals.
            // The clones are stored in SdkStreamingState::Streaming below.
            let error_tx_state = error_tx.clone();
            let done_tx_state = done_tx.clone();
            let buffer_pool_state = buffer_pool.clone();

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
                    host_summing_enabled, // bd-oqo7.7
                    host_summing_count, // bd-oqo7.7
                    smart_stream_count, // bd-oqo7.1: SMART Streaming exposure cycle length
                    use_prime_locate,   // bd-ldjy.4: PrimeLocate event-buffer metadata bridge
                );
            });

            // Transition sdk_state: Idle -> Streaming with all resources.
            // This single lock acquisition replaces 6 separate Arc<Mutex<Option<T>>> stores.
            {
                use super::SdkStreamingState;
                let mut state = self.sdk_state.lock().await;
                *state = SdkStreamingState::Streaming {
                    poll_handle,
                    circ_buffer: circ_buf,
                    error_tx: error_tx_state,
                    frame_pool: buffer_pool_state,
                    poll_thread_done_rx: done_rx,
                    poll_thread_done_tx: done_tx_state,
                };
            }

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

                    // bd-9id0: Drop all tap observers so gRPC streaming tasks detect
                    // channel close via recv() → None and report failure to supervisor.
                    tap_registry_for_watcher.clear_all();

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
        host_summing_enabled: Parameter<bool>, // bd-oqo7.7
        host_summing_count: Parameter<u32>,    // bd-oqo7.7
    ) -> Result<Frame> {
        let mut rx = self.frame_tx.subscribe();
        // Single-frame acquisition doesn't use SMART streaming; pass disabled defaults.
        let smart_disabled = Parameter::new("_single_frame_smart_disabled", false);
        let smart_empty = Parameter::new("_single_frame_smart_exposures", "[]".to_string());
        #[cfg(feature = "pvcam_sdk")]
        let prime_locate_enabled = Parameter::new(
            "_single_frame_prime_locate",
            PvcamFeatures::get_prime_locate_enabled(conn).unwrap_or(false),
        );
        #[cfg(not(feature = "pvcam_sdk"))]
        let prime_locate_enabled = Parameter::new("_single_frame_prime_locate", false);
        let config = StreamConfig {
            roi,
            binning,
            exposure_ms,
            buffer_mode: BufferMode::from_str(&self.buffer_mode.get()),
            host_summing_enabled,
            host_summing_count,
            smart_stream_enabled: smart_disabled,
            smart_stream_exposures: smart_empty,
            prime_locate_enabled,
            prime_enhance_enabled: {
                #[cfg(feature = "pvcam_sdk")]
                {
                    Parameter::new(
                        "_single_frame_prime_enhance",
                        PvcamFeatures::get_prime_enhance_enabled(conn).unwrap_or(false),
                    )
                }
                #[cfg(not(feature = "pvcam_sdk"))]
                {
                    Parameter::new("_single_frame_prime_enhance", false)
                }
            },
            multi_roi_regions: vec![], // Single-frame always uses single ROI
        };
        self.start_stream(conn, config).await?;

        let frame = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .map_err(|_| anyhow!("Timed out waiting for frame"))?
            .map_err(|e| anyhow!("Frame channel closed: {e}"))?;

        let _ = self.stop_stream(conn).await;
        Ok((*frame).clone())
    }

    pub(super) async fn start_mock_stream(
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
            let binned_width = roi.width / u32::from(x_bin);
            let binned_height = roi.height / u32::from(y_bin);
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
            let binned_width = roi.width / u32::from(x_bin);
            let binned_height = roi.height / u32::from(y_bin);
            let frame_size = (binned_width * binned_height) as usize;

            while streaming.get() {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                // SAFETY: Exposure time is always positive and within millisecond range.
                tokio::time::sleep(Duration::from_millis(exposure_ms as u64)).await;
                if !streaming.get() {
                    break;
                }

                let frame_num = frame_count.fetch_add(1, Ordering::SeqCst);
                let mut pixels = vec![0u16; frame_size];
                for y in 0..binned_height {
                    for x in 0..binned_width {
                        #[allow(clippy::cast_possible_truncation)]
                        // SAFETY: Modulo 4096 guarantees value fits in u16.
                        let value =
                            (((x + y + frame_num as u32) % 4096) as u16).saturating_add(100);
                        pixels[(y * binned_width + x) as usize] = value;
                    }
                }

                // bd-5oss: Send through primary_tx if registered (pooled path)
                if let (Some(p_tx), Some(pool)) = (&primary_tx, &frame_pool) {
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
                        #[allow(clippy::cast_possible_truncation)]
                        // SAFETY: Nanosecond timestamps won't exceed u64 until year ~2554
                        {
                            frame_data.timestamp_ns = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0);
                        }

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

            // Transition sdk_state: Streaming -> Idle, extracting resources into
            // locals. CRITICAL SAFETY: The circ_buffer MUST stay alive until AFTER
            // FFI cleanup (pl_exp_stop_cont + callback deregistration) because the
            // PVCAM SDK holds raw pointers into the buffer during acquisition.
            tracing::debug!("PVCAM stop_stream: transitioning sdk_state to Idle");
            let streaming_resources = {
                use super::SdkStreamingState;
                let mut state = self.sdk_state.lock().await;
                let old_state = std::mem::replace(&mut *state, SdkStreamingState::Idle);
                match old_state {
                    SdkStreamingState::Streaming {
                        poll_handle,
                        circ_buffer,
                        ..
                    } => Some((poll_handle, circ_buffer)),
                    SdkStreamingState::Idle => {
                        tracing::debug!("PVCAM stop_stream: sdk_state already Idle");
                        None
                    }
                }
            };

            // bd-hehw: Await poll handle outside the lock to avoid holding mutex across .await
            tracing::debug!("PVCAM stop_stream: waiting for poll thread to complete");
            // Extract poll_handle; keep _circ_buffer alive in scope until after FFI cleanup.
            let _circ_buffer_guard = if let Some((handle, circ_buffer)) = streaming_resources {
                tracing::debug!("PVCAM stop_stream: awaiting poll handle");
                let _ = handle.await;
                tracing::debug!("PVCAM stop_stream: poll handle completed");
                Some(circ_buffer)
            } else {
                tracing::debug!("PVCAM stop_stream: no poll handle to wait for");
                None
            };

            // SAFETY: circ_buffer is kept alive by _circ_buffer_guard above.
            // FFI cleanup (stop_acquisition, deregister_callback) runs here while
            // the buffer is still valid. The buffer drops at the end of this block
            // when _circ_buffer_guard goes out of scope.
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
                // bd-oqo7.1: Disable SMART streaming on stop to return camera to normal mode.
                // Best-effort: we don't fail the stop if this doesn't succeed.
                if let Err(e) = PvcamFeatures::set_smart_stream_enabled(conn, false) {
                    tracing::debug!("SMART streaming disable on stop (non-fatal): {}", e);
                }
            } else {
                tracing::debug!("PVCAM stop_stream: no camera handle, skipping SDK cleanup");
            }
            // Clear remaining atomic state
            tracing::debug!("PVCAM stop_stream: clearing atomic state");
            self.active_hcam.store(-1, Ordering::Release); // -1 = no active handle
            tracing::debug!("PVCAM stop_stream: cleanup complete");
            // _circ_buffer_guard drops here — safe because FFI cleanup is complete.
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
        host_summing_enabled: Parameter<bool>, // bd-oqo7.7
        host_summing_count: Parameter<u32>,    // bd-oqo7.7
        smart_stream_count: usize,             // bd-oqo7.1
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

        // Create a dummy error_tx for state machine (sequence mode doesn't use involuntary stop signaling,
        // but we need a sender to satisfy the SdkStreamingState::Streaming variant).
        let (seq_error_tx, _seq_error_rx) =
            tokio::sync::mpsc::unbounded_channel::<AcquisitionError>();

        // Create a dummy buffer pool (sequence mode allocates per-batch, not from a pool).
        let dummy_pool = BufferPool::new(1, 1);

        // Clone done_tx before the closure captures (moves) the original.
        let done_tx_state = done_tx.clone();

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
                tap_registry,         // bd-0dax.4: For tap observers
                host_summing_enabled, // bd-oqo7.7
                host_summing_count,   // bd-oqo7.7
                smart_stream_count,   // bd-oqo7.1
            );
        });

        // Transition sdk_state: Idle -> Streaming.
        // Sequence mode doesn't use circ_buffer, but we need a PageAlignedBuffer
        // to satisfy the Streaming variant. Use a minimal 4KB (one page) buffer
        // since PVCAM doesn't DMA in sequence mode.
        let seq_circ_buffer = PageAlignedBuffer::new(4096)?;
        {
            use super::SdkStreamingState;
            let mut state = self.sdk_state.lock().await;
            *state = SdkStreamingState::Streaming {
                poll_handle,
                circ_buffer: seq_circ_buffer,
                error_tx: seq_error_tx,
                frame_pool: dummy_pool,
                poll_thread_done_rx: done_rx,
                poll_thread_done_tx: done_tx_state,
            };
        }
        Ok(())
    }
}
