//! SDK-equivalent streaming test: mirrors PVCAM FastStreamingToDisk defaults
//! - Uses TIMED_MODE + CIRC_OVERWRITE with a 255-frame circular buffer
//! - Captures 200 full-frame images and asserts no lost frames / gaps
//!
//! Hardware-only: run on Prime BSI (maitai)
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && cargo test -p daq-driver-pvcam --features pvcam_hardware \
//!     --test sdk_fast_stream_equiv -- --nocapture --test-threads=1'
//! ```

#![cfg(feature = "pvcam_hardware")]
#![cfg(not(target_arch = "wasm32"))]
use pvcam_sys::*;
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ffi::CStr;
use std::time::Instant;

const TARGET_FRAMES: usize = 200;
const BUFFER_FRAMES: usize = 255; // matches FastStreamingToDisk default
const EXPOSURE_MS: u32 = 10; // FastStreaming default prompt uses 10ms on Prime BSI

fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

fn open_first_camera() -> Option<i16> {
    unsafe {
        if pl_pvcam_init() == 0 {
            eprintln!("pl_pvcam_init failed: {}", get_error_message());
            return None;
        }
        let mut name = [0i8; PARAM_NAME_LEN as usize];
        if pl_cam_get_name(0, name.as_mut_ptr()) == 0 {
            eprintln!("pl_cam_get_name failed: {}", get_error_message());
            return None;
        }
        let mut hcam: i16 = 0;
        if pl_cam_open(name.as_mut_ptr(), &mut hcam, 0) == 0 {
            eprintln!("pl_cam_open failed: {}", get_error_message());
            return None;
        }
        Some(hcam)
    }
}

fn close_camera(hcam: i16) {
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
}

fn read_param_uns16(hcam: i16, param_id: u32) -> Option<u16> {
    let mut val: uns16 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT as i16,
            &mut val as *mut _ as *mut _,
        ) != 0
        {
            Some(val as u16)
        } else {
            None
        }
    }
}

#[test]
fn fast_streaming_equivalent() {
    let hcam = open_first_camera().expect("camera open");

    // Discover full-frame size
    let ser_size = read_param_uns16(hcam, PARAM_SER_SIZE as u32).expect("PARAM_SER_SIZE");
    let par_size = read_param_uns16(hcam, PARAM_PAR_SIZE as u32).expect("PARAM_PAR_SIZE");

    // Configure full-frame region
    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    // Setup continuous acquisition preferring CIRC_OVERWRITE
    let mut frame_bytes: uns32 = 0;
    let mut circ_mode = CIRC_OVERWRITE;
    let setup_ok = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            TIMED_MODE,
            EXPOSURE_MS,
            &mut frame_bytes,
            circ_mode,
        )
    } != 0;

    if !setup_ok {
        eprintln!(
            "CIRC_OVERWRITE setup failed ({}), retrying no-overwrite",
            get_error_message()
        );
        circ_mode = CIRC_NO_OVERWRITE;
        let retry_ok = unsafe {
            pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                TIMED_MODE,
                EXPOSURE_MS,
                &mut frame_bytes,
                circ_mode,
            )
        } != 0;
        assert!(
            retry_ok,
            "pl_exp_setup_cont failed in both modes: {}",
            get_error_message()
        );
    }

    // Avoid allocating multi-GB buffers for full-frame Prime BSI. Cap at 512MB while
    // keeping at least 32 frames to exercise overwrite/lock semantics.
    let mut buffer_frames = BUFFER_FRAMES;
    let max_bytes: usize = 512 * 1024 * 1024;
    let mut frame_bytes_usize = frame_bytes as usize;
    if frame_bytes_usize.saturating_mul(buffer_frames) > max_bytes {
        buffer_frames = (max_bytes / frame_bytes_usize).max(32);
    }

    let mut circ_size = frame_bytes_usize * buffer_frames;
    let mut layout = Layout::from_size_align(circ_size, 4096).expect("layout");
    let mut circ_ptr = unsafe { alloc_zeroed(layout) };
    assert!(!circ_ptr.is_null(), "alloc circ buffer");

    eprintln!(
        "setup: mode={}, frame_bytes={}, buffer_frames={}, circ_size={} bytes",
        if circ_mode == CIRC_OVERWRITE {
            "overwrite"
        } else {
            "no-overwrite"
        },
        frame_bytes_usize,
        buffer_frames,
        circ_size
    );

    // Start acquisition (fallback to CIRC_NO_OVERWRITE if start fails in overwrite mode)
    let mut start_ok =
        unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size as uns32) } != 0;
    if !start_ok && circ_mode == CIRC_OVERWRITE {
        eprintln!(
            "CIRC_OVERWRITE start failed ({}), retrying no-overwrite",
            get_error_message()
        );
        unsafe { dealloc(circ_ptr, layout) };

        circ_mode = CIRC_NO_OVERWRITE;
        frame_bytes = 0;
        let setup_retry = unsafe {
            pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                TIMED_MODE,
                EXPOSURE_MS,
                &mut frame_bytes,
                circ_mode,
            )
        } != 0;
        assert!(
            setup_retry,
            "pl_exp_setup_cont (retry) failed: {}",
            get_error_message()
        );

        frame_bytes_usize = frame_bytes as usize;
        buffer_frames = BUFFER_FRAMES;
        if frame_bytes_usize.saturating_mul(buffer_frames) > max_bytes {
            buffer_frames = (max_bytes / frame_bytes_usize).max(32);
        }
        circ_size = frame_bytes_usize * buffer_frames;
        layout = Layout::from_size_align(circ_size, 4096).expect("layout");
        circ_ptr = unsafe { alloc_zeroed(layout) };
        assert!(!circ_ptr.is_null(), "alloc circ buffer (retry)");

        start_ok = unsafe { pl_exp_start_cont(hcam, circ_ptr as *mut _, circ_size as uns32) } != 0;
    }

    assert!(
        start_ok,
        "pl_exp_start_cont (mode {}): {}",
        if circ_mode == CIRC_OVERWRITE {
            "overwrite"
        } else {
            "no-overwrite"
        },
        get_error_message()
    );

    // Drain frames
    let mut frame_info: FRAME_INFO = unsafe { std::mem::zeroed() };
    frame_info.hCam = hcam;
    let mut last_nr: i32 = 0;
    let mut acquired: usize = 0;
    let mut gap_events: u32 = 0;
    let mut first_frame_wait_logged = false;
    let deadline = Instant::now() + std::time::Duration::from_secs(20);

    while acquired < TARGET_FRAMES {
        if Instant::now() > deadline {
            panic!("timeout: got {} frames", acquired);
        }

        let mut status: i16 = 0;
        let mut byte_cnt: uns32 = 0;
        let mut buf_cnt: uns32 = 0;
        unsafe {
            if pl_exp_check_cont_status(hcam, &mut status, &mut byte_cnt, &mut buf_cnt) == 0 {
                panic!("status check failed: {}", get_error_message());
            }
        }

        if !first_frame_wait_logged
            && acquired == 0
            && Instant::now() + std::time::Duration::from_secs(0)
                > deadline - std::time::Duration::from_secs(19)
        {
            first_frame_wait_logged = true;
            eprintln!(
                "waiting for first frame: status={}, buf_cnt={}, byte_cnt={}, last_error={}",
                status,
                buf_cnt,
                byte_cnt,
                get_error_message()
            );
        }

        if status == READOUT_IN_PROGRESS || buf_cnt == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }

        let mut address: *mut std::ffi::c_void = std::ptr::null_mut();
        let got_frame =
            unsafe { pl_exp_get_oldest_frame_ex(hcam, &mut address, &mut frame_info) } != 0;
        if !got_frame || address.is_null() {
            continue;
        }

        // Frame numbering check (FrameNr is 1-based)
        let current = frame_info.FrameNr;
        if last_nr != 0 && current != last_nr + 1 {
            gap_events += 1;
            eprintln!("Frame discontinuity: prev {}, got {}", last_nr, current);
        }
        last_nr = current;
        acquired += 1;

        // Release oldest frame if not overwrite mode
        if circ_mode == CIRC_NO_OVERWRITE {
            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }
        }
    }

    // Stop and cleanup
    unsafe {
        pl_exp_stop_cont(hcam, CCS_HALT);
    }

    eprintln!(
        "completed: acquired {}, gap_events {}, mode {}",
        acquired,
        gap_events,
        if circ_mode == CIRC_OVERWRITE {
            "overwrite"
        } else {
            "no-overwrite"
        }
    );

    unsafe {
        dealloc(circ_ptr, layout);
    }

    close_camera(hcam);
}
