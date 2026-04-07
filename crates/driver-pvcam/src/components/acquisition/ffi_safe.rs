use super::*;

/// Stop continuous acquisition on a camera.
///
/// # Safety Contract
/// - `hcam` must be a valid, open camera handle
/// - Acquisition must have been started with `pl_exp_start_cont`
/// - Must be called before closing the camera
pub fn stop_acquisition(hcam: i16, mode: i16) {
    debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
    tracing::debug!(
        "ffi_safe::stop_acquisition called: hcam={}, mode={} (CCS_HALT={})",
        hcam,
        mode,
        CCS_HALT
    );
    // SAFETY: Caller guarantees hcam is valid and acquisition is active
    unsafe {
        pl_exp_stop_cont(hcam, mode);
    }
    tracing::debug!("ffi_safe::stop_acquisition completed");
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
    if hcam < 0 {
        return Err(format!("Invalid camera handle: {hcam}"));
    }
    if circ_ptr.is_null() {
        return Err("Circular buffer pointer is null".to_string());
    }
    if circ_size_bytes == 0 {
        return Err("Circular buffer size must be > 0".to_string());
    }

    // SAFETY: Caller guarantees hcam is valid, circ_ptr is valid page-aligned buffer
    let result = unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size_bytes) };
    if result == 0 {
        let err_msg = get_pvcam_error();
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
/// - `circ_overwrite`: Whether to configure CIRC_OVERWRITE (falls back to NO_OVERWRITE on failure)
///
/// # Returns
/// `Ok(frame_bytes)` on success, `Err(String)` on failure
#[expect(
    clippy::too_many_arguments,
    reason = "PVCAM SDK acquisition setup requires all these parameters for ROI, binning, exposure, and buffer config"
)]
pub(crate) fn full_restart_acquisition(
    hcam: i16,
    roi_x: u32,
    roi_y: u32,
    width: u32,
    height: u32,
    binning: (u16, u16),
    exposure_ms: f64,
    circ_ptr: *mut u8,
    circ_size_bytes: u32,
    circ_overwrite: bool,
) -> Result<uns32, String> {
    if hcam < 0 {
        return Err(format!("Invalid camera handle: {hcam}"));
    }
    if circ_ptr.is_null() {
        return Err("Circular buffer pointer is null".to_string());
    }
    if circ_size_bytes == 0 {
        return Err("Circular buffer size must be > 0".to_string());
    }

    let (x_bin, y_bin) = binning;

    // Setup region (same as initial setup)
    // SAFETY: rgn_type is a POD C struct with only primitive uns16 fields.
    // Zero-initialization followed by explicit assignment of all fields is safe.
    // See start_stream() for detailed safety justification.
    // Precondition: ROI values were validated by start_stream (bd-8zcu) before
    // reaching this function — width/height > 0 and endpoints fit in uns16.
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
    let buffer_mode = if circ_overwrite {
        CIRC_OVERWRITE
    } else {
        CIRC_NO_OVERWRITE
    };
    let mut frame_bytes: uns32 = 0;

    // Probe PARAM_CIRC_BUFFER for visibility only; do not override user choice unless setup fails.
    if circ_overwrite {
        // SAFETY: `hcam` is a valid camera handle (guaranteed by caller).
        // All output parameters are stack-allocated with correct types:
        // rs_bool for ATTR_AVAIL, uns32 for ATTR_MIN/ATTR_MAX.
        // These are read-only queries with no side effects.
        unsafe {
            let mut circ_avail: rs_bool = 0;
            if pl_get_param(
                hcam,
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
                    hcam,
                    PARAM_CIRC_BUFFER,
                    ATTR_MIN as i16,
                    &mut circ_min as *mut _ as *mut std::ffi::c_void,
                ) != 0
                    && pl_get_param(
                        hcam,
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
    let circ_overwrite = buffer_mode == CIRC_OVERWRITE;
    let mut selected_buffer_mode = if circ_overwrite {
        CIRC_OVERWRITE
    } else {
        CIRC_NO_OVERWRITE
    };

    // Step 1: pl_exp_setup_cont (try overwrite, then fall back)
    // SAFETY: `hcam` is a valid camera handle. `region` points to a valid
    // stack-allocated rgn_type. `frame_bytes` is a stack-allocated uns32
    // output parameter. The SDK writes the required buffer size into it.
    // `selected_buffer_mode` is a valid CIRC_* constant per PVCAM SDK docs.
    let setup_overwrite = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms as uns32,
            &mut frame_bytes,
            selected_buffer_mode,
        )
    };
    if setup_overwrite == 0 {
        let err_msg = get_pvcam_error();
        tracing::warn!(
            "pl_exp_setup_cont failed (overwrite): {}, retrying with NO_OVERWRITE",
            err_msg
        );
        // Retry with no-overwrite
        selected_buffer_mode = CIRC_NO_OVERWRITE;
        frame_bytes = 0;
        // SAFETY: Same as above — valid handle, region, and output pointer.
        // CIRC_NO_OVERWRITE is the fallback mode for cameras that don't
        // support overwrite (e.g., Prime BSI returns error 185).
        if unsafe {
            pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                exp_mode,
                exposure_ms as uns32,
                &mut frame_bytes,
                selected_buffer_mode,
            )
        } == 0
        {
            let err_msg = get_pvcam_error();
            return Err(format!(
                "pl_exp_setup_cont failed (both modes): {}",
                err_msg
            ));
        }
    }

    // Step 2: pl_exp_start_cont
    // SAFETY: `hcam` is a valid camera handle. `circ_ptr` points to a
    // caller-owned buffer with `circ_size_bytes` capacity, sized by
    // the preceding pl_exp_setup_cont output. The buffer remains valid
    // for the lifetime of the acquisition.
    let start_result = unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size_bytes) };
    if start_result == 0 {
        let err_msg = get_pvcam_error();
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
    let result =
        unsafe { pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) };

    if result == 0 {
        // SAFETY: pl_error_code() is thread-safe and stateless per SDK docs.
        let err_code = unsafe { pl_error_code() };
        let err_msg = get_pvcam_error();
        tracing::debug!(
            "ffi_safe::check_cont_status FAILED: hcam={}, err_code={}, err_msg={}",
            hcam,
            err_code,
            err_msg
        );
        Err(())
    } else {
        tracing::trace!(
            "ffi_safe::check_cont_status: hcam={}, status={}, bytes_arrived={}, buffer_cnt={}",
            hcam,
            status,
            bytes_arrived,
            buffer_cnt
        );
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
///
/// bd-fix-2026-01-17: Reverted to pl_exp_get_oldest_frame_ex to get correct
/// FrameNr for each frame. The non-ex version relied on callback_ctx.latest_frame_nr
/// which causes false "Duplicate Frame" detection when draining a backlog
/// (all backlog frames appear to have the latest callback's FrameNr).
pub fn get_oldest_frame(
    hcam: i16,
    frame_info: &mut FRAME_INFO,
) -> Result<*mut std::ffi::c_void, ()> {
    debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
    let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

    // SAFETY: hcam is valid, frame_ptr is a valid stack allocation, frame_info is valid
    let result = unsafe { pl_exp_get_oldest_frame_ex(hcam, &mut frame_ptr, frame_info) };

    if result == 0 || frame_ptr.is_null() {
        // bd-3gnv: Log error code to diagnose why get_oldest_frame is failing
        // SAFETY: pl_error_code() is thread-safe and stateless per SDK docs.
        let err_code = unsafe { pl_error_code() };
        // Filter out legitimate "no frame" error (3025 = READOUT_FAILED? No, usually 0 is generic fail)
        // But for get_oldest_frame, failure usually means no frame ready.
        // Only log if it's NOT just empty buffer
        if err_code != 0 {
            let err_msg = get_pvcam_error();
            tracing::debug!(
                "ffi_safe::get_oldest_frame_ex FAILED: hcam={}, result={}, err_code={}, err_msg={}, frame_ptr_null={}",
                hcam,
                result,
                err_code,
                err_msg,
                frame_ptr.is_null()
            );
        }
        Err(())
    } else {
        tracing::trace!(
            "ffi_safe::get_oldest_frame_ex succeeded: hcam={}, frame_ptr={:?}, nr={}",
            hcam,
            frame_ptr,
            frame_info.FrameNr
        );
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
    tracing::trace!("ffi_safe::release_oldest_frame called: hcam={}", hcam);
    // SAFETY: Caller guarantees hcam is valid and a frame was retrieved
    // bd-3gnv: Check return value - silent unlock failures would stall CIRC_NO_OVERWRITE mode
    let result = unsafe { pl_exp_unlock_oldest_frame(hcam) };
    if result == 0 {
        // Unlock failed - this is critical for continuous acquisition
        // SAFETY: pl_error_code() is thread-safe and stateless per SDK docs.
        let err_code = unsafe { pl_error_code() };
        let err_msg = get_pvcam_error();
        tracing::error!(
            "ffi_safe::release_oldest_frame FAILED: hcam={}, err_code={}, err_msg={} (bd-3gnv)",
            hcam,
            err_code,
            err_msg
        );
        false
    } else {
        tracing::trace!("ffi_safe::release_oldest_frame succeeded: hcam={}", hcam);
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
    if roi_count == 0 {
        tracing::error!("create_md_frame called with roi_count=0");
        return None;
    }
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

/// RAII guard for `md_frame` pointer (bd-u602).
///
/// Ensures the md_frame struct is released even if the acquisition loop exits
/// early via `?`, `return Err(...)`, or panic. Replaces manual
/// `create_md_frame`/`release_md_frame` pairing with deterministic Drop cleanup.
pub struct MdFrameGuard {
    ptr: *mut md_frame,
}

impl MdFrameGuard {
    /// Create a new metadata frame guard, or `None` if allocation fails.
    pub fn new(roi_count: u16) -> Option<Self> {
        create_md_frame(roi_count).map(|ptr| Self { ptr })
    }

    /// Create a null (no-op) guard for when metadata is disabled.
    pub fn null() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
        }
    }

    /// Get the raw md_frame pointer for FFI calls.
    pub fn as_ptr(&self) -> *mut md_frame {
        self.ptr
    }

    /// Returns true if the guard holds a null pointer (metadata disabled).
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }
}

impl Drop for MdFrameGuard {
    fn drop(&mut self) {
        release_md_frame(self.ptr);
    }
}

// SAFETY: MdFrameGuard wraps a *mut md_frame allocated by the PVCAM SDK.
// Raw pointers are Send in Rust (they have no negative auto-trait impls),
// so this impl is technically redundant — but we keep it explicit to
// document the safety invariant: the pointer is only dereferenced within
// a single spawn_blocking context (the acquisition loop) and is never
// aliased across threads.
unsafe impl Send for MdFrameGuard {}

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
    if md_frame_ptr.is_null() || frame_ptr.is_null() || frame_size == 0 {
        tracing::error!(
            "decode_frame_metadata: invalid args (md_null={}, frame_null={}, size={})",
            md_frame_ptr.is_null(),
            frame_ptr.is_null(),
            frame_size
        );
        return false;
    }

    // SAFETY: All pointers are valid per caller contract, frame_size matches buffer
    let result = unsafe { pl_md_frame_decode(md_frame_ptr, frame_ptr as *mut _, frame_size) };

    result != 0
}

/// Per-ROI pixel data extracted from a multi-ROI frame (bd-0o6b).
///
/// After `pl_md_frame_decode` populates the `md_frame.roiArray`, this struct
/// holds a copy of the pixel data and region coordinates for a single ROI.
#[derive(Debug, Clone)]
pub struct RoiData {
    /// ROI index (0-based) within the frame
    pub roi_nr: u16,
    /// Region coordinates (serial start, end, binning; parallel start, end, binning)
    pub s1: u16,
    pub s2: u16,
    pub sbin: u16,
    pub p1: u16,
    pub p2: u16,
    pub pbin: u16,
    /// Width in pixels after binning
    pub width: u16,
    /// Height in pixels after binning
    pub height: u16,
    /// Pixel data copied from the frame buffer (owned, safe to use after unlock)
    pub pixels: Vec<u8>,
}

/// Extract per-ROI pixel data from a decoded metadata frame (bd-0o6b).
///
/// PVCAM 3.x uses metadata-embedded frames where `pl_md_frame_decode` populates
/// `md_frame.roiArray` with per-ROI data pointers and region headers. This
/// replaces the legacy `pl_exp_unravel` approach.
///
/// **Requires metadata to be enabled** (`PARAM_METADATA_ENABLED = true`).
/// Without metadata, frames contain raw interleaved data that cannot be
/// separated without knowing the exact layout.
///
/// # Arguments
/// * `md_frame_ptr` - A valid `md_frame` pointer that has been decoded via
///   `decode_frame_metadata` (i.e., `pl_md_frame_decode` was called successfully)
///
/// # Returns
/// * `Ok(Vec<RoiData>)` - Per-ROI pixel data with region coordinates
/// * `Err(String)` - If the md_frame is null or has no ROIs
///
/// # Safety Contract
/// - `md_frame_ptr` must have been created with `create_md_frame` and decoded
///   with `decode_frame_metadata`
/// - The frame buffer that was decoded must still be valid (not yet unlocked
///   from the circular buffer)
pub fn extract_roi_data(md_frame_ptr: *const md_frame) -> Result<Vec<RoiData>, String> {
    if md_frame_ptr.is_null() {
        return Err("md_frame_ptr is null".to_string());
    }

    // SAFETY: `md_frame_ptr` is non-null and was created by `pl_md_create_frame_struct_cont`,
    // then decoded by `pl_md_frame_decode`. The header pointer is set by the SDK during
    // decode and points into the frame buffer. We only read fields — no mutation.
    let hdr = unsafe { &*(*md_frame_ptr).header };
    let roi_count = hdr.roiCount as usize;

    if roi_count == 0 {
        return Ok(Vec::new());
    }

    let mut result = Vec::with_capacity(roi_count);

    for i in 0..roi_count {
        // SAFETY: `roiArray` is a C array of `md_frame_roi` allocated by
        // `pl_md_create_frame_struct_cont` with capacity >= roiCount.
        // Index `i` is in range [0, roiCount). Each element's `header` and
        // `data` pointers were populated by `pl_md_frame_decode` and point
        // into the still-valid frame buffer.
        let roi = unsafe { &*(*md_frame_ptr).roiArray.add(i) };
        let roi_hdr = unsafe { &*roi.header };

        let s1 = roi_hdr.roi.s1;
        let s2 = roi_hdr.roi.s2;
        let sbin = roi_hdr.roi.sbin;
        let p1 = roi_hdr.roi.p1;
        let p2 = roi_hdr.roi.p2;
        let pbin = roi_hdr.roi.pbin;

        // Calculate dimensions after binning
        let width = (s2 - s1 + 1) / sbin.max(1);
        let height = (p2 - p1 + 1) / pbin.max(1);

        let data_size = roi.dataSize as usize;
        if data_size == 0 || roi.data.is_null() {
            tracing::warn!(
                "ROI {} has no data (dataSize={}, null={})",
                i,
                data_size,
                roi.data.is_null()
            );
            continue;
        }

        // Copy pixel data to owned Vec so it survives frame buffer unlock
        // SAFETY: `roi.data` points into the frame buffer with `dataSize` valid
        // bytes. The frame buffer is still locked (caller guarantees this).
        // We copy to an owned Vec to decouple lifetime from the SDK buffer.
        let pixels =
            unsafe { std::slice::from_raw_parts(roi.data as *const u8, data_size).to_vec() };

        result.push(RoiData {
            roi_nr: roi_hdr.roiNr,
            s1,
            s2,
            sbin,
            p1,
            p2,
            pbin,
            width,
            height,
            pixels,
        });
    }

    tracing::debug!(
        "Extracted {} ROIs from metadata frame (bd-0o6b)",
        result.len()
    );
    Ok(result)
}

/// Set up continuous acquisition with multiple ROIs (bd-0o6b).
///
/// Extends the single-ROI `pl_exp_setup_cont` call to accept multiple regions.
/// The camera must support multi-ROI (check `PARAM_ROI_COUNT` first).
/// Up to 16 ROIs are supported by the Prime BSI.
///
/// # Arguments
/// * `hcam` - Valid camera handle
/// * `regions` - Slice of rgn_type regions (1-16)
/// * `exp_mode` - Exposure mode (typically TIMED_MODE)
/// * `exposure_ms` - Exposure time in milliseconds
/// * `buffer_mode` - CIRC_OVERWRITE or CIRC_NO_OVERWRITE
///
/// # Returns
/// * `Ok(frame_bytes)` - Size of each frame in bytes (includes all ROIs + metadata)
/// * `Err(String)` - If setup fails
///
/// # Safety Contract
/// - `hcam` must be a valid, open camera handle
/// - `regions` must contain valid ROI coordinates within sensor bounds
pub fn setup_multi_roi_cont(
    hcam: i16,
    regions: &[rgn_type],
    exp_mode: i16,
    exposure_ms: u32,
    buffer_mode: i16,
) -> Result<uns32, String> {
    if hcam < 0 {
        return Err(format!("Invalid camera handle: {hcam}"));
    }
    if regions.is_empty() {
        return Err("At least one region required".to_string());
    }
    if regions.len() > 16 {
        return Err(format!("Maximum 16 ROIs supported, got {}", regions.len()));
    }

    let rgn_count = regions.len() as u16;
    let mut frame_bytes: uns32 = 0;

    // SAFETY: `hcam` is a valid camera handle. `regions` is a contiguous slice
    // of rgn_type (C-compatible POD struct). `rgn_count` matches the slice length.
    // `frame_bytes` is a stack-allocated uns32 output parameter. The SDK reads
    // `rgn_count` elements from the regions pointer and writes the total frame
    // size (all ROIs + metadata) into `frame_bytes`.
    let result = unsafe {
        pl_exp_setup_cont(
            hcam,
            rgn_count,
            regions.as_ptr() as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            buffer_mode,
        )
    };

    if result == 0 {
        let err_msg = get_pvcam_error();
        Err(format!(
            "pl_exp_setup_cont failed for {} ROIs: {}",
            rgn_count, err_msg
        ))
    } else {
        tracing::info!(
            "Multi-ROI setup complete: {} ROIs, frame_bytes={} (bd-0o6b)",
            rgn_count,
            frame_bytes
        );
        Ok(frame_bytes)
    }
}
