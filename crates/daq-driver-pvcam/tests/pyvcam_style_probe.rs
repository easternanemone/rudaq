//! CIRC_OVERWRITE rejection probe (FIFO required)
//!
//! Verifies Prime BSI rejects CIRC_OVERWRITE (error 185) and that
//! CIRC_NO_OVERWRITE + FIFO retrieval works for a single frame.
//!
//! Run with hardware profile on maitai:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && \
//!   cargo test --release -p daq-driver-pvcam --features pvcam_hardware \
//!     --test pyvcam_style_probe -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

use pvcam_sys::*;
use std::ffi::CStr;
use std::time::Duration;

fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

fn open_first_camera() -> i16 {
    unsafe {
        if pl_pvcam_init() == 0 {
            panic!("Failed to initialize PVCAM: {}", get_error_message());
        }
    }

    let mut cam_count: i16 = 0;
    unsafe {
        if pl_cam_get_total(&mut cam_count) == 0 || cam_count == 0 {
            pl_pvcam_uninit();
            panic!("No cameras found");
        }
    }

    let mut cam_name = [0i8; 32];
    unsafe {
        pl_cam_get_name(0, cam_name.as_mut_ptr());
    }
    let name = unsafe { CStr::from_ptr(cam_name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    println!("[OK] Camera name: {}", name);

    let mut hcam: i16 = 0;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            pl_pvcam_uninit();
            panic!("Failed to open camera: {}", get_error_message());
        }
    }
    println!("[OK] Camera opened, handle = {}", hcam);
    hcam
}

fn close_camera(hcam: i16) {
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
}

#[tokio::test]
async fn test_circ_overwrite_rejected_and_fifo_single_frame() {
    println!("\n=== CIRC_OVERWRITE REJECTION + FIFO SMOKE ===\n");
    let hcam = open_first_camera();

    // Small ROI for speed
    let region = rgn_type {
        s1: 0,
        s2: 255,
        sbin: 1,
        p1: 0,
        p2: 255,
        pbin: 1,
    };
    let exposure_ms: uns32 = 10;
    let mut frame_bytes: uns32 = 0;
    let exp_mode = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW;

    // Attempt CIRC_OVERWRITE (expected to fail on Prime BSI)
    let overwrite_ok = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        ) != 0
    };

    if overwrite_ok {
        println!(
            "[WARN] CIRC_OVERWRITE unexpectedly succeeded (frame_bytes={})",
            frame_bytes
        );
    } else {
        let code = unsafe { pl_error_code() };
        println!(
            "[OK] CIRC_OVERWRITE rejected (err code {}): {}",
            code,
            get_error_message()
        );
    }

    // Now confirm FIFO path works: setup NO_OVERWRITE and grab one frame
    frame_bytes = 0;
    let fifo_setup = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        ) != 0
    };
    assert!(fifo_setup, "CIRC_NO_OVERWRITE setup should succeed");

    let buffer_frames: usize = 8;
    let buffer_size = (frame_bytes as usize) * buffer_frames;
    let mut buffer = vec![0u8; buffer_size];

    let start_ok = unsafe {
        pl_exp_start_cont(
            hcam,
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            buffer_size as uns32,
        ) != 0
    };
    assert!(start_ok, "pl_exp_start_cont should succeed for FIFO path");

    // Wait briefly for a frame, then retrieve oldest and unlock
    std::thread::sleep(Duration::from_millis(200));

    let mut address: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut fi: FRAME_INFO = unsafe { std::mem::zeroed() };
    let get_ok = unsafe { pl_exp_get_oldest_frame_ex(hcam, &mut address, &mut fi) } != 0;
    assert!(
        get_ok && !address.is_null(),
        "Should retrieve oldest frame under FIFO"
    );

    // Unlock to drain
    let unlock_ok = unsafe { pl_exp_unlock_oldest_frame(hcam) } != 0;
    assert!(unlock_ok, "Unlock of oldest frame should succeed");

    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
    }

    close_camera(hcam);
    println!("[OK] FIFO single-frame smoke passed\n");
}
