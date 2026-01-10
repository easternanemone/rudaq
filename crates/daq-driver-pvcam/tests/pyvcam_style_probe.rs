//! PyVCAM-style CIRC_OVERWRITE probe test
//!
//! This test mimics PyVCAM's exact approach to continuous acquisition:
//! 1. 4096-byte aligned buffer allocation
//! 2. pl_exp_setup_cont with CIRC_OVERWRITE
//! 3. Callback registration AFTER setup
//! 4. pl_exp_get_latest_frame_ex for frame retrieval (no unlock needed)
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && git pull && \
//!   cargo test --release -p daq-driver-pvcam --features pvcam_hardware \
//!     --test pyvcam_style_probe -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

use pvcam_sys::*;
use std::alloc::{alloc, dealloc, Layout};
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Buffer mode constants
const CIRC_OVERWRITE: i16 = 0;
const CIRC_NO_OVERWRITE: i16 = 1;
const CCS_HALT: i16 = 1;
const PL_CALLBACK_EOF: i32 = 1;

// PyVCAM uses 4096-byte alignment
const ALIGNMENT_BOUNDARY: usize = 4096;

// Exposure modes (from previous probe test)
const EXT_TRIG_INTERNAL: i16 = 1792;
const EXPOSE_OUT_FIRST_ROW: i16 = 0;

/// Get PVCAM error message
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Allocate 4096-byte aligned buffer (like PyVCAM)
fn allocate_aligned_buffer(size: usize) -> *mut u8 {
    let layout = Layout::from_size_align(size, ALIGNMENT_BOUNDARY)
        .expect("Invalid layout");
    unsafe { alloc(layout) }
}

/// Deallocate aligned buffer
unsafe fn deallocate_aligned_buffer(ptr: *mut u8, size: usize) {
    let layout = Layout::from_size_align(size, ALIGNMENT_BOUNDARY)
        .expect("Invalid layout");
    dealloc(ptr, layout);
}

// Global state for callback
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
static CALLBACK_ERRORS: AtomicU32 = AtomicU32::new(0);
static STOP_FLAG: AtomicBool = AtomicBool::new(false);

/// PyVCAM-style EOF callback
/// Uses pl_exp_get_latest_frame_ex like PyVCAM does
extern "system" fn pyvcam_eof_callback(frame_info: *const FRAME_INFO, _context: *mut c_void) {
    if STOP_FLAG.load(Ordering::Relaxed) {
        return;
    }

    let hcam = unsafe { (*frame_info).hCam };

    // PyVCAM calls pl_exp_get_latest_frame_ex inside the callback
    let mut address: *mut c_void = std::ptr::null_mut();
    // Use zeroed memory for FRAME_INFO to ensure all fields are initialized
    let mut fi: FRAME_INFO = unsafe { std::mem::zeroed() };

    let result = unsafe { pl_exp_get_latest_frame_ex(hcam, &mut address, &mut fi) };

    if result != 0 && !address.is_null() {
        FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        CALLBACK_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Test CIRC_OVERWRITE with PyVCAM-style setup
fn test_pyvcam_style(hcam: i16, exp_mode: i16, buffer_frames: u32) -> (bool, bool, u32, String) {
    // Reset counters
    FRAME_COUNT.store(0, Ordering::Relaxed);
    CALLBACK_ERRORS.store(0, Ordering::Relaxed);
    STOP_FLAG.store(false, Ordering::Relaxed);

    // Small ROI for fast testing (256x256)
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

    // Step 1: pl_exp_setup_cont with CIRC_OVERWRITE (like PyVCAM)
    let setup_ok = unsafe {
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

    if !setup_ok {
        let err = get_error_message();
        return (false, false, 0, format!("setup_cont failed: {}", err));
    }

    println!("  [OK] pl_exp_setup_cont succeeded, frame_bytes = {}", frame_bytes);

    // Step 2: Register callback AFTER setup (like PyVCAM)
    let callback_ok = unsafe {
        pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pyvcam_eof_callback as *mut c_void,
            std::ptr::null_mut(),
        ) != 0
    };

    if !callback_ok {
        let err = get_error_message();
        return (true, false, 0, format!("callback registration failed: {}", err));
    }

    println!("  [OK] Callback registered");

    // Step 3: Allocate 4096-byte aligned buffer (like PyVCAM)
    let buffer_size = (frame_bytes as usize) * (buffer_frames as usize);
    let buffer_ptr = allocate_aligned_buffer(buffer_size);

    if buffer_ptr.is_null() {
        unsafe { pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF); }
        return (true, false, 0, "Failed to allocate aligned buffer".to_string());
    }

    println!("  [OK] Allocated {} bytes with 4096-byte alignment", buffer_size);

    // Step 4: pl_exp_start_cont
    let start_ok = unsafe {
        pl_exp_start_cont(
            hcam,
            buffer_ptr as *mut c_void,
            buffer_size as uns32,
        ) != 0
    };

    if !start_ok {
        let err_code = unsafe { pl_error_code() };
        let err = get_error_message();
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            deallocate_aligned_buffer(buffer_ptr, buffer_size);
        }
        return (true, false, 0, format!("start_cont failed (err {}): {}", err_code, err));
    }

    println!("  [OK] pl_exp_start_cont succeeded!");

    // Step 5: Run for 2 seconds and count frames
    let start = Instant::now();
    let run_duration = Duration::from_secs(2);

    while start.elapsed() < run_duration {
        std::thread::sleep(Duration::from_millis(100));
        let frames = FRAME_COUNT.load(Ordering::Relaxed);
        let errors = CALLBACK_ERRORS.load(Ordering::Relaxed);
        print!("\r    Frames: {}, Errors: {}", frames, errors);
    }
    println!();

    // Stop acquisition
    STOP_FLAG.store(true, Ordering::Relaxed);
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        deallocate_aligned_buffer(buffer_ptr, buffer_size);
    }

    let final_frames = FRAME_COUNT.load(Ordering::Relaxed);
    let final_errors = CALLBACK_ERRORS.load(Ordering::Relaxed);

    (true, true, final_frames, format!("frames={}, errors={}", final_frames, final_errors))
}

#[tokio::test]
async fn test_pyvcam_style_circ_overwrite() {
    println!("\n=== PYVCAM-STYLE CIRC_OVERWRITE PROBE ===\n");
    println!("Testing with 4096-byte aligned buffers (like PyVCAM)\n");

    // Initialize PVCAM
    unsafe {
        if pl_pvcam_init() == 0 {
            panic!("Failed to initialize PVCAM: {}", get_error_message());
        }
    }
    println!("[OK] PVCAM initialized");

    // Get camera count
    let mut cam_count: i16 = 0;
    unsafe {
        if pl_cam_get_total(&mut cam_count) == 0 || cam_count == 0 {
            pl_pvcam_uninit();
            panic!("No cameras found");
        }
    }
    println!("[OK] Found {} camera(s)", cam_count);

    // Get camera name and open
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
    println!("[OK] Camera opened, handle = {}\n", hcam);

    // Test configurations
    let test_configs = [
        (EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW, "Internal+FirstRow", 10u32),
        (EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW, "Internal+FirstRow", 20u32),
        (EXT_TRIG_INTERNAL | 3, "Internal+Rolling", 10u32), // Rolling Shutter = 3
    ];

    println!("=== Testing CIRC_OVERWRITE with PyVCAM-style setup ===\n");

    for (exp_mode, name, buffer_frames) in test_configs.iter() {
        println!("Test: exp_mode={} ({}) buffer_frames={}", exp_mode, name, buffer_frames);

        let (setup_ok, start_ok, frames, msg) = test_pyvcam_style(hcam, *exp_mode, *buffer_frames);

        if start_ok && frames > 0 {
            println!("  [SUCCESS] Acquired {} frames! {}\n", frames, msg);
        } else if setup_ok && !start_ok {
            println!("  [FAIL] Setup OK but start failed: {}\n", msg);
        } else {
            println!("  [FAIL] {}\n", msg);
        }

        // Small delay between tests
        std::thread::sleep(Duration::from_millis(500));
    }

    // Also test CIRC_NO_OVERWRITE for comparison
    println!("\n=== Testing CIRC_NO_OVERWRITE for comparison ===\n");

    // Reset counters
    FRAME_COUNT.store(0, Ordering::Relaxed);
    CALLBACK_ERRORS.store(0, Ordering::Relaxed);
    STOP_FLAG.store(false, Ordering::Relaxed);

    let region = rgn_type {
        s1: 0, s2: 255, sbin: 1,
        p1: 0, p2: 255, pbin: 1,
    };
    let exposure_ms: uns32 = 10;
    let mut frame_bytes: uns32 = 0;
    let exp_mode = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW;
    let buffer_frames = 20u32;

    println!("Test: CIRC_NO_OVERWRITE with aligned buffer, exp_mode={}", exp_mode);

    let setup_ok = unsafe {
        pl_exp_setup_cont(
            hcam, 1, &region as *const _, exp_mode,
            exposure_ms, &mut frame_bytes, CIRC_NO_OVERWRITE,
        ) != 0
    };

    if setup_ok {
        println!("  [OK] setup_cont succeeded");

        let callback_ok = unsafe {
            pl_cam_register_callback_ex3(
                hcam, PL_CALLBACK_EOF,
                pyvcam_eof_callback as *mut c_void,
                std::ptr::null_mut(),
            ) != 0
        };

        if callback_ok {
            let buffer_size = (frame_bytes as usize) * (buffer_frames as usize);
            let buffer_ptr = allocate_aligned_buffer(buffer_size);

            if !buffer_ptr.is_null() {
                let start_ok = unsafe {
                    pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32) != 0
                };

                if start_ok {
                    println!("  [OK] start_cont succeeded");

                    // Run for 2 seconds
                    let start = Instant::now();
                    while start.elapsed() < Duration::from_secs(2) {
                        std::thread::sleep(Duration::from_millis(100));
                        print!("\r    Frames: {}, Errors: {}",
                               FRAME_COUNT.load(Ordering::Relaxed),
                               CALLBACK_ERRORS.load(Ordering::Relaxed));
                    }
                    println!();

                    STOP_FLAG.store(true, Ordering::Relaxed);
                    unsafe { pl_exp_abort(hcam, CCS_HALT); }

                    let frames = FRAME_COUNT.load(Ordering::Relaxed);
                    println!("  [RESULT] CIRC_NO_OVERWRITE got {} frames", frames);
                } else {
                    let err = get_error_message();
                    println!("  [FAIL] start_cont failed: {}", err);
                }

                unsafe { deallocate_aligned_buffer(buffer_ptr, buffer_size); }
            }

            unsafe { pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF); }
        }
    } else {
        println!("  [FAIL] setup_cont failed: {}", get_error_message());
    }

    // Cleanup
    println!("\n--- Cleanup ---");
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
    println!("[OK] Done\n");
}
