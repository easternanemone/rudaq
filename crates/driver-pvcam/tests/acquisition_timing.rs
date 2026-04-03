//! PVCAM Acquisition Timing, Throughput, and Driver Infrastructure Tests
//!
//! These tests verify timing-sensitive acquisition patterns and the full
//! driver infrastructure (callbacks, channels, watch patterns, fallback logic).
//! Originally tests 23-28 from `circ_buffer_diagnostic.rs`.
//!
//! Run with:
//! ```bash
//! cargo test --release -p driver-pvcam --features "pvcam_sdk" \
//!   --test acquisition_timing -- --nocapture --test-threads=1
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_sdk")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

mod common;

use common::circ_buffer_fixtures::*;
use driver_pvcam::components::acquisition::{
    CallbackContext, GLOBAL_CALLBACK_CTX, clear_global_callback_ctx, pvcam_eof_callback,
    set_global_callback_ctx,
};
use pvcam_sys::*;
use std::alloc::{Layout, alloc, alloc_zeroed, dealloc};
use std::ffi::{CStr, c_void};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

// =============================================================================
// TEST 23: Post-unlock check_cont_status call
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_23_post_unlock_status_check() {
    println!("\n=== TEST 23: Post-Unlock check_cont_status Pattern Test ===");
    println!("Tests whether calling check_cont_status after unlock causes the 19-frame issue.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21;

    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed");
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed");
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    let callback_ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let callback_ctx_ptr = &**callback_ctx as *const FullCallbackContext;
    FULL_CTX.store(
        callback_ctx_ptr as *mut FullCallbackContext,
        Ordering::Release,
    );

    println!("[SETUP] Registering EOF callback...");
    let callback_registered = unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        );
        result != 0
    };
    if !callback_registered {
        println!("ERROR: pl_cam_register_callback_ex3 failed");
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    let setup_result = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region,
            TIMED_MODE,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        )
    };
    if setup_result == 0 {
        println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    let total_size = frame_bytes as usize * BUFFER_FRAMES;
    let layout = Layout::from_size_align(total_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let start_result = unsafe { pl_exp_start_cont(hcam, buffer as *mut _, total_size as uns32) };
    if start_result == 0 {
        println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
        unsafe {
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Acquisition started");

    let callback_ctx_for_loop = callback_ctx.clone();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let mut frames = 0i32;
        let mut loop_iteration = 0u64;

        while frames < TARGET_FRAMES {
            loop_iteration += 1;

            let pending = callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                eprintln!(
                    "[TIMEOUT] iter={}, status={}, bytes={}, cnt={}",
                    loop_iteration, status, bytes, cnt
                );
                if status == 0 {
                    eprintln!("[FATAL] SDK status=0");
                    break;
                }
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                continue;
            }

            frames += 1;

            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] unlock_oldest_frame failed");
                }
            }

            // KEY DIFFERENCE: Call check_cont_status AFTER unlock (like driver)
            if frames <= 5 || frames % 30 == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                eprintln!(
                    "[POST-UNLOCK STATUS] frame={}, status={}, bytes={}, cnt={}",
                    frames, status, bytes, cnt
                );
            }

            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired (iter={})", frames, loop_iteration);
            }
        }

        frames
    })
    .await
    .unwrap();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 23 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Post-unlock check_cont_status is NOT the issue");
    } else {
        println!(
            "RESULT: Post-unlock check_cont_status MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Post-unlock check_cont_status may be affecting SDK callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 24: Use ACTUAL driver callback infrastructure (GLOBAL_CALLBACK_CTX)
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_24_driver_callback_infrastructure() {
    println!("\n=== TEST 24: Driver Callback Infrastructure Test ===");
    println!("Uses the EXACT same GLOBAL_CALLBACK_CTX and pvcam_eof_callback as the driver.\n");
    println!("If this FAILS at ~19 frames: issue is in driver callback infrastructure.");
    println!("If this PASSES with 200 frames: issue is in how driver sets up the context.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21;

    // Initialize SDK
    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }
    println!("[OK] PVCAM SDK initialized");

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed");
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed");
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    // Create driver's CallbackContext
    let callback_ctx = Arc::new(Box::pin(CallbackContext::new(hcam)));
    let callback_ctx_ptr = &**callback_ctx as *const CallbackContext;
    set_global_callback_ctx(callback_ctx_ptr);
    println!(
        "[OK] Driver CallbackContext created, ptr={:?}",
        callback_ctx_ptr
    );

    // Register driver's actual callback
    println!("[SETUP] Registering driver's pvcam_eof_callback...");
    let callback_registered = unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pvcam_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        );
        result != 0
    };
    if !callback_registered {
        println!(
            "ERROR: callback registration failed: {}",
            get_error_message()
        );
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Driver's pvcam_eof_callback registered");

    // Setup
    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    let setup_result = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region,
            TIMED_MODE,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        )
    };
    if setup_result == 0 {
        println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    let total_size = frame_bytes as usize * BUFFER_FRAMES;
    let layout = Layout::from_size_align(total_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let start_result = unsafe { pl_exp_start_cont(hcam, buffer as *mut _, total_size as uns32) };
    if start_result == 0 {
        println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
        unsafe {
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Acquisition started");

    let callback_ctx_for_loop = callback_ctx.clone();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let mut frames = 0i32;

        while frames < TARGET_FRAMES {
            let pending = rt.block_on(callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS));
            if pending == 0 {
                eprintln!("[TIMEOUT] at frame {}", frames);
                break;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                continue;
            }

            frames += 1;

            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }
            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired", frames);
            }
        }

        frames
    })
    .await
    .unwrap();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        clear_global_callback_ctx();
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 24 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Driver callback infrastructure is NOT the issue");
    } else {
        println!(
            "RESULT: Driver callback infrastructure MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Driver callback infrastructure may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 25: Pre-wait check_cont_status call (driver loop pattern)
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_25_pre_wait_status_check() {
    println!("\n=== TEST 25: Pre-Wait check_cont_status Pattern Test ===");
    println!("Tests whether calling check_cont_status at loop START (before wait) causes issue.\n");
    println!("This matches driver pattern at acquisition.rs lines 2502-2517.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21;

    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }
    println!("[OK] PVCAM SDK initialized");

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed");
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed");
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    let callback_ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let callback_ctx_ptr = &**callback_ctx as *const FullCallbackContext;
    FULL_CTX.store(
        callback_ctx_ptr as *mut FullCallbackContext,
        Ordering::Release,
    );

    println!("[SETUP] Registering EOF callback...");
    let callback_registered = unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        );
        result != 0
    };
    if !callback_registered {
        println!("ERROR: pl_cam_register_callback_ex3 failed");
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    let setup_result = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region,
            TIMED_MODE,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        )
    };
    if setup_result == 0 {
        println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    let total_size = frame_bytes as usize * BUFFER_FRAMES;
    let layout = Layout::from_size_align(total_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let start_result = unsafe { pl_exp_start_cont(hcam, buffer as *mut _, total_size as uns32) };
    if start_result == 0 {
        println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
        unsafe {
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Acquisition started");

    println!(
        "\n=== FRAME ACQUISITION LOOP (with PRE-WAIT status check, target: {} frames) ===\n",
        TARGET_FRAMES
    );

    let callback_ctx_for_loop = callback_ctx.clone();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let mut frames = 0i32;
        let mut loop_iteration = 0u64;

        while frames < TARGET_FRAMES {
            loop_iteration += 1;

            // === KEY DIFFERENCE: Call check_cont_status at START of loop ===
            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                let pending = callback_ctx_for_loop.pending_frames.load(Ordering::Acquire);
                eprintln!(
                    "[PRE-WAIT STATUS] iter={}, status={}, bytes={}, cnt={}, pending={}",
                    loop_iteration, status, bytes, cnt, pending
                );
            }

            let pending = callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS);

            if pending == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                eprintln!(
                    "[TIMEOUT] iter={}, status={}, bytes={}, cnt={}",
                    loop_iteration, status, bytes, cnt
                );
                if status == 0 {
                    eprintln!("[FATAL] SDK status=0 (READOUT_NOT_ACTIVE) - acquisition stopped!");
                    break;
                }
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                continue;
            }

            frames += 1;

            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] unlock_oldest_frame failed");
                }
            }

            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired (iter={})", frames, loop_iteration);
            }
        }

        frames
    })
    .await
    .unwrap();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 25 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Pre-wait check_cont_status is NOT the issue (200 frames achieved)");
    } else {
        println!(
            "RESULT: Pre-wait check_cont_status MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Pre-wait check_cont_status may be affecting SDK state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 26: Watch channel loop condition + CallbackContext(-1) pattern
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_26_watch_channel_loop_condition() {
    println!("\n=== TEST 26: Watch Channel Loop Condition Test ===");
    println!("Tests driver's exact pattern:");
    println!("  1. CallbackContext created with hcam=-1");
    println!("  2. set_hcam(actual_hcam) called before registration");
    println!("  3. watch::Sender::borrow().clone() in loop condition\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21;

    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed");
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed");
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    // KEY DIFFERENCE: Create CallbackContext with -1 (like driver does at construction)
    let callback_ctx = Arc::new(Box::pin(CallbackContext::new(-1))); // <-- -1 like driver
    callback_ctx.set_hcam(hcam); // <-- Update to actual hcam
    let callback_ctx_ptr = &**callback_ctx as *const CallbackContext;
    println!(
        "[OK] CallbackContext created with -1, then set_hcam({})",
        hcam
    );

    set_global_callback_ctx(callback_ctx_ptr);

    // Create watch channel for streaming (like driver's Parameter<bool>)
    let (streaming_tx, _streaming_rx) = tokio::sync::watch::channel(true);
    let streaming_tx = Arc::new(streaming_tx);

    // Register callback
    println!("[SETUP] Registering EOF callback...");
    let callback_registered = unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pvcam_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        );
        result != 0
    };
    if !callback_registered {
        println!(
            "ERROR: Failed to register callback: {}",
            get_error_message()
        );
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    let setup_result = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        )
    };
    if setup_result == 0 {
        println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let buffer_size = frame_bytes as usize * BUFFER_FRAMES;
    let layout = Layout::from_size_align(buffer_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    let start_result =
        unsafe { pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32) };
    if start_result == 0 {
        println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
        unsafe {
            dealloc(buffer, layout);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Acquisition started");

    let callback_ctx_for_loop = callback_ctx.clone();
    let streaming_tx_for_stop = streaming_tx.clone();

    let loop_start = std::time::Instant::now();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let mut frames: i32 = 0;
        let mut loop_iteration: u64 = 0;
        let mut consecutive_timeouts: u32 = 0;

        // KEY DIFFERENCE: Use watch channel borrow in loop condition
        while *streaming_tx.borrow() && frames < TARGET_FRAMES {
            loop_iteration += 1;

            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                let pending = callback_ctx_for_loop.pending_frames.load(Ordering::Acquire);
                eprintln!(
                    "[PRE-WAIT STATUS] iter={}, status={}, bytes={}, cnt={}, pending={}",
                    loop_iteration, status, bytes, cnt, pending
                );
            }

            let pending = rt.block_on(callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS));
            if pending == 0 {
                consecutive_timeouts += 1;
                eprintln!(
                    "[TIMEOUT] #{} at iter={}",
                    consecutive_timeouts, loop_iteration
                );
                if consecutive_timeouts >= 5 {
                    eprintln!("[FATAL] Max timeouts reached");
                    break;
                }
                continue;
            }
            consecutive_timeouts = 0;

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                continue;
            }

            frames += 1;

            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }
            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired (iter={})", frames, loop_iteration);
            }
        }

        frames
    })
    .await
    .unwrap();

    let total_time = loop_start.elapsed().as_millis();

    let _ = streaming_tx_for_stop.send(false);

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        clear_global_callback_ctx();
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 26 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
    println!("Total time: {}ms", total_time);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Watch channel + CallbackContext(-1) pattern is NOT the issue");
    } else {
        println!(
            "RESULT: Watch channel or CallbackContext(-1) pattern MAY be the issue ({} frames)",
            frames_acquired
        );
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Watch channel or -1 pattern may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 27: Callback deregister/re-register during fallback
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_27_callback_rereg_during_fallback() {
    println!("\n=== TEST 27: Callback Deregister/Re-register During Fallback ===");
    println!("Replicates driver's EXACT fallback sequence:");
    println!("  1. Register callback");
    println!("  2. Try CIRC_OVERWRITE setup (may fail)");
    println!("  3. Try CIRC_OVERWRITE start -> FAILS on Prime BSI");
    println!("  4. Re-setup with CIRC_NO_OVERWRITE");
    println!("  5. DEREGISTER callback");
    println!("  6. RE-REGISTER callback");
    println!("  7. Re-start with CIRC_NO_OVERWRITE\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21;

    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed");
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed");
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    let callback_ctx = Arc::new(Box::pin(CallbackContext::new(-1)));
    callback_ctx.set_hcam(hcam);
    let callback_ctx_ptr = &**callback_ctx as *const CallbackContext;
    set_global_callback_ctx(callback_ctx_ptr);

    // Step 1: Register callback FIRST
    println!("[STEP 1] Registering EOF callback...");
    let first_reg = unsafe {
        pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pvcam_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        )
    };
    if first_reg == 0 {
        println!(
            "ERROR: First callback registration failed: {}",
            get_error_message()
        );
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] First callback registered");

    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    // Step 2: Try CIRC_OVERWRITE setup
    println!("[STEP 2] Trying CIRC_OVERWRITE setup...");
    let mut frame_bytes: uns32 = 0;
    let overwrite_setup = unsafe {
        pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_OVERWRITE as i16,
        )
    };
    if overwrite_setup == 0 {
        println!(
            "[INFO] CIRC_OVERWRITE setup failed (expected on some cameras): {}",
            get_error_message()
        );
    } else {
        println!(
            "[OK] CIRC_OVERWRITE setup succeeded, frame_bytes={}",
            frame_bytes
        );
    }

    // Allocate buffer
    let frame_bytes_val = if overwrite_setup != 0 {
        frame_bytes as usize
    } else {
        (ser_size as usize) * (par_size as usize) * 2
    };
    let buffer_size = frame_bytes_val * BUFFER_FRAMES;
    let layout = Layout::from_size_align(buffer_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    // Step 3: Try CIRC_OVERWRITE start
    println!("[STEP 3] Trying CIRC_OVERWRITE start (expected to fail on Prime BSI)...");
    let overwrite_start = if overwrite_setup != 0 {
        unsafe { pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32) }
    } else {
        0
    };

    if overwrite_start != 0 {
        println!("[UNEXPECTED] CIRC_OVERWRITE start SUCCEEDED");
    } else {
        println!(
            "[OK] CIRC_OVERWRITE start failed as expected: {}",
            get_error_message()
        );

        // Step 4: Re-setup with CIRC_NO_OVERWRITE
        println!("[STEP 4] Re-setup with CIRC_NO_OVERWRITE...");
        frame_bytes = 0;
        let no_overwrite_setup = unsafe {
            pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                TIMED_MODE as i16,
                EXPOSURE_MS,
                &mut frame_bytes,
                CIRC_NO_OVERWRITE as i16,
            )
        };
        if no_overwrite_setup == 0 {
            println!(
                "ERROR: CIRC_NO_OVERWRITE setup failed: {}",
                get_error_message()
            );
            unsafe {
                dealloc(buffer, layout);
                pl_cam_close(hcam);
                pl_pvcam_uninit();
            }
            return;
        }

        callback_ctx.set_circ_overwrite(false);

        // Step 5: DEREGISTER callback
        println!("[STEP 5] Deregistering callback before re-registration...");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        }

        // Step 6: RE-REGISTER callback
        println!("[STEP 6] Re-registering callback...");
        let rereg = unsafe {
            pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                pvcam_eof_callback as *mut std::ffi::c_void,
                callback_ctx_ptr as *mut std::ffi::c_void,
            )
        };
        if rereg == 0 {
            println!(
                "ERROR: Callback re-registration failed: {}",
                get_error_message()
            );
            unsafe {
                dealloc(buffer, layout);
                pl_cam_close(hcam);
                pl_pvcam_uninit();
            }
            return;
        }

        // Step 7: Re-start with CIRC_NO_OVERWRITE
        println!("[STEP 7] Starting with CIRC_NO_OVERWRITE...");
        let no_overwrite_start =
            unsafe { pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32) };
        if no_overwrite_start == 0 {
            println!(
                "ERROR: CIRC_NO_OVERWRITE start failed: {}",
                get_error_message()
            );
            unsafe {
                dealloc(buffer, layout);
                pl_cam_close(hcam);
                pl_pvcam_uninit();
            }
            return;
        }
        println!("[OK] Acquisition started with CIRC_NO_OVERWRITE");
    }

    // Frame loop
    println!(
        "\n=== FRAME ACQUISITION LOOP (after fallback, target: {} frames) ===\n",
        TARGET_FRAMES
    );

    let callback_ctx_for_loop = callback_ctx.clone();
    let loop_start = std::time::Instant::now();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let mut frames: i32 = 0;
        let mut loop_iteration: u64 = 0;
        let mut consecutive_timeouts: u32 = 0;

        while frames < TARGET_FRAMES {
            loop_iteration += 1;

            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                let pending = callback_ctx_for_loop.pending_frames.load(Ordering::Acquire);
                eprintln!(
                    "[PRE-WAIT STATUS] iter={}, status={}, bytes={}, cnt={}, pending={}",
                    loop_iteration, status, bytes, cnt, pending
                );
            }

            let pending = rt.block_on(callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS));
            if pending == 0 {
                consecutive_timeouts += 1;
                eprintln!(
                    "[TIMEOUT] #{} at iter={}",
                    consecutive_timeouts, loop_iteration
                );
                if consecutive_timeouts >= 5 {
                    eprintln!("[FATAL] Max timeouts reached");
                    break;
                }
                continue;
            }
            consecutive_timeouts = 0;

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                continue;
            }

            frames += 1;

            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }
            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired (iter={})", frames, loop_iteration);
            }
        }

        frames
    })
    .await
    .unwrap();

    let total_time = loop_start.elapsed().as_millis();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        clear_global_callback_ctx();
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 27 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
    println!("Total time: {}ms", total_time);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Callback deregister/re-register during fallback is NOT the issue");
    } else {
        println!(
            "RESULT: Callback deregister/re-register during fallback MAY be the issue ({} frames)",
            frames_acquired
        );
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Fallback callback re-registration may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 28: Full Driver Channel Infrastructure Test
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_28_full_driver_channel_infrastructure() {
    use std::alloc::{Layout, alloc, dealloc};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::time::Duration;

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;

    println!("\n=== TEST 28: Full Driver Channel Infrastructure Test ===");
    println!("Tests driver's exact channel infrastructure:");
    println!("  1. tokio::sync::broadcast::channel for frame_tx");
    println!("  2. tokio::sync::mpsc::unbounded_channel for errors");
    println!("  3. std::sync::mpsc::channel for done signaling");
    println!("  4. Storing buffer in tokio::sync::Mutex");
    println!("  5. Spawning error watcher task");
    println!();

    // ========== SETUP PHASE ==========
    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed: {}", get_error_message());
            return;
        }
    }

    let mut hcam: i16 = 0;
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, hcam={}", hcam);

    let callback_ctx = Arc::new(Box::pin(CallbackContext::new(-1)));
    callback_ctx.set_hcam(hcam);
    let callback_ctx_ptr = &**callback_ctx as *const CallbackContext;
    set_global_callback_ctx(callback_ctx_ptr);

    // ========== DRIVER CHANNEL INFRASTRUCTURE ==========
    let (frame_tx, _frame_rx) = tokio::sync::broadcast::channel::<u32>(100);
    let (error_tx, mut error_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let circ_buffer_storage: Arc<tokio::sync::Mutex<Option<*mut u8>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let (streaming_tx, _streaming_rx) = tokio::sync::watch::channel(true);

    // ========== SPAWN ERROR WATCHER TASK ==========
    let streaming_for_watcher = streaming_tx.clone();
    let _error_watcher = tokio::spawn(async move {
        if let Some(err) = error_rx.recv().await {
            eprintln!("[ERROR WATCHER] Received error: {}", err);
            let _ = streaming_for_watcher.send(false);
        }
    });

    // ========== REGISTER CALLBACK ==========
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        if pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pvcam_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        ) == 0
        {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            println!(
                "ERROR: Failed to register callback: {}",
                get_error_message()
            );
            return;
        }
    }

    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        );
        pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        );
        (ser, par)
    };

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    unsafe {
        if pl_exp_setup_cont(
            hcam,
            1,
            &region,
            TIMED_MODE as i16,
            100,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        ) == 0
        {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            return;
        }
    }

    let buffer_count = 21;
    let buffer_size = frame_bytes as usize * buffer_count;
    let layout = Layout::from_size_align(buffer_size, 4096).unwrap();
    let buffer = unsafe { alloc(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    // ========== STORE BUFFER IN ASYNC MUTEX ==========
    {
        let mut guard = circ_buffer_storage.lock().await;
        *guard = Some(buffer);
    }

    // Start acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        if pl_exp_start_cont(hcam, buffer as *mut _, buffer_size as uns32) == 0 {
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            clear_global_callback_ctx();
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            return;
        }
    }
    println!("[OK] Acquisition started");

    // ========== FRAME ACQUISITION LOOP IN spawn_blocking ==========
    println!(
        "\n=== FRAME ACQUISITION LOOP (with full channel infrastructure, target: {} frames) ===\n",
        TARGET_FRAMES
    );

    let callback_ctx_clone = callback_ctx.clone();
    let frame_tx_clone = frame_tx.clone();
    let error_tx_clone = error_tx.clone();
    let streaming_tx_clone = streaming_tx.clone();
    let done_tx_clone = done_tx.clone();

    let start_time = std::time::Instant::now();
    let frames_acquired = Arc::new(AtomicI32::new(0));
    let frames_acquired_clone = frames_acquired.clone();

    let poll_handle = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let mut loop_iteration = 0i32;
        let mut frame_info: FRAME_INFO = unsafe { std::mem::zeroed() };

        while *streaming_tx_clone.borrow() {
            loop_iteration += 1;

            if loop_iteration > TARGET_FRAMES {
                break;
            }

            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                let mut status: i16 = 0;
                let mut bytes_arrived: uns32 = 0;
                let mut buffer_cnt: uns32 = 0;
                unsafe {
                    if pl_exp_check_cont_status(
                        hcam,
                        &mut status,
                        &mut bytes_arrived,
                        &mut buffer_cnt,
                    ) != 0
                    {
                        let pending = callback_ctx_clone.pending_frames.load(Ordering::Acquire);
                        println!(
                            "[PRE-WAIT STATUS] iter={}, status={}, bytes={}, cnt={}, pending={}",
                            loop_iteration, status, bytes_arrived, buffer_cnt, pending
                        );
                    }
                }
            }

            let pending = rt.block_on(callback_ctx_clone.wait_for_frames(TIMEOUT_MS));
            if pending == 0 {
                let _ = error_tx_clone.send("Timeout waiting for callback".to_string());
                break;
            }

            let mut frame_ptr: *mut c_void = std::ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 {
                continue;
            }

            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }

            let current_frame = frames_acquired_clone.fetch_add(1, Ordering::Relaxed) + 1;

            let receiver_count = frame_tx_clone.receiver_count();
            if current_frame <= 25 || current_frame % 50 == 0 {
                println!(
                    "[FRAME {}] acquired (iter={}), broadcast receivers={}",
                    current_frame, loop_iteration, receiver_count
                );
            }
            let _ = frame_tx_clone.send(current_frame as u32);

            callback_ctx_clone
                .pending_frames
                .fetch_sub(1, Ordering::AcqRel);
        }

        let _ = done_tx_clone.send(());
    });

    let _ = poll_handle.await;
    let _ = done_rx.recv_timeout(Duration::from_secs(5));

    let total_time = start_time.elapsed().as_millis();
    let frames_acquired = frames_acquired.load(Ordering::Relaxed);

    // Cleanup
    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        let buffer_ptr = *circ_buffer_storage.lock().await;
        if let Some(ptr) = buffer_ptr {
            dealloc(ptr, layout);
        }
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        clear_global_callback_ctx();
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 28 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
    println!("Total time: {}ms", total_time);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Full driver channel infrastructure is NOT the issue");
    } else {
        println!(
            "RESULT: Full driver channel infrastructure MAY be the issue ({} frames)",
            frames_acquired
        );
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Channel infrastructure may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}
