//! PVCAM Circular Buffer Diagnostic Test
//!
//! This test investigates why CIRC_OVERWRITE fails with error 185 on Prime BSI.
//! It queries camera capabilities and tries different configuration sequences.
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
//!   cd ~/rust-daq && git pull && \
//!   cargo test --release -p daq-driver-pvcam --features "pvcam_hardware" \
//!     --test circ_buffer_diagnostic -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_hardware")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    unused_imports,
    dead_code
)]

use pvcam_sys::*;
use std::ffi::{c_void, CStr, CString};
use std::ptr;

// Buffer mode constants (not exported by bindgen, need manual definition)
const CIRC_OVERWRITE: i16 = 0;
const CIRC_NO_OVERWRITE: i16 = 1;
const TIMED_MODE: i16 = 0;
const EXT_TRIG_INTERNAL: i16 = (7 + 0) << 8; // 1792
const EXPOSE_OUT_FIRST_ROW: i16 = 0;
const CCS_HALT: i16 = 1;
const PL_CALLBACK_EOF: i32 = 1;

// PARAM IDs - use actual values from SDK bindings (verified from generated bindings.rs)
const PARAM_CIRC_BUFFER: u32 = 184746283;      // from bindings.rs
const PARAM_EXPOSURE_MODE: u32 = 151126551;    // from bindings.rs
const PARAM_EXPOSE_OUT_MODE: u32 = 151126585;  // calculated: should be near PARAM_EXPOSURE_MODE
const PARAM_SER_SIZE: u32 = 100794426;         // from bindings.rs
const PARAM_PAR_SIZE: u32 = 100794425;         // from bindings.rs

// Attribute constants for pl_get_param
const ATTR_AVAIL: i16 = 8;
const ATTR_CURRENT: i16 = 0;
const ATTR_COUNT: i16 = 1;
const ATTR_DEFAULT: i16 = 5;  // Key for getting camera's default value!

/// Get PVCAM error message
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// Check if a parameter is available
fn is_param_available(hcam: i16, param_id: u32) -> bool {
    let mut avail: rs_bool = 0;
    unsafe {
        if pl_get_param(hcam, param_id, ATTR_AVAIL, &mut avail as *mut _ as *mut c_void) != 0 {
            avail != 0
        } else {
            false
        }
    }
}

/// Get current value of a boolean parameter
fn get_bool_param(hcam: i16, param_id: u32) -> Option<bool> {
    let mut value: rs_bool = 0;
    unsafe {
        if pl_get_param(hcam, param_id, ATTR_CURRENT, &mut value as *mut _ as *mut c_void) != 0 {
            Some(value != 0)
        } else {
            None
        }
    }
}

/// Get enum count for a parameter
fn get_enum_count(hcam: i16, param_id: u32) -> Option<u32> {
    let mut count: uns32 = 0;
    unsafe {
        if pl_get_param(hcam, param_id, ATTR_COUNT, &mut count as *mut _ as *mut c_void) != 0 {
            Some(count)
        } else {
            None
        }
    }
}

/// Get default value of an i32 parameter (uses ATTR_DEFAULT)
/// This is what PVCamTestCli uses for "<camera default>" values!
fn get_default_i32_param(hcam: i16, param_id: u32) -> Option<i32> {
    let mut value: i32 = 0;
    unsafe {
        if pl_get_param(hcam, param_id, ATTR_DEFAULT, &mut value as *mut _ as *mut c_void) != 0 {
            Some(value)
        } else {
            None
        }
    }
}

/// Get enum entry name and value
fn get_enum_entry(hcam: i16, param_id: u32, index: u32) -> Option<(i32, String)> {
    let mut value: i32 = 0;
    let mut name = [0i8; 100];
    let mut name_len: uns32 = 100;

    unsafe {
        if pl_enum_str_length(hcam, param_id, index, &mut name_len) == 0 {
            return None;
        }
        if pl_get_enum_param(
            hcam,
            param_id,
            index,
            &mut value,
            name.as_mut_ptr(),
            100,
        ) != 0
        {
            let name_str = CStr::from_ptr(name.as_ptr())
                .to_string_lossy()
                .into_owned();
            Some((value, name_str))
        } else {
            None
        }
    }
}

/// Simple EOF callback for testing
extern "system" fn test_eof_callback(_frame_info: *const FRAME_INFO, _context: *mut c_void) {
    // Just log that we received a callback
    eprintln!("[CALLBACK] EOF received");
}

#[tokio::test]
async fn test_circ_buffer_capabilities() {
    println!("\n=== PVCAM Circular Buffer Diagnostic Test ===\n");

    // Initialize PVCAM
    unsafe {
        if pl_pvcam_init() == 0 {
            println!("ERROR: pl_pvcam_init failed: {}", get_error_message());
            return;
        }
    }
    println!("[OK] PVCAM initialized");

    // Get camera count
    let mut cam_count: i16 = 0;
    unsafe {
        if pl_cam_get_total(&mut cam_count) == 0 {
            println!("ERROR: pl_cam_get_total failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Found {} camera(s)", cam_count);

    if cam_count == 0 {
        println!("No cameras found, skipping test");
        unsafe { pl_pvcam_uninit(); }
        return;
    }

    // Get camera name
    let mut cam_name = [0i8; 32];
    unsafe {
        if pl_cam_get_name(0, cam_name.as_mut_ptr()) == 0 {
            println!("ERROR: pl_cam_get_name failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
    }
    let name = unsafe { CStr::from_ptr(cam_name.as_ptr()).to_string_lossy() };
    println!("[OK] Camera name: {}", name);

    // Open camera
    let mut hcam: i16 = -1;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, handle = {}", hcam);

    // ========================================
    // Query PARAM_CIRC_BUFFER
    // ========================================
    println!("\n--- Querying PARAM_CIRC_BUFFER ---");
    if is_param_available(hcam, PARAM_CIRC_BUFFER) {
        println!("[OK] PARAM_CIRC_BUFFER is available");
        if let Some(value) = get_bool_param(hcam, PARAM_CIRC_BUFFER) {
            println!("     Current value: {} (true=supported)", value);
        }
        // Also try to read as an enum to see if it indicates which modes are supported
        if let Some(count) = get_enum_count(hcam, PARAM_CIRC_BUFFER) {
            println!("     Enum count: {}", count);
            for i in 0..count {
                if let Some((value, name)) = get_enum_entry(hcam, PARAM_CIRC_BUFFER, i) {
                    println!("       [{}] {} = {}", i, name, value);
                }
            }
        }
    } else {
        println!("[WARN] PARAM_CIRC_BUFFER is NOT available");
    }

    // ========================================
    // Query PARAM_EXPOSURE_MODE
    // ========================================
    println!("\n--- Querying PARAM_EXPOSURE_MODE ---");
    let mut default_exp_mode: Option<i32> = None;
    if is_param_available(hcam, PARAM_EXPOSURE_MODE) {
        println!("[OK] PARAM_EXPOSURE_MODE is available");

        // Get the DEFAULT value - this is what PVCamTestCli uses!
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_EXPOSURE_MODE) {
            println!("     ATTR_DEFAULT = {} (0x{:04X})", def_val, def_val);
            default_exp_mode = Some(def_val);
            if def_val == TIMED_MODE as i32 {
                println!("     ^ Default is TIMED_MODE");
            } else if def_val == EXT_TRIG_INTERNAL as i32 {
                println!("     ^ Default is EXT_TRIG_INTERNAL");
            } else {
                println!("     ^ Default is an extended mode (not TIMED_MODE)");
            }
        }

        if let Some(count) = get_enum_count(hcam, PARAM_EXPOSURE_MODE) {
            println!("     {} modes available:", count);
            for i in 0..count {
                if let Some((value, name)) = get_enum_entry(hcam, PARAM_EXPOSURE_MODE, i) {
                    let is_default = default_exp_mode == Some(value);
                    let marker = if is_default { " <-- DEFAULT" } else { "" };
                    println!("       [{}] {} = {}{}", i, name, value, marker);
                    // Check if this is TIMED_MODE or EXT_TRIG_INTERNAL
                    if value == TIMED_MODE as i32 {
                        println!("           ^ This is TIMED_MODE (not the default!)");
                    }
                    if value == EXT_TRIG_INTERNAL as i32 {
                        println!("           ^ This is EXT_TRIG_INTERNAL");
                    }
                }
            }
        }
    } else {
        println!("[WARN] PARAM_EXPOSURE_MODE is NOT available");
    }

    // ========================================
    // Query PARAM_EXPOSE_OUT_MODE
    // ========================================
    println!("\n--- Querying PARAM_EXPOSE_OUT_MODE ---");
    if is_param_available(hcam, PARAM_EXPOSE_OUT_MODE) {
        println!("[OK] PARAM_EXPOSE_OUT_MODE is available");
        if let Some(count) = get_enum_count(hcam, PARAM_EXPOSE_OUT_MODE) {
            println!("     {} modes available:", count);
            for i in 0..count {
                if let Some((value, name)) = get_enum_entry(hcam, PARAM_EXPOSE_OUT_MODE, i) {
                    println!("       [{}] {} = {}", i, name, value);
                }
            }
        }
    } else {
        println!("[WARN] PARAM_EXPOSE_OUT_MODE is NOT available");
    }

    // ========================================
    // Test 1: SDK Example Order (callback → setup → start) with CIRC_OVERWRITE
    // ========================================
    println!("\n--- Test 1: SDK Order (callback first) + CIRC_OVERWRITE ---");

    // Register callback FIRST (like SDK example)
    let mut callback_registered = false;
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            test_eof_callback as *mut c_void,
            ptr::null_mut(),
        );
        if result != 0 {
            println!("[OK] Callback registered BEFORE setup");
            callback_registered = true;
        } else {
            println!("[WARN] Failed to register callback: {}", get_error_message());
        }
    }

    // Setup region (full frame, but small for testing)
    let region = rgn_type {
        s1: 0,
        s2: 511,  // 512 pixels wide
        sbin: 1,
        p1: 0,
        p2: 511,  // 512 pixels tall
        pbin: 1,
    };

    let exposure_ms: u32 = 10;
    let mut frame_bytes: uns32 = 0;

    // Try pl_exp_setup_cont with CIRC_OVERWRITE
    let exp_mode = TIMED_MODE;
    println!("     Using exp_mode = TIMED_MODE ({})", exp_mode);

    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        );
        if result != 0 {
            println!("[OK] pl_exp_setup_cont with CIRC_OVERWRITE succeeded");
            println!("     frame_bytes = {}", frame_bytes);

            // Allocate buffer
            let buffer_count = 20usize;
            let buffer_size = (frame_bytes as usize) * buffer_count;
            let mut buffer = vec![0u8; buffer_size];

            // Try pl_exp_start_cont
            let start_result = pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            );
            if start_result != 0 {
                println!("[OK] pl_exp_start_cont with CIRC_OVERWRITE succeeded!");
                println!("\n*** CIRC_OVERWRITE WORKS! ***\n");

                // Stop acquisition
                pl_exp_abort(hcam, CCS_HALT);
            } else {
                println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }
        } else {
            println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
        }
    }

    // Deregister callback
    if callback_registered {
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        }
    }

    // ========================================
    // Test 2: Try EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW
    // ========================================
    println!("\n--- Test 2: EXT_TRIG_INTERNAL | EXPOSE_OUT mode + CIRC_OVERWRITE ---");

    // Register callback first
    callback_registered = false;
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            test_eof_callback as *mut c_void,
            ptr::null_mut(),
        );
        if result != 0 {
            callback_registered = true;
        }
    }

    let exp_mode_ext = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW;
    println!("     Using exp_mode = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW ({})", exp_mode_ext);

    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode_ext,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        );
        if result != 0 {
            println!("[OK] pl_exp_setup_cont succeeded");

            let buffer_count = 20usize;
            let buffer_size = (frame_bytes as usize) * buffer_count;
            let mut buffer = vec![0u8; buffer_size];

            let start_result = pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            );
            if start_result != 0 {
                println!("[OK] pl_exp_start_cont with EXT_TRIG_INTERNAL + CIRC_OVERWRITE succeeded!");
                pl_exp_abort(hcam, CCS_HALT);
            } else {
                println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }
        } else {
            println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
        }
    }

    if callback_registered {
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        }
    }

    // ========================================
    // Test 3: NO callback + CIRC_OVERWRITE
    // ========================================
    println!("\n--- Test 3: NO callback + CIRC_OVERWRITE ---");

    let exp_mode = TIMED_MODE;
    println!("     Using exp_mode = TIMED_MODE, NO callback");

    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        );
        if result != 0 {
            println!("[OK] pl_exp_setup_cont succeeded");

            let buffer_count = 20usize;
            let buffer_size = (frame_bytes as usize) * buffer_count;
            let mut buffer = vec![0u8; buffer_size];

            let start_result = pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            );
            if start_result != 0 {
                println!("[OK] pl_exp_start_cont WITHOUT callback + CIRC_OVERWRITE succeeded!");
                pl_exp_abort(hcam, CCS_HALT);
            } else {
                println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }
        } else {
            println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
        }
    }

    // ========================================
    // Test 4: Our current order (setup → callback → start) with CIRC_OVERWRITE
    // ========================================
    println!("\n--- Test 4: Our Order (setup → callback → start) + CIRC_OVERWRITE ---");

    let exp_mode = TIMED_MODE;

    unsafe {
        // Setup FIRST
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_OVERWRITE,
        );
        if result != 0 {
            println!("[OK] pl_exp_setup_cont succeeded");

            // Then register callback
            callback_registered = false;
            let cb_result = pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                test_eof_callback as *mut c_void,
                ptr::null_mut(),
            );
            if cb_result != 0 {
                println!("[OK] Callback registered AFTER setup");
                callback_registered = true;
            }

            let buffer_count = 20usize;
            let buffer_size = (frame_bytes as usize) * buffer_count;
            let mut buffer = vec![0u8; buffer_size];

            // Then start
            let start_result = pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            );
            if start_result != 0 {
                println!("[OK] Our order + CIRC_OVERWRITE succeeded!");
                pl_exp_abort(hcam, CCS_HALT);
            } else {
                println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }

            if callback_registered {
                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            }
        } else {
            println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
        }
    }

    // ========================================
    // Test 5: CIRC_NO_OVERWRITE (control test - should work)
    // ========================================
    println!("\n--- Test 5: CIRC_NO_OVERWRITE (control test) ---");

    // Register callback first
    callback_registered = false;
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            test_eof_callback as *mut c_void,
            ptr::null_mut(),
        );
        if result != 0 {
            callback_registered = true;
            println!("[OK] Callback registered");
        }
    }

    let exp_mode = TIMED_MODE;
    println!("     Using exp_mode = TIMED_MODE, CIRC_NO_OVERWRITE");

    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const _,
            exp_mode,
            exposure_ms,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        );
        if result != 0 {
            println!("[OK] pl_exp_setup_cont succeeded");

            let buffer_count = 20usize;
            let buffer_size = (frame_bytes as usize) * buffer_count;
            let mut buffer = vec![0u8; buffer_size];

            let start_result = pl_exp_start_cont(
                hcam,
                buffer.as_mut_ptr() as *mut c_void,
                buffer_size as uns32,
            );
            if start_result != 0 {
                println!("[OK] pl_exp_start_cont with CIRC_NO_OVERWRITE succeeded!");
                println!("\n*** CIRC_NO_OVERWRITE works (as expected) ***\n");
                pl_exp_abort(hcam, CCS_HALT);
            } else {
                println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
            }
        } else {
            println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
        }
    }

    if callback_registered {
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        }
    }

    // ========================================
    // Test 6: Camera DEFAULT exposure mode + CIRC_OVERWRITE (like PVCamTestCli!)
    // ========================================
    println!("\n--- Test 6: Camera DEFAULT exposure mode + CIRC_OVERWRITE ---");
    println!("     This is what PVCamTestCli uses with '<camera default>'!");

    if let Some(def_mode) = default_exp_mode {
        println!("     Using exp_mode = {} (camera's ATTR_DEFAULT value)", def_mode);

        // Register callback first (like SDK example)
        callback_registered = false;
        unsafe {
            let result = pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                test_eof_callback as *mut c_void,
                ptr::null_mut(),
            );
            if result != 0 {
                callback_registered = true;
                println!("[OK] Callback registered");
            }
        }

        unsafe {
            let result = pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                def_mode as i16,  // Use camera's default mode!
                exposure_ms,
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );
            if result != 0 {
                println!("[OK] pl_exp_setup_cont with DEFAULT mode + CIRC_OVERWRITE succeeded");

                let buffer_count = 20usize;
                let buffer_size = (frame_bytes as usize) * buffer_count;
                let mut buffer = vec![0u8; buffer_size];

                let start_result = pl_exp_start_cont(
                    hcam,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer_size as uns32,
                );
                if start_result != 0 {
                    println!("[OK] pl_exp_start_cont with DEFAULT mode + CIRC_OVERWRITE succeeded!");
                    println!("\n*** CAMERA DEFAULT MODE + CIRC_OVERWRITE WORKS! ***");
                    println!("*** This is the solution - use camera's default exposure mode! ***\n");
                    pl_exp_abort(hcam, CCS_HALT);
                } else {
                    println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                    let err_code = pl_error_code();
                    println!("       Error code: {}", err_code);
                }
            } else {
                println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
            }
        }

        if callback_registered {
            unsafe {
                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            }
        }
    } else {
        println!("[SKIP] Could not get default exposure mode");
    }

    // ========================================
    // Test 7: Camera DEFAULT mode WITHOUT callback + CIRC_OVERWRITE
    // ========================================
    println!("\n--- Test 7: Camera DEFAULT mode WITHOUT callback + CIRC_OVERWRITE ---");

    if let Some(def_mode) = default_exp_mode {
        println!("     Using exp_mode = {} (camera's ATTR_DEFAULT value), NO callback", def_mode);

        unsafe {
            let result = pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                def_mode as i16,
                exposure_ms,
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );
            if result != 0 {
                println!("[OK] pl_exp_setup_cont succeeded");

                let buffer_count = 20usize;
                let buffer_size = (frame_bytes as usize) * buffer_count;
                let mut buffer = vec![0u8; buffer_size];

                let start_result = pl_exp_start_cont(
                    hcam,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer_size as uns32,
                );
                if start_result != 0 {
                    println!("[OK] DEFAULT mode WITHOUT callback + CIRC_OVERWRITE succeeded!");
                    pl_exp_abort(hcam, CCS_HALT);
                } else {
                    println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                    let err_code = pl_error_code();
                    println!("       Error code: {}", err_code);
                }
            } else {
                println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
            }
        }
    } else {
        println!("[SKIP] Could not get default exposure mode");
    }

    // ========================================
    // Query some common parameters to see what's available
    // ========================================
    println!("\n--- Checking other relevant parameters ---");

    // Use the constants we've defined above (correct values from bindings)
    let params_to_check: &[(u32, &str)] = &[
        (PARAM_SER_SIZE, "PARAM_SER_SIZE"),
        (PARAM_PAR_SIZE, "PARAM_PAR_SIZE"),
        (PARAM_EXPOSURE_MODE, "PARAM_EXPOSURE_MODE"),
        (PARAM_CIRC_BUFFER, "PARAM_CIRC_BUFFER"),
    ];

    for (param_id, name) in params_to_check {
        let avail = is_param_available(hcam, *param_id);
        println!("  {} (0x{:08X}): {}", name, param_id, if avail { "available" } else { "NOT available" });

        // If PARAM_SER_SIZE or PARAM_PAR_SIZE is available, read its value
        if avail && (*param_id == PARAM_SER_SIZE || *param_id == PARAM_PAR_SIZE) {
            let mut value: uns16 = 0;
            unsafe {
                if pl_get_param(hcam, *param_id, ATTR_CURRENT, &mut value as *mut _ as *mut c_void) != 0 {
                    println!("      Value: {}", value);
                }
            }
        }
    }

    // Cleanup
    println!("\n--- Cleanup ---");
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
    println!("[OK] Camera closed and PVCAM uninitialized");

    println!("\n=== Diagnostic Test Complete ===\n");
}
