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
    debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
    debug_assert!(!circ_ptr.is_null(), "Circular buffer pointer is null");
    debug_assert!(circ_size_bytes > 0, "Circular buffer size must be > 0");

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
    circ_overwrite: bool,
) -> Result<uns32, String> {
    debug_assert!(hcam >= 0, "Invalid camera handle: {}", hcam);
    debug_assert!(!circ_ptr.is_null(), "Circular buffer pointer is null");
    debug_assert!(circ_size_bytes > 0, "Circular buffer size must be > 0");

    let (x_bin, y_bin) = binning;

    // Setup region (same as initial setup)
    // SAFETY: rgn_type is a POD C struct with only primitive uns16 fields.
    // Zero-initialization followed by explicit assignment of all fields is safe.
    // See start_stream() for detailed safety justification.
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
    let mut buffer_mode = if circ_overwrite {
        CIRC_OVERWRITE
    } else {
        CIRC_NO_OVERWRITE
    };
    let mut frame_bytes: uns32 = 0;

    // Probe PARAM_CIRC_BUFFER for visibility only; do not override user choice unless setup fails.
    if circ_overwrite {
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
    let mut circ_overwrite = buffer_mode == CIRC_OVERWRITE;
    let mut selected_buffer_mode = if circ_overwrite {
        CIRC_OVERWRITE
    } else {
        CIRC_NO_OVERWRITE
    };

    // Step 1: pl_exp_setup_cont (try overwrite, then fall back)
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
        circ_overwrite = false;
        frame_bytes = 0;
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
