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
//!   cargo test --release -p daq-driver-pvcam --features "pvcam_sdk" \
//!     --test circ_buffer_diagnostic -- --nocapture --test-threads=1'
//! ```

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "pvcam_sdk")]
#![allow(clippy::unwrap_used, clippy::expect_used, unused_imports, dead_code)]

use pvcam_sys::*;
use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::ffi::{c_void, CStr, CString};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::AtomicI16;

// Use constants from pvcam_sys (CIRC_OVERWRITE, CIRC_NO_OVERWRITE, TIMED_MODE,
// EXT_TRIG_INTERNAL, EXPOSE_OUT_FIRST_ROW, CCS_HALT, PL_CALLBACK_EOF,
// ATTR_AVAIL, ATTR_CURRENT, ATTR_COUNT, ATTR_DEFAULT)

// PARAM IDs - camera-specific values verified on maitai (not in pvcam_sys)
const PARAM_CIRC_BUFFER: u32 = 184746283;
const PARAM_EXPOSURE_MODE: u32 = 151126551;
const PARAM_EXPOSE_OUT_MODE: u32 = 151126576;
const PARAM_SER_SIZE: u32 = 100794426;
const PARAM_PAR_SIZE: u32 = 100794425;
const PARAM_READOUT_PORT: u32 = 151126263;
const PARAM_SPDTAB_INDEX: u32 = 16908801;
const PARAM_GAIN_INDEX: u32 = 16908800;
const PARAM_BIT_DEPTH: u32 = 16908799;
const PARAM_FRAME_BUFFER_SIZE: u32 = 184746284;

/// Get PVCAM error message
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Check if a parameter is available
fn is_param_available(hcam: i16, param_id: u32) -> bool {
    let mut avail: rs_bool = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_AVAIL,
            &mut avail as *mut _ as *mut c_void,
        ) != 0
        {
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
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
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
        if pl_get_param(
            hcam,
            param_id,
            ATTR_COUNT,
            &mut count as *mut _ as *mut c_void,
        ) != 0
        {
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
        if pl_get_param(
            hcam,
            param_id,
            ATTR_DEFAULT,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value)
        } else {
            None
        }
    }
}

/// Set an i32 parameter value
fn set_i32_param(hcam: i16, param_id: u32, value: i32) -> bool {
    unsafe {
        // pl_set_param takes *mut c_void - cast through raw pointer
        let value_ptr = &value as *const i32 as *mut c_void;
        pl_set_param(hcam, param_id, value_ptr) != 0
    }
}

/// Mimic SDK BuildSpeedTable() - set readout parameters to their defaults
/// The SDK does this in Camera::Open BEFORE any acquisition setup
fn init_readout_params_like_sdk(hcam: i16) -> bool {
    println!("  Setting PARAM_READOUT_PORT to default...");
    if is_param_available(hcam, PARAM_READOUT_PORT) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_READOUT_PORT) {
            if set_i32_param(hcam, PARAM_READOUT_PORT, def_val) {
                println!("    [OK] PARAM_READOUT_PORT = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_READOUT_PORT: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_READOUT_PORT not available");
    }

    println!("  Setting PARAM_SPDTAB_INDEX to default...");
    if is_param_available(hcam, PARAM_SPDTAB_INDEX) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_SPDTAB_INDEX) {
            if set_i32_param(hcam, PARAM_SPDTAB_INDEX, def_val) {
                println!("    [OK] PARAM_SPDTAB_INDEX = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_SPDTAB_INDEX: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_SPDTAB_INDEX not available");
    }

    println!("  Setting PARAM_GAIN_INDEX to default...");
    if is_param_available(hcam, PARAM_GAIN_INDEX) {
        if let Some(def_val) = get_default_i32_param(hcam, PARAM_GAIN_INDEX) {
            if set_i32_param(hcam, PARAM_GAIN_INDEX, def_val) {
                println!("    [OK] PARAM_GAIN_INDEX = {}", def_val);
            } else {
                println!(
                    "    [WARN] Failed to set PARAM_GAIN_INDEX: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("    [SKIP] PARAM_GAIN_INDEX not available");
    }

    // Check BIT_DEPTH to see current state
    println!("  Checking PARAM_BIT_DEPTH...");
    if is_param_available(hcam, PARAM_BIT_DEPTH) {
        let mut bit_depth: i32 = 0;
        unsafe {
            if pl_get_param(
                hcam,
                PARAM_BIT_DEPTH,
                ATTR_CURRENT,
                &mut bit_depth as *mut _ as *mut c_void,
            ) != 0
            {
                println!("    [OK] PARAM_BIT_DEPTH = {} bits", bit_depth);
            }
        }
    }

    true
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
        if pl_get_enum_param(hcam, param_id, index, &mut value, name.as_mut_ptr(), 100) != 0 {
            let name_str = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
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
        unsafe {
            pl_pvcam_uninit();
        }
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
            println!(
                "[WARN] Failed to register callback: {}",
                get_error_message()
            );
        }
    }

    // Setup region (full frame, but small for testing)
    let region = rgn_type {
        s1: 0,
        s2: 511, // 512 pixels wide
        sbin: 1,
        p1: 0,
        p2: 511, // 512 pixels tall
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
    println!(
        "     Using exp_mode = EXT_TRIG_INTERNAL | EXPOSE_OUT_FIRST_ROW ({})",
        exp_mode_ext
    );

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
                println!(
                    "[OK] pl_exp_start_cont with EXT_TRIG_INTERNAL + CIRC_OVERWRITE succeeded!"
                );
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
        println!(
            "     Using exp_mode = {} (camera's ATTR_DEFAULT value)",
            def_mode
        );

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
                def_mode as i16, // Use camera's default mode!
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
                    println!(
                        "[OK] pl_exp_start_cont with DEFAULT mode + CIRC_OVERWRITE succeeded!"
                    );
                    println!("\n*** CAMERA DEFAULT MODE + CIRC_OVERWRITE WORKS! ***");
                    println!(
                        "*** This is the solution - use camera's default exposure mode! ***\n"
                    );
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
        println!(
            "     Using exp_mode = {} (camera's ATTR_DEFAULT value), NO callback",
            def_mode
        );

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
    // Test 8: 4KB ALIGNED buffer + 50 frames + camera default mode + NO callback
    // This matches EXACTLY what PVCamTestCli uses:
    // - align4k allocator (4096 byte alignment)
    // - 50 frame buffer (--buffer-frames default)
    // - camera default exposure mode
    // ========================================
    println!("\n--- Test 8: 4KB ALIGNED buffer + 50 frames + DEFAULT mode + NO callback ---");
    println!("     This matches PVCamTestCli defaults exactly!");

    if let Some(def_mode) = default_exp_mode {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 50; // SDK default

        println!("     exp_mode = {} (camera default)", def_mode);
        println!("     frame_count = {} (SDK default)", FRAME_COUNT);
        println!("     alignment = {} bytes (SDK default)", ALIGN_4K);

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
                println!(
                    "[OK] pl_exp_setup_cont succeeded, frame_bytes = {}",
                    frame_bytes
                );

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                // Round up to 4KB boundary
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if buffer_ptr.is_null() {
                    println!("[FAIL] Failed to allocate 4KB-aligned buffer");
                } else {
                    let ptr_val = buffer_ptr as usize;
                    println!(
                        "     Buffer allocated: {} bytes at 0x{:X}",
                        aligned_size, ptr_val
                    );
                    println!(
                        "     Alignment check: 0x{:X} % 4096 = {}",
                        ptr_val,
                        ptr_val % ALIGN_4K
                    );

                    let start_result = pl_exp_start_cont(
                        hcam,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32, // Original size, not padded
                    );

                    if start_result != 0 {
                        println!("[OK] pl_exp_start_cont with 4KB ALIGNED buffer + CIRC_OVERWRITE succeeded!");
                        println!("\n*** 4KB ALIGNMENT WORKS! ***");
                        println!("*** SOLUTION: Use 4KB aligned buffers for CIRC_OVERWRITE! ***\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    // Free the aligned buffer
                    dealloc(buffer_ptr, layout);
                }
            } else {
                println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
            }
        }
    } else {
        println!("[SKIP] Could not get default exposure mode");
    }

    // ========================================
    // Test 9: 4KB ALIGNED buffer + 50 frames + DEFAULT mode + WITH callback
    // Complete SDK-style setup
    // ========================================
    println!("\n--- Test 9: 4KB ALIGNED buffer + 50 frames + DEFAULT mode + WITH callback ---");

    if let Some(def_mode) = default_exp_mode {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 50;

        // Register callback
        callback_registered = false;
        unsafe {
            let result = pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                test_eof_callback as *mut c_void,
                ptr::null_mut(),
            );
            if result != 0 {
                println!("[OK] Callback registered");
                callback_registered = true;
            } else {
                println!(
                    "[WARN] Failed to register callback: {}",
                    get_error_message()
                );
            }
        }

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

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result =
                        pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32);

                    if start_result != 0 {
                        println!("[OK] 4KB ALIGNED buffer + callback + CIRC_OVERWRITE succeeded!");
                        println!("\n*** COMPLETE SDK-STYLE SETUP WORKS! ***\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                } else {
                    println!("[FAIL] Failed to allocate aligned buffer");
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
    // Test 10: FULL FRAME (2048x2048) + 4KB aligned + DEFAULT mode + NO callback
    // PVCamTestCli uses full sensor resolution. Maybe ROI size matters?
    // ========================================
    println!("\n--- Test 10: FULL FRAME (2048x2048) + 4KB aligned + DEFAULT mode ---");
    println!("     Matching PVCamTestCli exactly: full sensor, 4KB align, 50 frames");

    if let Some(def_mode) = default_exp_mode {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 50;

        // Use full sensor resolution like PVCamTestCli
        let full_region = rgn_type {
            s1: 0,
            s2: 2047, // 2048 pixels wide (0-2047)
            sbin: 1,
            p1: 0,
            p2: 2047, // 2048 pixels tall (0-2047)
            pbin: 1,
        };

        println!("     Region: full sensor 2048x2048");
        println!("     exp_mode = {} (camera default)", def_mode);

        unsafe {
            let result = pl_exp_setup_cont(
                hcam,
                1,
                &full_region as *const _,
                def_mode as i16,
                exposure_ms,
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );
            if result != 0 {
                println!(
                    "[OK] pl_exp_setup_cont succeeded, frame_bytes = {}",
                    frame_bytes
                );

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result =
                        pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32);

                    if start_result != 0 {
                        println!("[OK] FULL FRAME + 4KB aligned + CIRC_OVERWRITE succeeded!");
                        println!("\n*** FULL FRAME WORKS! ROI SIZE MATTERS! ***\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                } else {
                    println!("[FAIL] Failed to allocate buffer");
                }
            } else {
                println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
            }
        }
    } else {
        println!("[SKIP] Could not get default exposure mode");
    }

    // ========================================
    // Test 11: SDK-STYLE INITIALIZATION (BuildSpeedTable) + CIRC_OVERWRITE
    // The SDK calls BuildSpeedTable() which sets PARAM_READOUT_PORT,
    // PARAM_SPDTAB_INDEX, and PARAM_GAIN_INDEX to their defaults.
    // This might be the missing initialization step!
    // ========================================
    println!("\n--- Test 11: SDK-style init (BuildSpeedTable) + CIRC_OVERWRITE ---");
    println!("     This mimics SDK Camera::Open → BuildSpeedTable()");
    println!(
        "     Setting PARAM_READOUT_PORT, PARAM_SPDTAB_INDEX, PARAM_GAIN_INDEX to defaults...\n"
    );

    // Initialize readout parameters like SDK does
    init_readout_params_like_sdk(hcam);

    if let Some(def_mode) = default_exp_mode {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 50;

        println!("\n     Now trying CIRC_OVERWRITE with SDK-initialized camera...");
        println!("     exp_mode = {} (camera default)", def_mode);

        // Register callback (like SDK StartExp)
        let mut callback_registered_11 = false;
        unsafe {
            let result = pl_cam_register_callback_ex3(
                hcam,
                PL_CALLBACK_EOF,
                test_eof_callback as *mut c_void,
                ptr::null_mut(),
            );
            if result != 0 {
                println!("[OK] Callback registered");
                callback_registered_11 = true;
            }
        }

        // Use 512x512 region first (smaller for quick test)
        let region = rgn_type {
            s1: 0,
            s2: 511,
            sbin: 1,
            p1: 0,
            p2: 511,
            pbin: 1,
        };

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
                println!(
                    "[OK] pl_exp_setup_cont succeeded, frame_bytes = {}",
                    frame_bytes
                );

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    let start_result =
                        pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32);

                    if start_result != 0 {
                        println!("[OK] SDK-style init + CIRC_OVERWRITE SUCCEEDED!");
                        println!("\n*********************************************");
                        println!("*** SOLUTION: Initialize readout params! ***");
                        println!("*** BuildSpeedTable() is the missing step ***");
                        println!("*********************************************\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!("[FAIL] pl_exp_start_cont failed: {}", get_error_message());
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                } else {
                    println!("[FAIL] Failed to allocate buffer");
                }
            } else {
                println!("[FAIL] pl_exp_setup_cont failed: {}", get_error_message());
            }
        }

        if callback_registered_11 {
            unsafe {
                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
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
        println!(
            "  {} (0x{:08X}): {}",
            name,
            param_id,
            if avail { "available" } else { "NOT available" }
        );

        // If PARAM_SER_SIZE or PARAM_PAR_SIZE is available, read its value
        if avail && (*param_id == PARAM_SER_SIZE || *param_id == PARAM_PAR_SIZE) {
            let mut value: uns16 = 0;
            unsafe {
                if pl_get_param(
                    hcam,
                    *param_id,
                    ATTR_CURRENT,
                    &mut value as *mut _ as *mut c_void,
                ) != 0
                {
                    println!("      Value: {}", value);
                }
            }
        }
    }

    // ========================================
    // Test 12: CORRECT SDK ORDER - setup_cont THEN register_callback THEN start_cont
    // ========================================
    // The SDK does:
    //   1. pl_exp_setup_cont (SetupExp)
    //   2. pl_cam_register_callback_ex3 (StartExp, AFTER setup!)
    //   3. pl_exp_start_cont (StartExp)
    // Our tests have been registering the callback BEFORE setup - that may be the issue!
    println!("\n--- Test 12: CORRECT SDK ORDER (setup → callback → start) ---");
    println!("     SDK registers callback AFTER setup_cont, BEFORE start_cont!");
    println!("     All previous tests registered callback BEFORE setup - testing fix...\n");

    if let Some(def_mode) = default_exp_mode {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 100; // Use 100 frames like PVCamTestCli --buffer-frames=100

        let region = rgn_type {
            s1: 0,
            s2: 511, // 512x512 for quick test
            sbin: 1,
            p1: 0,
            p2: 511,
            pbin: 1,
        };

        println!("     exp_mode = {} (camera default)", def_mode);
        println!("     region = 512x512");
        println!("     frame_count = {}", FRAME_COUNT);

        unsafe {
            // Step 1: SETUP FIRST (no callback yet!)
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
                println!(
                    "[OK] Step 1: pl_exp_setup_cont succeeded, frame_bytes = {}",
                    frame_bytes
                );

                // Step 2: REGISTER CALLBACK AFTER SETUP (SDK order!)
                let callback_result = pl_cam_register_callback_ex3(
                    hcam,
                    PL_CALLBACK_EOF,
                    test_eof_callback as *mut c_void,
                    ptr::null_mut(),
                );
                if callback_result != 0 {
                    println!("[OK] Step 2: Callback registered AFTER setup (SDK order!)");
                } else {
                    println!(
                        "[WARN] Failed to register callback: {}",
                        get_error_message()
                    );
                }

                // Allocate 4KB-aligned buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    // Step 3: START (after callback registration)
                    let start_result =
                        pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32);

                    if start_result != 0 {
                        println!("[OK] Step 3: pl_exp_start_cont SUCCEEDED!");
                        println!("\n*********************************************");
                        println!("*** SOLUTION FOUND: CALLBACK ORDER MATTERS! ***");
                        println!("*** Register callback AFTER setup_cont! ***");
                        println!("*********************************************\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!(
                            "[FAIL] Step 3: pl_exp_start_cont failed: {}",
                            get_error_message()
                        );
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                } else {
                    println!("[FAIL] Failed to allocate buffer");
                }

                // Cleanup callback
                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            } else {
                println!(
                    "[FAIL] Step 1: pl_exp_setup_cont failed: {}",
                    get_error_message()
                );
            }
        }
    } else {
        println!("[SKIP] Could not get default exposure mode");
    }

    // ========================================
    // Test 13: Correct SDK order + TIMED_MODE (0) instead of camera default
    // ========================================
    println!("\n--- Test 13: SDK ORDER + TIMED_MODE (0) ---");
    println!("     PVCamTestCli default trigger mode is TIMED_MODE (0), not EXT_TRIG_INTERNAL");
    println!("     Testing with explicit TIMED_MODE = 0...\n");

    {
        const ALIGN_4K: usize = 4096;
        const FRAME_COUNT: usize = 100;

        let region = rgn_type {
            s1: 0,
            s2: 511,
            sbin: 1,
            p1: 0,
            p2: 511,
            pbin: 1,
        };

        println!("     exp_mode = TIMED_MODE (0)");

        unsafe {
            // Step 1: SETUP with TIMED_MODE = 0
            let result = pl_exp_setup_cont(
                hcam,
                1,
                &region as *const _,
                TIMED_MODE, // Explicit 0, like PVCamTestCli default
                exposure_ms,
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );
            if result != 0 {
                println!("[OK] Step 1: pl_exp_setup_cont with TIMED_MODE succeeded");

                // Step 2: Register callback AFTER setup
                let callback_result = pl_cam_register_callback_ex3(
                    hcam,
                    PL_CALLBACK_EOF,
                    test_eof_callback as *mut c_void,
                    ptr::null_mut(),
                );
                if callback_result != 0 {
                    println!("[OK] Step 2: Callback registered after setup");
                }

                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);

                let layout =
                    Layout::from_size_align(aligned_size, ALIGN_4K).expect("Invalid layout");
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    // Step 3: START
                    let start_result =
                        pl_exp_start_cont(hcam, buffer_ptr as *mut c_void, buffer_size as uns32);

                    if start_result != 0 {
                        println!("[OK] Step 3: TIMED_MODE + SDK order WORKS!");
                        println!("\n*** TIMED_MODE with correct callback order works! ***\n");
                        pl_exp_abort(hcam, CCS_HALT);
                    } else {
                        println!(
                            "[FAIL] Step 3: pl_exp_start_cont failed: {}",
                            get_error_message()
                        );
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                }

                pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            } else {
                println!(
                    "[FAIL] Step 1: pl_exp_setup_cont failed: {}",
                    get_error_message()
                );
            }
        }
    }

    // ========================================
    // Test 14: FRESH CAMERA + frame_info_struct (mimic exact SDK Open sequence)
    // ========================================
    println!("\n--- Test 14: FRESH CAMERA + frame_info_struct + CIRC_OVERWRITE ---");
    println!("     Close/reopen camera, create frame_info_struct, single CIRC_OVERWRITE attempt");
    println!("     This tests if accumulated state from previous tests causes issues\n");

    // Close current camera
    unsafe {
        pl_cam_close(hcam);
    }
    println!("[OK] Camera closed");

    // Reopen fresh
    let mut hcam_fresh: i16 = -1;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam_fresh, 0) == 0 {
            println!("[FAIL] Failed to reopen camera: {}", get_error_message());
        } else {
            println!("[OK] Camera reopened fresh, handle = {}", hcam_fresh);

            // Create frame_info_struct like SDK does in Open
            let mut frame_info: *mut FRAME_INFO = ptr::null_mut();
            if pl_create_frame_info_struct(&mut frame_info) != 0 {
                println!("[OK] frame_info_struct created (SDK Open requirement)");
            } else {
                println!(
                    "[WARN] Failed to create frame_info_struct: {}",
                    get_error_message()
                );
            }

            // Now try CIRC_OVERWRITE with fresh state
            const ALIGN_4K: usize = 4096;
            const FRAME_COUNT: usize = 100;

            let region = rgn_type {
                s1: 0,
                s2: 2047, // Full frame
                sbin: 1,
                p1: 0,
                p2: 2047,
                pbin: 1,
            };

            println!("     Region: full sensor 2048x2048");
            println!("     exp_mode = TIMED_MODE (0)");

            // Step 1: Setup FIRST
            let mut frame_bytes: uns32 = 0;
            let setup_result = pl_exp_setup_cont(
                hcam_fresh,
                1,
                &region as *const _,
                TIMED_MODE,
                20, // 20ms exposure
                &mut frame_bytes,
                CIRC_OVERWRITE,
            );

            if setup_result != 0 {
                println!(
                    "[OK] Step 1: pl_exp_setup_cont succeeded, frame_bytes = {}",
                    frame_bytes
                );

                // Step 2: Register callback AFTER setup
                let callback_result = pl_cam_register_callback_ex3(
                    hcam_fresh,
                    PL_CALLBACK_EOF,
                    test_eof_callback as *mut c_void,
                    ptr::null_mut(),
                );
                if callback_result != 0 {
                    println!("[OK] Step 2: Callback registered after setup");
                }

                // Allocate buffer
                let buffer_size = (frame_bytes as usize) * FRAME_COUNT;
                let aligned_size = (buffer_size + (ALIGN_4K - 1)) & !(ALIGN_4K - 1);
                let layout = Layout::from_size_align(aligned_size, ALIGN_4K).unwrap();
                let buffer_ptr = alloc_zeroed(layout);

                if !buffer_ptr.is_null() {
                    // Step 3: Start
                    let start_result = pl_exp_start_cont(
                        hcam_fresh,
                        buffer_ptr as *mut c_void,
                        buffer_size as uns32,
                    );

                    if start_result != 0 {
                        println!("[OK] Step 3: pl_exp_start_cont SUCCEEDED!");
                        println!("\n*********************************************");
                        println!("*** SOLUTION: Fresh camera + frame_info! ***");
                        println!("*********************************************\n");
                        pl_exp_abort(hcam_fresh, CCS_HALT);
                    } else {
                        println!(
                            "[FAIL] Step 3: pl_exp_start_cont failed: {}",
                            get_error_message()
                        );
                        let err_code = pl_error_code();
                        println!("       Error code: {}", err_code);
                    }

                    dealloc(buffer_ptr, layout);
                }

                pl_cam_deregister_callback(hcam_fresh, PL_CALLBACK_EOF);
            } else {
                println!(
                    "[FAIL] Step 1: pl_exp_setup_cont failed: {}",
                    get_error_message()
                );
            }

            // Release frame_info_struct
            if !frame_info.is_null() {
                pl_release_frame_info_struct(frame_info);
            }

            pl_cam_close(hcam_fresh);
        }
    }

    // ========================================
    // Test 15: SEQUENCE MODE with EXT_TRIG_INTERNAL (camera default)
    // Camera default is EXT_TRIG_INTERNAL (1792), not TIMED_MODE (0).
    // Internal trigger should self-fire exposures automatically.
    // ========================================
    println!("\n--- Test 15: SEQUENCE MODE with EXT_TRIG_INTERNAL (camera default) ---");
    println!("     Testing sequence mode with camera's default exposure mode");
    println!("     EXT_TRIG_INTERNAL = 1792 (should auto-trigger exposures)\n");

    // Reopen camera for clean state
    let mut hcam_seq: i16 = -1;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam_seq, 0) == 0 {
            println!("[FAIL] Failed to reopen camera: {}", get_error_message());
        } else {
            println!(
                "[OK] Camera opened for sequence mode test, handle = {}",
                hcam_seq
            );

            // Create frame_info_struct
            let mut frame_info: *mut FRAME_INFO = ptr::null_mut();
            let _ = pl_create_frame_info_struct(&mut frame_info);

            const FRAME_COUNT: uns16 = 3; // Capture 3 frames
            const EXPOSURE_MS: uns32 = 100; // Longer exposure to ensure completion

            let region = rgn_type {
                s1: 0,
                s2: 255, // 256x256 for quick test
                sbin: 1,
                p1: 0,
                p2: 255,
                pbin: 1,
            };

            println!("     Region: 256x256");
            println!("     Frame count: {}", FRAME_COUNT);
            println!("     Exposure: {}ms", EXPOSURE_MS);

            // Setup sequence acquisition with EXT_TRIG_INTERNAL (camera's default)
            let mut buffer_bytes: uns32 = 0;
            let setup_result = pl_exp_setup_seq(
                hcam_seq,
                FRAME_COUNT,
                1, // region count
                &region as *const _,
                EXT_TRIG_INTERNAL, // Camera's default mode (1792)
                EXPOSURE_MS,
                &mut buffer_bytes,
            );

            if setup_result != 0 {
                println!(
                    "[OK] pl_exp_setup_seq with EXT_TRIG_INTERNAL succeeded, buffer_bytes = {}",
                    buffer_bytes
                );

                // Allocate buffer for all frames
                let mut buffer = vec![0u8; buffer_bytes as usize];

                // Start sequence acquisition
                let start_result = pl_exp_start_seq(hcam_seq, buffer.as_mut_ptr() as *mut c_void);

                if start_result != 0 {
                    println!("[OK] pl_exp_start_seq succeeded!");

                    // Poll for completion with status monitoring
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut last_status: i16 = -1;
                    let mut frame_count = 0;
                    let start_time = std::time::Instant::now();
                    let timeout = std::time::Duration::from_secs(5);

                    loop {
                        pl_exp_check_status(hcam_seq, &mut status, &mut bytes_arrived);

                        // Status codes: READOUT_NOT_ACTIVE=0, EXPOSURE_IN_PROGRESS=1,
                        // READOUT_IN_PROGRESS=2, READOUT_COMPLETE=3, READOUT_FAILED=5
                        const READOUT_COMPLETE: i16 = 3;
                        const READOUT_FAILED: i16 = 5;
                        const READOUT_NOT_ACTIVE: i16 = 0;

                        if status != last_status {
                            println!(
                                "     Status changed: {} -> {} (bytes: {})",
                                last_status, status, bytes_arrived
                            );
                            last_status = status;
                        }

                        if status == READOUT_COMPLETE {
                            frame_count = FRAME_COUNT;
                            println!("[OK] Sequence complete! {} frames captured", frame_count);
                            println!("     bytes_arrived = {}", bytes_arrived);
                            println!("\n*********************************************");
                            println!("*** SEQUENCE MODE with EXT_TRIG_INTERNAL WORKS! ***");
                            println!("*********************************************\n");
                            break;
                        } else if status == READOUT_FAILED {
                            println!("[FAIL] Sequence readout failed");
                            break;
                        } else if status == READOUT_NOT_ACTIVE {
                            println!("[WARN] Acquisition not active");
                            break;
                        }

                        if start_time.elapsed() > timeout {
                            println!("[FAIL] Sequence acquisition timed out after 5s");
                            println!("       Last status: {}, bytes: {}", status, bytes_arrived);
                            break;
                        }

                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }

                    // Abort and finish
                    pl_exp_abort(hcam_seq, CCS_HALT);
                    pl_exp_finish_seq(hcam_seq, buffer.as_mut_ptr() as *mut c_void, 0);
                } else {
                    println!("[FAIL] pl_exp_start_seq failed: {}", get_error_message());
                    let err_code = pl_error_code();
                    println!("       Error code: {}", err_code);
                }
            } else {
                println!("[FAIL] pl_exp_setup_seq failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }

            // Cleanup
            if !frame_info.is_null() {
                pl_release_frame_info_struct(frame_info);
            }
            pl_cam_close(hcam_seq);
        }
    }

    // ========================================
    // Test 16: SEQUENCE MODE with TIMED_MODE (software-triggered)
    // TIMED_MODE (0) means software starts exposure immediately.
    // ========================================
    println!("\n--- Test 16: SEQUENCE MODE with TIMED_MODE ---");
    println!("     Testing sequence mode with TIMED_MODE = 0");
    println!("     TIMED_MODE should start exposure immediately without external trigger\n");

    let mut hcam_seq2: i16 = -1;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam_seq2, 0) == 0 {
            println!("[FAIL] Failed to reopen camera: {}", get_error_message());
        } else {
            println!(
                "[OK] Camera opened for TIMED_MODE sequence test, handle = {}",
                hcam_seq2
            );

            // Create frame_info_struct
            let mut frame_info: *mut FRAME_INFO = ptr::null_mut();
            let _ = pl_create_frame_info_struct(&mut frame_info);

            const FRAME_COUNT: uns16 = 1; // Single frame for simplicity
            const EXPOSURE_MS: uns32 = 200; // 200ms single exposure

            let region = rgn_type {
                s1: 0,
                s2: 255, // 256x256
                sbin: 1,
                p1: 0,
                p2: 255,
                pbin: 1,
            };

            println!("     Region: 256x256");
            println!("     Frame count: {}", FRAME_COUNT);
            println!("     Exposure: {}ms", EXPOSURE_MS);

            // Setup sequence acquisition with TIMED_MODE
            let mut buffer_bytes: uns32 = 0;
            let setup_result = pl_exp_setup_seq(
                hcam_seq2,
                FRAME_COUNT,
                1, // region count
                &region as *const _,
                TIMED_MODE, // 0 = immediate software-triggered
                EXPOSURE_MS,
                &mut buffer_bytes,
            );

            if setup_result != 0 {
                println!(
                    "[OK] pl_exp_setup_seq with TIMED_MODE succeeded, buffer_bytes = {}",
                    buffer_bytes
                );

                // Allocate buffer
                let mut buffer = vec![0u8; buffer_bytes as usize];

                // Start sequence acquisition
                let start_result = pl_exp_start_seq(hcam_seq2, buffer.as_mut_ptr() as *mut c_void);

                if start_result != 0 {
                    println!("[OK] pl_exp_start_seq succeeded!");

                    // Poll for completion with detailed status
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut last_status: i16 = -1;
                    let start_time = std::time::Instant::now();
                    let timeout = std::time::Duration::from_secs(3);

                    loop {
                        pl_exp_check_status(hcam_seq2, &mut status, &mut bytes_arrived);

                        const READOUT_COMPLETE: i16 = 3;
                        const READOUT_FAILED: i16 = 5;
                        const READOUT_NOT_ACTIVE: i16 = 0;

                        if status != last_status {
                            let status_name = match status {
                                0 => "READOUT_NOT_ACTIVE",
                                1 => "EXPOSURE_IN_PROGRESS",
                                2 => "READOUT_IN_PROGRESS",
                                3 => "READOUT_COMPLETE",
                                5 => "READOUT_FAILED",
                                _ => "UNKNOWN",
                            };
                            println!(
                                "     Status: {} ({}) - elapsed: {:?}, bytes: {}",
                                status,
                                status_name,
                                start_time.elapsed(),
                                bytes_arrived
                            );
                            last_status = status;
                        }

                        if status == READOUT_COMPLETE {
                            println!("[OK] Single frame captured!");
                            println!("     bytes_arrived = {}", bytes_arrived);

                            // Check if buffer has non-zero data
                            let non_zero = buffer.iter().filter(|&&b| b != 0).count();
                            println!("     Non-zero bytes in buffer: {}", non_zero);

                            println!("\n*********************************************");
                            println!("*** SEQUENCE MODE with TIMED_MODE WORKS! ***");
                            println!("*********************************************\n");
                            break;
                        } else if status == READOUT_FAILED {
                            println!("[FAIL] Sequence readout failed");
                            break;
                        } else if status == READOUT_NOT_ACTIVE {
                            println!("[WARN] Acquisition not active");
                            break;
                        }

                        if start_time.elapsed() > timeout {
                            println!("[FAIL] Sequence acquisition timed out after 3s");
                            println!("       Last status: {}, bytes: {}", status, bytes_arrived);
                            break;
                        }

                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }

                    // Abort and finish
                    pl_exp_abort(hcam_seq2, CCS_HALT);
                    pl_exp_finish_seq(hcam_seq2, buffer.as_mut_ptr() as *mut c_void, 0);
                } else {
                    println!("[FAIL] pl_exp_start_seq failed: {}", get_error_message());
                    let err_code = pl_error_code();
                    println!("       Error code: {}", err_code);
                }
            } else {
                println!("[FAIL] pl_exp_setup_seq failed: {}", get_error_message());
                let err_code = pl_error_code();
                println!("       Error code: {}", err_code);
            }

            // Cleanup
            if !frame_info.is_null() {
                pl_release_frame_info_struct(frame_info);
            }
            pl_cam_close(hcam_seq2);
        }
    }

    // ========================================
    // Test 17: SINGLE FRAME with pl_exp_start_seq (simplest case)
    // Try the absolute simplest acquisition: 1 frame, full sensor, long exposure
    // ========================================
    println!("\n--- Test 17: SINGLE FRAME ACQUISITION (simplest case) ---");
    println!("     1 frame, 256x256, 500ms exposure, TIMED_MODE\n");

    let mut hcam_seq3: i16 = -1;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam_seq3, 0) == 0 {
            println!("[FAIL] Failed to reopen camera: {}", get_error_message());
        } else {
            println!("[OK] Camera opened, handle = {}", hcam_seq3);

            let region = rgn_type {
                s1: 0,
                s2: 255,
                sbin: 1,
                p1: 0,
                p2: 255,
                pbin: 1,
            };

            let mut buffer_bytes: uns32 = 0;

            // Setup for single frame
            if pl_exp_setup_seq(hcam_seq3, 1, 1, &region, TIMED_MODE, 500, &mut buffer_bytes) != 0 {
                println!(
                    "[OK] Setup for single frame, buffer_bytes = {}",
                    buffer_bytes
                );

                let mut buffer = vec![0u8; buffer_bytes as usize];

                if pl_exp_start_seq(hcam_seq3, buffer.as_mut_ptr() as *mut c_void) != 0 {
                    println!("[OK] Acquisition started");

                    // Wait longer for 500ms exposure
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut last_status: i16 = -1;
                    let start_time = std::time::Instant::now();

                    for _ in 0..200 {
                        // 2 seconds max
                        pl_exp_check_status(hcam_seq3, &mut status, &mut bytes_arrived);

                        if status != last_status {
                            let status_name = match status {
                                0 => "NOT_ACTIVE",
                                1 => "EXPOSING",
                                2 => "READING",
                                3 => "COMPLETE",
                                5 => "FAILED",
                                _ => "???",
                            };
                            println!(
                                "     {:?}: status={} ({}) bytes={}",
                                start_time.elapsed(),
                                status,
                                status_name,
                                bytes_arrived
                            );
                            last_status = status;
                        }

                        if status == 3 {
                            // READOUT_COMPLETE
                            let non_zero = buffer.iter().filter(|&&b| b != 0).count();
                            println!("\n*********************************************");
                            println!("*** SINGLE FRAME CAPTURE SUCCESSFUL! ***");
                            println!("*** Non-zero bytes: {} ***", non_zero);
                            println!("*********************************************\n");
                            break;
                        } else if status == 5 || status == 0 {
                            println!("[FAIL] status = {}", status);
                            break;
                        }

                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }

                    pl_exp_abort(hcam_seq3, CCS_HALT);
                    pl_exp_finish_seq(hcam_seq3, buffer.as_mut_ptr() as *mut c_void, 0);
                } else {
                    println!("[FAIL] start_seq failed: {}", get_error_message());
                }
            } else {
                println!("[FAIL] setup_seq failed: {}", get_error_message());
            }

            pl_cam_close(hcam_seq3);
        }
    }

    // Cleanup
    println!("\n--- Cleanup ---");
    unsafe {
        pl_pvcam_uninit();
    }
    println!("[OK] PVCAM uninitialized");

    println!("\n=== Diagnostic Test Complete ===\n");
}

// ============================================================================
// TEST 17: Minimal SDK-style callback test (bd-callback-isolation)
// ============================================================================
// This test mimics the SDK C++ example exactly to isolate the callback issue.
// If this test passes for 20+ frames but the daemon fails, the issue is in
// the daemon's callback/synchronization implementation, not the FFI/SDK.

use std::sync::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Minimal callback context matching C++ SDK pattern
struct MinimalContext {
    mutex: Mutex<bool>,
    condvar: Condvar,
    eof_flag: AtomicBool,
    frame_nr: AtomicI32,
    callback_count: AtomicI32,
}

impl MinimalContext {
    fn new() -> Self {
        Self {
            mutex: Mutex::new(false),
            condvar: Condvar::new(),
            eof_flag: AtomicBool::new(false),
            frame_nr: AtomicI32::new(0),
            callback_count: AtomicI32::new(0),
        }
    }

    fn signal(&self, frame_nr: i32) {
        self.frame_nr.store(frame_nr, Ordering::Release);
        self.callback_count.fetch_add(1, Ordering::AcqRel);
        let mut guard = self.mutex.lock().unwrap();
        *guard = true;
        self.eof_flag.store(true, Ordering::Release);
        self.condvar.notify_one();
    }

    fn wait(&self, timeout_ms: u64) -> bool {
        let guard = self.mutex.lock().unwrap();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let result = self.condvar.wait_timeout_while(guard, timeout, |flag| !*flag);
        match result {
            Ok((mut guard, timeout_result)) => {
                *guard = false;
                self.eof_flag.store(false, Ordering::Release);
                !timeout_result.timed_out()
            }
            Err(_) => false,
        }
    }
}

/// Static context for callback (C++ pattern uses local stack variable + pointer)
static mut MINIMAL_CTX: Option<*const MinimalContext> = None;

/// Minimal callback - NO catch_unwind, NO extra synchronization
/// This matches the C++ SDK example exactly
extern "system" fn minimal_eof_callback(
    p_frame_info: *const FRAME_INFO,
    _p_context: *mut c_void,
) {
    unsafe {
        if let Some(ctx_ptr) = MINIMAL_CTX {
            let ctx = &*ctx_ptr;
            let frame_nr = if !p_frame_info.is_null() {
                (*p_frame_info).FrameNr
            } else {
                -1
            };
            let count = ctx.callback_count.load(Ordering::Acquire) + 1;
            eprintln!("[CALLBACK {}] Frame {} ready", count, frame_nr);
            ctx.signal(frame_nr);
        }
    }
}

#[tokio::test]
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

    let mut cam_name = [0i8; 32];
    unsafe {
        pl_cam_get_name(0, cam_name.as_mut_ptr());
    }
    let cam_name_str = unsafe { CStr::from_ptr(cam_name.as_ptr()).to_string_lossy() };
    println!("[OK] Camera: {}", cam_name_str);

    // Open camera
    let mut hcam: i16 = 0;
    unsafe {
        if pl_cam_open(cam_name.as_mut_ptr(), &mut hcam, 0) == 0 {
            println!("ERROR: pl_cam_open failed: {}", get_error_message());
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Camera opened, handle={}", hcam);

    // Get sensor size
    let mut ser_size: u16 = 0;
    let mut par_size: u16 = 0;
    unsafe {
        pl_get_param(hcam, PARAM_SER_SIZE, ATTR_CURRENT as i16, &mut ser_size as *mut _ as *mut c_void);
        pl_get_param(hcam, PARAM_PAR_SIZE, ATTR_CURRENT as i16, &mut par_size as *mut _ as *mut c_void);
    }
    println!("[OK] Sensor size: {}x{}", ser_size, par_size);

    // Create context and set global pointer (C++ pattern)
    let ctx = Box::new(MinimalContext::new());
    let ctx_ptr = &*ctx as *const MinimalContext;
    unsafe {
        MINIMAL_CTX = Some(ctx_ptr);
    }
    println!("[OK] Callback context created");

    // Register callback BEFORE setup (SDK pattern)
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            minimal_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region (full sensor)
    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    // Setup continuous acquisition with CIRC_NO_OVERWRITE
    let exposure_ms: u32 = 100;
    let buffer_frames: u16 = 20;
    let mut frame_bytes: u32 = 0;

    println!("[SETUP] pl_exp_setup_cont with CIRC_NO_OVERWRITE...");
    unsafe {
        let result = pl_exp_setup_cont(
            hcam,
            1,
            &region as *const rgn_type,
            TIMED_MODE,
            exposure_ms,
            &mut frame_bytes,
            CIRC_NO_OVERWRITE,
        );
        if result == 0 {
            println!("ERROR: pl_exp_setup_cont failed: {}", get_error_message());
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] Setup complete, frame_bytes={}", frame_bytes);

    // Allocate buffer
    let buffer_size = (frame_bytes as usize) * (buffer_frames as usize);
    let layout = Layout::from_size_align(buffer_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Buffer allocation failed");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Buffer allocated: {} frames, {:.2} MB", buffer_frames, buffer_size as f64 / 1024.0 / 1024.0);

    // Start acquisition
    println!("[START] pl_exp_start_cont...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32);
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

    // Frame loop (SDK pattern)
    println!("\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n", TARGET_FRAMES);
    let mut frames_acquired: i32 = 0;
    let loop_start = std::time::Instant::now();

    while frames_acquired < TARGET_FRAMES {
        println!("[MAIN LOOP {}] Waiting for EOF (timeout {}ms)...", frames_acquired + 1, TIMEOUT_MS);
        let wait_start = std::time::Instant::now();

        if !ctx.wait(TIMEOUT_MS) {
            println!("[TIMEOUT] No EOF event after {}ms", TIMEOUT_MS);
            break;
        }

        let wait_elapsed = wait_start.elapsed().as_millis();
        let frame_nr = ctx.frame_nr.load(Ordering::Acquire);
        println!("[MAIN LOOP {}] EOF received after {}ms, FrameNr={}", frames_acquired + 1, wait_elapsed, frame_nr);

        // Retrieve frame using get_oldest_frame
        let mut frame_ptr: *mut c_void = ptr::null_mut();
        unsafe {
            if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                println!("[ERROR] pl_exp_get_oldest_frame failed: {}", get_error_message());
                break;
            }
        }
        println!("[MAIN LOOP {}] Frame retrieved, ptr={:?}", frames_acquired + 1, frame_ptr);

        frames_acquired += 1;

        // Unlock frame (CRITICAL for CIRC_NO_OVERWRITE)
        unsafe {
            if pl_exp_unlock_oldest_frame(hcam) == 0 {
                println!("[ERROR] pl_exp_unlock_oldest_frame failed: {}", get_error_message());
            } else {
                println!("[MAIN LOOP {}] Frame unlocked", frames_acquired);
            }
        }

        println!("[MAIN LOOP {}] SUCCESS\n", frames_acquired);
    }

    let total_time = loop_start.elapsed().as_millis();
    println!("\n=== ACQUISITION SUMMARY ===");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
    println!("Total callbacks: {}", ctx.callback_count.load(Ordering::Acquire));
    println!("Total time: {}ms", total_time);
    if frames_acquired > 0 {
        println!("Average FPS: {:.2}", frames_acquired as f64 * 1000.0 / total_time as f64);
    }

    // Cleanup
    println!("\n[STOP] Cleanup...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        pl_cam_close(hcam);
        MINIMAL_CTX = None;
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

/// CallbackContext matching the full driver's structure exactly
#[derive(Debug)]
struct FullCallbackContext {
    pending_frames: std::sync::atomic::AtomicU32,
    latest_frame_nr: AtomicI32,
    condvar: std::sync::Condvar,
    mutex: std::sync::Mutex<bool>,
    shutdown: AtomicBool,
    hcam: AtomicI16,
    frame_ptr: std::sync::atomic::AtomicPtr<c_void>,
    frame_info: std::sync::Mutex<FRAME_INFO>,
    circ_overwrite: AtomicBool,
}

impl FullCallbackContext {
    fn new(hcam: i16) -> Self {
        Self {
            pending_frames: std::sync::atomic::AtomicU32::new(0),
            latest_frame_nr: AtomicI32::new(-1),
            condvar: std::sync::Condvar::new(),
            mutex: std::sync::Mutex::new(false),
            shutdown: AtomicBool::new(false),
            hcam: AtomicI16::new(hcam),
            frame_ptr: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()),
            frame_info: std::sync::Mutex::new(unsafe { std::mem::zeroed() }),
            circ_overwrite: AtomicBool::new(false), // CIRC_NO_OVERWRITE
        }
    }

    fn signal_frame_ready(&self, frame_nr: i32) {
        self.latest_frame_nr.store(frame_nr, Ordering::Release);
        self.pending_frames.fetch_add(1, Ordering::AcqRel);
        let mut guard = match self.mutex.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = true;
        self.condvar.notify_one();
    }

    fn wait_for_frames(&self, timeout_ms: u64) -> u32 {
        if self.shutdown.load(Ordering::Acquire) {
            return 0;
        }
        let guard = match self.mutex.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let timeout_duration = std::time::Duration::from_millis(timeout_ms);
        let result = self.condvar.wait_timeout_while(guard, timeout_duration, |notified| {
            !*notified && !self.shutdown.load(Ordering::Acquire)
        });
        match result {
            Ok((mut guard, timeout_result)) => {
                *guard = false;
                if timeout_result.timed_out() { 0 } else {
                    self.pending_frames.load(Ordering::Acquire).max(1)
                }
            }
            Err(poisoned) => {
                let (mut guard, _) = poisoned.into_inner();
                *guard = false;
                0
            }
        }
    }

    fn consume_one(&self) {
        let _ = self.pending_frames.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n > 0 { Some(n - 1) } else { None }
        });
    }

    fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut guard) = self.mutex.lock() {
            *guard = true;
            self.condvar.notify_all();
        }
    }
}

/// Static global pointer for test 18 callback (like full driver's GLOBAL_CALLBACK_CTX)
static FULL_CTX: std::sync::atomic::AtomicPtr<FullCallbackContext> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Callback for test 18 - matches full driver's pvcam_eof_callback
extern "system" fn full_eof_callback(
    p_frame_info: *const FRAME_INFO,
    _p_context: *mut c_void,
) {
    static CALLBACK_ENTRY_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let entry_count = CALLBACK_ENTRY_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let ctx_ptr = FULL_CTX.load(Ordering::Acquire);

    if entry_count <= 25 || entry_count % 50 == 0 {
        eprintln!("[TEST18 CALLBACK ENTRY] #{}, ctx={:?}", entry_count, ctx_ptr);
    }

    if ctx_ptr.is_null() { return; }
    let ctx = unsafe { &*ctx_ptr };

    let frame_nr = if !p_frame_info.is_null() {
        let info = unsafe { *p_frame_info };
        if info.FrameNr <= 25 || info.FrameNr % 50 == 0 {
            eprintln!("[TEST18 CALLBACK] Frame {} ready, timestamp={}", info.FrameNr, info.TimeStamp);
        }
        info.FrameNr
    } else { -1 };

    ctx.signal_frame_ready(frame_nr);
}

#[tokio::test]
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
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created (Arc<Pin<Box>>), ptr={:?}", ctx_ptr);

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
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

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
    println!("[OK] Allocated {} bytes ({} frames) at {:?}", buffer_size, BUFFER_FRAMES, buffer);

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
    let streaming = std::sync::Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    // Spawn frame loop in blocking thread (like full driver)
    println!("\n=== FRAME ACQUISITION LOOP (spawn_blocking, target: {} frames) ===\n", TARGET_FRAMES);
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;
        let loop_start = std::time::Instant::now();

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            // Wait for callback (like full driver)
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);
                // Check SDK status
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        eprintln!("[SDK STATUS] status={}, bytes={}, cnt={}", status, bytes_arrived, buffer_cnt);
                    }
                }
                continue;
            }

            // Get oldest frame (like full driver's get_oldest_frame)
            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    eprintln!("[ERROR] pl_exp_get_oldest_frame failed: {}", get_error_message());
                    continue;
                }
            }

            frames_acquired += 1;

            // Unlock immediately (like minimal test and full driver's SKIP_PROCESSING)
            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] pl_exp_unlock_oldest_frame failed");
                }
            }

            // Consume callback notification (like full driver)
            ctx_clone.consume_one();

            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                println!("[FRAME {}] acquired", frames_acquired);
            }
        }

        let total_time = loop_start.elapsed().as_millis();
        println!("\n=== ACQUISITION SUMMARY ===");
        println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
        println!("Total time: {}ms", total_time);
        if frames_acquired > 0 && total_time > 0 {
            println!("Average FPS: {:.2}", frames_acquired as f64 * 1000.0 / total_time as f64);
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

    println!("\n=== TEST 18 COMPLETE ===\n");

    // Assert success
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. spawn_blocking may be causing the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}

/// Test 19: check_cont_status Isolation Test
///
/// HYPOTHESIS: The full driver calls pl_exp_check_cont_status multiple times per iteration,
/// which test_18 does NOT do. This test adds those calls to test_18's working pattern
/// to see if check_cont_status is interfering with SDK callback state.
///
/// If this test FAILS at ~19 frames: check_cont_status is likely the cause
/// If this test PASSES with 200 frames: check_cont_status is NOT the issue
#[tokio::test]
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
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created (Arc<Pin<Box>>), ptr={:?}", ctx_ptr);

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
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

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
    println!("[OK] Allocated {} bytes ({} frames) at {:?}", buffer_size, BUFFER_FRAMES, buffer);

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
    let streaming = std::sync::Arc::new(AtomicBool::new(true));
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
            // Full driver does this at loop start every 5 iterations or every 30th
            if loop_iteration <= 5 || loop_iteration % 30 == 0 {
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        if loop_iteration <= 10 {
                            eprintln!("[ITER {}] check_cont_status: status={}, bytes={}, cnt={}",
                                loop_iteration, status, bytes_arrived, buffer_cnt);
                        }
                    }
                }
            }

            // Wait for callback (like full driver)
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);

                // KEY DIFFERENCE: Add check_cont_status on timeout like full driver
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        eprintln!("[TIMEOUT SDK] status={}, bytes={}, cnt={}", status, bytes_arrived, buffer_cnt);
                    }
                }
                continue;
            }

            // Get oldest frame
            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    eprintln!("[ERROR] pl_exp_get_oldest_frame failed: {}", get_error_message());
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
                println!("[FRAME {}] acquired (iter {})", frames_acquired, loop_iteration);
            }
        }

        let total_time = loop_start.elapsed().as_millis();
        println!("\n=== ACQUISITION SUMMARY ===");
        println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
        println!("Total time: {}ms", total_time);
        println!("Loop iterations: {}", loop_iteration);
        if frames_acquired > 0 && total_time > 0 {
            println!("Average FPS: {:.2}", frames_acquired as f64 * 1000.0 / total_time as f64);
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
        println!("RESULT: check_cont_status MAY be causing the issue ({} frames)", frames_acquired);
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. check_cont_status may be interfering with SDK state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

/// Test 20: Parameter Query Isolation Test
///
/// HYPOTHESIS: The full driver queries many camera parameters (PARAM_SER_SIZE, PARAM_PAR_SIZE,
/// speed table, etc.) after opening the camera. These pl_get_param calls might be changing
/// some camera state that affects callback behavior.
///
/// This test adds parameter queries to test_18's working pattern (before streaming starts)
/// to see if parameter queries are interfering with callback registration or acquisition.
///
/// If this test FAILS at ~19 frames: pl_get_param calls are likely causing the issue
/// If this test PASSES with 200 frames: pl_get_param calls are NOT the issue
#[tokio::test]
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
        // Query sensor size (PARAM_SER_SIZE, PARAM_PAR_SIZE)
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        if pl_get_param(hcam, PARAM_SER_SIZE, ATTR_CURRENT, &mut ser as *mut _ as *mut _) != 0 {
            println!("  PARAM_SER_SIZE: {}", ser);
        }
        if pl_get_param(hcam, PARAM_PAR_SIZE, ATTR_CURRENT, &mut par as *mut _ as *mut _) != 0 {
            println!("  PARAM_PAR_SIZE: {}", par);
        }

        // Query bit depth
        let mut bit_depth: i16 = 0;
        if pl_get_param(hcam, PARAM_BIT_DEPTH, ATTR_CURRENT, &mut bit_depth as *mut _ as *mut _) != 0 {
            println!("  PARAM_BIT_DEPTH: {}", bit_depth);
        }

        // Query chip name
        let mut chip_name = [0i8; 64];
        if pl_get_param(hcam, PARAM_CHIP_NAME, ATTR_CURRENT, chip_name.as_mut_ptr() as *mut _) != 0 {
            let name = std::ffi::CStr::from_ptr(chip_name.as_ptr()).to_string_lossy();
            println!("  PARAM_CHIP_NAME: {}", name);
        }

        // Query temperature
        let mut temp: i16 = 0;
        if pl_get_param(hcam, PARAM_TEMP, ATTR_CURRENT, &mut temp as *mut _ as *mut _) != 0 {
            println!("  PARAM_TEMP: {} (raw)", temp);
        }

        // Query speed table entries (PARAM_SPDTAB_INDEX)
        let mut speed_idx: i16 = 0;
        if pl_get_param(hcam, PARAM_SPDTAB_INDEX, ATTR_CURRENT, &mut speed_idx as *mut _ as *mut _) != 0 {
            println!("  PARAM_SPDTAB_INDEX: {}", speed_idx);
        }

        // Query gain index
        let mut gain_idx: i16 = 0;
        if pl_get_param(hcam, PARAM_GAIN_INDEX, ATTR_CURRENT, &mut gain_idx as *mut _ as *mut _) != 0 {
            println!("  PARAM_GAIN_INDEX: {}", gain_idx);
        }

        // Query frame buffer size recommendation
        let mut frame_buf_size: u64 = 0;
        if pl_get_param(hcam, PARAM_FRAME_BUFFER_SIZE, ATTR_CURRENT, &mut frame_buf_size as *mut _ as *mut _) != 0 {
            println!("  PARAM_FRAME_BUFFER_SIZE: {}", frame_buf_size);
        }

        // Query metadata capability
        let mut md_enabled: rs_bool = 0;
        if pl_get_param(hcam, PARAM_METADATA_ENABLED, ATTR_AVAIL, &mut md_enabled as *mut _ as *mut _) != 0 {
            println!("  PARAM_METADATA_ENABLED (avail): {}", md_enabled);
        }

        // Query circular buffer mode capability
        let mut circ_avail: rs_bool = 0;
        if pl_get_param(hcam, PARAM_CIRC_BUFFER, ATTR_AVAIL, &mut circ_avail as *mut _ as *mut _) != 0 {
            println!("  PARAM_CIRC_BUFFER (avail): {}", circ_avail);
        }
    }
    println!("[OK] Parameter queries complete");

    // === REST OF STREAMING SETUP (same as test_18) ===

    // Create callback context
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created, ptr={:?}", ctx_ptr);

    // Register callback BEFORE setup
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

    // Allocate buffer
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

    // Frame loop
    let ctx_clone = ctx.clone();
    let streaming = std::sync::Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    println!("\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n", TARGET_FRAMES);
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);
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
        println!("RESULT: pl_get_param calls MAY be causing the issue ({} frames)", frames_acquired);
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. pl_get_param calls may be affecting SDK callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

/// Test 21: Metadata Enable/Disable Isolation Test
///
/// HYPOTHESIS: The full driver checks and potentially modifies PARAM_METADATA_ENABLED
/// before streaming. This pl_set_param call might be affecting callback behavior.
///
/// This test adds metadata enable/disable logic to test_20's working pattern.
///
/// If this test FAILS at ~19 frames: metadata param manipulation is likely causing the issue
/// If this test PASSES with 200 frames: metadata is NOT the issue
#[tokio::test]
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

    // === PARAMETER QUERIES (like full driver) ===
    println!("\n[PARAM QUERIES] Querying camera parameters...");
    unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        if pl_get_param(hcam, PARAM_SER_SIZE, ATTR_CURRENT, &mut ser as *mut _ as *mut _) != 0 {
            println!("  PARAM_SER_SIZE: {}", ser);
        }
        if pl_get_param(hcam, PARAM_PAR_SIZE, ATTR_CURRENT, &mut par as *mut _ as *mut _) != 0 {
            println!("  PARAM_PAR_SIZE: {}", par);
        }
    }

    // === METADATA MANIPULATION (like full driver's start_stream) ===
    println!("\n[METADATA] Checking and setting PARAM_METADATA_ENABLED (like full driver)...");
    let use_metadata = false; // Full driver often has this false unless explicitly enabled
    unsafe {
        // Check if metadata is available
        let mut md_avail: rs_bool = 0;
        if pl_get_param(hcam, PARAM_METADATA_ENABLED, ATTR_AVAIL, &mut md_avail as *mut _ as *mut _) != 0 {
            println!("  PARAM_METADATA_ENABLED available: {}", md_avail != 0);

            if md_avail != 0 {
                // Get current value
                let mut current_md: rs_bool = 0;
                if pl_get_param(hcam, PARAM_METADATA_ENABLED, ATTR_CURRENT, &mut current_md as *mut _ as *mut _) != 0 {
                    println!("  PARAM_METADATA_ENABLED current: {}", current_md != 0);
                }

                // The full driver conditionally enables/disables metadata
                // Let's mimic the "disable if not using" path (most common)
                if !use_metadata && current_md != 0 {
                    println!("  Disabling metadata (like full driver when not decoding)...");
                    let disable: rs_bool = 0;
                    if pl_set_param(hcam, PARAM_METADATA_ENABLED, &disable as *const _ as *mut _) == 0 {
                        println!("  WARNING: Failed to disable metadata: {}", get_error_message());
                    } else {
                        println!("  Metadata disabled successfully");
                    }
                } else if use_metadata && current_md == 0 {
                    println!("  Enabling metadata (like full driver when decoding)...");
                    let enable: rs_bool = 1;
                    if pl_set_param(hcam, PARAM_METADATA_ENABLED, &enable as *const _ as *mut _) == 0 {
                        println!("  WARNING: Failed to enable metadata: {}", get_error_message());
                    } else {
                        println!("  Metadata enabled successfully");
                    }
                } else {
                    println!("  Metadata already in desired state");
                }
            }
        } else {
            println!("  PARAM_METADATA_ENABLED not available");
        }
    }
    println!("[OK] Metadata handling complete");

    // === REST OF STREAMING SETUP (same as test_20) ===

    // Create callback context
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created, ptr={:?}", ctx_ptr);

    // Register callback BEFORE setup
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

    // Allocate buffer
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

    // Frame loop
    let ctx_clone = ctx.clone();
    let streaming = std::sync::Arc::new(AtomicBool::new(true));
    let streaming_clone = streaming.clone();

    println!("\n=== FRAME ACQUISITION LOOP (target: {} frames) ===\n", TARGET_FRAMES);
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);
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

    println!("\n=== TEST 21 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: PARAM_METADATA_ENABLED manipulation is NOT the issue (200 frames achieved)");
    } else {
        println!("RESULT: PARAM_METADATA_ENABLED manipulation MAY be causing the issue ({} frames)", frames_acquired);
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Metadata param manipulation may be affecting SDK callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

/// TEST 22: PARAM_CIRC_BUFFER query after setup isolation
///
/// The full driver queries PARAM_CIRC_BUFFER ATTR_CURRENT after pl_exp_setup_cont
/// but before pl_exp_start_cont. This test adds that exact query pattern to see if
/// it affects streaming.
#[tokio::test]
async fn test_22_post_setup_circ_query() {
    println!("\n=== TEST 22: Post-Setup PARAM_CIRC_BUFFER Query Isolation Test ===");
    println!("Tests whether querying PARAM_CIRC_BUFFER after setup causes the 19-frame cutoff.\n");
    println!("This matches the full driver pattern at acquisition.rs lines 1591-1607.\n");

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

    // Create callback context
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created, ptr={:?}", ctx_ptr);

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
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

    // === THE KEY DIFFERENCE: Query PARAM_CIRC_BUFFER ATTR_CURRENT AFTER setup ===
    // This matches lines 1591-1607 in acquisition.rs
    println!("\n[POST-SETUP QUERY] Querying PARAM_CIRC_BUFFER ATTR_CURRENT (like full driver)...");
    unsafe {
        let mut circ_current: uns32 = 0;
        if pl_get_param(
            hcam,
            PARAM_CIRC_BUFFER,
            ATTR_CURRENT,
            &mut circ_current as *mut _ as *mut c_void,
        ) == 0
        {
            println!("  [WARN] PARAM_CIRC_BUFFER ATTR_CURRENT query failed: {}", get_error_message());
        } else {
            println!("  [OK] PARAM_CIRC_BUFFER current mode: {}", circ_current);
        }
    }
    println!("[OK] Post-setup query complete");

    // Allocate buffer
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

    // Start acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32);
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

    // Spawn frame loop in blocking thread (like test_18)
    let streaming = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let streaming_clone = streaming.clone();
    let ctx_clone = ctx.clone();

    println!("\n=== FRAME ACQUISITION LOOP (spawn_blocking, target: {} frames) ===\n", TARGET_FRAMES);
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        eprintln!("[SDK STATUS] status={}, bytes={}, cnt={}", status, bytes_arrived, buffer_cnt);
                    }
                }
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    eprintln!("[ERROR] pl_exp_get_oldest_frame failed: {}", get_error_message());
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

    println!("\n=== TEST 22 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Post-setup PARAM_CIRC_BUFFER query is NOT the issue (200 frames achieved)");
    } else {
        println!("RESULT: Post-setup PARAM_CIRC_BUFFER query MAY be causing the issue ({} frames)", frames_acquired);
    }
    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Post-setup PARAM_CIRC_BUFFER query may be affecting SDK callback state!",
        TARGET_FRAMES,
        frames_acquired
    );
}

/// TEST 23: Post-unlock check_cont_status call isolation
///
/// The full driver calls check_cont_status after EACH unlock for the first 25 frames.
/// This test adds that exact pattern to see if it causes the 19-frame cutoff.
#[tokio::test]
async fn test_23_post_unlock_status_check() {
    println!("\n=== TEST 23: Post-Unlock check_cont_status Isolation Test ===");
    println!("Tests whether calling check_cont_status after each unlock causes the 19-frame cutoff.\n");
    println!("This matches the full driver pattern at acquisition.rs lines 2696-2703.\n");

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

    // Create callback context
    let ctx = std::sync::Arc::new(std::pin::Pin::new(Box::new(FullCallbackContext::new(hcam))));
    let ctx_ptr = &**ctx as *const FullCallbackContext;
    FULL_CTX.store(ctx_ptr as *mut FullCallbackContext, Ordering::Release);
    println!("[OK] Callback context created, ptr={:?}", ctx_ptr);

    // Register callback BEFORE setup
    println!("[SETUP] Registering EOF callback...");
    unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            full_eof_callback as *mut c_void,
            ctx_ptr as *mut c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
            pl_cam_close(hcam);
            pl_pvcam_uninit();
            return;
        }
    }
    println!("[OK] EOF callback registered");

    // Setup region
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
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

    // Allocate buffer
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

    // Start acquisition
    println!("[SETUP] Starting continuous acquisition...");
    unsafe {
        let result = pl_exp_start_cont(hcam, buffer as *mut c_void, buffer_size as u32);
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

    // Spawn frame loop
    let streaming = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let streaming_clone = streaming.clone();
    let ctx_clone = ctx.clone();

    println!("\n=== FRAME ACQUISITION LOOP (with post-unlock status check, target: {} frames) ===\n", TARGET_FRAMES);
    let handle = tokio::task::spawn_blocking(move || {
        let mut frames_acquired: i32 = 0;

        while frames_acquired < TARGET_FRAMES && streaming_clone.load(Ordering::Acquire) {
            let pending = ctx_clone.wait_for_frames(TIMEOUT_MS);
            if pending == 0 {
                println!("[TIMEOUT] No frame after {}ms (acquired {})", TIMEOUT_MS, frames_acquired);
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        eprintln!("[SDK STATUS] status={}, bytes={}, cnt={}", status, bytes_arrived, buffer_cnt);
                    }
                }
                continue;
            }

            let mut frame_ptr: *mut c_void = ptr::null_mut();
            unsafe {
                if pl_exp_get_oldest_frame(hcam, &mut frame_ptr) == 0 {
                    eprintln!("[ERROR] pl_exp_get_oldest_frame failed: {}", get_error_message());
                    continue;
                }
            }

            frames_acquired += 1;

            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] pl_exp_unlock_oldest_frame failed");
                }
            }

            // === THE KEY DIFFERENCE: check_cont_status AFTER each unlock (like full driver) ===
            // This matches lines 2696-2703 in acquisition.rs
            if frames_acquired <= 25 || frames_acquired % 50 == 0 {
                unsafe {
                    let mut status: i16 = 0;
                    let mut bytes_arrived: uns32 = 0;
                    let mut buffer_cnt: uns32 = 0;
                    if pl_exp_check_cont_status(hcam, &mut status, &mut bytes_arrived, &mut buffer_cnt) != 0 {
                        eprintln!("[POST-UNLOCK] status={}, bytes={}, cnt={}, frame={}", status, bytes_arrived, buffer_cnt, frames_acquired);
                    }
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

    println!("\n=== TEST 23 COMPLETE ===\n");

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Post-unlock check_cont_status is NOT the issue (200 frames achieved)");
    } else {
        println!("RESULT: Post-unlock check_cont_status MAY be causing the issue ({} frames)", frames_acquired);
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
//
// This test imports and uses the EXACT same callback infrastructure as the driver:
// - GLOBAL_CALLBACK_CTX static from acquisition.rs
// - pvcam_eof_callback function from acquisition.rs
// - CallbackContext struct from acquisition.rs
//
// If this test FAILS at ~19 frames: the issue is in the driver's callback infrastructure
// If this test PASSES with 200 frames: the issue is in how the driver sets up/uses it

use daq_driver_pvcam::components::acquisition::{
    CallbackContext, GLOBAL_CALLBACK_CTX, pvcam_eof_callback, set_global_callback_ctx,
};

#[tokio::test]
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

    // Create CallbackContext using the DRIVER's struct (Arc<Pin<Box<>>> like driver does)
    let callback_ctx = Arc::new(Box::pin(CallbackContext::new(hcam)));
    let callback_ctx_ptr = &**callback_ctx as *const CallbackContext;
    println!("[OK] Driver CallbackContext created, ptr={:?}", callback_ctx_ptr);

    // Set GLOBAL_CALLBACK_CTX (exactly like driver does)
    set_global_callback_ctx(callback_ctx_ptr);
    println!("[OK] GLOBAL_CALLBACK_CTX set to {:?}", callback_ctx_ptr);

    // Register callback using DRIVER's pvcam_eof_callback
    println!("[SETUP] Registering EOF callback (driver's pvcam_eof_callback)...");
    let callback_registered = unsafe {
        let result = pl_cam_register_callback_ex3(
            hcam,
            PL_CALLBACK_EOF,
            pvcam_eof_callback as *mut std::ffi::c_void,
            callback_ctx_ptr as *mut std::ffi::c_void,
        );
        if result == 0 {
            println!("ERROR: pl_cam_register_callback_ex3 failed: {}", get_error_message());
            false
        } else {
            println!("[OK] EOF callback registered using driver's pvcam_eof_callback");
            true
        }
    };

    if !callback_registered {
        unsafe {
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }

    // Query sensor size and setup region
    let (ser_size, par_size) = unsafe {
        let mut ser: uns16 = 0;
        let mut par: uns16 = 0;
        pl_get_param(hcam, PARAM_SER_SIZE, ATTR_CURRENT, &mut ser as *mut _ as *mut _);
        pl_get_param(hcam, PARAM_PAR_SIZE, ATTR_CURRENT, &mut par as *mut _ as *mut _);
        (ser, par)
    };
    println!("[OK] Sensor size: {}x{}", ser_size, par_size);

    let region = rgn_type {
        s1: 0,
        s2: ser_size - 1,
        sbin: 1,
        p1: 0,
        p2: par_size - 1,
        pbin: 1,
    };

    // Setup continuous acquisition
    println!("[SETUP] Setting up continuous acquisition...");
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
            GLOBAL_CALLBACK_CTX.store(std::ptr::null_mut(), Ordering::Release);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] pl_exp_setup_cont succeeded, frame_bytes={}", frame_bytes);

    // Set circ_overwrite to false in callback context (like driver does for CIRC_NO_OVERWRITE)
    callback_ctx.set_circ_overwrite(false);

    // Allocate page-aligned buffer (like driver's PageAlignedBuffer)
    let total_size = frame_bytes as usize * BUFFER_FRAMES;
    let layout = Layout::from_size_align(total_size, 4096).unwrap();
    let buffer = unsafe { alloc_zeroed(layout) };
    if buffer.is_null() {
        println!("ERROR: Failed to allocate buffer");
        unsafe {
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            GLOBAL_CALLBACK_CTX.store(std::ptr::null_mut(), Ordering::Release);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Allocated {} bytes at {:?}", total_size, buffer);

    // Start continuous acquisition
    println!("[SETUP] Starting continuous acquisition...");
    let start_result = unsafe { pl_exp_start_cont(hcam, buffer as *mut _, total_size as uns32) };
    if start_result == 0 {
        println!("ERROR: pl_exp_start_cont failed: {}", get_error_message());
        unsafe {
            dealloc(buffer, layout);
            pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
            GLOBAL_CALLBACK_CTX.store(std::ptr::null_mut(), Ordering::Release);
            pl_cam_close(hcam);
            pl_pvcam_uninit();
        }
        return;
    }
    println!("[OK] Acquisition started");

    println!("\n=== FRAME ACQUISITION LOOP (using driver's wait_for_frames, target: {} frames) ===\n", TARGET_FRAMES);

    // Run frame loop using spawn_blocking like driver does
    let callback_ctx_for_loop = callback_ctx.clone();
    let loop_start = std::time::Instant::now();

    let frames_acquired = tokio::task::spawn_blocking(move || {
        let mut frames = 0i32;
        let mut loop_iteration = 0u64;

        while frames < TARGET_FRAMES {
            loop_iteration += 1;

            // Wait for frames using driver's CallbackContext method
            let pending = callback_ctx_for_loop.wait_for_frames(TIMEOUT_MS);

            if pending == 0 {
                // Check SDK status on timeout
                let mut status: i16 = 0;
                let mut bytes: uns32 = 0;
                let mut cnt: uns32 = 0;
                unsafe {
                    pl_exp_check_cont_status(hcam, &mut status, &mut bytes, &mut cnt);
                }
                eprintln!(
                    "[TIMEOUT] iter={}, status={}, bytes={}, cnt={}, pending={}",
                    loop_iteration, status, bytes, cnt, callback_ctx_for_loop.pending_frames.load(Ordering::Acquire)
                );

                // Exit if SDK stopped
                if status == 0 {
                    eprintln!("[FATAL] SDK status=0 (READOUT_NOT_ACTIVE) - acquisition stopped!");
                    break;
                }
                continue;
            }

            // Get oldest frame
            let mut frame_ptr: *mut c_void = ptr::null_mut();
            let get_result = unsafe { pl_exp_get_oldest_frame(hcam, &mut frame_ptr) };
            if get_result == 0 || frame_ptr.is_null() {
                eprintln!("[ERROR] get_oldest_frame failed: {}", get_error_message());
                continue;
            }

            frames += 1;

            // Unlock immediately
            unsafe {
                if pl_exp_unlock_oldest_frame(hcam) == 0 {
                    eprintln!("[ERROR] unlock_oldest_frame failed");
                }
            }

            // Consume callback notification (like driver does)
            callback_ctx_for_loop.consume_one();

            if frames <= 25 || frames % 50 == 0 {
                eprintln!("[FRAME {}] acquired (iter={})", frames, loop_iteration);
            }
        }

        frames
    }).await.unwrap();

    let total_time = loop_start.elapsed().as_millis();

    println!("\n[CLEANUP] Stopping acquisition...");
    unsafe {
        pl_exp_abort(hcam, CCS_HALT);
        dealloc(buffer, layout);
        pl_cam_deregister_callback(hcam, PL_CALLBACK_EOF);
        GLOBAL_CALLBACK_CTX.store(std::ptr::null_mut(), Ordering::Release);
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }

    println!("\n=== TEST 24 COMPLETE ===\n");
    println!("Frames acquired: {}/{}", frames_acquired, TARGET_FRAMES);
    println!("Total time: {}ms", total_time);

    if frames_acquired >= TARGET_FRAMES {
        println!("RESULT: Driver callback infrastructure is NOT the issue (200 frames achieved)");
    } else {
        println!("RESULT: Driver callback infrastructure MAY be causing the issue ({} frames)", frames_acquired);
    }

    assert!(
        frames_acquired >= TARGET_FRAMES,
        "Expected {} frames, got {}. Driver callback infrastructure may be the issue!",
        TARGET_FRAMES,
        frames_acquired
    );
}
