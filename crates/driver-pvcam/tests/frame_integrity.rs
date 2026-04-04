//! PVCAM Frame Data Integrity and Callback Isolation Tests
//!
//! These tests verify callback behavior, frame data integrity, and isolate
//! specific driver patterns to identify callback/acquisition issues.
//! Originally tests 17-22 from `circ_buffer_diagnostic.rs`.
//!
//! Run with:
//! ```bash
//! cargo test --release -p driver-pvcam --features "pvcam_sdk" \
//!   --test frame_integrity -- --nocapture --test-threads=1
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_sdk")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

mod common;

use common::circ_buffer_fixtures::*;
use pvcam_sys::*;
use std::alloc::{Layout, alloc, alloc_zeroed, dealloc};
use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

// ============================================================================
// TEST 17: Minimal SDK-style callback test (bd-callback-isolation)
// ============================================================================
// This test mimics the SDK C++ example exactly to isolate the callback issue.
// If this test passes for 20+ frames but the daemon fails, the issue is in
// the daemon's callback/synchronization implementation, not the FFI/SDK.

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_17_minimal_sdk_callback() {
    println!("\n=== TEST 17: Minimal SDK-style Callback Test ===");
    println!("This test mimics the C++ SDK example to isolate callback issues.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 5000;

    // Initialize PVCAM
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed: {}", get_error_message());
            return;
        }
    }
    println!("[OK] PVCAM initialized");

    // Get camera
    let mut cam_count: i16 = 0;
    unsafe {
        if pl_cam_get_total(&mut cam_count) == 0 || cam_count == 0 {
            println!("No cameras found, skipping test");
            pl_pvcam_uninit();
            return;
        }
    }

    // Open camera
    let mut cam_name = [0i8; 32];
    let mut hcam: i16 = 0;
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

    // Create minimal context
    let ctx = MinimalContext::new();
    MINIMAL_CTX.store(
        &ctx as *const MinimalContext as *mut MinimalContext,
        std::sync::atomic::Ordering::Release,
    );

    // Register callback (SDK C++ pattern)
    println!("[SETUP] Registering EOF callback (minimal C++ pattern)...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            minimal_eof_callback as *mut c_void,
            ptr::null_mut(),
        );
        if result == 0 {
            println!(
                "ERROR: pl_cam_register_callback_ex3 failed: {}",
                get_error_message()
            );
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Callback registered");

    // Setup region (full sensor)
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

    // Setup continuous acquisition
    let mut frame_bytes: uns32 = 0;
    println!("[SETUP] Setting up continuous acquisition (CIRC_NO_OVERWRITE)...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const rgn_type,
            TIMED_MODE as i16,
            100, // 100ms exposure
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    // Allocate page-aligned buffer
    let buffer_frames = 21;
    let total_size = frame_bytes as usize * buffer_frames;
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
    println!("[OK] Allocated {} bytes at {:?}", total_size, buffer);

    // Start continuous acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, total_size as uns32);
        if result == 0 {
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Acquisition started");

    // Frame acquisition loop (minimal - matches C++ SDK example)
    println!(
        "\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n",
        TARGET_FRAMES
    );

    let mut frames_acquired: i32 = 0;

    while frames_acquired < TARGET_FRAMES {
        // Wait for callback signal
        if !ctx.wait(TIMEOUT_MS) {
            println!("[TIMEOUT] No frame after {}ms", TIMEOUT_MS);
            break;
        }

        // Get oldest frame
        let mut frame_ptr: *mut c_void = ptr::null_mut();
        unsafe {
            if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                println!(
                    "[ERROR] pl_exp_get_oldest_frame failed: {}",
                    get_error_message()
                );
                continue;
            }
        }

        frames_acquired += 1;

        // Unlock immediately
        unsafe {
            if pl_exp_unlock_oldest_frame(hcam) == 0 {
                println!("[ERROR] pl_exp_unlock_oldest_frame failed");
            }
        }

        if frames_acquired <= 25 || frames_acquired % 50 == 0 {
            let frame_nr = ctx.frame_nr.load(Ordering::Acquire);
            let cb_count = ctx.callback_count.load(Ordering::Acquire);
            println!(
                "[FRAME {}] frame_nr={}, callbacks={}",
                frames_acquired, frame_nr, cb_count
            );
        }
    }

    // Cleanup
    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        MINIMAL_CTX.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 17 COMPLETE ===\n");

    // Assert success
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Callbacks stopped prematurely!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 18: spawn_blocking isolation test
// =============================================================================
//
// This test runs the frame loop INSIDE spawn_blocking like the full driver,
// but uses the minimal loop pattern. This isolates whether the threading model
// (spawn_blocking) is causing the callback issue.
//
// If this test FAILS at ~19 frames: The issue is with spawn_blocking threading
// If this test PASSES with 200 frames: The issue is in the full driver logic

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_18_spawn_blocking_isolation() {
    println!("\n=== TEST 18: spawn_blocking Isolation Test ===");
    println!("Runs frame loop in spawn_blocking like full driver.\n");
    println!("If this fails at ~19 frames: spawn_blocking is the issue.");
    println!("If this passes with 200 frames: issue is in full driver logic.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 2000;
    const EXPOSURE_MS: u32 = 100;
    const BUFFER_FRAMES: usize = 21; // Match full driver diagnostic

    // Initialize SDK
    println!("[SETUP] Initializing PVCAM SDK...");
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed");
            return;
        }
    }
    println!("[OK] PVCAM SDK initialized");

    // Open camera
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

    // Create callback context (like full driver: Arc<Pin<Box<...>>>)
    let ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!(
        "[OK] Callback context created (Arc<Pin<Box>>), ptr={:?}",
        ctx_ptr
    );

    // Register callback BEFORE setup (like full driver pattern)
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed");
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region (full sensor)
    let region = rgn_type {
        s1: 0,
        s2: 2047,
        sbin: 1,
        p1: 0,
        p2: 2047,
        pbin: 1,
    };

    // Setup continuous acquisition
    let mut frame_bytes: uns32 = 0;
    println!("[SETUP] Setting up continuous acquisition...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const rgn_type,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    // Allocate page-aligned buffer
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
    println!("[OK] Allocated {} bytes at {:?}", total_size, buffer);

    // Start continuous acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, total_size as uns32);
        if result == 0 {
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Acquisition started");

    // Clone for spawn_blocking
    let ctx_clone = ctx.clone();
    let streaming = Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    println!(
        "\n=== FRAME ACQUISITION LOOP (in spawn_blocking, target: {} frames) ===\n",
        TARGET_FRAMES
    );

    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;
        let mut loop_iteration: u64 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            loop_iteration += 1;

            // Wait for callback
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!(
                    "[TIMEOUT] No frame after {}ms (acquired {})",
                    TIMEOUT_MS, frames_acquired
                );
                continue;
            }

            // Get oldest frame
            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    continue;
                }
            }

            frames_acquired += 1;

            // Unlock immediately
            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] pl_exp_unlock_oldest_frame failed");
                }
            }

            // Consume callback notification
            ctx_clone.consume_one();

            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                println!(
                    "[FRAME {}] acquired (iter {})",
                    frames_acquired, loop_iteration
                );
            }
        }

        frames_acquired
    });

    let frames_acquired = handle.await.unwrap();

    streaming.store(false, Ordering::Release);
    ctx.signal_shutdown();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 18 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: spawn_blocking is NOT the issue (200 frames achieved)");
    } else {
        println!(
            "RESULT: spawn_blocking MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. spawn_blocking may be causing the callback issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 19: check_cont_status isolation test
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_19_check_cont_status_isolation() {
    println!("\n=== TEST 19: check_cont_status Isolation Test ===");
    println!("Adds check_cont_status calls like full driver to test_18's working pattern.\n");
    println!("If this fails at ~19 frames: check_cont_status is the issue.");
    println!("If this passes with 200 frames: check_cont_status is NOT the issue.\n");

    const TARGET_FRAMES: i32 = 200;
    const TIMEOUT_MS: u64 = 5000;
    const EXPOSURE_MS: uns32 = 100;
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

    // Open camera
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

    // Create callback context (like full driver: Arc<Pin<Box<...>>>)
    let ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!(
        "[OK] Callback context created (Arc<Pin<Box>>), ptr={:?}",
        ctx_ptr
    );

    // Register callback BEFORE setup (SDK pattern)
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!(
                "ERROR: pl_cam_register_callback_ex3 failed: {}",
                get_error_message()
            );
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region (full sensor)
    let region = rgn_type {
        s1: 0,
        s2: 2047,
        sbin: 1,
        p1: 0,
        p2: 2047,
        pbin: 1,
    };

    // Setup continuous acquisition with CIRC_NO_OVERWRITE
    let mut frame_bytes: uns32 = 0;
    println!("[SETUP] Setting up continuous acquisition (CIRC_NO_OVERWRITE)...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const rgn_type,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    // Allocate 4K-aligned buffer
    const ALIGN_4K: usize = 4096;
    let buffer_size = (frame_bytes as usize) * BUFFER_FRAMES;
    let layout = Layout::from_size_align(buffer_size, ALIGN_4K).unwrap();
    let buffer = unsafe { alloc(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!(
        "[OK] Allocated {} bytes ({} frames) at {:?}",
        buffer_size, BUFFER_FRAMES, buffer
    );

    // Start acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as uns32);
        if result == 0 {
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Acquisition started");

    // Clone Arc for spawn_blocking
    let ctx_clone = ctx.clone();
    let streaming = Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    // Spawn frame loop in blocking thread (like full driver)
    println!("\n=== FRAME ACQUISITION LOOP (with check_cont_status calls) ===\n");
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;
        let mut loop_iteration: u64 = 0;
        let loop_start = std::time::Instant::now();

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            loop_iteration += 1;

            // KEY DIFFERENCE FROM TEST_18: Add check_cont_status call like full driver
            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(
                        hcam,
                        &mut status,
                        &mut bytes_arrived,
                        &mut buffer_cnt,
                    ) != 0
                    {
                        if loop_iteration <= 10 {
                            eprintln!(
                                "[ITER {}] check_cont_status: status={}, bytes={}, cnt={}",
                                loop_iteration, status, bytes_arrived, buffer_cnt
                            );
                        }
                    }
                }
            }

            // Wait for callback (like full driver)
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!(
                    "[TIMEOUT] No frame after {}ms (acquired {})",
                    TIMEOUT_MS, frames_acquired
                );

                // KEY DIFFERENCE: Add check_cont_status on timeout like full driver
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(
                        hcam,
                        &mut status,
                        &mut bytes_arrived,
                        &mut buffer_cnt,
                    ) != 0
                    {
                        eprintln!(
                            "[TIMEOUT SDK] status={}, bytes={}, cnt={}",
                            status, bytes_arrived, buffer_cnt
                        );
                    }
                }
                continue;
            }

            // Get oldest frame
            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    eprintln!(
                        "[ERROR] pl_exp_get_oldest_frame failed: {}",
                        get_error_message()
                    );
                    continue;
                }
            }

            frames_acquired += 1;

            // Unlock immediately (like minimal test and test_18)
            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] pl_exp_unlock_oldest_frame failed");
                }
            }

            // Consume callback notification
            ctx_clone.consume_one();

            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                println!(
                    "[FRAME {}] acquired (iter {})",
                    frames_acquired, loop_iteration
                );
            }
        }

        let total_time = loop_start.elapsed().as_millis();
        println!("\n=== ACQUISITION SUMMARY ===");
        println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
        println!("Total time: {}ms", total_time);
        println!("Loop iterations: {}", loop_iteration);
        if frames_acquired > 0 && total_time > 0 {
            println!(
                "Average FPS: {:.2}",
                frames_acquired as f64 * 1000.0 / total_time as f64
            );
        }

        frames_acquired
    });

    // Wait for frame loop to complete
    let frames_acquired = handle.await.unwrap();

    // Stop streaming
    streaming.store(false, Ordering::Release);
    ctx.signal_shutdown();

    // Cleanup
    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 19 COMPLETE ===\n");

    // Assert success
    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: check_cont_status is NOT the issue (200 frames achieved)");
    } else {
        println!(
            "RESULT: check_cont_status MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. check_cont_status may be interfering with SDK state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 20: Parameter Query Isolation Test
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_20_param_query_isolation() {
    println!("\n=== TEST 20: Parameter Query Isolation Test ===");
    println!("Tests whether pl_get_param calls before streaming cause the 19-frame cutoff.\n");

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

    // Open camera
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

    // === PARAMETER QUERIES (like full driver) ===
    println!("\n[PARAM QUERIES] Querying camera parameters like full driver...");
    unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        if pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT,
            &mut ser as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_SER_SIZE: {}", ser);
        }
        if pl_get_param(
            hcam,
            PARAM_PAR_SIZE,
            ATTR_CURRENT,
            &mut par as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_PAR_SIZE: {}", par);
        }

        let mut bit_depth: i16 = 0;
        if pl_get_param(
            hcam,
            PARAM_BIT_DEPTH,
            ATTR_CURRENT,
            &mut bit_depth as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_BIT_DEPTH: {}", bit_depth);
        }

        let mut chip_name = [0i8; 64];
        if pl_get_param(
            hcam,
            PARAM_CHIP_NAME,
            ATTR_CURRENT,
            chip_name.as_mut_ptr() as *mut _,
        ) != 0
        {
            let name = CStr::from_ptr(chip_name.as_ptr()).to_string_lossy();
            println!("  PARAM_CHIP_NAME: {}", name);
        }

        let mut temp: i16 = 0;
        if pl_get_param(
            hcam,
            PARAM_TEMP,
            ATTR_CURRENT,
            &mut temp as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_TEMP: {} (raw)", temp);
        }

        let mut speed_idx: i16 = 0;
        if pl_get_param(
            hcam,
            PARAM_SPDTAB_INDEX,
            ATTR_CURRENT,
            &mut speed_idx as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_SPDTAB_INDEX: {}", speed_idx);
        }

        let mut gain_idx: i16 = 0;
        if pl_get_param(
            hcam,
            PARAM_GAIN_INDEX,
            ATTR_CURRENT,
            &mut gain_idx as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_GAIN_INDEX: {}", gain_idx);
        }

        let mut frame_buf_size: u64 = 0;
        if pl_get_param(
            hcam,
            PARAM_FRAME_BUFFER_SIZE,
            ATTR_CURRENT,
            &mut frame_buf_size as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_FRAME_BUFFER_SIZE: {}", frame_buf_size);
        }

        let mut md_enabled: rs_bool = 0;
        if pl_get_param(
            hcam,
            PARAM_METADATA_ENABLED,
            ATTR_AVAIL,
            &mut md_enabled as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_METADATA_ENABLED (avail): {}", md_enabled);
        }

        let mut circ_avail: rs_bool = 0;
        if pl_get_param(
            hcam,
            PARAM_CIRC_BUFFER,
            ATTR_AVAIL,
            &mut circ_avail as *mut _ as *mut _,
        ) != 0
        {
            println!("  PARAM_CIRC_BUFFER (avail): {}", circ_avail);
        }
    }
    println!("[OK] Parameter queries complete");

    // === REST OF STREAMING SETUP (same as test_18) ===

    let ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created, ptr={:?}", ctx_ptr);

    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!(
                "ERROR: pl_cam_register_callback_ex3 failed: {}",
                get_error_message()
            );
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    let region = rgn_type {
        s1: 0,
        s2: 2047,
        sbin: 1,
        p1: 0,
        p2: 2047,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    println!("[SETUP] Setting up continuous acquisition...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const rgn_type,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    const ALIGN_4K: usize = 4096;
    let buffer_size = (frame_bytes as usize) * BUFFER_FRAMES;
    let layout = Layout::from_size_align(buffer_size, ALIGN_4K).unwrap();
    let buffer = unsafe { alloc(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Allocated {} bytes at {:?}", buffer_size, buffer);

    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as uns32);
        if result == 0 {
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Acquisition started");

    let ctx_clone = ctx.clone();
    let streaming = Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    println!(
        "\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n",
        TARGET_FRAMES
    );
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!(
                    "[TIMEOUT] No frame after {}ms (acquired {})",
                    TIMEOUT_MS, frames_acquired
                );
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    continue;
                }
            }

            frames_acquired += 1;

            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] pl_exp_unlock_oldest_frame failed");
                }
            }

            ctx_clone.consume_one();

            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                println!("[FRAME {}] acquired", frames_acquired);
            }
        }

        frames_acquired
    });

    let frames_acquired = handle.await.unwrap();

    streaming.store(false, Ordering::Release);
    ctx.signal_shutdown();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 20 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: pl_get_param calls are NOT the issue (200 frames achieved)");
    } else {
        println!(
            "RESULT: pl_get_param calls MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. pl_get_param calls may be affecting SDK callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 21: Metadata Enable/Disable Isolation Test
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_21_metadata_isolation() {
    println!("\n=== TEST 21: Metadata Enable/Disable Isolation Test ===");
    println!("Tests whether PARAM_METADATA_ENABLED manipulation causes the 19-frame cutoff.\n");

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

    // Open camera
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

    // === METADATA MANIPULATION (like full driver) ===
    println!("\n[METADATA] Checking and setting PARAM_METADATA_ENABLED...");
    unsafe {
        let mut md_avail: rs_bool = 0;
        if pl_get_param(
            hcam,
            PARAM_METADATA_ENABLED,
            ATTR_AVAIL,
            &mut md_avail as *mut _ as *mut _,
        ) != 0
            && md_avail != 0
        {
            println!("  PARAM_METADATA_ENABLED is available");

            // Read current value
            let mut md_current: rs_bool = 0;
            if pl_get_param(
                hcam,
                PARAM_METADATA_ENABLED,
                ATTR_CURRENT,
                &mut md_current as *mut _ as *mut _,
            ) != 0
            {
                println!("  Current value: {}", md_current);
            }

            // Try to enable metadata (like driver does)
            let enable_val: rs_bool = 1;
            if pl_set_param(
                hcam,
                PARAM_METADATA_ENABLED,
                &enable_val as *const _ as *mut _,
            ) != 0
            {
                println!("  [OK] Metadata ENABLED");
            } else {
                println!(
                    "  [WARN] Failed to enable metadata: {}",
                    get_error_message()
                );
            }

            // Read back
            let mut md_after: rs_bool = 0;
            if pl_get_param(
                hcam,
                PARAM_METADATA_ENABLED,
                ATTR_CURRENT,
                &mut md_after as *mut _ as *mut _,
            ) != 0
            {
                println!("  After setting: {}", md_after);
            }
        } else {
            println!("  PARAM_METADATA_ENABLED not available");
        }
    }

    // === REST IS SAME AS TEST 18/20 ===
    let ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);

    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: callback registration failed");
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    let region = rgn_type {
        s1: 0,
        s2: 2047,
        sbin: 1,
        p1: 0,
        p2: 2047,
        pbin: 1,
    };

    let mut frame_bytes: uns32 = 0;
    println!("[SETUP] Setting up continuous acquisition...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region,
            TIMED_MODE as i16,
            EXPOSURE_MS,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE as i16,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!(
        "[OK] pl_exp_setup_cont succeeded, frame_bytes={}",
        frame_bytes
    );

    let buffer_size = (frame_bytes as usize) * BUFFER_FRAMES;
    let layout = Layout::from_size_align(buffer_size, 4096).unwrap();
    let buffer = unsafe { alloc(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as uns32);
        if result == 0 {
            println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Acquisition started");

    let ctx_clone = ctx.clone();
    let streaming = Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    println!(
        "\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n",
        TARGET_FRAMES
    );
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!(
                    "[TIMEOUT] No frame after {}ms (acquired {})",
                    TIMEOUT_MS, frames_acquired
                );
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    continue;
                }
            }

            frames_acquired += 1;

            unsafe {
                pl_exp_unlock_oldest_frame(hcam);
            }

            ctx_clone.consume_one();

            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                println!("[FRAME {}] acquired", frames_acquired);
            }
        }

        frames_acquired
    });

    let frames_acquired = handle.await.unwrap();

    streaming.store(false, Ordering::Release);
    ctx.signal_shutdown();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 21 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Metadata manipulation is NOT the issue");
    } else {
        println!(
            "RESULT: Metadata manipulation MAY be causing the issue ({} frames)",
            frames_acquired
        );
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Metadata manipulation may be affecting callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

// =============================================================================
// TEST 22: Post-setup circular buffer query
// =============================================================================

#[tokio::test]
#[ignore] // Requires physical PVCAM hardware — run with --ignored on maitai
async fn test_22_post_setup_circ_query() {
    println!("\n=== TEST 22: Post-Setup Circular Buffer Query Test ===");
    println!("Tests whether querying PARAM_CIRC_BUFFER after setup causes issues.\n");

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

    let ctx = Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);

    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: callback registration failed");
            pl_cam_close(hcam);
            pl_pvcam_uninit();
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

    // KEY DIFFERENCE: Query PARAM_CIRC_BUFFER after setup
    println!("[POST-SETUP] Querying PARAM_CIRC_BUFFER after setup...");
    if is_param_available(hcam, PARAM_CIRC_BUFFER) {
        if let Some(value) = get_bool_param(hcam, PARAM_CIRC_BUFFER) {
            println!("  PARAM_CIRC_BUFFER = {} (after setup)", value);
        }
    }

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

    println!("[SETUP] Starting continuous acquisition...");
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

    let ctx_clone = ctx.clone();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let mut frames = 0i32;

        while frames < TARGET_FRAMES {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                eprintln!("[TIMEOUT] at frame {}", frames);
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
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
                pl_exp_unlock_oldest_frame(hcam);
            }
            ctx_clone.consume_one();

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
        FULL_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 22 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Post-setup PARAM_CIRC_BUFFER query may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}
